//! Metadata/glob overhead for a catalog whose application roots are absent.
use space_clean::{scan_rules, Rule, ScanOptions};
use std::time::Instant;
fn main() {
    let template: Rule = serde_yaml::from_str(
        r#"
id: fixture
name: fixture
description: Missing application cache fixture
category: caches
platform: any
paths: []
include_globs: ['**/*.log', '**/*.tmp', '**/Cache/**']
exclude_globs: ['**/keep/**']
"#,
    )
    .unwrap();
    let root = std::env::temp_dir().join(format!("space-missing-profile-{}", std::process::id()));
    assert!(!root.exists());
    let rules: Vec<_> = (0..10_000)
        .map(|i| {
            let mut rule = template.clone();
            rule.id = format!("fixture-{i}");
            rule.paths = vec![root.join(i.to_string()).to_string_lossy().into_owned()];
            rule
        })
        .collect();
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let report = scan_rules(&rules, &ScanOptions::default()).unwrap();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(report.rules_scanned, rules.len());
        assert!(report.findings.is_empty());
    }
    times.sort_by(f64::total_cmp);
    println!("missing_roots=10000 median_ms={:.3}", times[2]);
}
