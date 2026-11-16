//! Space Scanner CLI (index + junk cleaner + rule manager).

use std::path::{Path, PathBuf};

mod docker_scenario;

use clap::{Parser, Subcommand};
use space_clean::{
    clean, coalesce_nested_paths, delete_candidates_permanently_with_progress, revalidate_path,
    scan_rules, trash_paths_batch, CleanProgress, DeleteCandidate, RuleFilter, RuleManager,
    RulePrefs, ScanOptions, TRASH_BATCH_SIZE,
};
use space_core::{
    export, format_bytes, scan_path_with_options, CacheStore, FindQuery, NodeId,
    ScanOptions as CoreScanOptions, SizeMode, SpaceTree,
};

#[derive(Parser)]
#[command(
    name = "space",
    version,
    about = "Space Scanner CLI (index + junk cleaner)"
)]
struct Cli {
    /// Rule pack directory (default: discovery; also honours SPACE_RULES_DIR)
    #[arg(long, global = true, value_name = "DIR")]
    rules: Option<PathBuf>,
    /// Index cache directory (default: per-user cache; also SPACE_CACHE_DIR)
    #[arg(long, global = true, value_name = "DIR")]
    cache_dir: Option<PathBuf>,
    /// Files smaller than this are aggregated per folder when scanning
    /// (bytes, or 10K / 5M; default 512K). Non-default values bypass the cache.
    #[arg(long, global = true, value_parser = parse_size, value_name = "SIZE")]
    small_file_threshold: Option<u64>,
    /// Scan worker threads (default: automatic)
    #[arg(long, global = true, value_name = "N")]
    threads: Option<usize>,
    /// Skip directories whose path or name matches (repeatable; bypasses cache)
    #[arg(long, global = true, value_name = "PATTERN")]
    exclude: Vec<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Guided diagnostic scenarios (report only)
    Scenario {
        #[command(subcommand)]
        cmd: ScenarioCmd,
    },
    /// Index a path and save to cache
    Scan { path: PathBuf },
    /// List children of a path from cache (or scan if missing)
    Ls {
        path: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        force: bool,
    },
    /// Show total size for a path from cache
    Du {
        path: PathBuf,
        #[arg(long)]
        apparent: bool,
        #[arg(long)]
        force: bool,
    },
    /// Search scanned tree by name/regex, extension, size, or age
    Find {
        path: PathBuf,
        #[arg(short = 'q', long)]
        query: Option<String>,
        #[arg(long)]
        regex: bool,
        #[arg(short = 'e', long = "ext", value_delimiter = ',')]
        exts: Vec<String>,
        #[arg(long, value_parser = parse_size)]
        min_size: Option<u64>,
        #[arg(long, value_parser = parse_size)]
        max_size: Option<u64>,
        #[arg(long)]
        dirs: bool,
        #[arg(long)]
        files: bool,
        #[arg(long)]
        min_age: Option<u64>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        apparent: bool,
        #[arg(long)]
        force: bool,
    },
    /// Show largest files or folders under a path
    Largest {
        path: PathBuf,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        dirs: bool,
        #[arg(long)]
        apparent: bool,
        #[arg(long)]
        force: bool,
    },
    /// Oldest large files (by modification time) under a path
    Oldest {
        path: PathBuf,
        /// Minimum size (bytes, or 10K / 5M / 1G)
        #[arg(long, value_parser = parse_size, default_value = "100M")]
        min_size: u64,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        apparent: bool,
        #[arg(long)]
        force: bool,
    },
    /// Likely duplicate files (same size and exact name)
    Dupes {
        path: PathBuf,
        /// Minimum file size (bytes, or 10K / 5M / 1G)
        #[arg(long, value_parser = parse_size, default_value = "1M")]
        min_size: u64,
        /// Maximum number of groups to print
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        apparent: bool,
        #[arg(long)]
        force: bool,
    },
    /// Totals grouped by file extension
    Exts {
        path: PathBuf,
        #[arg(long, default_value_t = 30)]
        limit: usize,
        #[arg(long)]
        apparent: bool,
        #[arg(long)]
        force: bool,
    },
    /// Export a scanned tree as json, json-full, csv, largest-csv, tsv, or svg
    Export {
        path: PathBuf,
        #[arg(long, default_value = "json")]
        format: String,
        #[arg(short = 'o', long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        apparent: bool,
        #[arg(long)]
        force: bool,
    },
    /// Inspect or clear on-disk scan cache
    Cache {
        #[command(subcommand)]
        cmd: CacheCmd,
    },
    /// Junk rule manager
    Rules {
        #[command(subcommand)]
        cmd: Option<RulesCmd>,
        /// Legacy: list default-enabled (or --all) when no subcommand
        #[arg(long, global = true)]
        all: bool,
    },
    /// List junk categories (effective counts with prefs)
    Categories {
        #[arg(long)]
        all: bool,
    },
    /// List junk tags (inferred + explicit)
    Tags {
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 80)]
        limit: usize,
    },
    /// List junk kinds (logs, temp, cache, …)
    Kinds {
        #[arg(long)]
        all: bool,
    },
    /// List deletion danger levels
    Risks {
        #[arg(long)]
        all: bool,
    },
    /// List hard safety rules for data that must never be deleted
    Protections {
        /// Include rules for every OS instead of this host only
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 80)]
        limit: usize,
    },
    /// Scan regenerable junk (caches, logs, temp, …)
    Junk {
        /// Categories or rule ids (comma-separated). Empty = all enabled.
        #[arg(short = 'c', long = "category", value_delimiter = ',')]
        categories: Vec<String>,
        /// Require tags (AND, comma-separated)
        #[arg(short = 't', long = "tag", value_delimiter = ',')]
        tags: Vec<String>,
        /// Junk kind: logs, temp, cache, browser, packages, …
        #[arg(short = 'k', long = "kind")]
        kind: Option<String>,
        /// Exact danger: low|medium|high|report-only (aliases: safe|caution|dangerous)
        #[arg(long = "risk")]
        risk: Option<String>,
        /// Include rules up to this danger (inclusive)
        #[arg(long = "max-risk")]
        max_risk: Option<String>,
        /// Ignore prefs / YAML defaults — scan every host rule
        #[arg(long)]
        all: bool,
        #[arg(long)]
        min_age: Option<u64>,
        /// Actually move findings to Trash (default is dry-run report)
        #[arg(long)]
        clean: bool,
        /// Permanently delete findings instead of moving to Trash (with --clean --yes)
        #[arg(long)]
        permanent: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Remove files or folders (moves to Trash unless --permanent is passed)
    Rm {
        /// Paths to remove
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Bypass Trash and permanently unlink from disk
        #[arg(long)]
        permanent: bool,
        /// Execute deletion (without --yes, prints a dry-run preview)
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ScenarioCmd {
    /// Review Docker volumes before cleanup; never deletes data
    DockerVolumes {
        /// Print the complete inventory as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum CacheCmd {
    /// List cached scan roots
    Ls,
    /// Remove cached entry for a path or wipe all with --all
    Clear {
        path: Option<PathBuf>,
        #[arg(long)]
        all: bool,
    },
    /// Print the cache directory path
    Dir,
}

#[derive(Subcommand)]
enum RulesCmd {
    /// List rules (search / filter)
    List {
        #[arg(short = 'q', long = "query", default_value = "")]
        query: String,
        #[arg(short = 'c', long = "category")]
        category: Option<String>,
        /// Require tags (AND, comma-separated)
        #[arg(short = 't', long = "tag", value_delimiter = ',')]
        tags: Vec<String>,
        /// Junk kind: logs, temp, cache, …
        #[arg(short = 'k', long = "kind")]
        kind: Option<String>,
        /// Exact danger: low|medium|high|report-only
        #[arg(long = "risk")]
        risk: Option<String>,
        /// Include rules up to this danger
        #[arg(long = "max-risk")]
        max_risk: Option<String>,
        #[arg(long)]
        enabled: bool,
        #[arg(long)]
        disabled: bool,
        #[arg(long)]
        overridden: bool,
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 80)]
        limit: usize,
    },
    /// Show manager stats
    Status,
    /// Force-enable rule id(s)
    Enable { ids: Vec<String> },
    /// Force-disable rule id(s)
    Disable { ids: Vec<String> },
    /// Clear per-rule override(s)
    Reset {
        /// Rule ids. Empty + --all → wipe prefs file.
        ids: Vec<String>,
        #[arg(long)]
        all: bool,
    },
    /// Force a whole category on/off/reset
    Category { action: CatAction, name: String },
    /// Force a tag on/off/reset
    Tag { action: CatAction, name: String },
}

#[derive(Clone, clap::ValueEnum)]
enum CatAction {
    On,
    Off,
    Reset,
}

fn main() {
    let cli = Cli::parse();
    let ctx = CliContext::from_cli(&cli);
    match cli.cmd {
        Cmd::Scenario {
            cmd: ScenarioCmd::DockerVolumes { json },
        } => {
            if let Err(error) = docker_scenario::run(json) {
                eprintln!("Docker volume scenario failed: {error}");
                std::process::exit(1);
            }
        }
        Cmd::Scan { path } => {
            let tree = scan_blocking(&path, &ctx.scan);
            let cache = ctx.open_cache();
            cache.save(&tree).expect("save");
            println!(
                "Indexed {} · {} items · cache saved",
                format_bytes(tree.total_size()),
                tree.item_count
            );
        }
        Cmd::Ls { path, limit, force } => {
            let tree = ctx.load_or_scan(&path, force);
            let mode = SizeMode::OnDisk;
            let kids = tree.children(tree.root);
            for (i, &id) in kids.iter().take(limit).enumerate() {
                let Some(n) = tree.try_node(id) else {
                    continue;
                };
                let mark = if n.is_dir { "/" } else { "" };
                println!(
                    "{:>2}. {:>10}  {}{}",
                    i + 1,
                    format_bytes(n.size(mode)),
                    n.name,
                    mark
                );
            }
        }
        Cmd::Du {
            path,
            apparent,
            force,
        } => {
            let tree = ctx.load_or_scan(&path, force);
            let mode = if apparent {
                SizeMode::Apparent
            } else {
                SizeMode::OnDisk
            };
            let root = NodeId(0);
            let n = tree.node(root);
            println!(
                "{}\t{}\t{} files\t{}",
                format_bytes(n.size(mode)),
                if apparent { "apparent" } else { "on-disk" },
                n.file_count,
                tree.root_path
            );
        }
        Cmd::Find {
            path,
            query,
            regex,
            exts,
            min_size,
            max_size,
            dirs,
            files,
            min_age,
            limit,
            apparent,
            force,
        } => {
            let tree = ctx.load_or_scan(&path, force);
            let mode = if apparent {
                SizeMode::Apparent
            } else {
                SizeMode::OnDisk
            };
            let q = FindQuery {
                name_pattern: query,
                use_regex: regex,
                exts,
                min_bytes: min_size,
                max_bytes: max_size,
                dirs_only: dirs && !files,
                files_only: files && !dirs,
                min_age_days: min_age,
                max_visit: 0,
            };
            let hits = match tree.find_nodes(tree.root, mode, &q, limit) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("invalid query: {e}");
                    std::process::exit(2);
                }
            };
            for id in hits {
                let Some(n) = tree.try_node(id) else {
                    continue;
                };
                println!(
                    "{:>10}  {}",
                    format_bytes(n.size(mode)),
                    tree.path_string(id)
                );
            }
        }
        Cmd::Largest {
            path,
            limit,
            dirs,
            apparent,
            force,
        } => {
            let tree = ctx.load_or_scan(&path, force);
            let mode = if apparent {
                SizeMode::Apparent
            } else {
                SizeMode::OnDisk
            };
            let hits = if dirs {
                tree.largest_dirs(tree.root, mode, limit)
            } else {
                tree.largest_files(tree.root, mode, limit)
            };
            for (i, id) in hits.into_iter().enumerate() {
                let Some(n) = tree.try_node(id) else {
                    continue;
                };
                println!(
                    "{:>2}. {:>10}  {}",
                    i + 1,
                    format_bytes(n.size(mode)),
                    tree.path_string(id)
                );
            }
        }
        Cmd::Oldest {
            path,
            min_size,
            limit,
            apparent,
            force,
        } => {
            let tree = ctx.load_or_scan(&path, force);
            let mode = if apparent {
                SizeMode::Apparent
            } else {
                SizeMode::OnDisk
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            for (i, id) in tree
                .oldest_large_files(tree.root, mode, min_size, limit)
                .into_iter()
                .enumerate()
            {
                let n = tree.node(id);
                println!(
                    "{:>3}. {:>10}  {:>9}  {}",
                    i + 1,
                    format_bytes(n.size(mode)),
                    age_label(now.saturating_sub(n.mtime_unix)),
                    tree.path_string(id)
                );
            }
        }
        Cmd::Dupes {
            path,
            min_size,
            limit,
            apparent,
            force,
        } => {
            let tree = ctx.load_or_scan(&path, force);
            let mode = if apparent {
                SizeMode::Apparent
            } else {
                SizeMode::OnDisk
            };
            let groups = tree.duplicate_name_size(tree.root, mode, min_size, usize::MAX);
            let mut wasted_total = 0u64;
            for (i, group) in groups.iter().enumerate() {
                let wasted = group.size.saturating_mul(group.nodes.len() as u64 - 1);
                wasted_total = wasted_total.saturating_add(wasted);
                if i >= limit {
                    continue;
                }
                println!(
                    "{:>3}. {} copies · {} each · {} reclaimable",
                    i + 1,
                    group.nodes.len(),
                    format_bytes(group.size),
                    format_bytes(wasted)
                );
                for &id in &group.nodes {
                    println!("       {}", tree.path_string(id));
                }
            }
            eprintln!(
                "{} group(s) · up to {} reclaimable (name+size match, verify before deleting)",
                groups.len(),
                format_bytes(wasted_total)
            );
        }
        Cmd::Exts {
            path,
            limit,
            apparent,
            force,
        } => {
            let tree = ctx.load_or_scan(&path, force);
            let mode = if apparent {
                SizeMode::Apparent
            } else {
                SizeMode::OnDisk
            };
            for (i, g) in tree
                .group_by_ext(tree.root, mode, limit)
                .into_iter()
                .enumerate()
            {
                let label = if g.ext.is_empty() {
                    "(none)"
                } else {
                    g.ext.as_str()
                };
                println!(
                    "{:>3}. {:>10}  {:>8} files  {}",
                    i + 1,
                    format_bytes(g.total_bytes),
                    g.file_count,
                    label
                );
            }
        }
        Cmd::Export {
            path,
            format,
            out,
            limit,
            apparent,
            force,
        } => {
            let tree = ctx.load_or_scan(&path, force);
            let mode = if apparent {
                SizeMode::Apparent
            } else {
                SizeMode::OnDisk
            };
            let body = match format.to_ascii_lowercase().as_str() {
                "csv" => export::children_csv(&tree, tree.root, mode, limit),
                "largest-csv" => export::largest_csv(&tree, tree.root, mode, limit),
                "tsv" => {
                    let cap = if limit == 0 { usize::MAX } else { limit };
                    let nodes: Vec<NodeId> =
                        tree.children(tree.root).iter().copied().take(cap).collect();
                    export::nodes_tsv(&tree, &nodes, mode)
                }
                "json" => export::json_shallow(&tree, tree.root, mode, limit),
                "json-full" => export::json_full(&tree).expect("serialize full tree"),
                "svg" => export::sunburst_svg(&tree, tree.root, mode, 3, 900),
                other => {
                    eprintln!(
                        "unknown format '{other}' (expected: json, json-full, csv, largest-csv, tsv, svg)"
                    );
                    std::process::exit(2);
                }
            };
            if let Some(dest) = out {
                std::fs::write(&dest, body).expect("write export file");
                println!("Wrote {}", dest.display());
            } else {
                print!("{body}");
            }
        }
        Cmd::Cache { cmd } => {
            let cache = ctx.open_cache();
            match cmd {
                CacheCmd::Dir => println!("{}", cache.dir().display()),
                CacheCmd::Ls => {
                    let items = cache.list_recent().expect("list cache");
                    if items.is_empty() {
                        println!("Cache is empty");
                    } else {
                        for m in items {
                            println!(
                                "{:>10}  {:>7} items  {}",
                                format_bytes(m.total_size),
                                m.item_count,
                                m.root_path
                            );
                        }
                    }
                }
                CacheCmd::Clear { path, all } => {
                    if all {
                        let count = cache.wipe_all().expect("wipe cache");
                        println!("Removed {count} cached entries");
                    } else if let Some(p) = path {
                        cache.invalidate(&p).expect("invalidate cache");
                        println!("Cleared cache for {}", p.display());
                    } else {
                        eprintln!("Specify a path or pass --all");
                        std::process::exit(2);
                    }
                }
            }
        }
        Cmd::Rules { cmd, all } => match cmd {
            None => {
                let mgr = ctx.load_rules();
                let filter = RuleFilter {
                    host_only: true,
                    enabled: if all { None } else { Some(true) },
                    ..Default::default()
                };
                for r in mgr.list(&filter) {
                    let flag = if r.effectively_enabled { "on " } else { "off" };
                    let mark = if r.overridden { "*" } else { " " };
                    let ro = if r.report_only { " report" } else { "" };
                    println!(
                        "[{flag}]{mark} {:>28}  {:<14}  {}{ro}",
                        r.id, r.category, r.name
                    );
                }
            }
            Some(RulesCmd::List {
                query,
                category,
                tags,
                kind,
                risk,
                max_risk,
                enabled,
                disabled,
                overridden,
                all,
                limit,
            }) => {
                let mgr = ctx.load_rules();
                let en = if enabled {
                    Some(true)
                } else if disabled {
                    Some(false)
                } else {
                    None
                };
                let filter = RuleFilter {
                    query,
                    category,
                    tags,
                    kind: parse_kind_opt(kind.as_deref()),
                    risk: parse_risk_opt(risk.as_deref()),
                    max_risk: parse_risk_opt(max_risk.as_deref()),
                    enabled: en,
                    overridden_only: overridden,
                    host_only: !all,
                    ..Default::default()
                };
                let rows = mgr.list(&filter);
                println!("{} matches (showing up to {limit})", rows.len());
                for r in rows.iter().take(limit) {
                    let flag = if r.effectively_enabled { "on " } else { "off" };
                    let mark = if r.overridden { "*" } else { " " };
                    println!(
                        "[{flag}]{mark} {:>28}  {:<10}  {:<8}  {:<10}  {}",
                        r.id, r.kind, r.danger, r.category, r.name
                    );
                }
            }
            Some(RulesCmd::Status) => {
                let mgr = ctx.load_rules();
                let s = mgr.stats();
                let mut win = 0usize;
                let mut linux = 0usize;
                let mut macos = 0usize;
                let mut any = 0usize;
                for r in &mgr.pack.rules {
                    match r.platform {
                        space_clean::Platform::Windows => win += 1,
                        space_clean::Platform::Linux => linux += 1,
                        space_clean::Platform::Macos => macos += 1,
                        space_clean::Platform::Any => any += 1,
                    }
                }
                println!("total\t{}", s.total);
                println!("host\t{}", s.host);
                println!("enabled\t{}", s.enabled);
                println!("disabled\t{}", s.disabled);
                println!("overridden\t{}", s.overridden);
                println!("categories\t{}", s.categories);
                println!("tags\t{}", s.tags);
                println!("kinds\t{}", s.kinds);
                println!("platform.macos\t{macos}");
                println!("platform.linux\t{linux}");
                println!("platform.windows\t{win}");
                println!("platform.any\t{any}");
                if let Some(p) = RulePrefs::default_path() {
                    println!("prefs\t{}", p.display());
                }
            }
            Some(RulesCmd::Enable { ids }) => {
                let mut mgr = ctx.load_rules();
                for id in &ids {
                    mgr.set_rule_enabled(id, true).unwrap_or_else(|e| {
                        eprintln!("{e}");
                    });
                }
                mgr.save_prefs()
                    .unwrap_or_else(|e| eprintln!("warn: save prefs: {e}"));
                println!("enabled {} rule(s)", ids.len());
            }
            Some(RulesCmd::Disable { ids }) => {
                let mut mgr = ctx.load_rules();
                for id in &ids {
                    mgr.set_rule_enabled(id, false).unwrap_or_else(|e| {
                        eprintln!("{e}");
                    });
                }
                mgr.save_prefs()
                    .unwrap_or_else(|e| eprintln!("warn: save prefs: {e}"));
                println!("disabled {} rule(s)", ids.len());
            }
            Some(RulesCmd::Reset { ids, all }) => {
                let mut mgr = ctx.load_rules();
                if all {
                    mgr.reset_all();
                    mgr.save_prefs()
                        .unwrap_or_else(|e| eprintln!("warn: save prefs: {e}"));
                    println!("prefs reset");
                } else if ids.is_empty() {
                    eprintln!("pass rule ids or --all");
                } else {
                    for id in &ids {
                        mgr.reset_rule(id);
                    }
                    mgr.save_prefs()
                        .unwrap_or_else(|e| eprintln!("warn: save prefs: {e}"));
                    println!("cleared {} override(s)", ids.len());
                }
            }
            Some(RulesCmd::Category { action, name }) => {
                let mut mgr = ctx.load_rules();
                match action {
                    CatAction::On => mgr.set_category_enabled(&name, true).expect("category"),
                    CatAction::Off => mgr.set_category_enabled(&name, false).expect("category"),
                    CatAction::Reset => mgr.reset_category(&name),
                }
                let label = match action {
                    CatAction::On => "on",
                    CatAction::Off => "off",
                    CatAction::Reset => "reset",
                };
                if let Err(e) = mgr.save_prefs() {
                    eprintln!("warn: save prefs: {e}");
                }
                println!("category {name} → {label}");
            }
            Some(RulesCmd::Tag { action, name }) => {
                let mut mgr = ctx.load_rules();
                match action {
                    CatAction::On => mgr.set_tag_enabled(&name, true).expect("tag"),
                    CatAction::Off => mgr.set_tag_enabled(&name, false).expect("tag"),
                    CatAction::Reset => mgr.reset_tag(&name),
                }
                let label = match action {
                    CatAction::On => "on",
                    CatAction::Off => "off",
                    CatAction::Reset => "reset",
                };
                if let Err(e) = mgr.save_prefs() {
                    eprintln!("warn: save prefs: {e}");
                }
                println!("tag {name} → {label}");
            }
        },
        Cmd::Categories { all } => {
            let mgr = ctx.load_rules();
            for cat in mgr.categories(true) {
                if !all && cat.enabled == 0 {
                    continue;
                }
                let pol = match cat.policy {
                    space_clean::CategoryPolicy::Default => "-",
                    space_clean::CategoryPolicy::ForceOn => "on",
                    space_clean::CategoryPolicy::ForceOff => "off",
                };
                println!("{}\t{}/{}\t{pol}", cat.name, cat.enabled, cat.total);
            }
        }
        Cmd::Tags { all, limit } => {
            let mgr = ctx.load_rules();
            let tags = mgr.tags(true);
            println!("{} tags (showing up to {limit})", tags.len());
            for tag in tags.iter().take(limit) {
                if !all && tag.enabled == 0 {
                    continue;
                }
                let pol = match tag.policy {
                    space_clean::CategoryPolicy::Default => "-",
                    space_clean::CategoryPolicy::ForceOn => "on",
                    space_clean::CategoryPolicy::ForceOff => "off",
                };
                println!("{}\t{}/{}\t{pol}", tag.name, tag.enabled, tag.total);
            }
        }
        Cmd::Kinds { all } => {
            let mgr = ctx.load_rules();
            for kind in mgr.kinds(true) {
                if !all && kind.enabled == 0 {
                    continue;
                }
                println!("{}\t{}/{}", kind.name, kind.enabled, kind.total);
            }
        }
        Cmd::Risks { all } => {
            let mgr = ctx.load_rules();
            for risk in mgr.risks(true) {
                if !all && risk.enabled == 0 {
                    continue;
                }
                println!(
                    "{}\t{}\t{}/{}",
                    risk.name, risk.danger, risk.enabled, risk.total
                );
            }
        }
        Cmd::Protections { all, limit } => {
            let dirs = ctx
                .rules
                .as_ref()
                .map(|d| vec![d.clone()])
                .unwrap_or_else(space_clean::default_rules_dirs);
            let pack = space_clean::ProtectionPack::load_many(&dirs).expect("protection rules");
            let rules: Vec<_> = pack
                .rules
                .iter()
                .filter(|r| all || r.platform.matches_host())
                .collect();
            println!("{} protection rules (showing up to {limit})", rules.len());
            for rule in rules.iter().take(limit) {
                println!(
                    "{:<44}  {:<8}  {}",
                    rule.id,
                    rule.platform.as_str(),
                    rule.name
                );
            }
        }
        Cmd::Junk {
            categories,
            tags,
            kind,
            risk,
            max_risk,
            all,
            min_age,
            clean: do_clean,
            permanent,
            yes,
        } => {
            let mgr = ctx.load_rules();
            let kind = parse_kind_opt(kind.as_deref());
            let risk = parse_risk_opt(risk.as_deref());
            let max_risk = parse_risk_opt(max_risk.as_deref());
            let opts = ScanOptions {
                categories: categories.clone(),
                include_disabled: all,
                min_age_override: min_age,
                prefs_min_age: mgr.prefs.min_age.clone(),
            };
            let report = if all {
                let rules: Vec<_> = mgr
                    .pack
                    .rules
                    .iter()
                    .filter(|r| r.platform.matches_host())
                    .filter(|r| {
                        if categories.is_empty() {
                            true
                        } else {
                            categories.iter().any(|c| c == &r.category || c == &r.id)
                        }
                    })
                    .filter(|r| kind.map(|k| r.kind == k).unwrap_or(true))
                    .filter(|r| risk.map(|rk| r.risk == rk).unwrap_or(true))
                    .filter(|r| {
                        max_risk
                            .map(|max| r.risk.rank() <= max.rank())
                            .unwrap_or(true)
                    })
                    .filter(|r| {
                        if tags.is_empty() {
                            true
                        } else {
                            tags.iter().all(|t| {
                                let n = space_clean::normalize_tag(t);
                                r.tags.iter().any(|rt| rt == &n)
                            })
                        }
                    })
                    .collect();
                scan_rules(rules, &opts).expect("junk scan")
            } else {
                let mut rules = mgr.scan_rules_filtered(&categories, &tags, kind, max_risk);
                if let Some(rk) = risk {
                    rules.retain(|r| r.risk == rk);
                }
                scan_rules(rules, &opts).expect("junk scan")
            };
            println!(
                "Junk scan · {} · {} findings · {} rules matched / {} scanned",
                format_bytes(report.total_bytes),
                report.findings.len(),
                report.rules_matched,
                report.rules_scanned
            );
            for f in report.findings.iter().take(40) {
                let tag = if f.report_only { "report" } else { "clean" };
                println!(
                    "  [{tag}] {:>10}  {}  ({})",
                    format_bytes(f.bytes),
                    f.path.display(),
                    f.rule_id
                );
            }
            if report.findings.len() > 40 {
                println!("… {} more", report.findings.len() - 40);
            }
            if do_clean {
                if !yes {
                    if permanent {
                        eprintln!("Dry-run only. Pass --yes to permanently delete findings.");
                    } else {
                        eprintln!("Dry-run only. Pass --yes to move deletable findings to Trash.");
                    }
                    return;
                }
                let result = clean(&report, permanent).expect("clean");
                if permanent {
                    println!(
                        "Permanently deleted {} items · freed {} · {} errors · {} report-only skipped",
                        result.deleted.len(),
                        format_bytes(result.freed_bytes),
                        result.errors.len(),
                        result.skipped_report_only
                    );
                } else {
                    println!(
                        "Moved {} items to Trash · {} · {} errors · {} report-only skipped",
                        result.deleted.len(),
                        format_bytes(result.processed_bytes),
                        result.errors.len(),
                        result.skipped_report_only
                    );
                }
                for e in result.errors.iter().take(10) {
                    eprintln!("  ! {e}");
                }
            }
        }
        Cmd::Rm {
            paths,
            permanent,
            yes,
        } => run_rm(paths, permanent, yes),
    }
}

#[derive(Clone)]
struct RmTarget {
    path: PathBuf,
    is_dir: bool,
    is_symlink: bool,
    bytes: u64,
}

fn run_rm(paths: Vec<PathBuf>, permanent: bool, yes: bool) {
    let mut raw_targets = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        if space_clean::safety::is_never_touch(&path) {
            errors.push(format!("Protected path refused: {}", path.display()));
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            errors.push(format!("Not found: {}", path.display()));
            continue;
        };
        let ft = meta.file_type();
        let is_symlink = ft.is_symlink();
        let is_dir = ft.is_dir();
        let bytes = if is_dir { 0 } else { meta.len() };
        raw_targets.push(RmTarget {
            path,
            is_dir,
            is_symlink,
            bytes,
        });
    }

    let targets = coalesce_nested_paths(raw_targets, |t| t.path.as_path());
    if !yes {
        for t in &targets {
            let kind = if t.is_symlink {
                "symlink"
            } else if t.is_dir {
                "dir"
            } else {
                "file"
            };
            println!("  [{kind}] {}", t.path.display());
        }
        let action = if permanent {
            "permanently deleted"
        } else {
            "moved to Trash"
        };
        eprintln!(
            "Dry-run only: {} target(s) would be {action}. Pass --yes to execute.",
            targets.len()
        );
        for e in errors.iter().take(10) {
            eprintln!("  ! {e}");
        }
        if !errors.is_empty() {
            std::process::exit(1);
        }
        return;
    }

    let mut deleted_items = 0usize;
    if permanent {
        let mut candidates = Vec::new();
        for t in targets {
            match revalidate_path(&t.path, t.is_dir, None, 0, true) {
                Ok(true) => candidates.push(DeleteCandidate {
                    path: t.path,
                    bytes: t.bytes,
                    is_dir: t.is_dir,
                }),
                Ok(false) => deleted_items += 1,
                Err(reason) => errors.push(format!("{}: {reason}", t.path.display())),
            }
        }
        let mut files_deleted = 0usize;
        let mut folders_deleted = 0usize;
        if !candidates.is_empty() {
            let outcomes = delete_candidates_permanently_with_progress(
                &candidates,
                CleanProgress {
                    items_total: candidates.len(),
                    ..Default::default()
                },
                |_| {},
            );
            for (cand, outcome) in candidates.into_iter().zip(outcomes) {
                match outcome {
                    Ok(stats) => {
                        deleted_items += 1;
                        files_deleted += stats.files_deleted;
                        folders_deleted += stats.folders_deleted;
                    }
                    Err(e) => errors.push(format!("{}: {e}", cand.path.display())),
                }
            }
        }
        println!(
            "Permanently deleted {} items ({} files, {} folders) · {} errors",
            deleted_items,
            files_deleted,
            folders_deleted,
            errors.len()
        );
    } else {
        for chunk in targets.chunks(TRASH_BATCH_SIZE) {
            let batch: Vec<PathBuf> = chunk.iter().map(|t| t.path.clone()).collect();
            for (path, outcome) in batch.iter().zip(trash_paths_batch(&batch)) {
                match outcome {
                    Ok(()) => deleted_items += 1,
                    Err(e) => errors.push(format!("{}: {e}", path.display())),
                }
            }
        }
        println!(
            "Moved {} items to Trash · {} errors",
            deleted_items,
            errors.len()
        );
    }

    for e in errors.iter().take(10) {
        eprintln!("  ! {e}");
    }
    if !errors.is_empty() {
        std::process::exit(1);
    }
}

struct CliContext {
    rules: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    scan: CoreScanOptions,
}

impl CliContext {
    fn from_cli(cli: &Cli) -> Self {
        let mut scan = CoreScanOptions {
            threads: cli.threads,
            exclude_names: cli.exclude.clone(),
            ..CoreScanOptions::default()
        };
        if let Some(bytes) = cli.small_file_threshold {
            scan.small_file_threshold = bytes;
        }
        Self {
            rules: cli.rules.clone(),
            cache_dir: cli.cache_dir.clone(),
            scan,
        }
    }

    fn load_rules(&self) -> RuleManager {
        if let Some(dir) = &self.rules {
            RuleManager::load_with_dirs(std::slice::from_ref(dir)).expect("rules")
        } else {
            RuleManager::load_default().expect("rules")
        }
    }

    fn open_cache(&self) -> CacheStore {
        if let Some(dir) = &self.cache_dir {
            CacheStore::with_dir(dir.clone()).expect("cache dir")
        } else {
            CacheStore::open().expect("cache dir")
        }
    }

    fn options_affect_tree(&self) -> bool {
        let defaults = CoreScanOptions::default();
        self.scan.small_file_threshold != defaults.small_file_threshold
            || !self.scan.exclude_names.is_empty()
    }

    fn load_or_scan(&self, path: &Path, force: bool) -> SpaceTree {
        if self.options_affect_tree() {
            return scan_blocking(path, &self.scan);
        }
        let cache = self.open_cache();
        if !force {
            if let Ok(loaded) = cache.load(path) {
                return loaded.tree;
            }
        }
        let tree = scan_blocking(path, &self.scan);
        let _ = cache.save(&tree);
        tree
    }
}

fn scan_blocking(path: &Path, options: &CoreScanOptions) -> SpaceTree {
    let result = scan_path_with_options(path.to_path_buf(), options.clone()).wait_with(|p| {
        eprint!(
            "\rscan {} files · {} · {}          ",
            p.files_seen,
            format_bytes(p.bytes_seen),
            p.current_path
        );
    });
    eprintln!();
    match result {
        Ok(tree) => tree,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn parse_size(raw: &str) -> Result<u64, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("size cannot be empty".into());
    }
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (digits, suffix) = (&s[..split], s[split..].trim());
    let value: u64 = digits
        .parse()
        .map_err(|_| format!("invalid size number: {raw}"))?;
    let unit = suffix.to_ascii_lowercase();
    let mult: u64 = match unit.as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024_u64.pow(2),
        "g" | "gb" | "gib" => 1024_u64.pow(3),
        "t" | "tb" | "tib" => 1024_u64.pow(4),
        _ => return Err(format!("unknown size suffix '{suffix}' in '{raw}'")),
    };
    value
        .checked_mul(mult)
        .ok_or_else(|| format!("size overflow: {raw}"))
}

fn age_label(age_secs: u64) -> String {
    match age_secs {
        0..=59 => "just now".into(),
        60..=3_599 => format!("{}m ago", age_secs / 60),
        3_600..=86_399 => format!("{}h ago", age_secs / 3_600),
        86_400..=31_535_999 => format!("{}d ago", age_secs / 86_400),
        _ => format!("{}y ago", age_secs / 31_536_000),
    }
}

fn parse_kind_opt(raw: Option<&str>) -> Option<space_clean::JunkKind> {
    raw.map(|s| {
        space_clean::JunkKind::parse(s).unwrap_or_else(|| {
            eprintln!("unknown kind '{s}' (try: cache logs temp downloads packages browser …)");
            std::process::exit(2);
        })
    })
}

fn parse_risk_opt(raw: Option<&str>) -> Option<space_clean::Risk> {
    raw.map(|s| {
        space_clean::Risk::parse(s).unwrap_or_else(|| {
            eprintln!(
                "unknown risk '{s}' (try: low medium high report-only / safe caution dangerous)"
            );
            std::process::exit(2);
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn size_parser_accepts_units() {
        assert_eq!(parse_size("42"), Ok(42));
        assert_eq!(parse_size("10K"), Ok(10 * 1024));
        assert_eq!(parse_size("5MiB"), Ok(5 * 1024 * 1024));
        assert_eq!(parse_size("2g"), Ok(2 * 1024 * 1024 * 1024));
        assert!(parse_size("").is_err());
        assert!(parse_size("12x").is_err());
    }

    #[test]
    fn global_scan_flags_build_options() {
        let cli = Cli::try_parse_from([
            "space",
            "--small-file-threshold",
            "1K",
            "--threads",
            "2",
            "--exclude",
            "node_modules",
            "du",
            ".",
        ])
        .unwrap();
        let ctx = CliContext::from_cli(&cli);
        assert_eq!(ctx.scan.small_file_threshold, 1024);
        assert_eq!(ctx.scan.threads, Some(2));
        assert_eq!(ctx.scan.exclude_names, vec!["node_modules".to_string()]);
        assert!(ctx.options_affect_tree());

        let plain = Cli::try_parse_from(["space", "--threads", "4", "du", "."]).unwrap();
        let plain_ctx = CliContext::from_cli(&plain);
        assert!(!plain_ctx.options_affect_tree());
        assert_eq!(plain_ctx.scan.threads, Some(4));
    }
}
