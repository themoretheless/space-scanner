use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use space_core::{
    format_bytes, scan_path, CacheStore, NodeId, ScanEvent, SizeMode, SpaceTree,
};

#[derive(Parser)]
#[command(name = "space", about = "Space Scanner CLI (duc-style index)")]
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
