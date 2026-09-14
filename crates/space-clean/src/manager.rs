//! Flexible junk rule manager: prefs overlays, filters, category bulk toggles.

use crate::rules::{Risk, Rule, RulePack};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Persisted user overrides on top of YAML `default_enabled`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RulePrefs {
    /// Per-rule force on/off. Absent → use YAML `default_enabled` (or category policy).
    #[serde(default)]
    pub rules: BTreeMap<String, bool>,
    /// Categories forced off (wins over per-rule on).
    #[serde(default)]
    pub categories_off: BTreeSet<String>,
    /// Categories forced on (all host rules in category, unless per-rule off).
    #[serde(default)]
    pub categories_on: BTreeSet<String>,
    /// Per-rule min_age_days override.
    #[serde(default)]
    pub min_age: BTreeMap<String, u64>,
    /// When true, treat every rule as default-on before overlays.
    #[serde(default)]
    pub enable_all_by_default: bool,
    /// Include `report_only` rules in scans (still not deleted by clean).
    #[serde(default)]
    pub include_report_only: bool,
}

impl RulePrefs {
    pub fn default_path() -> Option<PathBuf> {
        let base = directories::ProjectDirs::from("com", "themoretheless", "SpaceScanner")?;
        Some(base.config_dir().join("rule-prefs.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::default_path() else {
            return Self::default();
        };
        Self::load_from(&path).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn save(&self) -> Result<()> {
        let Some(path) = Self::default_path() else {
            anyhow::bail!("no config dir");
        };
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self)?;
        fs::write(path, bytes).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    pub fn is_enabled(&self, rule: &Rule) -> bool {
        if self.categories_off.contains(&rule.category) {
            return false;
        }
        if let Some(&forced) = self.rules.get(&rule.id) {
            return forced;
        }
        if self.categories_on.contains(&rule.category) {
            return true;
        }
        if self.enable_all_by_default {
            return true;
        }
        rule.default_enabled
    }

    pub fn is_overridden(&self, rule: &Rule) -> bool {
        self.rules.contains_key(&rule.id)
            || self.categories_off.contains(&rule.category)
            || self.categories_on.contains(&rule.category)
            || self.enable_all_by_default
            || self.min_age.contains_key(&rule.id)
    }

    pub fn min_age_for(&self, rule: &Rule) -> u64 {
        self.min_age
            .get(&rule.id)
            .copied()
            .unwrap_or(rule.min_age_days)
    }

    pub fn set_rule(&mut self, id: impl Into<String>, enabled: bool) {
        self.rules.insert(id.into(), enabled);
    }

    pub fn clear_rule(&mut self, id: &str) {
        self.rules.remove(id);
    }

    pub fn set_category(&mut self, category: &str, enabled: bool) {
        if enabled {
            self.categories_off.remove(category);
            self.categories_on.insert(category.to_string());
            // Drop per-rule offs inside this category? Keep them — more flexible.
        } else {
            self.categories_on.remove(category);
            self.categories_off.insert(category.to_string());
        }
    }

    pub fn clear_category_policy(&mut self, category: &str) {
        self.categories_on.remove(category);
        self.categories_off.remove(category);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Query for listing / managing rules.
#[derive(Debug, Clone, Default)]
pub struct RuleFilter {
    pub query: String,
    pub category: Option<String>,
    pub risk: Option<Risk>,
    /// `Some(true)` → effectively enabled only; `Some(false)` → disabled only.
    pub enabled: Option<bool>,
    pub report_only: Option<bool>,
    pub host_only: bool,
    pub overridden_only: bool,
}

impl RuleFilter {
    pub fn matches(&self, rule: &Rule, prefs: &RulePrefs) -> bool {
        if self.host_only && !rule.platform.matches_host() {
            return false;
        }
        if let Some(cat) = &self.category {
            if &rule.category != cat && &rule.id != cat {
                return false;
            }
        }
        if let Some(risk) = self.risk {
            if rule.risk != risk {
                return false;
            }
        }
        if let Some(ro) = self.report_only {
            let is_ro = rule.report_only || rule.risk == Risk::ReportOnly;
            if is_ro != ro {
                return false;
            }
        }
        let on = prefs.is_enabled(rule);
        if let Some(want) = self.enabled {
            if on != want {
                return false;
            }
        }
        if self.overridden_only && !prefs.is_overridden(rule) {
            return false;
        }
        let q = self.query.trim().to_ascii_lowercase();
        if !q.is_empty() {
            let hay = format!(
                "{} {} {} {}",
                rule.id, rule.name, rule.category, rule.description
            )
            .to_ascii_lowercase();
            if !hay.contains(&q) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
pub struct RuleRow {
    pub id: String,
    pub name: String,
    pub category: String,
    pub risk: String,
    pub platform: String,
    pub default_enabled: bool,
    pub effectively_enabled: bool,
    pub overridden: bool,
    pub report_only: bool,
    pub min_age_days: u64,
    pub paths: Vec<String>,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct CategoryRow {
    pub name: String,
    pub total: usize,
    pub enabled: usize,
    pub policy: CategoryPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryPolicy {
    Default,
    ForceOn,
    ForceOff,
}

/// Pack + prefs: list, filter, toggle, feed scans.
#[derive(Debug, Clone)]
pub struct RuleManager {
    pub pack: RulePack,
    pub prefs: RulePrefs,
}

impl RuleManager {
    pub fn load_default() -> Result<Self> {
        Ok(Self {
            pack: crate::load_default_pack()?,
            prefs: RulePrefs::load(),
        })
    }

    pub fn save_prefs(&self) -> Result<()> {
        self.prefs.save()
    }

    pub fn effective_enabled(&self, rule: &Rule) -> bool {
        self.prefs.is_enabled(rule)
    }

    pub fn list(&self, filter: &RuleFilter) -> Vec<RuleRow> {
        self.pack
            .rules
            .iter()
            .filter(|r| filter.matches(r, &self.prefs))
            .map(|r| RuleRow {
                id: r.id.clone(),
                name: r.name.clone(),
                category: r.category.clone(),
                risk: r.risk.as_str().to_string(),
                platform: r.platform.as_str().to_string(),
                default_enabled: r.default_enabled,
                effectively_enabled: self.prefs.is_enabled(r),
                overridden: self.prefs.is_overridden(r),
                report_only: r.report_only || r.risk == Risk::ReportOnly,
                min_age_days: self.prefs.min_age_for(r),
                paths: r.paths.clone(),
                description: r.description.clone(),
            })
            .collect()
    }

    pub fn categories(&self, host_only: bool) -> Vec<CategoryRow> {
        let mut totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for r in &self.pack.rules {
            if host_only && !r.platform.matches_host() {
                continue;
            }
            let e = totals.entry(r.category.clone()).or_insert((0, 0));
            e.0 += 1;
            if self.prefs.is_enabled(r) {
                e.1 += 1;
            }
        }
        totals
            .into_iter()
            .map(|(name, (total, enabled))| {
                let policy = if self.prefs.categories_off.contains(&name) {
                    CategoryPolicy::ForceOff
                } else if self.prefs.categories_on.contains(&name) {
                    CategoryPolicy::ForceOn
                } else {
                    CategoryPolicy::Default
                };
                CategoryRow {
                    name,
                    total,
                    enabled,
                    policy,
                }
            })
            .collect()
    }

    pub fn set_rule_enabled(&mut self, id: &str, enabled: bool) -> Result<()> {
        if self.pack.get(id).is_none() {
            anyhow::bail!("unknown rule id '{id}'");
        }
        self.prefs.set_rule(id, enabled);
        Ok(())
    }

    pub fn reset_rule(&mut self, id: &str) {
        self.prefs.clear_rule(id);
    }

    pub fn set_category_enabled(&mut self, category: &str, enabled: bool) -> Result<()> {
        let exists = self.pack.rules.iter().any(|r| r.category == category);
        if !exists {
            anyhow::bail!("unknown category '{category}'");
        }
        self.prefs.set_category(category, enabled);
        Ok(())
    }

    pub fn reset_category(&mut self, category: &str) {
        self.prefs.clear_category_policy(category);
        let ids: Vec<String> = self
            .pack
            .rules
            .iter()
            .filter(|r| r.category == category)
            .map(|r| r.id.clone())
            .collect();
        for id in ids {
            self.prefs.clear_rule(&id);
        }
    }

    pub fn reset_all(&mut self) {
        self.prefs.reset();
    }

    /// Rules that should run in a junk scan under current prefs (+ optional category filter).
    pub fn scan_rules(&self, categories: &[String]) -> Vec<&Rule> {
        self.pack
            .rules
            .iter()
            .filter(|r| r.platform.matches_host())
            .filter(|r| self.prefs.is_enabled(r))
            .filter(|r| {
                if categories.is_empty() {
                    true
                } else {
                    categories
                        .iter()
                        .any(|c| c == &r.category || c == &r.id)
                }
            })
            .collect()
    }

    pub fn stats(&self) -> ManagerStats {
        let host: Vec<_> = self
            .pack
            .rules
            .iter()
            .filter(|r| r.platform.matches_host())
            .collect();
        let enabled = host.iter().filter(|r| self.prefs.is_enabled(r)).count();
        let overridden = host.iter().filter(|r| self.prefs.is_overridden(r)).count();
        ManagerStats {
            total: self.pack.rules.len(),
            host: host.len(),
            enabled,
            disabled: host.len().saturating_sub(enabled),
            overridden,
            categories: self.categories(true).len(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ManagerStats {
    pub total: usize,
    pub host: usize,
    pub enabled: usize,
    pub disabled: usize,
    pub overridden: usize,
    pub categories: usize,
}
