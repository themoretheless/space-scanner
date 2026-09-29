//! Repeat scans of an existing tree without including fixture creation or cleanup.
use space_core::{scan_path, ScanEvent};
use std::time::{Duration, Instant};

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .expect("usage: scan_profile PATH");
    let mut times = Vec::new();
    let mut totals = None;
    for iteration in 0..12 {
        let start = Instant::now();
        let scan = scan_path(&path);
        let tree = loop {
            match scan.events.recv_timeout(Duration::from_secs(120)).unwrap() {
                ScanEvent::Done(tree) => break tree,
                ScanEvent::Progress(_) => {}
                event => panic!("scan failed: {event:?}"),
            }
        };
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        let root = tree.node(tree.root);
        let observed = (
            root.file_count,
            root.apparent_size,
            root.allocated_size,
            tree.item_count,
            tree.skipped_errors,
        );
        if let Some(expected) = totals {
            assert_eq!(observed, expected, "tree changed during measurement");
        }
        totals = Some(observed);
        if iteration > 0 {
            times.push(elapsed);
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "median_ms={:.3} min_ms={:.3} max_ms={:.3} totals={:?}",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1],
        totals.unwrap()
    );
}
