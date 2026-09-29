//! Compare separate and shared traversals of identical predicates with distinct IDs/tags.
use space_clean::{scan_rules, Rule, ScanOptions};
use std::{fs, time::Instant};
fn main() -> anyhow::Result<()> {
    let root = std::env::temp_dir().join(format!("space-repeated-profile-{}", std::process::id()));
    fs::create_dir(&root)?;
    for i in 0..5000 {
        fs::write(root.join(format!("{i}.log")), b"fixture")?;
    }
    let rules: Vec<Rule> = (0..12)
        .map(|i| {
            serde_json::from_value(serde_json::json!({
                "id": format!("rule-{i}"), "name": "fixture", "tags": [format!("tag-{i}")],
                "paths": [root], "min_age_days": 0, "include_globs": ["**/*.log"], "platform": "any"
            }))
            .unwrap()
        })
        .collect();
    let options = ScanOptions::default();
    let mut separate = Vec::new();
    let mut shared = Vec::new();
    for iteration in 0..6 {
        let start = Instant::now();
        let mut findings = Vec::new();
        for rule in &rules {
            findings.extend(scan_rules([rule], &options)?.findings);
        }
        findings.sort_by(|a, b| a.path.cmp(&b.path));
        findings.dedup_by(|a, b| a.path == b.path);
        let baseline_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let report = scan_rules(&rules, &options)?;
        let shared_ms = start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(
            serde_json::to_value(findings)?,
            serde_json::to_value(&report.findings)?
        );
        assert_eq!(report.rules_matched, 12);
        assert_eq!(report.total_bytes, 35000);
        if iteration > 0 {
            separate.push(baseline_ms);
            shared.push(shared_ms);
        }
    }
    separate.sort_by(f64::total_cmp);
    shared.sort_by(f64::total_cmp);
    println!(
        "rules=12 files=5000 separate_ms={:.3} shared_ms={:.3}",
        separate[2], shared[2]
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
