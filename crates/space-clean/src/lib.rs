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
    CleanResult, CleanedItem, DeleteMethod, TRASH_BATCH_SIZE,
};
pub use manager::{
    CategoryPolicy, CategoryRow, KindRow, ManagerStats, PrefsError, RiskRow, RuleFilter,
    RuleManager, RulePrefs, RuleRow, TagRow,
};
pub use protection::{is_protected_by_rule, protected_paths, ProtectionPack, ProtectionRule};
pub use rules::{
    enrich_rule_kind, expand_path_template, host_platform_name, yaml_files_under, DeleteMode,
    JunkKind, Platform, Risk, Rule, RuleIndex, RulePack,
};
pub use scan::{
    scan, scan_rules, scan_rules_with_progress, Finding, JunkScanProgress, ScanOptions, ScanReport,
};
pub use tags::{effective_tags, enrich_rule_tags, explicit_tags, infer_tags, normalize_tag};

use std::path::{Path, PathBuf};

/// Environment variable naming the rule pack root (`SPACE_RULES_DIR`).
pub const RULES_DIR_ENV: &str = "SPACE_RULES_DIR";

/// Discover the rule pack root. Order: `SPACE_RULES_DIR`, next to the
/// executable / inside the `.app` bundle, then the repository checkout this
/// binary was built from. The current directory is never consulted: a
/// `./rules` in an arbitrary cwd must not silently become the catalog.
/// Pass directories explicitly (CLI `--rules`) through
/// [`RuleManager::load_with_dirs`] / [`load_pack_from_dirs`].
///
/// Candidates are fallbacks, not a union: merging the `.app` copy with the
/// compile-time repo `rules/` would load every rule id twice.
pub fn default_rules_dirs() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os(RULES_DIR_ENV).filter(|value| !value.is_empty()) {
        candidates.push(PathBuf::from(dir));
    }
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
    first_existing_dir(&candidates)
}

/// First existing directory from `candidates`, canonicalized, as a one-element
/// list (or empty).
fn first_existing_dir(candidates: &[PathBuf]) -> Vec<PathBuf> {
    candidates
        .iter()
        .find(|dir| dir.is_dir())
        .map(|dir| vec![dir.canonicalize().unwrap_or_else(|_| dir.clone())])
        .unwrap_or_default()
}

pub fn load_default_pack() -> anyhow::Result<RulePack> {
    RulePack::load_many(&default_rules_dirs())
}

/// Load a catalog from explicit directories (CLI `--rules`), bypassing discovery.
pub fn load_pack_from_dirs(dirs: &[PathBuf]) -> anyhow::Result<RulePack> {
    RulePack::load_many(dirs)
}

/// Explicit rule directories as the CLI would pass them, normalized.
pub fn rules_dirs_from(dirs: impl IntoIterator<Item = impl AsRef<Path>>) -> Vec<PathBuf> {
    dirs.into_iter()
        .map(|dir| {
            let dir = dir.as_ref();
            dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rules_dirs_picks_a_single_root() {
        let dirs = default_rules_dirs();
        assert_eq!(dirs.len(), 1);
        assert!(dirs[0].is_dir());
        assert!(dirs[0].is_absolute());
        load_default_pack().expect("bundled rules should load without duplicate ids");
    }

    #[test]
    fn discovery_never_uses_the_working_directory() {
        let cwd = std::env::current_dir().unwrap();
        let fallback = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../rules")
            .canonicalize()
            .unwrap();
        // The crate directory has no `rules/`, so a cwd-relative candidate
        // would be the only way for `rules` to resolve differently.
        assert!(!cwd.join("rules").is_dir() || cwd.join("rules").canonicalize().unwrap() == fallback);
        assert!(first_existing_dir(&[PathBuf::from("rules-does-not-exist")]).is_empty());
        let temp = tempfile::tempdir().unwrap();
        let explicit = first_existing_dir(&[PathBuf::from("nope"), temp.path().to_path_buf()]);
        assert_eq!(explicit, vec![temp.path().canonicalize().unwrap()]);
        assert_eq!(rules_dirs_from([temp.path()]), explicit);
    }
}
