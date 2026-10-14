//! Space Scanner CLI (index + junk cleaner + rule manager).
//!
//! Exit codes: `0` success, `1` failure, `2` usage error, `3` partial clean
//! (some findings could not be moved to Trash).

use std::fmt;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod docker_scenario;

use anyhow::anyhow;
use clap::{Args, Parser, Subcommand, ValueEnum};
use space_clean::{
    clean, scan_rules_with_progress, JunkKind, Risk, RuleFilter, RuleManager, RulePrefs,
    ScanOptions,
};
use space_core::{
    format_bytes, largest_entries, scan_path, CacheStore, NodeId, SizeMode, SpaceTree,
};

pub const EXIT_FAILURE: u8 = 1;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_PARTIAL: u8 = 3;

#[derive(Parser)]
#[command(name = "space", version, about = "Space Scanner CLI (index + junk cleaner)")]
struct Cli {
    /// Rule pack directory (default: discovery; also honours SPACE_RULES_DIR)
    #[arg(long, global = true, env = "SPACE_RULES_DIR", value_name = "DIR")]
    rules: Option<PathBuf>,
    /// Index cache directory (default: per-user cache; also SPACE_CACHE_DIR)
    #[arg(long, global = true, env = "SPACE_CACHE_DIR", value_name = "DIR")]
    cache_dir: Option<PathBuf>,
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
        /// Show apparent (logical) sizes instead of on-disk
        #[arg(long)]
        apparent: bool,
    },
    /// Show total size for a path from cache
    Du {
        path: PathBuf,
        #[arg(long)]
        apparent: bool,
    },
    /// Largest entries anywhere under a path (files under 512 KiB are
    /// aggregated by the indexer and never listed individually)
    Largest {
        path: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Directories only
        #[arg(long, conflicts_with = "files")]
        dirs: bool,
        /// Files only
        #[arg(long)]
        files: bool,
        #[arg(long)]
        apparent: bool,
    },
    /// Find entries by name (substring, glob or regex) and size; files under
    /// 512 KiB are aggregated by the indexer and cannot be matched
    Find {
        path: PathBuf,
        /// Substring (case-insensitive) or glob (`*`, `?`, `[..]`); with a `/`
        /// the glob matches the full path
        #[arg(long)]
        name: Option<String>,
        /// Treat --name as a regular expression
        #[arg(long, requires = "name")]
        regex: bool,
        /// Minimum size (bytes, or 10K / 5M / 1G)
        #[arg(long, value_parser = parse_size)]
        min_size: Option<u64>,
        /// Maximum size (bytes, or 10K / 5M / 1G)
        #[arg(long, value_parser = parse_size)]
        max_size: Option<u64>,
        #[arg(long, conflicts_with = "files_only")]
        dirs_only: bool,
        #[arg(long)]
        files_only: bool,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        apparent: bool,
    },
    /// Export an index as JSON or CSV (streamed)
    Export {
        format: ExportFormat,
        path: PathBuf,
        /// Levels below the root to include (default: all)
        #[arg(long)]
        depth: Option<usize>,
        /// Write to a file instead of stdout
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        #[arg(long)]
        apparent: bool,
    },
    /// Manage the index cache
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
        /// Junk kind
        #[arg(short = 'k', long = "kind")]
        kind: Option<KindArg>,
        /// Exact danger level
        #[arg(long = "risk")]
        risk: Option<RiskArg>,
        /// Include rules up to this danger (inclusive)
        #[arg(long = "max-risk")]
        max_risk: Option<RiskArg>,
        /// Ignore prefs / YAML defaults (explicit per-rule offs are kept;
        /// risk is capped at low unless --max-risk is given)
        #[arg(long)]
        all: bool,
        #[arg(long)]
        min_age: Option<u64>,
        /// Actually move findings to Trash (default is dry-run report)
        #[arg(long)]
        clean: bool,
        #[arg(long)]
        yes: bool,
        /// Findings to print (text output)
        #[arg(long, default_value_t = 40)]
        limit: usize,
        /// Print the full report as JSON
        #[arg(long)]
        json: bool,
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
    /// List cached indexes
    Ls,
    /// Remove the cached index for one path
    Rm { path: PathBuf },
    /// Delete every cached index
    Wipe,
    /// Remove indexes scanned more than N days ago
    Prune {
        #[arg(long, value_name = "N")]
        ttl_days: u64,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ExportFormat {
    Json,
    Csv,
}

#[derive(Subcommand)]
enum RulesCmd {
    /// List rules (search / filter)
    List(ListArgs),
    /// Show manager stats
    Status,
    /// Force-enable rule id(s)
    Enable {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Force-disable rule id(s)
    Disable {
        #[arg(required = true)]
        ids: Vec<String>,
    },
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

#[derive(Args)]
struct ListArgs {
    #[arg(short = 'q', long = "query", default_value = "")]
    query: String,
    #[arg(short = 'c', long = "category")]
    category: Option<String>,
    /// Require tags (AND, comma-separated)
    #[arg(short = 't', long = "tag", value_delimiter = ',')]
    tags: Vec<String>,
    /// Junk kind
    #[arg(short = 'k', long = "kind")]
    kind: Option<KindArg>,
    /// Exact danger level
    #[arg(long = "risk")]
    risk: Option<RiskArg>,
    /// Include rules up to this danger
    #[arg(long = "max-risk")]
    max_risk: Option<RiskArg>,
    #[arg(long, conflicts_with = "disabled")]
    enabled: bool,
    #[arg(long)]
    disabled: bool,
    #[arg(long)]
    overridden: bool,
    #[arg(long)]
    all: bool,
    #[arg(long, default_value_t = 80)]
    limit: usize,
}

#[derive(Clone, Copy, ValueEnum)]
enum CatAction {
    On,
    Off,
    Reset,
}

impl fmt::Display for CatAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CatAction::On => "on",
            CatAction::Off => "off",
            CatAction::Reset => "reset",
        })
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    Cache,
    Logs,
    Temp,
    Downloads,
    Leftovers,
    Packages,
    Browser,
    System,
    AppData,
    Media,
    Databases,
    Models,
    Vm,
    Dev,
    Other,
}

impl From<KindArg> for JunkKind {
    fn from(kind: KindArg) -> Self {
        match kind {
            KindArg::Cache => JunkKind::Cache,
            KindArg::Logs => JunkKind::Logs,
            KindArg::Temp => JunkKind::Temp,
            KindArg::Downloads => JunkKind::Downloads,
            KindArg::Leftovers => JunkKind::Leftovers,
            KindArg::Packages => JunkKind::Packages,
            KindArg::Browser => JunkKind::Browser,
            KindArg::System => JunkKind::System,
            KindArg::AppData => JunkKind::AppData,
            KindArg::Media => JunkKind::Media,
            KindArg::Databases => JunkKind::Databases,
            KindArg::Models => JunkKind::Models,
            KindArg::Vm => JunkKind::Vm,
            KindArg::Dev => JunkKind::Dev,
            KindArg::Other => JunkKind::Other,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum RiskArg {
    #[value(alias = "safe")]
    Low,
    #[value(alias = "caution")]
    Medium,
    #[value(alias = "dangerous")]
    High,
    #[value(alias = "report")]
    ReportOnly,
}

impl From<RiskArg> for Risk {
    fn from(risk: RiskArg) -> Self {
        match risk {
            RiskArg::Low => Risk::Low,
            RiskArg::Medium => Risk::Medium,
            RiskArg::High => Risk::High,
            RiskArg::ReportOnly => Risk::ReportOnly,
        }
    }
}

/// Error carrying the process exit code.
#[derive(Debug)]
struct Exit {
    code: u8,
    message: String,
}

impl<E: Into<anyhow::Error>> From<E> for Exit {
    fn from(error: E) -> Self {
        Exit {
            code: EXIT_FAILURE,
            message: format!("{:#}", error.into()),
        }
    }
}

fn usage(message: impl Into<String>) -> Exit {
    Exit {
        code: EXIT_USAGE,
        message: message.into(),
    }
}

fn partial(message: impl Into<String>) -> Exit {
    Exit {
        code: EXIT_PARTIAL,
        message: message.into(),
    }
}

type CliResult<T = ()> = Result<T, Exit>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(exit) => {
            if !exit.message.is_empty() {
                eprintln!("error: {}", exit.message);
            }
            ExitCode::from(exit.code)
        }
    }
}

fn run(cli: Cli) -> CliResult {
    let ctx = Context {
        rules: cli.rules,
        cache_dir: cli.cache_dir,
    };
    match cli.cmd {
        Cmd::Scenario {
            cmd: ScenarioCmd::DockerVolumes { json },
        } => docker_scenario::run(json).map_err(|e| anyhow!("Docker volume scenario: {e}"))?,
        Cmd::Scan { path } => {
            let tree = scan_blocking(&path)?;
            ctx.cache()?.save(&tree)?;
            println!(
                "Indexed {} · {} items · cache saved",
                format_bytes(tree.total_size()),
                tree.item_count
            );
        }
        Cmd::Ls {
            path,
            limit,
            apparent,
        } => {
            let tree = ctx.load_or_scan(&path)?;
            let mode = size_mode(apparent);
            for (i, &id) in tree.children(tree.root).iter().take(limit).enumerate() {
                let n = tree.node(id);
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
        Cmd::Du { path, apparent } => {
            let tree = ctx.load_or_scan(&path)?;
            let n = tree.node(tree.root);
            println!(
                "{}\t{}\t{} files\t{}",
                format_bytes(n.size(size_mode(apparent))),
                if apparent { "apparent" } else { "on-disk" },
                n.file_count,
                tree.root_path
            );
        }
        Cmd::Largest {
            path,
            limit,
            dirs,
            files,
            apparent,
        } => {
            let tree = ctx.load_or_scan(&path)?;
            let mode = size_mode(apparent);
            let (want_dirs, want_files) = if dirs || files {
                (dirs, files)
            } else {
                (true, true)
            };
            for (i, id) in largest_entries(&tree, mode, limit, want_dirs, want_files)
                .into_iter()
                .enumerate()
            {
                let n = tree.node(id);
                let mark = if n.is_dir { "/" } else { "" };
                println!(
                    "{:>3}. {:>10}  {}{}",
                    i + 1,
                    format_bytes(n.size(mode)),
                    tree.path_string(id),
                    mark
                );
            }
        }
        Cmd::Find {
            path,
            name,
            regex,
            min_size,
            max_size,
            dirs_only,
            files_only,
            limit,
            apparent,
        } => {
            let matcher = NameMatcher::new(name.as_deref(), regex)?;
            let tree = ctx.load_or_scan(&path)?;
            let mode = size_mode(apparent);
            let mut shown = 0usize;
            let mut total = 0usize;
            for (i, n) in tree.nodes.iter().enumerate() {
                let id = NodeId(i as u32);
                if id == tree.root || n.synthetic {
                    continue;
                }
                if (dirs_only && !n.is_dir) || (files_only && n.is_dir) {
                    continue;
                }
                let size = n.size(mode);
                if min_size.is_some_and(|min| size < min) || max_size.is_some_and(|max| size > max)
                {
                    continue;
                }
                if !matcher.matches(&n.name, || tree.path_string(id)) {
                    continue;
                }
                total += 1;
                if shown < limit {
                    shown += 1;
                    let mark = if n.is_dir { "/" } else { "" };
                    println!(
                        "{:>10}  {}{}",
                        format_bytes(size),
                        tree.path_string(id),
                        mark
                    );
                }
            }
            if total > shown {
                eprintln!("… {} more match(es) (raise --limit)", total - shown);
            }
        }
        Cmd::Export {
            format,
            path,
            depth,
            out,
            apparent,
        } => {
            let tree = ctx.load_or_scan(&path)?;
            let mode = size_mode(apparent);
            let sink: Box<dyn Write> = match &out {
                Some(file) => Box::new(std::fs::File::create(file)?),
                None => Box::new(std::io::stdout().lock()),
            };
            let mut writer = BufWriter::new(sink);
            match format {
                ExportFormat::Json => export_json(&tree, mode, depth, &mut writer)?,
                ExportFormat::Csv => export_csv(&tree, mode, depth, &mut writer)?,
            }
            writer.flush()?;
            if let Some(file) = out {
                eprintln!("wrote {}", file.display());
            }
        }
        Cmd::Cache { cmd } => {
            let cache = ctx.cache()?;
            match cmd {
                CacheCmd::Ls => {
                    let entries = cache.list_recent()?;
                    for meta in &entries {
                        println!(
                            "{:>10}  {:>9} items  scanned {}  {}",
                            format_bytes(meta.total_size),
                            meta.item_count,
                            age_label(meta.scanned_at_unix),
                            meta.root_path
                        );
                    }
                    println!(
                        "{} cached index(es) · {} on disk · {}",
                        entries.len(),
                        format_bytes(cache.total_bytes()?),
                        cache.dir().display()
                    );
                }
                CacheCmd::Rm { path } => {
                    if cache.peek_meta(&path).is_none() {
                        return Err(anyhow!("no cached index for {}", path.display()).into());
                    }
                    cache.invalidate(&path)?;
                    println!("removed cached index for {}", path.display());
                }
                CacheCmd::Wipe => {
                    let (files, bytes) = cache.wipe_all()?;
                    println!("wiped {files} file(s) · {}", format_bytes(bytes));
                }
                CacheCmd::Prune { ttl_days } => {
                    if ttl_days == 0 {
                        return Err(usage("--ttl-days must be at least 1"));
                    }
                    let (entries, bytes) = cache.with_ttl_days(ttl_days).prune_expired()?;
                    println!("pruned {entries} index(es) · {}", format_bytes(bytes));
                }
            }
        }
        Cmd::Rules { cmd, all } => rules_cmd(&ctx, cmd, all)?,
        Cmd::Categories { all } => {
            let mgr = ctx.rules()?;
            for cat in mgr.categories(true) {
                if !all && cat.enabled == 0 {
                    continue;
                }
                println!(
                    "{}\t{}/{}\t{}",
                    cat.name,
                    cat.enabled,
                    cat.total,
                    policy_label(cat.policy)
                );
            }
        }
        Cmd::Tags { all, limit } => {
            let mgr = ctx.rules()?;
            let tags: Vec<_> = mgr
                .tags(true)
                .into_iter()
                .filter(|tag| all || tag.enabled > 0)
                .collect();
            println!("{} tags (showing up to {limit})", tags.len());
            for tag in tags.iter().take(limit) {
                println!(
                    "{}\t{}/{}\t{}",
                    tag.name,
                    tag.enabled,
                    tag.total,
                    policy_label(tag.policy)
                );
            }
        }
        Cmd::Kinds { all } => {
            let mgr = ctx.rules()?;
            for kind in mgr.kinds(true) {
                if !all && kind.enabled == 0 {
                    continue;
                }
                println!("{}\t{}/{}", kind.name, kind.enabled, kind.total);
            }
        }
        Cmd::Risks { all } => {
            let mgr = ctx.rules()?;
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
            let pack = space_clean::ProtectionPack::load_many(&ctx.rules_dirs()?)?;
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
            yes,
            limit,
            json,
        } => {
            let mgr = ctx.rules()?;
            let kind = kind.map(JunkKind::from);
            let risk = risk.map(Risk::from);
            let max_risk = max_risk.map(Risk::from);
            let opts = ScanOptions {
                categories: categories.clone(),
                include_disabled: all,
                min_age_override: min_age,
                prefs_min_age: mgr.prefs.min_age.clone(),
            };
            let rules = if all {
                mgr.scan_rules_unrestricted(&categories, &tags, kind, risk, max_risk)
            } else {
                let mut rules = mgr.scan_rules_filtered(&categories, &tags, kind, max_risk);
                if let Some(rk) = risk {
                    rules.retain(|r| r.risk == rk);
                }
                rules
            };
            let mut progress_shown = false;
            let report = scan_rules_with_progress(rules, &opts, |p| {
                progress_shown = true;
                eprint!(
                    "\rjunk {}/{} rules · {} entries · {} found · {}          ",
                    p.rules_done,
                    p.rules_total,
                    p.entries_seen,
                    p.findings_found,
                    format_bytes(p.bytes_found)
                );
            })?;
            if progress_shown {
                eprintln!();
            }
            if json {
                let mut out = BufWriter::new(std::io::stdout().lock());
                serde_json::to_writer_pretty(&mut out, &report)?;
                out.write_all(b"\n")?;
                out.flush()?;
            } else {
                println!(
                    "Junk scan · {} · {} findings · {} rules matched / {} scanned",
                    format_bytes(report.total_bytes),
                    report.findings.len(),
                    report.rules_matched,
                    report.rules_scanned
                );
                for f in report.findings.iter().take(limit) {
                    let tag = if f.report_only { "report" } else { "clean" };
                    println!(
                        "  [{tag}] {:>10}  {}  ({})",
                        format_bytes(f.bytes),
                        f.path.display(),
                        f.rule_id
                    );
                }
                if report.findings.len() > limit {
                    println!("… {} more", report.findings.len() - limit);
                }
                for root in report.skipped_roots.iter().take(10) {
                    eprintln!("  skipped: {root}");
                }
            }
            if do_clean {
                if !yes {
                    eprintln!("Dry-run only. Pass --yes to move deletable findings to Trash.");
                    return Ok(());
                }
                let result = clean(&report, false)?;
                println!(
                    "Moved {} items to Trash · {} · {} errors · {} report-only skipped",
                    result.deleted.len(),
                    format_bytes(result.processed_bytes),
                    result.errors.len(),
                    result.skipped_report_only
                );
                for e in result.errors.iter().take(10) {
                    eprintln!("  ! {e}");
                }
                if !result.errors.is_empty() {
                    return Err(partial(format!(
                        "{} of {} findings could not be cleaned",
                        result.errors.len(),
                        result.deleted.len() + result.errors.len()
                    )));
                }
            }
        }
    }
    Ok(())
}

fn rules_cmd(ctx: &Context, cmd: Option<RulesCmd>, all: bool) -> CliResult {
    match cmd {
        None => {
            let mgr = ctx.rules()?;
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
        Some(RulesCmd::List(args)) => {
            let mgr = ctx.rules()?;
            let filter = RuleFilter {
                query: args.query,
                category: args.category,
                tags: args.tags,
                kind: args.kind.map(JunkKind::from),
                risk: args.risk.map(Risk::from),
                max_risk: args.max_risk.map(Risk::from),
                enabled: if args.enabled {
                    Some(true)
                } else if args.disabled {
                    Some(false)
                } else {
                    None
                },
                overridden_only: args.overridden,
                host_only: !args.all,
                ..Default::default()
            };
            let (total, rows) = mgr.list_page(&filter, args.limit);
            println!("{total} matches (showing up to {})", args.limit);
            for r in &rows {
                let flag = if r.effectively_enabled { "on " } else { "off" };
                let mark = if r.overridden { "*" } else { " " };
                println!(
                    "[{flag}]{mark} {:>28}  {:<10}  {:<8}  {:<10}  {}",
                    r.id, r.kind, r.danger, r.category, r.name
                );
            }
        }
        Some(RulesCmd::Status) => {
            let mgr = ctx.rules()?;
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
            for dir in ctx.rules_dirs()? {
                println!("rules\t{}", dir.display());
            }
            if let Some(p) = RulePrefs::default_path() {
                println!("prefs\t{}", p.display());
            }
        }
        Some(RulesCmd::Enable { ids }) => set_rules(ctx, &ids, true)?,
        Some(RulesCmd::Disable { ids }) => set_rules(ctx, &ids, false)?,
        Some(RulesCmd::Reset { ids, all }) => {
            if all && !ids.is_empty() {
                return Err(usage("pass rule ids or --all, not both"));
            }
            if !all && ids.is_empty() {
                return Err(usage("pass rule ids or --all"));
            }
            let mut mgr = ctx.rules()?;
            if all {
                mgr.reset_all();
                mgr.save_prefs()?;
                println!("prefs reset");
            } else {
                let mut cleared = 0usize;
                let mut unknown = Vec::new();
                for id in &ids {
                    if mgr.pack.get(id).is_some() {
                        mgr.reset_rule(id);
                        cleared += 1;
                    } else {
                        unknown.push(id.as_str());
                    }
                }
                if cleared > 0 {
                    mgr.save_prefs()?;
                }
                println!("cleared {cleared} override(s)");
                if !unknown.is_empty() {
                    return Err(anyhow!("unknown rule id(s): {}", unknown.join(", ")).into());
                }
            }
        }
        Some(RulesCmd::Category { action, name }) => {
            let mut mgr = ctx.rules()?;
            match action {
                CatAction::On => mgr.set_category_enabled(&name, true)?,
                CatAction::Off => mgr.set_category_enabled(&name, false)?,
                CatAction::Reset => mgr.reset_category(&name),
            }
            mgr.save_prefs()?;
            println!("category {name} → {action}");
        }
        Some(RulesCmd::Tag { action, name }) => {
            let mut mgr = ctx.rules()?;
            match action {
                CatAction::On => mgr.set_tag_enabled(&name, true)?,
                CatAction::Off => mgr.set_tag_enabled(&name, false)?,
                CatAction::Reset => mgr.reset_tag(&name),
            }
            mgr.save_prefs()?;
            println!("tag {name} → {action}");
        }
    }
    Ok(())
}

fn set_rules(ctx: &Context, ids: &[String], enabled: bool) -> CliResult {
    let mut mgr = ctx.rules()?;
    let mut changed = 0usize;
    let mut failed = Vec::new();
    for id in ids {
        match mgr.set_rule_enabled(id, enabled) {
            Ok(()) => changed += 1,
            Err(e) => failed.push(format!("{id}: {e:#}")),
        }
    }
    if changed > 0 {
        mgr.save_prefs()?;
    }
    let verb = if enabled { "enabled" } else { "disabled" };
    println!("{verb} {changed} rule(s)");
    if !failed.is_empty() {
        return Err(anyhow!("{} rule(s) not {verb}: {}", failed.len(), failed.join("; ")).into());
    }
    Ok(())
}

/// Global options resolved once per invocation.
struct Context {
    rules: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
}

impl Context {
    fn cache(&self) -> CliResult<CacheStore> {
        Ok(match &self.cache_dir {
            Some(dir) => CacheStore::with_dir(dir)?,
            None => CacheStore::open()?,
        })
    }

    fn rules_dirs(&self) -> CliResult<Vec<PathBuf>> {
        let dirs = match &self.rules {
            Some(dir) => {
                if !dir.is_dir() {
                    return Err(usage(format!(
                        "rules directory not found: {}",
                        dir.display()
                    )));
                }
                space_clean::rules_dirs_from([dir])
            }
            None => space_clean::default_rules_dirs(),
        };
        if dirs.is_empty() {
            return Err(anyhow!(
                "no rules directory found; pass --rules <dir> or set {}",
                space_clean::RULES_DIR_ENV
            )
            .into());
        }
        Ok(dirs)
    }

    fn rules(&self) -> CliResult<RuleManager> {
        Ok(RuleManager::load_with_dirs(&self.rules_dirs()?)?)
    }

    fn load_or_scan(&self, path: &Path) -> CliResult<SpaceTree> {
        if !path.exists() {
            return Err(usage(format!("path not found: {}", path.display())));
        }
        let cache = self.cache()?;
        if let Ok(loaded) = cache.load(path) {
            if !loaded.stale {
                return Ok(loaded.tree);
            }
        }
        let tree = scan_blocking(path)?;
        if let Err(e) = cache.save(&tree) {
            eprintln!("warn: cache save: {e}");
        }
        Ok(tree)
    }
}

fn scan_blocking(path: &Path) -> CliResult<SpaceTree> {
    if !path.is_dir() {
        return Err(usage(format!("not a directory: {}", path.display())));
    }
    let mut shown = false;
    let tree = scan_path(path)
        .wait_with(|p| {
            shown = true;
            eprint!(
                "\rscan {} files · {} · {}          ",
                p.files_seen,
                format_bytes(p.bytes_seen),
                p.current_path
            );
        })
        .map_err(|e| anyhow!("scan {}: {e}", path.display()))?;
    if shown {
        eprintln!();
    }
    Ok(tree)
}

fn size_mode(apparent: bool) -> SizeMode {
    if apparent {
        SizeMode::Apparent
    } else {
        SizeMode::OnDisk
    }
}

fn policy_label(policy: space_clean::CategoryPolicy) -> &'static str {
    match policy {
        space_clean::CategoryPolicy::Default => "-",
        space_clean::CategoryPolicy::ForceOn => "on",
        space_clean::CategoryPolicy::ForceOff => "off",
    }
}

fn age_label(scanned_at_unix: u64) -> String {
    let age = space_core::now_unix().saturating_sub(scanned_at_unix);
    if age < 60 {
        "just now".into()
    } else if age < 3600 {
        format!("{}m ago", age / 60)
    } else if age < 86_400 {
        format!("{}h ago", age / 3600)
    } else {
        format!("{}d ago", age / 86_400)
    }
}

/// Parse `123`, `10K`, `5M`, `1G`, `2T` (decimal, optional `B`/`iB` suffix).
fn parse_size(raw: &str) -> Result<u64, String> {
    let text = raw.trim();
    let digits_end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(digits_end);
    let value: f64 = number
        .parse()
        .map_err(|_| format!("invalid size '{raw}'"))?;
    let unit = unit.trim().to_ascii_lowercase();
    let unit = unit.trim_end_matches("ib").trim_end_matches('b');
    let factor: f64 = match unit {
        "" => 1.0,
        "k" => 1024.0,
        "m" => 1024.0 * 1024.0,
        "g" => 1024.0 * 1024.0 * 1024.0,
        "t" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return Err(format!("invalid size unit in '{raw}' (use K, M, G, T)")),
    };
    Ok((value * factor).round() as u64)
}

enum NameMatcher {
    Any,
    Substring(String),
    Glob {
        matcher: globset::GlobMatcher,
        full_path: bool,
    },
    Regex(regex::Regex),
}

impl NameMatcher {
    fn new(pattern: Option<&str>, regex: bool) -> CliResult<Self> {
        let Some(pattern) = pattern else {
            return Ok(NameMatcher::Any);
        };
        if regex {
            return Ok(NameMatcher::Regex(
                regex::Regex::new(pattern).map_err(|e| usage(format!("invalid regex: {e}")))?,
            ));
        }
        if pattern.contains(['*', '?', '[']) {
            let matcher = globset::GlobBuilder::new(pattern)
                .case_insensitive(true)
                .literal_separator(false)
                .build()
                .map_err(|e| usage(format!("invalid glob: {e}")))?
                .compile_matcher();
            return Ok(NameMatcher::Glob {
                matcher,
                full_path: pattern.contains('/'),
            });
        }
        Ok(NameMatcher::Substring(pattern.to_lowercase()))
    }

    fn matches(&self, name: &str, full_path: impl FnOnce() -> String) -> bool {
        match self {
            NameMatcher::Any => true,
            NameMatcher::Substring(needle) => name.to_lowercase().contains(needle.as_str()),
            NameMatcher::Glob {
                matcher,
                full_path: true,
            } => matcher.is_match(full_path()),
            NameMatcher::Glob { matcher, .. } => matcher.is_match(name),
            NameMatcher::Regex(re) => re.is_match(name),
        }
    }
}

/// Depth-first walk of real (non-synthetic) nodes up to `depth` levels below
/// the root; `None` = unlimited. The root is level 0.
fn walk_nodes(
    tree: &SpaceTree,
    depth: Option<usize>,
    mut visit: impl FnMut(NodeId, usize) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut stack = vec![(tree.root, 0usize)];
    while let Some((id, level)) = stack.pop() {
        let node = tree.node(id);
        if node.synthetic {
            continue;
        }
        visit(id, level)?;
        if depth.is_none_or(|max| level < max) {
            for &child in node.children.iter().rev() {
                stack.push((child, level + 1));
            }
        }
    }
    Ok(())
}

fn export_json(
    tree: &SpaceTree,
    mode: SizeMode,
    depth: Option<usize>,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let _ = mode;
    out.write_all(b"[")?;
    let mut first = true;
    walk_nodes(tree, depth, |id, level| {
        let node = tree.node(id);
        if !first {
            out.write_all(b",")?;
        }
        first = false;
        out.write_all(b"\n")?;
        let row = serde_json::json!({
            "path": tree.path_string(id),
            "depth": level,
            "size_disk": node.allocated_size,
            "size_apparent": node.apparent_size,
            "files": node.file_count,
            "is_dir": node.is_dir,
        });
        serde_json::to_writer(&mut *out, &row).map_err(std::io::Error::other)
    })?;
    out.write_all(b"\n]\n")
}

fn export_csv(
    tree: &SpaceTree,
    mode: SizeMode,
    depth: Option<usize>,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let _ = mode;
    out.write_all(b"path,size_disk,size_apparent,files,is_dir\n")?;
    walk_nodes(tree, depth, |id, _| {
        let node = tree.node(id);
        writeln!(
            out,
            "{},{},{},{},{}",
            csv_field(&tree.path_string(id)),
            node.allocated_size,
            node.apparent_size,
            node.file_count,
            node.is_dir
        )
    })
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn size_parser_accepts_units() {
        assert_eq!(parse_size("42"), Ok(42));
        assert_eq!(parse_size("10K"), Ok(10 * 1024));
        assert_eq!(parse_size("1.5M"), Ok(1_572_864));
        assert_eq!(parse_size("2GiB"), Ok(2 << 30));
        assert!(parse_size("abc").is_err());
        assert!(parse_size("1X").is_err());
    }

    #[test]
    fn csv_fields_are_escaped() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn name_matcher_modes() {
        let sub = NameMatcher::new(Some("LOG"), false).unwrap();
        assert!(sub.matches("app.log", || unreachable!()));
        let glob = NameMatcher::new(Some("*.log"), false).unwrap();
        assert!(glob.matches("app.LOG", || unreachable!()));
        assert!(!glob.matches("app.txt", || unreachable!()));
        let path_glob = NameMatcher::new(Some("*/cache/*"), false).unwrap();
        assert!(path_glob.matches("x", || "/tmp/cache/x".into()));
        let re = NameMatcher::new(Some("^a.*z$"), true).unwrap();
        assert!(re.matches("abcz", || unreachable!()));
        assert!(NameMatcher::new(Some("("), true).is_err());
    }
}
