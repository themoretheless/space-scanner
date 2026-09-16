//! Flexible junk rule manager: prefs overlays, filters, category bulk toggles.

use crate::rules::{DeleteMode, JunkKind, Risk, Rule, RulePack};
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
    /// Tags forced off (any matching tag disables the rule, unless per-rule on).
    #[serde(default)]
    pub tags_off: BTreeSet<String>,
    /// Tags forced on (any matching tag enables the rule, unless category/tag off or per-rule off).
    #[serde(default)]
    pub tags_on: BTreeSet<String>,
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
        if rule.tags.iter().any(|t| self.tags_off.contains(t)) {
            return false;
        }
        if let Some(&forced) = self.rules.get(&rule.id) {
            return forced;
        }
        if self.categories_on.contains(&rule.category) {
            return true;
        }
        if rule.tags.iter().any(|t| self.tags_on.contains(t)) {
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
            || rule
                .tags
                .iter()
                .any(|t| self.tags_off.contains(t) || self.tags_on.contains(t))
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

    pub fn set_tag(&mut self, tag: &str, enabled: bool) {
        let tag = crate::tags::normalize_tag(tag);
        if tag.is_empty() {
            return;
        }
        if enabled {
            self.tags_off.remove(&tag);
            self.tags_on.insert(tag);
        } else {
            self.tags_on.remove(&tag);
            self.tags_off.insert(tag);
        }
    }

    pub fn clear_tag_policy(&mut self, tag: &str) {
        let tag = crate::tags::normalize_tag(tag);
        self.tags_on.remove(&tag);
        self.tags_off.remove(&tag);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Query for listing / managing rules.
#[derive(Debug, Clone)]
pub struct RuleFilter {
    pub query: String,
    pub category: Option<String>,
    /// Require all listed tags (AND). Empty = no tag filter.
    pub tags: Vec<String>,
    pub kind: Option<JunkKind>,
    pub risk: Option<Risk>,
    /// Exclude rules riskier than this (inclusive).
    pub max_risk: Option<Risk>,
    /// `Some(true)` → effectively enabled only; `Some(false)` → disabled only.
    pub enabled: Option<bool>,
    pub report_only: Option<bool>,
    pub host_only: bool,
    pub overridden_only: bool,
}

impl Default for RuleFilter {
    fn default() -> Self {
        Self {
            query: String::new(),
            category: None,
            tags: Vec::new(),
            kind: None,
            risk: None,
            max_risk: None,
            enabled: None,
            report_only: None,
            host_only: true,
            overridden_only: false,
        }
    }
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
        if let Some(kind) = self.kind {
            if rule.kind != kind {
                return false;
            }
        }
        if !self.tags.is_empty() {
            let want: BTreeSet<String> = self
                .tags
                .iter()
                .map(|t| crate::tags::normalize_tag(t))
                .filter(|t| !t.is_empty())
                .collect();
            if !want.iter().all(|t| rule.tags.iter().any(|rt| rt == t)) {
                return false;
            }
        }
        if let Some(risk) = self.risk {
            if rule.risk != risk {
                return false;
            }
        }
        if let Some(max) = self.max_risk {
            if rule.risk.rank() > max.rank() {
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
            let tag_hay = rule.tags.join(" ");
            let hay = format!(
                "{} {} {} {} {} {}",
                rule.id,
                rule.name,
                rule.category,
                rule.kind.as_str(),
                rule.description,
                tag_hay
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
    pub kind: String,
    pub tags: Vec<String>,
    pub risk: String,
    pub danger: String,
    pub platform: String,
    pub default_enabled: bool,
    pub effectively_enabled: bool,
    pub overridden: bool,
    pub report_only: bool,
    pub min_age_days: u64,
    pub paths: Vec<String>,
    pub description: String,
    pub details: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CategoryRow {
    pub name: String,
    pub total: usize,
    pub enabled: usize,
    pub policy: CategoryPolicy,
}

#[derive(Debug, Clone)]
pub struct TagRow {
    pub name: String,
    pub total: usize,
    pub enabled: usize,
    pub policy: CategoryPolicy,
}

#[derive(Debug, Clone)]
pub struct KindRow {
    pub name: String,
    pub total: usize,
    pub enabled: usize,
}

#[derive(Debug, Clone)]
pub struct RiskRow {
    pub name: String,
    pub danger: String,
    pub total: usize,
    pub enabled: usize,
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
                kind: r.kind.as_str().to_string(),
                tags: r.tags.clone(),
                risk: r.risk.as_str().to_string(),
                danger: r.risk.danger_label().to_string(),
                platform: r.platform.as_str().to_string(),
                default_enabled: r.default_enabled,
                effectively_enabled: self.prefs.is_enabled(r),
                overridden: self.prefs.is_overridden(r),
                report_only: r.report_only || r.risk == Risk::ReportOnly,
                min_age_days: self.prefs.min_age_for(r),
                paths: r.paths.clone(),
                description: r.description.clone(),
                details: detailed_rule_description(r, self.prefs.min_age_for(r)),
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

    pub fn tags(&self, host_only: bool) -> Vec<TagRow> {
        let mut totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for r in &self.pack.rules {
            if host_only && !r.platform.matches_host() {
                continue;
            }
            for t in &r.tags {
                let e = totals.entry(t.clone()).or_insert((0, 0));
                e.0 += 1;
                if self.prefs.is_enabled(r) {
                    e.1 += 1;
                }
            }
        }
        totals
            .into_iter()
            .map(|(name, (total, enabled))| {
                let policy = if self.prefs.tags_off.contains(&name) {
                    CategoryPolicy::ForceOff
                } else if self.prefs.tags_on.contains(&name) {
                    CategoryPolicy::ForceOn
                } else {
                    CategoryPolicy::Default
                };
                TagRow {
                    name,
                    total,
                    enabled,
                    policy,
                }
            })
            .collect()
    }

    pub fn kinds(&self, host_only: bool) -> Vec<KindRow> {
        let mut totals: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
        for r in &self.pack.rules {
            if host_only && !r.platform.matches_host() {
                continue;
            }
            let e = totals.entry(r.kind.as_str()).or_insert((0, 0));
            e.0 += 1;
            if self.prefs.is_enabled(r) {
                e.1 += 1;
            }
        }
        totals
            .into_iter()
            .map(|(name, (total, enabled))| KindRow {
                name: name.to_string(),
                total,
                enabled,
            })
            .collect()
    }

    pub fn risks(&self, host_only: bool) -> Vec<RiskRow> {
        let mut totals: BTreeMap<Risk, (usize, usize)> = BTreeMap::new();
        for r in &self.pack.rules {
            if host_only && !r.platform.matches_host() {
                continue;
            }
            let e = totals.entry(r.risk).or_insert((0, 0));
            e.0 += 1;
            if self.prefs.is_enabled(r) {
                e.1 += 1;
            }
        }
        totals
            .into_iter()
            .map(|(risk, (total, enabled))| RiskRow {
                name: risk.as_str().to_string(),
                danger: risk.danger_label().to_string(),
                total,
                enabled,
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

    pub fn set_tag_enabled(&mut self, tag: &str, enabled: bool) -> Result<()> {
        let tag = crate::tags::normalize_tag(tag);
        if tag.is_empty() {
            anyhow::bail!("empty tag");
        }
        let exists = self
            .pack
            .rules
            .iter()
            .any(|r| r.tags.iter().any(|t| t == &tag));
        if !exists {
            anyhow::bail!("unknown tag '{tag}'");
        }
        self.prefs.set_tag(&tag, enabled);
        Ok(())
    }

    pub fn reset_tag(&mut self, tag: &str) {
        self.prefs.clear_tag_policy(tag);
    }

    pub fn reset_all(&mut self) {
        self.prefs.reset();
    }

    /// Rules that should run in a junk scan under current prefs (+ optional category/tag filter).
    pub fn scan_rules(&self, categories: &[String]) -> Vec<&Rule> {
        self.scan_rules_filtered(categories, &[], None, None)
    }

    pub fn scan_rules_filtered(
        &self,
        categories: &[String],
        tags: &[String],
        kind: Option<JunkKind>,
        max_risk: Option<Risk>,
    ) -> Vec<&Rule> {
        let want_tags: BTreeSet<String> = tags
            .iter()
            .map(|t| crate::tags::normalize_tag(t))
            .filter(|t| !t.is_empty())
            .collect();
        self.pack
            .rules
            .iter()
            .filter(|r| r.platform.matches_host())
            .filter(|r| self.prefs.is_enabled(r))
            .filter(|r| {
                self.prefs.include_report_only
                    || !(r.report_only || matches!(r.risk, Risk::ReportOnly))
            })
            .filter(|r| {
                if categories.is_empty() {
                    true
                } else {
                    categories.iter().any(|c| c == &r.category || c == &r.id)
                }
            })
            .filter(|r| kind.map(|k| r.kind == k).unwrap_or(true))
            .filter(|r| {
                max_risk
                    .map(|max| r.risk.rank() <= max.rank())
                    .unwrap_or(true)
            })
            .filter(|r| {
                if want_tags.is_empty() {
                    true
                } else {
                    want_tags.iter().all(|t| r.tags.iter().any(|rt| rt == t))
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
            tags: self.tags(true).len(),
            kinds: self.kinds(true).len(),
        }
    }
}

fn detailed_rule_description(rule: &Rule, min_age_days: u64) -> Vec<String> {
    let report_only = rule.report_only || rule.risk == Risk::ReportOnly;
    let action = if report_only {
        "Action: information only. Matches cannot be selected for automatic cleanup.".to_string()
    } else {
        match rule.delete_mode {
            DeleteMode::Contents => {
                "Action: moves matching entries to Trash while preserving the configured root folder."
                    .to_string()
            }
            DeleteMode::Path => {
                "Action: moves the matched file or directory, including its contents, to Trash."
                    .to_string()
            }
        }
    };
    let consequence = match rule.kind {
        JunkKind::Cache | JunkKind::Browser => {
            "After cleanup: applications may start more slowly once while caches are rebuilt."
        }
        JunkKind::Logs => {
            "After cleanup: historical diagnostic logs are lost; applications continue to create new logs."
        }
        JunkKind::Temp => {
            "After cleanup: stale temporary data is removed; active or recently changed entries are protected by the age rule."
        }
        JunkKind::Downloads | JunkKind::Packages => {
            "After cleanup: required packages or downloads may need to be downloaded again."
        }
        JunkKind::Leftovers => {
            "After cleanup: residual data from removed or changed applications is discarded."
        }
        JunkKind::Dev => {
            "After cleanup: generated build/dependency artifacts must be recreated by the next build or install."
        }
        JunkKind::System => {
            "After cleanup: macOS may recreate the data; protected system and user paths remain blocked."
        }
        JunkKind::AppData => {
            "Warning: matches may contain application state or settings; review every path before cleanup."
        }
        JunkKind::Media => {
            "Warning: matches may be previews, derived media, or original assets depending on the application."
        }
        JunkKind::Databases => {
            "Warning: database matches may contain unique user data and are therefore normally report-only."
        }
        JunkKind::Models => {
            "After cleanup: model files must be downloaded again before the related tool can run."
        }
        JunkKind::Vm => {
            "Warning: virtual-machine data may contain complete operating systems and unique files."
        }
        JunkKind::Other => {
            "After cleanup: inspect the matched path and the rule-specific description before proceeding."
        }
    };
    let age = if min_age_days == 0 {
        "Age filter: none; every matching entry is eligible.".to_string()
    } else {
        format!("Age filter: entries changed within the last {min_age_days} day(s) are preserved.")
    };
    let mut guards = Vec::new();
    if !rule.required_ancestor_globs.is_empty() {
        guards.push(format!(
            "requires project marker: {}",
            rule.required_ancestor_globs.join(", ")
        ));
    }
    if !rule.exclude_globs.is_empty() {
        guards.push(format!("{} exclusion pattern(s)", rule.exclude_globs.len()));
    }
    let guard_text = if guards.is_empty() {
        format!("Scope: {} configured root path(s).", rule.paths.len())
    } else {
        format!(
            "Scope: {} configured root path(s); {}.",
            rule.paths.len(),
            guards.join("; ")
        )
    };

    vec![action, consequence.to_string(), age, guard_text]
}

#[derive(Debug, Clone, Copy)]
pub struct ManagerStats {
    pub total: usize,
    pub host: usize,
    pub enabled: usize,
    pub disabled: usize,
    pub overridden: usize,
    pub categories: usize,
    pub tags: usize,
    pub kinds: usize,
}
