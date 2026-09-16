//! Rule-driven junk cleaner — ported from ppduster core (rules / scan / clean / safety).
//!
//! Rule packs live under repo `rules/categories/*.yaml` (recursive load; see `rules/README.md`).

pub mod clean;
pub mod manager;
pub mod protection;
pub mod rules;
pub mod safety;
pub mod scan;
pub mod tags;

pub use clean::{
    clean, clean_with_progress, coalesce_nested_paths, trash_paths_batch, CleanProgress,
    CleanResult, CleanedItem, TRASH_BATCH_SIZE,
};
pub use manager::{
    CategoryPolicy, CategoryRow, KindRow, ManagerStats, RiskRow, RuleFilter, RuleManager,
    RulePrefs, RuleRow, TagRow,
};
pub use protection::{is_protected_by_rule, protected_paths, ProtectionPack, ProtectionRule};
pub use rules::{
    enrich_rule_kind, expand_path_template, host_platform_name, DeleteMode, JunkKind, Platform,
    Risk, Rule, RulePack,
};
pub use scan::{
    scan, scan_rules, scan_rules_with_progress, Finding, JunkScanProgress, ScanOptions, ScanReport,
};
pub use tags::{effective_tags, enrich_rule_tags, infer_tags, normalize_tag};

use std::path::PathBuf;

/// Discover the rule pack root (cwd `./rules`, next to executable, or bundled path).
///
/// Candidates are fallbacks, not a union: merging the `.app` copy with the
/// compile-time repo `rules/` would load every rule id twice.
pub fn default_rules_dirs() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    candidates.push(PathBuf::from("rules"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // cargo run: …/target/release/space → …/target/release/rules (unusual)
            candidates.push(parent.join("rules"));
            // macOS .app: Contents/MacOS/SpaceScanner → Contents/rules
            if let Some(contents) = parent.parent() {
                candidates.push(contents.join("rules"));
                candidates.push(contents.join("Resources/rules"));
            }
            // cargo run: target/release → repo root/rules
            if let Some(grand) = parent.parent().and_then(|p| p.parent()) {
                candidates.push(grand.join("rules"));
            }
        }
    }
    // Workspace-relative when developing
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules"));

    let mut seen = Vec::new();
    for dir in candidates {
        if !dir.is_dir() {
            continue;
        }
        let canon = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if seen.iter().any(|d| d == &canon) {
            continue;
        }
        seen.push(canon);
        // First existing root wins so bundled + source copies are not merged.
        return seen;
    }
    Vec::new()
}

pub fn load_default_pack() -> anyhow::Result<RulePack> {
    RulePack::load_many(&default_rules_dirs())
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_rules_dirs_picks_a_single_root() {
        let dirs = super::default_rules_dirs();
        assert_eq!(dirs.len(), 1);
        assert!(dirs[0].is_dir());
        super::load_default_pack().expect("bundled rules should load without duplicate ids");
    }
}
