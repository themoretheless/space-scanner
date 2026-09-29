//! Measure existing-file matching and safety checks without deleting user data.
use space_clean::{scan_rules, Rule, ScanOptions};
use std::{fs, time::Instant};
fn main() {
    let root = std::env::temp_dir().join(format!("space-present-profile-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    for i in 0..5000 {
        fs::write(root.join(format!("{i}.log")), b"fixture").unwrap();
    }
    let mut rule: Rule = serde_json::from_value(serde_json::json!({
        "id": "fixture", "name": "fixture", "description": "existing-file fixture",
        "paths": [], "min_age_days": 0, "include_globs": ["**/*.log"], "platform": "any"
    }))
    .unwrap();
    rule.paths = vec![root.to_string_lossy().into_owned()];
    let mut times = Vec::new();
    for iteration in 0..8 {
        let start = Instant::now();
        let report = scan_rules([&rule], &ScanOptions::default()).unwrap();
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(report.findings.len(), 5000);
        assert_eq!(report.total_bytes, 35000);
        if iteration > 0 {
            times.push(elapsed);
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "files=5000 bytes=35000 median_ms={:.3}",
        times[times.len() / 2]
    );
    fs::remove_dir_all(root).unwrap();
}
