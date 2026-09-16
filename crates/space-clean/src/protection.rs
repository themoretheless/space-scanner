//! Data-preservation rules loaded separately from cleanup rules.
//!
//! Protection packs use `_protected-*.yaml`, so the cleanup rule loader ignores
//! them while this module turns their paths into hard deletion blockers.

use crate::rules::{expand_path_template, Platform};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
        paths.into_iter().collect()
    }
}

static HOST_PROTECTED_PATHS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// Paths from the bundled/user protection packs for the current OS.
pub fn protected_paths() -> &'static [PathBuf] {
    HOST_PROTECTED_PATHS
        .get_or_init(|| {
            ProtectionPack::load_many(&crate::default_rules_dirs())
                .map(|pack| pack.host_paths())
                .unwrap_or_default()
        })
        .as_slice()
}

pub fn is_protected_by_rule(path: &Path) -> bool {
    let Ok(candidate) = crate::safety::normalize(path) else {
        return true;
    };
    protected_paths()
        .iter()
        .any(|protected| candidate.as_path() == protected || candidate.starts_with(protected))
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
    }
}
