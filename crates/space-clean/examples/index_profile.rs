//! Measure the one-off catalog index build and the lookups it serves.
use space_clean::{RuleIndex, RulePack};
use std::{hint::black_box, time::Instant};

fn main() -> anyhow::Result<()> {
    let pack = RulePack::load_many_shared(&space_clean::default_rules_dirs())?;
    let start = Instant::now();
    let index = RuleIndex::build(&pack);
    println!(
        "rules={} index_build_ms={:.2} host={} host_tags={} host_categories={}",
        pack.rules.len(),
        start.elapsed().as_secs_f64() * 1000.0,
        index.host_positions().len(),
        index.host_tag_count(),
        index.host_category_count()
    );
    let shared = pack.index();
    let start = Instant::now();
    let mut hits = 0usize;
    for rule in pack.rules.iter().step_by(1000) {
        hits += usize::from(shared.position(&pack, &rule.id).is_some());
        hits += usize::from(pack.get(&rule.id).is_some());
    }
    println!(
        "lookups={} lookup_total_ms={:.2}",
        black_box(hits),
        start.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}
