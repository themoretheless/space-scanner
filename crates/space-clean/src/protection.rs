//! Data-preservation rules loaded separately from cleanup rules.
//!
//! Protection packs use `_protected-*.yaml`, so the cleanup rule loader ignores
//! them while this module turns their paths into hard deletion blockers.

use crate::rules::{expand_path_template, Platform};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use walkdir::WalkDir;

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
            for entry in WalkDir::new(dir).follow_links(false) {
                let entry = entry?;
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let is_yaml = matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("yaml" | "yml")
                );
                if is_yaml && name.starts_with("_protected-") {
                    files.insert(fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()));
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

struct ProtectedIndex {
    paths: Vec<PathBuf>,
    exact: HashSet<PathBuf>,
}

static HOST_PROTECTED_PATHS: OnceLock<Result<ProtectedIndex, String>> = OnceLock::new();

fn protected_index() -> Result<&'static ProtectedIndex, &'static str> {
    match HOST_PROTECTED_PATHS.get_or_init(|| {
        ProtectionPack::load_many(&crate::default_rules_dirs())
            .map(|pack| {
                let paths = pack.host_paths();
                let exact = paths.iter().cloned().collect();
                ProtectedIndex { paths, exact }
            })
            .map_err(|error| error.to_string())
    }) {
        Ok(index) => Ok(index),
        Err(error) => Err(error.as_str()),
    }
}

/// Paths from the bundled/user protection packs for the current OS.
pub fn protected_paths() -> Result<&'static [PathBuf], &'static str> {
    protected_index().map(|index| index.paths.as_slice())
}

pub fn is_protected_by_rule(path: &Path) -> bool {
    let Ok(candidate) = crate::safety::normalize(path) else {
        return true;
    };
    is_protected_normalized(&candidate)
}

/// Checks both directions: a protected ancestor blocks its descendants, and a
/// protected descendant blocks deleting an ancestor that would contain it.
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
    fn bundled_pack_contains_exactly_one_thousand_rules() {
        let pack = ProtectionPack::load_many(&crate::default_rules_dirs()).unwrap();
        assert_eq!(pack.rules.len(), 1_000);
        assert!(pack.rules.iter().all(|r| !r.paths.is_empty()));
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
}
