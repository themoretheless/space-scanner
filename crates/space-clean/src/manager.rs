//! Flexible junk rule manager: prefs overlays, filters, category bulk toggles.

use crate::rules::{DeleteMode, JunkKind, Risk, Rule, RuleIndex, RulePack};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Why saved preferences could not be used.
#[derive(Debug, thiserror::Error)]
pub enum PrefsError {
    /// The file exists but is not valid JSON for [`RulePrefs`]; it was moved
    /// aside so the next save does not clobber it.
    #[error("rule preferences at {path} are corrupt (moved to {backup}): {source}")]
    Corrupt {
        path: PathBuf,
        backup: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("read rule preferences at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Persisted user overrides on top of YAML `default_enabled`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulePrefs {
    /// Per-rule force on/off. Absent → use YAML `default_enabled` (or category policy).
    #[serde(default)]
    pub rules: BTreeMap<String, bool>,
    /// Categories forced off unless a rule has an explicit override.
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
    /// Preserve overrides after catalog deduplication. When old copies conflict,
    /// disabling wins and the longest age threshold wins.
    fn migrate_aliases(&mut self, aliases: &BTreeMap<String, String>) {
        for (old, canonical) in aliases {
            if old == canonical {
                continue;
            }
            if let Some(enabled) = self.rules.remove(old) {
                self.rules
                    .entry(canonical.clone())
                    .and_modify(|current| *current &= enabled)
                    .or_insert(enabled);
            }
            if let Some(age) = self.min_age.remove(old) {
                self.min_age
                    .entry(canonical.clone())
                    .and_modify(|current| *current = (*current).max(age))
                    .or_insert(age);
            }
        }
    }

    pub fn migrate_catalog_aliases(&mut self, dirs: &[PathBuf]) -> Result<()> {
        for dir in dirs {
            let path = dir.join("_rule-aliases.json");
            if path.is_file() {
                let aliases = serde_json::from_slice(&fs::read(&path)?)
                    .with_context(|| format!("parse {}", path.display()))?;
                self.migrate_aliases(&aliases);
            }
        }
        Ok(())
    }

    pub fn default_path() -> Option<PathBuf> {
        Some(crate::rules::app_dirs()?.config_dir().join("rule-prefs.json"))
    }

    /// Load saved preferences, falling back to defaults. A corrupt file is
    /// moved aside and reported on stderr; use [`RulePrefs::try_load`] to
    /// surface that as an error instead.
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(prefs) => prefs,
            Err(error) => {
                eprintln!("warning: {error}");
                Self::default()
            }
        }
    }

    /// Load saved preferences. Missing file or config dir → defaults; a
    /// corrupt file → [`PrefsError::Corrupt`] after renaming it to `*.bad`.
    pub fn try_load() -> Result<Self, PrefsError> {
        let Some(path) = Self::default_path() else {
            return Ok(Self::default());
        };
        Self::try_load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        Ok(Self::try_load_from(path)?)
    }

    pub fn try_load_from(path: &Path) -> Result<Self, PrefsError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(source) => {
                return Err(PrefsError::Io {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(prefs) => Ok(prefs),
            Err(source) => {
                let backup = path.with_extension("json.bad");
                let _ = fs::rename(path, &backup);
                Err(PrefsError::Corrupt {
                    path: path.to_path_buf(),
                    backup,
                    source,
                })
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let Some(path) = Self::default_path() else {
            anyhow::bail!("no config dir");
        };
        self.save_to(&path)
    }

    /// Atomic: the file is written next to `path` and renamed into place.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        crate::rules::write_atomic(path, &bytes)
            .with_context(|| format!("write {}", path.display()))
    }

    pub fn is_enabled(&self, rule: &Rule) -> bool {
        if let Some(&forced) = self.rules.get(&rule.id) {
            return forced;
        }
        if self.categories_off.contains(&rule.category) {
            return false;
        }
        if rule.tags.iter().any(|t| self.tags_off.contains(t)) {
            return false;
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
        self.prepared()
            .matches_prepared(rule, prefs, &mut String::new())
    }

    fn prepared(&self) -> Self {
        let mut filter = self.clone();
        filter.query = self.query.trim().to_ascii_lowercase();
        filter.tags = self
            .tags
            .iter()
            .map(|tag| crate::tags::normalize_tag(tag))
            .filter(|tag| !tag.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        filter
    }

    fn matches_prepared(&self, rule: &Rule, prefs: &RulePrefs, text: &mut String) -> bool {
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
        if !self
            .tags
            .iter()
            .all(|tag| rule.tags.iter().any(|rule_tag| rule_tag == tag))
        {
            return false;
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
            if rule.is_report_only() != ro {
                return false;
            }
        }
        if let Some(want) = self.enabled {
            if prefs.is_enabled(rule) != want {
                return false;
            }
        }
        if self.overridden_only && !prefs.is_overridden(rule) {
            return false;
        }
        if !self.query.is_empty() {
            text.clear();
            for field in [
                &rule.id,
                &rule.name,
                &rule.category,
                rule.kind.as_str(),
                &rule.description,
            ] {
                text.push_str(field);
                text.push(' ');
            }
            for (index, tag) in rule.tags.iter().enumerate() {
                if index > 0 {
                    text.push(' ');
                }
                text.push_str(tag);
            }
            text.make_ascii_lowercase();
            if !text.contains(&self.query) {
                return false;
            }
        }
        true
    }

    /// Rules a scan should run: this filter plus an OR-list of categories /
    /// rule ids, evaluated over the manager's host rules. The single place
    /// that turns preferences into a scan set.
    pub fn scan_set<'a>(&self, manager: &'a RuleManager, categories: &[String]) -> Vec<&'a Rule> {
        let filter = self.prepared();
        let mut text = String::new();
        let index = manager.index();
        let pack = &manager.pack;
        let mut rules: Vec<&Rule> = Vec::new();
        if categories.is_empty() {
            if filter.host_only {
                rules.extend(index.host_positions().iter().map(|&i| &pack.rules[i]));
            } else {
                rules.extend(pack.rules.iter());
            }
        } else {
            let mut positions = BTreeSet::new();
            for selector in categories {
                positions.extend(index.category_positions(selector).iter().copied());
                if let Some(position) = index.position(pack, selector) {
                    positions.insert(position);
                }
            }
            rules.extend(positions.into_iter().map(|i| &pack.rules[i]));
        }
        rules.retain(|rule| filter.matches_prepared(rule, &manager.prefs, &mut text));
        rules
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
    pub pack: std::sync::Arc<RulePack>,
    pub prefs: RulePrefs,
}

impl RuleManager {
    pub fn load_default() -> Result<Self> {
        Self::load_with_dirs(&crate::default_rules_dirs())
    }

    /// Load the catalog from explicit rule directories (CLI `--rules`). The
    /// protection pack is taken from the same directories.
    pub fn load_with_dirs(dirs: &[PathBuf]) -> Result<Self> {
        crate::protection::set_rules_dirs(dirs);
        let pack = RulePack::load_many_shared(dirs)?;
        let mut prefs = RulePrefs::load();
        prefs.migrate_catalog_aliases(dirs)?;
        Ok(Self { pack, prefs })
    }

    /// Lookup tables for this catalog (built once per shared pack).
    pub fn index(&self) -> Arc<RuleIndex> {
        self.pack.index()
    }

    pub fn save_prefs(&self) -> Result<()> {
        self.prefs.save()
    }

    pub fn effective_enabled(&self, rule: &Rule) -> bool {
        self.prefs.is_enabled(rule)
    }

    pub fn list(&self, filter: &RuleFilter) -> Vec<RuleRow> {
        self.list_page(filter, usize::MAX).1
    }

    /// Count every match but allocate display rows only for the visible page.
    pub fn list_page(&self, filter: &RuleFilter, limit: usize) -> (usize, Vec<RuleRow>) {
        let mut total = 0;
        let mut rows = Vec::new();
        let filter = filter.prepared();
        let mut text = String::new();
        let index = self.index();
        let mut consider = |rule: &Rule| {
            if filter.matches_prepared(rule, &self.prefs, &mut text) {
                total += 1;
                if rows.len() < limit {
                    rows.push(self.row_for(rule));
                }
            }
        };
        if filter.host_only {
            for &position in index.host_positions() {
                consider(&self.pack.rules[position]);
            }
        } else {
            for rule in &self.pack.rules {
                consider(rule);
            }
        }
        (total, rows)
    }

    /// [`Self::list_page`] narrowed by `keep` and ordered by `rank`, largest
    /// first (ties keep catalog order). Display rows are built only for the
    /// visible page, so the rules window can sort thousands of rules by size.
    pub fn list_page_ranked(
        &self,
        filter: &RuleFilter,
        limit: usize,
        keep: impl Fn(&Rule) -> bool,
        rank: impl Fn(&Rule) -> u64,
    ) -> (usize, Vec<RuleRow>) {
        let filter = filter.prepared();
        let mut text = String::new();
        let index = self.index();
        let mut matches: Vec<&Rule> = Vec::new();
        let mut consider = |rule: &'_ Rule| -> bool {
            filter.matches_prepared(rule, &self.prefs, &mut text) && keep(rule)
        };
        if filter.host_only {
            for &position in index.host_positions() {
                let rule = &self.pack.rules[position];
                if consider(rule) {
                    matches.push(rule);
                }
            }
        } else {
            for rule in &self.pack.rules {
                if consider(rule) {
                    matches.push(rule);
                }
            }
        }
        matches.sort_by_key(|rule| std::cmp::Reverse(rank(rule)));
        let rows = matches.iter().take(limit).map(|rule| self.row_for(rule)).collect();
        (matches.len(), rows)
    }

    fn row_for(&self, r: &Rule) -> RuleRow {
        RuleRow {
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
            report_only: r.is_report_only(),
            min_age_days: self.prefs.min_age_for(r),
            paths: r.paths.clone(),
            description: r.description.clone(),
            details: detailed_rule_description(r, self.prefs.min_age_for(r)),
        }
    }

    /// Aggregate borrowed catalog keys and evaluate preferences once per rule.
    fn facet_totals<'a, K: Ord, I: IntoIterator<Item = K>>(
        &'a self,
        host_only: bool,
        keys: impl Fn(&'a Rule) -> I,
    ) -> BTreeMap<K, (usize, usize)> {
        let mut totals = BTreeMap::new();
        let index = self.index();
        let mut tally = |rule: &'a Rule| {
            let enabled = usize::from(self.prefs.is_enabled(rule));
            for key in keys(rule) {
                let counts = totals.entry(key).or_insert((0, 0));
                counts.0 += 1;
                counts.1 += enabled;
            }
        };
        if host_only {
            for &position in index.host_positions() {
                tally(&self.pack.rules[position]);
            }
        } else {
            for rule in &self.pack.rules {
                tally(rule);
            }
        }
        totals
    }

    pub fn categories(&self, host_only: bool) -> Vec<CategoryRow> {
        self.facet_totals(host_only, |rule| std::iter::once(rule.category.as_str()))
            .into_iter()
            .map(|(name, (total, enabled))| CategoryRow {
                name: name.to_string(),
                total,
                enabled,
                policy: category_policy(
                    name,
                    &self.prefs.categories_off,
                    &self.prefs.categories_on,
                ),
            })
            .collect()
    }

    pub fn tags(&self, host_only: bool) -> Vec<TagRow> {
        self.facet_totals(host_only, |rule| rule.tags.iter().map(String::as_str))
            .into_iter()
            .map(|(name, (total, enabled))| TagRow {
                name: name.to_string(),
                total,
                enabled,
                policy: category_policy(name, &self.prefs.tags_off, &self.prefs.tags_on),
            })
            .collect()
    }

    pub fn kinds(&self, host_only: bool) -> Vec<KindRow> {
        self.facet_totals(host_only, |rule| std::iter::once(rule.kind.as_str()))
            .into_iter()
            .map(|(name, (total, enabled))| KindRow {
                name: name.to_string(),
                total,
                enabled,
            })
            .collect()
    }

    pub fn risks(&self, host_only: bool) -> Vec<RiskRow> {
        self.facet_totals(host_only, |rule| std::iter::once(rule.risk))
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
        if self.index().position(&self.pack, id).is_none() {
            anyhow::bail!("unknown rule id '{id}'");
        }
        self.prefs.set_rule(id, enabled);
        Ok(())
    }

    pub fn reset_rule(&mut self, id: &str) {
        self.prefs.clear_rule(id);
    }

    pub fn set_category_enabled(&mut self, category: &str, enabled: bool) -> Result<()> {
        if !self.index().has_category(category) {
            anyhow::bail!("unknown category '{category}'");
        }
        self.prefs.set_category(category, enabled);
        Ok(())
    }

    pub fn reset_category(&mut self, category: &str) {
        self.prefs.clear_category_policy(category);
        let index = self.index();
        for &position in index.category_positions(category) {
            self.prefs.clear_rule(&self.pack.rules[position].id);
        }
    }

    pub fn set_tag_enabled(&mut self, tag: &str, enabled: bool) -> Result<()> {
        let tag = crate::tags::normalize_tag(tag);
        if tag.is_empty() {
            anyhow::bail!("empty tag");
        }
        if !self.index().has_tag(&tag) {
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
        RuleFilter {
            tags: tags.to_vec(),
            kind,
            max_risk,
            enabled: Some(true),
            report_only: (!self.prefs.include_report_only).then_some(false),
            host_only: true,
            ..Default::default()
        }
        .scan_set(self, categories)
    }

    /// Rules for an "everything" scan (`junk --all`): default-off rules and
    /// category / tag policies are ignored, but an explicit per-rule "off"
    /// stays off, report-only rules are listed only with
    /// `include_report_only`, and without an explicit `max_risk` the set is
    /// capped at [`Risk::Low`] so a cleanup from it cannot touch risky rules.
    pub fn scan_rules_unrestricted(
        &self,
        categories: &[String],
        tags: &[String],
        kind: Option<JunkKind>,
        risk: Option<Risk>,
        max_risk: Option<Risk>,
    ) -> Vec<&Rule> {
        let cap = max_risk.unwrap_or(Risk::Low);
        let mut rules = RuleFilter {
            tags: tags.to_vec(),
            kind,
            risk,
            report_only: (!self.prefs.include_report_only).then_some(false),
            host_only: true,
            ..Default::default()
        }
        .scan_set(self, categories);
        // Report-only rules are never cleaned, so the cap does not apply to them.
        rules.retain(|rule| {
            self.prefs.rules.get(&rule.id) != Some(&false)
                && (rule.is_report_only() || rule.risk.rank() <= cap.rank())
        });
        rules
    }

    pub fn stats(&self) -> ManagerStats {
        let index = self.index();
        let mut stats = ManagerStats {
            total: self.pack.rules.len(),
            host: index.host_positions().len(),
            enabled: 0,
            disabled: 0,
            overridden: 0,
            categories: index.host_category_count(),
            tags: index.host_tag_count(),
            kinds: index.host_kind_count(),
        };
        for &position in index.host_positions() {
            let rule = &self.pack.rules[position];
            stats.enabled += usize::from(self.prefs.is_enabled(rule));
            stats.overridden += usize::from(self.prefs.is_overridden(rule));
        }
        stats.disabled = stats.host - stats.enabled;
        stats
    }
}

fn category_policy(
    name: &str,
    forced_off: &BTreeSet<String>,
    forced_on: &BTreeSet<String>,
) -> CategoryPolicy {
    if forced_off.contains(name) {
        CategoryPolicy::ForceOff
    } else if forced_on.contains(name) {
        CategoryPolicy::ForceOn
    } else {
        CategoryPolicy::Default
    }
}

fn detailed_rule_description(rule: &Rule, min_age_days: u64) -> Vec<String> {
    let action = if rule.is_report_only() {
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

#[cfg(test)]
mod alias_tests {
    use super::*;

    #[test]
    fn ranked_page_keeps_selected_rules_and_puts_largest_first() {
        let manager = facet_fixture();
        let filter = RuleFilter { host_only: true, ..Default::default() };
        let size = |rule: &Rule| match rule.id.as_str() {
            "c" => 30,
            "b" => 20,
            _ => 0,
        };
        let (total, rows) = manager.list_page_ranked(&filter, 2, |rule| rule.id != "d", size);
        assert_eq!(total, 3);
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["c", "b"]);
        let (_, all) = manager.list_page_ranked(&filter, 10, |_| true, |_| 0);
        let (_, plain) = manager.list_page(&filter, 10);
        assert_eq!(
            all.iter().map(|r| &r.id).collect::<Vec<_>>(),
            plain.iter().map(|r| &r.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn individual_toggle_overrides_disabled_groups_and_survives_reload() {
        let mut manager = facet_fixture();
        manager.prefs.set_category("cache", false);
        manager.prefs.set_tag("alpha", false);
        let filter = RuleFilter { enabled: Some(false), host_only: true, ..Default::default() };
        assert!(manager.list_page(&filter, 100).1.iter().any(|r| r.id == "a"));
        manager.set_rule_enabled("a", true).unwrap();
        assert!(!manager.list_page(&filter, 100).1.iter().any(|r| r.id == "a"));
        assert!(manager.list_page(&RuleFilter { enabled: Some(true), ..filter.clone() }, 100).1.iter().any(|r| r.id == "a"));
        let root = std::env::temp_dir().join(format!("space-rule-prefs-{}", std::process::id()));
        let path = root.join("prefs.json");
        manager.prefs.save_to(&path).unwrap();
        manager.prefs = RulePrefs::load_from(&path).unwrap();
        assert!(manager.prefs.is_enabled(manager.pack.get("a").unwrap()));
        manager.set_rule_enabled("a", false).unwrap();
        manager.prefs.save_to(&path).unwrap();
        manager.prefs = RulePrefs::load_from(&path).unwrap();
        assert!(!manager.prefs.is_enabled(manager.pack.get("a").unwrap()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enabling_category_preserves_exceptions_and_reset_restores_inheritance() {
        let mut manager = facet_fixture();
        manager.set_category_enabled("cache", false).unwrap();
        manager.set_rule_enabled("a", true).unwrap();
        let category = manager.categories(true).into_iter().find(|c| c.name == "cache").unwrap();
        assert_eq!((category.enabled, category.total), (1, 2));
        assert_eq!(category.policy, CategoryPolicy::ForceOff);
        manager.set_rule_enabled("c", false).unwrap();
        manager.set_category_enabled("cache", true).unwrap();
        assert!(manager.prefs.is_enabled(manager.pack.get("a").unwrap()));
        assert!(!manager.prefs.is_enabled(manager.pack.get("c").unwrap()));
        manager.reset_rule("c");
        assert!(manager.prefs.is_enabled(manager.pack.get("c").unwrap()));
        assert_eq!(manager.prefs.rules.get("a"), Some(&true));
        assert!(!manager.prefs.categories_off.contains("cache"));
    }

    fn facet_fixture() -> RuleManager {
        let foreign = if cfg!(target_os = "windows") {
            "linux"
        } else {
            "windows"
        };
        let rules = serde_json::from_value(serde_json::json!([
            {"id": "a", "name": "A", "paths": [], "category": "cache", "kind": "cache",
             "risk": "low", "default_enabled": true, "tags": ["common", "alpha"]},
            {"id": "b", "name": "B", "paths": [], "category": "logs", "kind": "logs",
             "risk": "medium", "default_enabled": false, "tags": ["common", "beta"]},
            {"id": "c", "name": "C", "paths": [], "category": "cache", "kind": "cache",
             "risk": "high", "default_enabled": true, "tags": ["beta"]},
            {"id": "d", "name": "D", "paths": [], "category": "other", "kind": "other",
             "risk": "report-only", "default_enabled": true, "tags": []},
            {"id": "e", "name": "E", "paths": [], "category": "foreign", "kind": "models",
             "platform": foreign, "risk": "low", "default_enabled": true,
             "tags": ["foreign"]}
        ]))
        .unwrap();
        RuleManager {
            pack: std::sync::Arc::new(RulePack {
                rules,
                sources: vec![],
            }),
            prefs: RulePrefs::default(),
        }
    }

    #[test]
    fn facets_preserve_sorted_counts_and_host_filtering() {
        let manager = facet_fixture();
        let categories = manager.categories(true);
        assert_eq!(
            categories
                .iter()
                .map(|r| (r.name.as_str(), r.total, r.enabled, r.policy))
                .collect::<Vec<_>>(),
            vec![
                ("cache", 2, 2, CategoryPolicy::Default),
                ("logs", 1, 0, CategoryPolicy::Default),
                ("other", 1, 1, CategoryPolicy::Default),
            ]
        );
        let tags = manager.tags(true);
        assert_eq!(
            tags.iter()
                .map(|r| (r.name.as_str(), r.total, r.enabled))
                .collect::<Vec<_>>(),
            vec![("alpha", 1, 1), ("beta", 2, 1), ("common", 2, 1)]
        );
        let kinds = manager.kinds(true);
        assert_eq!(
            kinds
                .iter()
                .map(|r| (r.name.as_str(), r.total, r.enabled))
                .collect::<Vec<_>>(),
            vec![("cache", 2, 2), ("logs", 1, 0), ("other", 1, 1)]
        );
        let risks = manager.risks(true);
        assert_eq!(
            risks
                .iter()
                .map(|r| (r.name.as_str(), r.total, r.enabled))
                .collect::<Vec<_>>(),
            vec![
                ("low", 1, 1),
                ("medium", 1, 0),
                ("high", 1, 1),
                ("report-only", 1, 1),
            ]
        );
        assert_eq!(manager.categories(false).len(), 4);
        assert_eq!(manager.tags(false).len(), 4);
        assert_eq!(manager.kinds(false).len(), 4);
        assert_eq!(manager.risks(false)[0].total, 2);
        let stats = manager.stats();
        assert_eq!(
            (stats.total, stats.host, stats.enabled, stats.disabled),
            (5, 4, 3, 1)
        );
        assert_eq!(
            (stats.overridden, stats.categories, stats.tags, stats.kinds),
            (0, 3, 3, 3)
        );
    }

    #[test]
    fn facet_preferences_preserve_off_precedence_and_untagged_rules() {
        let mut manager = facet_fixture();
        manager.prefs.categories_off.insert("cache".into());
        manager.prefs.categories_on.insert("cache".into());
        manager.prefs.categories_on.insert("logs".into());
        manager.prefs.tags_off.insert("beta".into());
        manager.prefs.tags_on.insert("beta".into());
        manager.prefs.tags_on.insert("common".into());
        manager.prefs.set_rule("d", false);
        assert!(manager.categories(true).iter().all(|r| r.enabled == 0));
        assert!(manager.tags(true).iter().all(|r| r.enabled == 0));
        assert!(manager.kinds(true).iter().all(|r| r.enabled == 0));
        assert!(manager.risks(true).iter().all(|r| r.enabled == 0));
        assert_eq!(manager.categories(true)[0].policy, CategoryPolicy::ForceOff);
        assert_eq!(manager.categories(true)[1].policy, CategoryPolicy::ForceOn);
        assert_eq!(manager.tags(true)[1].policy, CategoryPolicy::ForceOff);
        assert_eq!(manager.tags(true)[2].policy, CategoryPolicy::ForceOn);
        assert_eq!(manager.stats().enabled, 0);
        assert_eq!(manager.stats().disabled, 4);
        assert_eq!(manager.stats().overridden, 4);
        assert_eq!(
            manager
                .categories(false)
                .iter()
                .map(|r| r.enabled)
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn prepared_search_preserves_case_spacing_tags_and_preference_filters() {
        let rule: Rule = serde_json::from_value(serde_json::json!({
            "id": "example", "name": "Tool", "description": "Build Output É",
            "paths": ["/tmp/fixture"], "platform": "any", "category": "dev",
            "kind": "dev", "tags": ["alpha", "rust"]
        }))
        .unwrap();
        let hay = format!(
            "{} {} {} {} {} {}",
            rule.id,
            rule.name,
            rule.category,
            rule.kind.as_str(),
            rule.description,
            rule.tags.join(" ")
        )
        .to_ascii_lowercase();
        let mut manager = RuleManager {
            pack: std::sync::Arc::new(RulePack {
                rules: vec![rule],
                sources: vec![],
            }),
            prefs: RulePrefs::default(),
        };
        for query in [
            "TOOL DEV",
            "É ALPHA",
            "output  é",
            "é",
            " É ",
            "missing",
            "",
            "dev build",
        ] {
            let filter = RuleFilter {
                query: query.into(),
                ..Default::default()
            };
            let expected = usize::from(hay.contains(&query.trim().to_ascii_lowercase()));
            assert_eq!(manager.list_page(&filter, 1).0, expected, "{query}");
            assert_eq!(
                usize::from(filter.matches(&manager.pack.rules[0], &manager.prefs)),
                expected
            );
        }
        let mut filter = RuleFilter {
            tags: vec![" RUST ".into(), "ALPHA".into(), "---".into()],
            ..Default::default()
        };
        assert_eq!(manager.list_page(&filter, 1).0, 1);
        manager.prefs.rules.insert("example".into(), false);
        filter.enabled = Some(true);
        assert_eq!(manager.list_page(&filter, 1).0, 0);
        filter.enabled = Some(false);
        assert_eq!(manager.list_page(&filter, 1).0, 1);
        assert_eq!(manager.list_page(&filter, 0).1.len(), 0);
    }

    #[test]
    fn corrupt_prefs_are_backed_up_and_reported_and_saves_are_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rule-prefs.json");
        assert_eq!(RulePrefs::try_load_from(&path).unwrap(), RulePrefs::default());
        fs::write(&path, b"{ not json").unwrap();
        let error = RulePrefs::try_load_from(&path).unwrap_err();
        let backup = match &error {
            PrefsError::Corrupt { backup, .. } => backup.clone(),
            other => panic!("expected corrupt error, got {other:?}"),
        };
        assert!(error.to_string().contains("corrupt"));
        assert!(!path.exists(), "corrupt file must be moved aside");
        assert_eq!(fs::read(&backup).unwrap(), b"{ not json");
        assert!(RulePrefs::load_from(&path).is_ok(), "missing file → defaults");
        let mut prefs = RulePrefs::default();
        prefs.set_rule("x", false);
        prefs.save_to(&path).unwrap();
        assert_eq!(RulePrefs::try_load_from(&path).unwrap(), prefs);
        assert!(
            !fs::read_dir(dir.path()).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            }),
            "temporary file must be renamed away"
        );
    }

    #[test]
    fn unrestricted_scan_keeps_explicit_off_and_caps_risk_at_low() {
        let mut manager = facet_fixture();
        manager.prefs.set_category("cache", false);
        manager.prefs.set_rule("a", false);
        let ids = |rules: Vec<&Rule>| rules.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        // Default scan honours the category policy.
        assert_eq!(ids(manager.scan_rules(&[])), Vec::<String>::new());
        // Unrestricted ignores category / default-off, keeps explicit off, caps at low.
        assert_eq!(
            ids(manager.scan_rules_unrestricted(&[], &[], None, None, None)),
            Vec::<String>::new(),
            "b is medium, c is high, a is explicitly off, d is report-only"
        );
        assert_eq!(
            ids(manager.scan_rules_unrestricted(&[], &[], None, None, Some(Risk::High))),
            vec!["b".to_string(), "c".to_string()]
        );
        manager.reset_rule("a");
        assert_eq!(
            ids(manager.scan_rules_unrestricted(&[], &[], None, None, None)),
            vec!["a".to_string()]
        );
        manager.prefs.include_report_only = true;
        assert_eq!(
            ids(manager.scan_rules_unrestricted(&["other".into()], &[], None, None, None)),
            vec!["d".to_string()]
        );
        // Category selectors accept rule ids too, and foreign rules never run.
        assert_eq!(
            ids(manager.scan_rules_unrestricted(&["e".into(), "b".into()], &[], None, None, Some(Risk::High))),
            vec!["b".to_string()]
        );
        assert_eq!(
            ids(manager.scan_rules_filtered(&["cache".into()], &["beta".into()], None, Some(Risk::High))),
            Vec::<String>::new(),
            "category policy still disables cache"
        );
        manager.reset_category("cache");
        assert_eq!(
            ids(manager.scan_rules_filtered(&["cache".into()], &["beta".into()], None, Some(Risk::High))),
            vec!["c".to_string()]
        );
    }

    #[test]
    fn duplicate_overrides_migrate_conservatively_and_idempotently() {
        let aliases = BTreeMap::from([
            ("old".into(), "kept".into()),
            ("old2".into(), "kept".into()),
        ]);
        let mut prefs = RulePrefs::default();
        prefs.rules.extend([
            ("old".into(), false),
            ("old2".into(), true),
            ("kept".into(), true),
        ]);
        prefs
            .min_age
            .extend([("old".into(), 30), ("kept".into(), 7)]);
        prefs.migrate_aliases(&aliases);
        prefs.migrate_aliases(&aliases);
        assert_eq!(prefs.rules, BTreeMap::from([("kept".into(), false)]));
        assert_eq!(prefs.min_age, BTreeMap::from([("kept".into(), 30)]));
    }
}
