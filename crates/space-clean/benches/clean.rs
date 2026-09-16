use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use space_clean::coalesce_nested_paths;

fn main() {
    let input = make_nested_paths(100, 1_000);
    let expected = legacy_coalesce(input.clone());
    let optimized = coalesce_nested_paths(input.clone(), PathBuf::as_path);
    assert_eq!(optimized, expected);

    let legacy = median(5, || {
        black_box(legacy_coalesce(black_box(input.clone())).len())
    });
    let candidate = median(9, || {
        black_box(coalesce_nested_paths(
            black_box(input.clone()),
            PathBuf::as_path,
        ))
        .len()
    });
    println!(
        "coalesce/100000: legacy={:.3}ms optimized={:.3}ms speedup={:.2}x output={}",
        legacy.as_secs_f64() * 1_000.0,
        candidate.as_secs_f64() * 1_000.0,
        legacy.as_secs_f64() / candidate.as_secs_f64(),
        optimized.len()
    );
}

fn make_nested_paths(parents: usize, children: usize) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(parents * (children + 1));
    for parent in 0..parents {
        let root = PathBuf::from(format!("/tmp/space-clean-bench/group-{parent:04}"));
        paths.push(root.clone());
        for child in 0..children {
            paths.push(root.join(format!("nested-{child:04}")));
        }
    }
    paths.reverse();
    paths
}

fn legacy_coalesce(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort();
    let mut selected: Vec<PathBuf> = Vec::with_capacity(paths.len());
    for path in paths {
        if selected.iter().any(|parent| path.starts_with(parent)) {
            continue;
        }
        selected.push(path);
    }
    selected
}

fn median(samples: usize, mut operation: impl FnMut() -> usize) -> Duration {
    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        black_box(operation());
        timings.push(started.elapsed());
    }
    timings.sort_unstable();
    timings[timings.len() / 2]
}
