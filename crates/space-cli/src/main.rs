//! Space Scanner CLI (index + junk cleaner + rule manager).

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use space_clean::{
    clean, scan_rules, RuleFilter, RuleManager, RulePrefs, ScanOptions,
};
use space_core::{
    format_bytes, scan_path, CacheStore, NodeId, ScanEvent, SizeMode, SpaceTree,
};

#[derive(Parser)]
#[command(name = "space", about = "Space Scanner CLI (index + junk cleaner)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Index a path and save to cache
    Scan {
        path: PathBuf,
    },
    /// List children of a path from cache (or scan if missing)
    Ls {
        path: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show total size for a path from cache
    Du {
        path: PathBuf,
        #[arg(long)]
        apparent: bool,
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
    /// Scan regenerable junk (caches, logs, temp, …)
    Junk {
        /// Categories or rule ids (comma-separated). Empty = all enabled.
        #[arg(short = 'c', long = "category", value_delimiter = ',')]
        categories: Vec<String>,
        /// Require tags (AND, comma-separated)
        #[arg(short = 't', long = "tag", value_delimiter = ',')]
        tags: Vec<String>,
        /// Ignore prefs / YAML defaults — scan every host rule
        #[arg(long)]
        all: bool,
        #[arg(long)]
        min_age: Option<u64>,
        /// Actually move findings to Trash (default is dry-run report)
        #[arg(long)]
        clean: bool,
        #[arg(long)]
        yes: bool,
    },
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
    Enable {
        ids: Vec<String>,
    },
    /// Force-disable rule id(s)
    Disable {
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
    Category {
        action: CatAction,
        name: String,
    },
    /// Force a tag on/off/reset
    Tag {
        action: CatAction,
        name: String,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum CatAction {
    On,
    Off,
    Reset,
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Scan { path } => {
            let tree = scan_blocking(&path);
            let cache = CacheStore::open().expect("cache dir");
            cache.save(&tree).expect("save");
            println!(
                "Indexed {} · {} items · cache saved",
                format_bytes(tree.total_size()),
                tree.item_count
            );
        }
        Cmd::Ls { path, limit } => {
            let tree = load_or_scan(&path);
            let mode = SizeMode::OnDisk;
            let kids = tree.children(tree.root);
            for (i, &id) in kids.iter().take(limit).enumerate() {
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
            let tree = load_or_scan(&path);
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
        Cmd::Rules { cmd, all } => match cmd {
            None => {
                let mgr = RuleManager::load_default().expect("rules");
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
                enabled,
                disabled,
                overridden,
                all,
                limit,
            }) => {
                let mgr = RuleManager::load_default().expect("rules");
                let en = if enabled {
                    Some(true)
                } else if disabled {
                    Some(false)
                } else if all {
                    None
                } else {
                    None
                };
                let filter = RuleFilter {
                    query,
                    category,
                    tags,
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
                    let tag_s = if r.tags.is_empty() {
                        String::new()
                    } else {
                        format!("  [{}]", r.tags.iter().take(4).cloned().collect::<Vec<_>>().join(","))
                    };
                    println!(
                        "[{flag}]{mark} {:>28}  {:<14}  {}{tag_s}",
                        r.id, r.category, r.name
                    );
                }
            }
            Some(RulesCmd::Status) => {
                let mgr = RuleManager::load_default().expect("rules");
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
                println!("platform.macos\t{macos}");
                println!("platform.linux\t{linux}");
                println!("platform.windows\t{win}");
                println!("platform.any\t{any}");
                if let Some(p) = RulePrefs::default_path() {
                    println!("prefs\t{}", p.display());
                }
            }
            Some(RulesCmd::Enable { ids }) => {
                let mut mgr = RuleManager::load_default().expect("rules");
                for id in &ids {
                    mgr.set_rule_enabled(id, true).unwrap_or_else(|e| {
                        eprintln!("{e}");
                    });
                }
                mgr.save_prefs().unwrap_or_else(|e| eprintln!("warn: save prefs: {e}"));
                println!("enabled {} rule(s)", ids.len());
            }
            Some(RulesCmd::Disable { ids }) => {
                let mut mgr = RuleManager::load_default().expect("rules");
                for id in &ids {
                    mgr.set_rule_enabled(id, false).unwrap_or_else(|e| {
                        eprintln!("{e}");
                    });
                }
                mgr.save_prefs().unwrap_or_else(|e| eprintln!("warn: save prefs: {e}"));
                println!("disabled {} rule(s)", ids.len());
            }
            Some(RulesCmd::Reset { ids, all }) => {
                let mut mgr = RuleManager::load_default().expect("rules");
                if all {
                    mgr.reset_all();
                    mgr.save_prefs().unwrap_or_else(|e| eprintln!("warn: save prefs: {e}"));
                    println!("prefs reset");
                } else if ids.is_empty() {
                    eprintln!("pass rule ids or --all");
                } else {
                    for id in &ids {
                        mgr.reset_rule(id);
                    }
                    mgr.save_prefs().unwrap_or_else(|e| eprintln!("warn: save prefs: {e}"));
                    println!("cleared {} override(s)", ids.len());
                }
            }
            Some(RulesCmd::Category { action, name }) => {
                let mut mgr = RuleManager::load_default().expect("rules");
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
                let mut mgr = RuleManager::load_default().expect("rules");
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
            let mgr = RuleManager::load_default().expect("rules");
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
            let mgr = RuleManager::load_default().expect("rules");
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
        Cmd::Junk {
            categories,
            tags,
            all,
            min_age,
            clean: do_clean,
            yes,
        } => {
            let mgr = RuleManager::load_default().expect("rules");
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
                            categories
                                .iter()
                                .any(|c| c == &r.category || c == &r.id)
                        }
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
                scan_rules(mgr.scan_rules_filtered(&categories, &tags), &opts).expect("junk scan")
            };
            println!(
                "Junk scan · {} · {} findings · {} rules matched / {} scanned",
                format_bytes(report.total_bytes),
                report.findings.len(),
                report.rules_matched,
                report.rules_scanned
            );
            let mut shown = 0usize;
            for f in &report.findings {
                if shown >= 40 {
                    println!("… {} more", report.findings.len() - shown);
                    break;
                }
                let tag = if f.report_only { "report" } else { "clean" };
                println!(
                    "  [{tag}] {:>10}  {}  ({})",
                    format_bytes(f.bytes),
                    f.path.display(),
                    f.rule_id
                );
                shown += 1;
            }
            if do_clean {
                if !yes {
                    eprintln!("Dry-run only. Pass --yes to move deletable findings to Trash.");
                    return;
                }
                let result = clean(&report, false).expect("clean");
                println!(
                    "Moved {} items to Trash · {} · {} errors · {} report-only skipped",
                    result.deleted.len(),
                    format_bytes(result.freed_bytes),
                    result.errors.len(),
                    result.skipped_report_only
                );
                for e in result.errors.iter().take(10) {
                    eprintln!("  ! {e}");
                }
            }
        }
    }
}

fn scan_blocking(path: &std::path::Path) -> SpaceTree {
    let handle = scan_path(path);
    let mut tree = None;
    while let Ok(ev) = handle.events.recv() {
        match ev {
            ScanEvent::Progress(p) => {
                eprint!(
                    "\rscan {} files · {} · {}          ",
                    p.files_seen,
                    format_bytes(p.bytes_seen),
                    p.current_path
                );
            }
            ScanEvent::Done(t) => {
                tree = Some(*t);
                break;
            }
            ScanEvent::Cancelled => break,
            ScanEvent::Error(e) => eprintln!("\nwarn: {e}"),
        }
    }
    eprintln!();
    tree.expect("scan produced no tree")
}

fn load_or_scan(path: &std::path::Path) -> SpaceTree {
    let cache = CacheStore::open().expect("cache");
    if let Ok(loaded) = cache.load(path) {
        return loaded.tree;
    }
    let tree = scan_blocking(path);
    let _ = cache.save(&tree);
    tree
}
