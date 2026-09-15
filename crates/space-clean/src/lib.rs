//! Rule-driven junk cleaner — ported from ppduster core (rules / scan / clean / safety).
//!
//! Rule packs live under repo `rules/categories/*.yaml` (recursive load; see `rules/README.md`).

pub mod clean;
pub mod manager;
pub mod rules;
pub mod safety;
pub mod scan;
pub mod tags;

pub use clean::{clean, CleanResult, CleanedItem};
pub use manager::{
    CategoryPolicy, CategoryRow, ManagerStats, RuleFilter, RuleManager, RulePrefs, RuleRow, TagRow,
};
pub use rules::{
    expand_path_template, host_platform_name, DeleteMode, Platform, Risk, Rule, RulePack,
};
pub use scan::{scan, scan_rules, Finding, ScanOptions, ScanReport};
pub use tags::{effective_tags, enrich_rule_tags, infer_tags, normalize_tag};

use std::path::PathBuf;

/// Discover rule pack directories (cwd `./rules`, next to executable, or bundled path).
pub fn default_rules_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.push(PathBuf::from("rules"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // cargo run: …/target/release/space → …/target/release/rules (unusual)
            dirs.push(parent.join("rules"));
            // macOS .app: Contents/MacOS/SpaceScanner → Contents/rules
            if let Some(contents) = parent.parent() {
                dirs.push(contents.join("rules"));
                dirs.push(contents.join("Resources/rules"));
            }
            // cargo run: target/release → repo root/rules
            if let Some(grand) = parent.parent().and_then(|p| p.parent()) {
                dirs.push(grand.join("rules"));
            }
        }
    }
    // Workspace-relative when developing
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules"));
    dirs
}

pub fn load_default_pack() -> anyhow::Result<RulePack> {
    RulePack::load_many(&default_rules_dirs())
}
