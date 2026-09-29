//! Compare initial catalog parsing with a content-verified shared reload.
use space_clean::RulePack;
use std::{sync::Arc, time::Instant};
fn main() -> anyhow::Result<()> {
    let dirs = space_clean::default_rules_dirs();
    let start = Instant::now();
    let first = RulePack::load_many_shared(&dirs)?;
    let initial = start.elapsed();
    let start = Instant::now();
    let second = RulePack::load_many_shared(&dirs)?;
    let reload = start.elapsed();
    assert!(Arc::ptr_eq(&first, &second));
    println!(
        "rules={} initial_ms={:.2} verified_reload_ms={:.2}",
        first.rules.len(),
        initial.as_secs_f64() * 1000.0,
        reload.as_secs_f64() * 1000.0
    );
    let manager = space_clean::RuleManager {
        pack: second,
        prefs: space_clean::RulePrefs::default(),
    };
    let start = Instant::now();
    let stats = manager.stats();
    let stats_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let filter = space_clean::RuleFilter {
        host_only: true,
        ..Default::default()
    };
    let (matched, page) = manager.list_page(&filter, 200);
    println!(
        "host={} stats_ms={stats_ms:.2} matched={matched} page_rows={} page_ms={:.2}",
        stats.host,
        page.len(),
        start.elapsed().as_secs_f64() * 1000.0
    );
    for query in ["cache", "no-such-app-92871"] {
        let filter = space_clean::RuleFilter {
            query: query.into(),
            ..Default::default()
        };
        let mut samples = Vec::new();
        let mut count = 0;
        for _ in 0..5 {
            let start = Instant::now();
            let (matched, _) = manager.list_page(&filter, 200);
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            count = matched;
        }
        samples.sort_by(f64::total_cmp);
        println!("query={query} matched={count} median_ms={:.2}", samples[2]);
    }
    Ok(())
}
