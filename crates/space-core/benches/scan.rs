//! Scan throughput benches (airbug-bench) + sized fixtures for bottleneck report.
//!
//! ```bash
//! cargo bench -p space-core --features bench --bench scan -- --profile quick
//! cargo bench -p space-core --features bench --bench scan -- --profile normal
//! ```

use std::fs;
use std::hint::black_box;
use std::io::Write;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use airbug_bench::{DropPolicy, Suite};
use space_core::{rescan_subtree, scan_path, NodeId, ScanEvent, SpaceTree};

fn main() -> airbug_bench::Result<()> {
    // `cargo bench` appends `--bench`; airbug-bench rejects unknown flags.
    let raw: Vec<String> = std::env::args().collect();
    if raw.iter().any(|a| a == "--bench") {
        let status = std::process::Command::new(&raw[0])
            .args(raw[1..].iter().filter(|a| a.as_str() != "--bench"))
            .status()
            .expect("re-exec bench");
        std::process::exit(status.code().unwrap_or(1));
    }

    // One-shot phase profile (printed before Suite) for the slow-path report.
    if !raw.iter().any(|a| a == "--list" || a == "--dry-run") {
        run_phase_profile();
    }

    let mut suite = Suite::new("space_scan");

    // Drop outside timing: we measure scan, not rm -rf of the fixture.
    suite.bench_with_input(
        "scan/tree_512",
        || make_fixture(8, 8, 8),
        |root| {
            let tree = scan_blocking(root);
            black_box(tree.item_count);
        },
        DropPolicy::OutsideTiming,
    );

    suite.bench_with_input(
        "scan/tree_4096",
        || make_fixture(16, 16, 16),
        |root| {
            let tree = scan_blocking(root);
            black_box(tree.item_count);
        },
        DropPolicy::OutsideTiming,
    );

    // Reuse the large fixture: repeated creation/deletion dwarfs scan time.
    let large_fixture = std::sync::OnceLock::new();
    suite.bench_with_input(
        "scan/tree_65536",
        move || {
            std::sync::Arc::clone(
                large_fixture.get_or_init(|| std::sync::Arc::new(make_fixture(32, 32, 64))),
            )
        },
        |root| {
            let tree = scan_blocking(root);
            assert_eq!(tree.node(tree.root).file_count, 65_536);
            black_box(tree.item_count);
        },
        DropPolicy::OutsideTiming,
    );

    suite.bench_with_input(
        "scan/sums_4096",
        || make_fixture(16, 16, 16),
        |root| {
            let tree = scan_blocking(root);
            black_box(tree.total_size());
            black_box(tree.node(tree.root).file_count);
        },
        DropPolicy::OutsideTiming,
    );

    suite.bench_with_input(
        "rescan/subtree_256_of_4096",
        || {
            let root = make_fixture(16, 16, 16);
            let tree = scan_blocking(&root);
            let subtree = child_named(&tree, tree.root, "d0");
            (root, tree, subtree)
        },
        |(_, tree, subtree)| {
            let rescanned = rescan_subtree(tree, *subtree).expect("subtree rescan");
            black_box(rescanned.item_count);
        },
        DropPolicy::OutsideTiming,
    );

    suite.main()
}

fn run_phase_profile() {
    eprintln!("--- phase profile (tree_4096) ---");
    let fix = make_fixture(16, 16, 16);
    let t0 = Instant::now();
    let tree = scan_blocking(&fix);
    let wall = t0.elapsed();
    let files = tree.node(tree.root).file_count;
    let items = tree.item_count;
    let bytes = tree.total_size();
    eprintln!(
        "items={items} files={files} size={} wall={wall:?} ({:.0} files/s)",
        humansize_ish(bytes),
        files as f64 / wall.as_secs_f64().max(1e-9)
    );
    eprintln!(
        "per-file≈{:.0}µs  nodes={}",
        wall.as_secs_f64() * 1e6 / files.max(1) as f64,
        items
    );
    eprintln!("--- end phase profile ---\n");
    drop(fix);
}

fn humansize_ish(n: u64) -> String {
    if n >= 1 << 30 {
        format!("{:.1} GiB", n as f64 / (1u64 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.1} KiB", n as f64 / (1u64 << 10) as f64)
    } else {
        format!("{n} B")
    }
}

/// Temp tree that deletes itself when dropped.
struct Fixture(PathBuf);

impl Deref for Fixture {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `tops * mids * files` leaf files; all small (64 B) so the bench is metadata-bound.
fn make_fixture(tops: u8, mids: u8, files: u8) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "space-scan-bench-{}-{}-{}",
        tops,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("mkdir fixture");

    for a in 0..tops {
        for b in 0..mids {
            let dir = root.join(format!("d{a}")).join(format!("m{b}"));
            fs::create_dir_all(&dir).expect("mkdir");
            for f in 0..files {
                let path = dir.join(format!("f{f}.bin"));
                let mut file = fs::File::create(&path).expect("create");
                file.write_all(&[f; 64]).expect("write");
            }
        }
    }

    Fixture(root)
}

fn scan_blocking(path: &Path) -> SpaceTree {
    let handle = scan_path(path);
    loop {
        match handle.events.recv_timeout(Duration::from_secs(120)) {
            Ok(ScanEvent::Done(t)) => return *t,
            Ok(ScanEvent::Progress(_)) => continue,
            Ok(ScanEvent::Cancelled) => panic!("scan cancelled"),
            Ok(ScanEvent::Error(e)) => panic!("scan error: {e}"),
            Err(e) => panic!("scan recv: {e}"),
        }
    }
}

fn child_named(tree: &SpaceTree, parent: NodeId, name: &str) -> NodeId {
    tree.children(parent)
        .iter()
        .copied()
        .find(|id| tree.node(*id).name == name)
        .expect("fixture child")
}
