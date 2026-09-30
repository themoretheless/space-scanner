//! Measure catalog summaries independently of loading and verify stable results.
use space_clean::{RuleManager, RulePack, RulePrefs};
use std::{hint::black_box, time::Instant};

fn main() -> anyhow::Result<()> {
    let mut manager = RuleManager {
        pack: RulePack::load_many_shared(&space_clean::default_rules_dirs())?,
        prefs: RulePrefs::default(),
    };
    println!("rules={}", manager.pack.rules.len());
    for scenario in ["defaults", "overrides"] {
        if scenario == "overrides" {
            manager.prefs.set_tag("cache", true);
            manager.prefs.set_tag("generated", false);
            manager.prefs.set_category("logs", true);
            manager.prefs.set_category("app-data", false);
        }
        measure(scenario, "stats", || format!("{:?}", manager.stats()));
        measure(scenario, "categories", || {
            format!("{:?}", manager.categories(true))
        });
        measure(scenario, "tags", || format!("{:?}", manager.tags(true)));
        measure(scenario, "kinds", || format!("{:?}", manager.kinds(true)));
        measure(scenario, "risks", || format!("{:?}", manager.risks(true)));
    }
    Ok(())
}

fn measure(scenario: &str, name: &str, mut operation: impl FnMut() -> String) {
    let expected = operation();
    let mut samples = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let result = black_box(operation());
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(result, expected);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "scenario={scenario} operation={name} median_ms={:.2} digest={}",
        samples[2],
        blake3::hash(expected.as_bytes())
    );
}
