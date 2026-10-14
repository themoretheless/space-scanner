//! Data-preservation rules loaded separately from cleanup rules.
//!
//! Protection packs use `_protected-*.yaml`, so the cleanup rule loader ignores
//! them while this module turns their paths into hard deletion blockers.

use crate::rules::{expand_path_template, yaml_files_under, Platform};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectionRule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub platform: Platform,
    pub paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProtectionFile {
    #[serde(default)]
    protections: Vec<ProtectionRule>,
}

#[derive(Debug, Clone, Default)]
pub struct ProtectionPack {
    pub rules: Vec<ProtectionRule>,
}

impl ProtectionPack {
    pub fn load_many(dirs: &[PathBuf]) -> Result<Self> {
        let mut files = BTreeSet::new();
        for dir in dirs {
            if !dir.exists() {
                continue;
            }
            for path in yaml_files_under(dir) {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("_protected-") {
                    files.insert(fs::canonicalize(&path).unwrap_or(path));
                }
            }
        }

        let mut by_id = BTreeMap::new();
        for file in files {
            let text =
                fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
            let parsed: ProtectionFile =
                serde_yaml::from_str(&text).with_context(|| format!("parse {}", file.display()))?;
            for mut rule in parsed.protections {
                rule.id = rule.id.trim().to_string();
                rule.name = rule.name.trim().to_string();
                if rule.id.is_empty()
                    || rule.name.is_empty()
                    || rule.description.trim().is_empty()
                    || rule.paths.is_empty()
                {
                    anyhow::bail!("{}: invalid protection rule '{}'", file.display(), rule.id);
                }
                if by_id.insert(rule.id.clone(), rule).is_some() {
                    anyhow::bail!("{}: duplicate protection id", file.display());
                }
            }
        }
        Ok(Self {
            rules: by_id.into_values().collect(),
        })
    }

    pub fn host_paths(&self) -> Vec<PathBuf> {
        let mut paths = BTreeSet::new();
        for rule in self.rules.iter().filter(|r| r.platform.matches_host()) {
            for template in &rule.paths {
                if let Some(path) = expand_path_template(template) {
                    paths.insert(crate::safety::normalize(&path).unwrap_or(path));
                }
            }
        }
        crate::clean::coalesce_nested_paths(paths.into_iter().collect(), PathBuf::as_path)
    }
}

#[derive(Debug)]
struct ProtectedIndex {
    /// Sorted, prefix-minimized, case-folded like [`crate::safety::fold_case`].
    paths: Vec<PathBuf>,
    exact: HashSet<PathBuf>,
}

impl ProtectedIndex {
    fn from_pack(pack: &ProtectionPack) -> Self {
        let mut paths: Vec<PathBuf> = pack
            .host_paths()
            .iter()
            .map(|path| crate::safety::fold_case(path))
            .collect();
        paths.sort();
        paths.dedup();
        let paths = crate::clean::coalesce_nested_paths(paths, PathBuf::as_path);
        let exact = paths.iter().cloned().collect();
        Self { paths, exact }
    }
}

/// How long a failed load is remembered before the pack is read again, so a
/// transient error does not pin every path check to a 2000-file re-walk.
const RELOAD_BACKOFF: Duration = Duration::from_secs(5);

struct ProtectionState {
    /// Explicit rule directories; `None` falls back to [`crate::default_rules_dirs`].
    dirs: Option<Vec<PathBuf>>,
    loaded_for: Option<Vec<PathBuf>>,
    index: Option<Arc<ProtectedIndex>>,
    error: Option<(String, Instant)>,
}

static STATE: Mutex<ProtectionState> = Mutex::new(ProtectionState {
    dirs: None,
    loaded_for: None,
    index: None,
    error: None,
});

fn lock_state() -> std::sync::MutexGuard<'static, ProtectionState> {
    STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Use these rule directories for the protection pack instead of discovering
/// them. Called by [`crate::RuleManager::load_with_dirs`] so cleanup and
/// protection rules always come from the same place; a change drops the
/// cached index.
pub fn set_rules_dirs(dirs: &[PathBuf]) {
    let mut state = lock_state();
    if state.dirs.as_deref() != Some(dirs) {
        state.dirs = Some(dirs.to_vec());
        state.index = None;
        state.loaded_for = None;
        state.error = None;
    }
}

/// Current protection rule directories (explicit or discovered).
pub fn rules_dirs() -> Vec<PathBuf> {
    lock_state()
        .dirs
        .clone()
        .unwrap_or_else(crate::default_rules_dirs)
}

fn protected_index() -> Result<Arc<ProtectedIndex>, String> {
    index_for(&mut lock_state())
}

/// Load (or reuse) the index for the directories recorded in `state`. A
/// failure is remembered only for [`RELOAD_BACKOFF`], after which the pack is
/// read again, so fixing the files recovers protection without a restart.
fn index_for(state: &mut ProtectionState) -> Result<Arc<ProtectedIndex>, String> {
    let dirs = state
        .dirs
        .clone()
        .unwrap_or_else(crate::default_rules_dirs);
    if state.loaded_for.as_deref() == Some(dirs.as_slice()) {
        if let Some(index) = &state.index {
            return Ok(Arc::clone(index));
        }
        if let Some((error, at)) = &state.error {
            if at.elapsed() < RELOAD_BACKOFF {
                return Err(error.clone());
            }
        }
    }
    let result = ProtectionPack::load_many(&dirs)
        .map(|pack| Arc::new(ProtectedIndex::from_pack(&pack)))
        .map_err(|error| error.to_string());
    state.loaded_for = Some(dirs);
    match &result {
        Ok(index) => {
            state.index = Some(Arc::clone(index));
            state.error = None;
        }
        Err(error) => {
            state.index = None;
            state.error = Some((error.clone(), Instant::now()));
        }
    }
    result
}

/// The error that currently makes protection fail closed, if any. Scans add
/// it to `skipped_roots` so a broken pack is visible instead of silent.
pub fn load_error() -> Option<String> {
    protected_index().err()
}

/// Paths from the bundled/user protection packs for the current OS.
pub fn protected_paths() -> Result<Vec<PathBuf>, String> {
    protected_index().map(|index| index.paths.clone())
}

pub fn is_protected_by_rule(path: &Path) -> bool {
    let Ok(candidate) = crate::safety::normalize(path) else {
        return true;
    };
    is_protected_normalized(&crate::safety::fold_case(&candidate))
}

/// Checks both directions: a protected ancestor blocks its descendants, and a
/// protected descendant blocks deleting an ancestor that would contain it.
/// `candidate` must already be normalized and case-folded.
pub(crate) fn is_protected_normalized(candidate: &Path) -> bool {
    let Ok(index) = protected_index() else {
        // Hard safety must fail closed if its rule pack cannot be loaded.
        return true;
    };
    intersects_index(candidate, &index.paths, &index.exact)
}

fn intersects_index(candidate: &Path, paths: &[PathBuf], exact: &HashSet<PathBuf>) -> bool {
    if candidate
        .ancestors()
        .any(|ancestor| exact.contains(ancestor))
    {
        return true;
    }
    let lower = paths.partition_point(|protected| protected.as_path() < candidate);
    lower < paths.len() && paths[lower].starts_with(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_pack_loads_without_duplicates() {
        let pack = ProtectionPack::load_many(&crate::default_rules_dirs()).unwrap();
        assert!(!pack.rules.is_empty());
        assert!(pack.rules.iter().all(|r| !r.paths.is_empty()));
        let ids: HashSet<&str> = pack.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids.len(), pack.rules.len());
    }

    #[test]
    fn host_rule_blocks_its_children() {
        let pack = ProtectionPack::load_many(&crate::default_rules_dirs()).unwrap();
        let rule = pack
            .rules
            .iter()
            .find(|r| r.platform.matches_host())
            .unwrap();
        let root = expand_path_template(&rule.paths[0]).unwrap();
        assert!(is_protected_by_rule(&root.join("important.db")));
        assert!(is_protected_by_rule(root.parent().unwrap()));
    }

    #[test]
    fn host_paths_are_sorted_and_prefix_minimized() {
        let paths = protected_paths().unwrap();
        assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(paths.windows(2).all(|pair| !pair[1].starts_with(&pair[0])));
    }

    #[test]
    fn sibling_sort_order_cannot_hide_a_protected_ancestor() {
        let paths = vec![PathBuf::from("/a"), PathBuf::from("/a-child")];
        let exact = paths.iter().cloned().collect();
        assert!(intersects_index(Path::new("/a/nested"), &paths, &exact));
        assert!(!intersects_index(Path::new("/unrelated"), &paths, &exact));
    }

    #[test]
    fn broken_pack_fails_closed_and_recovers_without_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("_protected-test.yaml");
        fs::write(&file, "protections: [broken").unwrap();
        // A private state so the global keeps its discovered directories.
        let mut state = ProtectionState {
            dirs: Some(vec![dir.path().to_path_buf()]),
            loaded_for: None,
            index: None,
            error: None,
        };
        let broken = index_for(&mut state).unwrap_err();
        assert!(broken.contains("parse"), "{broken}");
        // The failure is cached during the backoff window only.
        assert!(index_for(&mut state).is_err());
        fs::write(
            &file,
            "protections:\n- id: keep\n  name: Keep\n  description: fixture\n  paths: ['$HOME/keep-me']\n",
        )
        .unwrap();
        state.error = Some((broken, Instant::now() - RELOAD_BACKOFF));
        let index = index_for(&mut state).unwrap();
        let keep = crate::safety::fold_case(&expand_path_template("$HOME/keep-me").unwrap());
        assert!(intersects_index(&keep.join("db"), &index.paths, &index.exact));
        assert!(intersects_index(keep.parent().unwrap(), &index.paths, &index.exact));
        assert!(!intersects_index(Path::new("/unrelated/x"), &index.paths, &index.exact));
        // Switching directories drops the index; a missing directory loads
        // an empty pack (nothing protected by rule, not an error).
        state.dirs = Some(vec![dir.path().join("missing")]);
        assert!(index_for(&mut state).unwrap().paths.is_empty());
    }

    #[test]
    fn fail_closed_is_visible_to_scans() {
        // With the real pack the error is absent; the accessor itself is what
        // scans consult, so it must never panic and must agree with the index.
        assert_eq!(load_error().is_none(), protected_paths().is_ok());
    }
}
