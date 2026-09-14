//! Rule-driven junk cleaner — ported from ppduster core (rules / scan / clean / safety).

pub mod clean;
pub mod rules;
pub mod safety;
pub mod scan;

pub use clean::{clean, CleanResult, CleanedItem};
pub use rules::{
    expand_path_template, host_platform_name, DeleteMode, Platform, Risk, Rule, RulePack,
};
pub use scan::{scan, Finding, ScanOptions, ScanReport};

use std::path::PathBuf;

/// Discover rule pack directories (cwd `./rules`, next to executable, or bundled path).
pub fn default_rules_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.push(PathBuf::from("rules"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("rules"));
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
