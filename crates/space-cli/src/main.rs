use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use space_clean::{clean, load_default_pack, scan, ScanOptions};
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
    /// List junk-cleaner rules (from ppduster-style YAML packs)
    Rules {
        #[arg(long)]
        all: bool,
    },
    /// List junk categories
    Categories {
        #[arg(long)]
        all: bool,
    },
    /// Scan regenerable junk (caches, logs, temp, …)
    Junk {
        /// Categories or rule ids (comma-separated). Empty = all enabled.
        #[arg(short = 'c', long = "category", value_delimiter = ',')]
        categories: Vec<String>,
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
        Cmd::Rules { all } => {
            let pack = load_default_pack().expect("load rules");
            for r in &pack.rules {
                if !r.platform.matches_host() {
                    continue;
                }
                if !all && !r.default_enabled {
                    continue;
                }
                let flag = if r.default_enabled { "on " } else { "off" };
                let ro = if r.report_only { " report" } else { "" };
                println!(
                    "[{flag}] {:>28}  {:<14}  {}{ro}",
                    r.id, r.category, r.name
                );
            }
        }
        Cmd::Categories { all } => {
            let pack = load_default_pack().expect("load rules");
            for (cat, n) in pack.categories(all) {
                println!("{cat}\t{n}");
            }
        }
        Cmd::Junk {
            categories,
            all,
            min_age,
            clean: do_clean,
            yes,
        } => {
            let pack = load_default_pack().expect("load rules");
            let opts = ScanOptions {
                categories,
                include_disabled: all,
                min_age_override: min_age,
            };
            let report = scan(&pack, &opts).expect("junk scan");
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

fn load_or_scan(path: &PathBuf) -> SpaceTree {
    if let Ok(cache) = CacheStore::open() {
        if let Ok(loaded) = cache.load(path) {
            return loaded.tree;
        }
    }
    let tree = scan_blocking(path);
    if let Ok(cache) = CacheStore::open() {
        let _ = cache.save(&tree);
    }
    tree
}

fn scan_blocking(path: &PathBuf) -> SpaceTree {
    let handle = scan_path(path);
    loop {
        match handle.events.recv_timeout(Duration::from_secs(3600)) {
            Ok(ScanEvent::Done(t)) => return *t,
            Ok(ScanEvent::Progress(p)) => {
                eprint!(
                    "\r{} files · {}        ",
                    p.files_seen,
                    format_bytes(p.bytes_seen)
                );
            }
            Ok(ScanEvent::Cancelled) => panic!("cancelled"),
            Ok(ScanEvent::Error(e)) => panic!("{e}"),
            Err(e) => panic!("{e}"),
        }
    }
}
