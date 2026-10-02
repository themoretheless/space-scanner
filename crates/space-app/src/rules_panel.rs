//! Flexible rule manager UI — browse / search / toggle categories, tags & rules.

use crate::i18n::t;
use std::sync::Arc;

use eframe::egui::{self, Align, Layout, RichText};
use space_clean::{CategoryPolicy, JunkKind, Risk, RuleFilter, RuleManager, RuleRow};

use crate::settings::FeatureFlags;
use crate::theme::{ACCENT, DANGER, MUTED, TEXT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnabledFilter {
    All,
    On,
    Off,
    Overridden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SideTab {
    Categories,
    Tags,
    Kinds,
    Danger,
}

pub struct RulesPanelState {
    pub open: bool,
    pub query: String,
    pub category: Option<String>,
    pub tag: Option<String>,
    pub kind: Option<JunkKind>,
    pub risk: Option<Risk>,
    pub enabled: EnabledFilter,
    pub side_tab: SideTab,
    pub error: Option<String>,
    pub dirty: bool,
    /// Cap rows drawn per frame for responsiveness with many rules.
    pub page_size: usize,
    summary: Option<Arc<RuleSummary>>,
    page: Option<Arc<RulePage>>,
}

impl Default for RulesPanelState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            category: None,
            tag: None,
            kind: None,
            risk: None,
            enabled: EnabledFilter::All,
            side_tab: SideTab::Categories,
            error: None,
            dirty: false,
            page_size: 200,
            summary: None,
            page: None,
        }
    }
}

struct RuleSummary {
    pack: Arc<space_clean::RulePack>,
    prefs: space_clean::RulePrefs,
    stats: space_clean::ManagerStats,
    categories: Vec<space_clean::CategoryRow>,
    tags: Vec<space_clean::TagRow>,
    kinds: Vec<space_clean::KindRow>,
    risks: Vec<space_clean::RiskRow>,
}

struct RulePage {
    pack: Arc<space_clean::RulePack>,
    prefs: space_clean::RulePrefs,
    filter: RuleFilter,
    limit: usize,
    total: usize,
    rows: Vec<RuleRow>,
}

impl RulesPanelState {
    fn summary(&mut self, mgr: &RuleManager) -> Arc<RuleSummary> {
        if let Some(summary) = &self.summary {
            if Arc::ptr_eq(&summary.pack, &mgr.pack) && summary.prefs == mgr.prefs {
                return Arc::clone(summary);
            }
        }
        let categories = mgr.categories(true);
        let tags = mgr.tags(true);
        let kinds = mgr.kinds(true);
        let risks = mgr.risks(true);
        let host = categories.iter().map(|row| row.total).sum::<usize>();
        let enabled = categories.iter().map(|row| row.enabled).sum::<usize>();
        let stats = space_clean::ManagerStats {
            total: mgr.pack.rules.len(),
            host,
            enabled,
            disabled: host - enabled,
            overridden: mgr
                .pack
                .rules
                .iter()
                .filter(|rule| rule.platform.matches_host())
                .filter(|rule| mgr.prefs.is_overridden(rule))
                .count(),
            categories: categories.len(),
            tags: tags.len(),
            kinds: kinds.len(),
        };
        let summary = Arc::new(RuleSummary {
            pack: Arc::clone(&mgr.pack),
            prefs: mgr.prefs.clone(),
            stats,
            categories,
            tags,
            kinds,
            risks,
        });
        self.summary = Some(Arc::clone(&summary));
        summary
    }

    fn page(&mut self, mgr: &RuleManager, filter: RuleFilter) -> Arc<RulePage> {
        if let Some(page) = &self.page {
            if Arc::ptr_eq(&page.pack, &mgr.pack)
                && page.prefs == mgr.prefs
                && page.filter == filter
                && page.limit == self.page_size
            {
                return Arc::clone(page);
            }
        }
        let (total, rows) = mgr.list_page(&filter, self.page_size);
        let page = Arc::new(RulePage {
            pack: Arc::clone(&mgr.pack),
            prefs: mgr.prefs.clone(),
            filter,
            limit: self.page_size,
            total,
            rows,
        });
        self.page = Some(Arc::clone(&page));
        page
    }
}

pub enum RulesAction {
    None,
    Close,
    /// Prefs changed — junk scan should use new set.
    PrefsChanged,
}

pub fn draw(
    ui: &mut egui::Ui,
    state: &mut RulesPanelState,
    mgr: &mut RuleManager,
    flags: &mut FeatureFlags,
) -> RulesAction {
    let mut action = RulesAction::None;
    let summary = state.summary(mgr);
    let stats = &summary.stats;

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(crate::tr!(
                "{} of {} rules on · {} off",
                stats.enabled,
                stats.host,
                stats.disabled,
            ))
            .size(12.0)
            .color(MUTED),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .button(t("Reset all"))
                .on_hover_text(t("Restore built-in defaults for every rule"))
                .clicked()
            {
                mgr.reset_all();
                state.dirty = true;
                state.error = save(mgr);
                action = RulesAction::PrefsChanged;
            }
            let mut all = mgr.prefs.enable_all_by_default;
            if ui
                .checkbox(&mut all, t("Turn every rule on"))
                .on_hover_text(t(
                    "Then you can still turn categories, tags, or single rules off",
                ))
                .changed()
            {
                mgr.prefs.enable_all_by_default = all;
                state.dirty = true;
                state.error = save(mgr);
                action = RulesAction::PrefsChanged;
            }
        });
    });

    ui.horizontal(|ui| {
        ui.label(RichText::new(t("Privacy")).color(MUTED));
        if ui
            .checkbox(&mut flags.scan_browser_cookies, "Cookies")
            .on_hover_text(
                "Scan/clean browser cookies. Off by default — signs you out of websites.",
            )
            .changed()
        {
            action = RulesAction::PrefsChanged;
        }
        if ui
            .checkbox(&mut flags.scan_browser_history, t("History"))
            .on_hover_text(t("Scan/clean browser history. Off by default."))
            .changed()
        {
            action = RulesAction::PrefsChanged;
        }
    });

    if let Some(err) = &state.error {
        ui.colored_label(egui::Color32::from_rgb(220, 90, 90), err);
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(t("Search")).color(MUTED));
        let resp = ui.add(
            egui::TextEdit::singleline(&mut state.query)
                .desired_width(220.0)
                .hint_text(t("name or description…")),
        );
        let _ = resp;
        ui.separator();
        for (label, mode) in [
            ("All", EnabledFilter::All),
            ("On", EnabledFilter::On),
            ("Off", EnabledFilter::Off),
            ("Changed", EnabledFilter::Overridden),
        ] {
            if ui
                .selectable_label(state.enabled == mode, label)
                .on_hover_text(match mode {
                    EnabledFilter::All => "All rules",
                    EnabledFilter::On => "Rules that will run",
                    EnabledFilter::Off => "Rules that will not run",
                    EnabledFilter::Overridden => "Rules you turned on or off yourself",
                })
                .clicked()
            {
                state.enabled = mode;
            }
        }
    });

    ui.add_space(8.0);

    let cats = &summary.categories;
    let tags = &summary.tags;
    let kinds = &summary.kinds;
    let risks = &summary.risks;
    let filter = build_filter(state);
    let page = state.page(mgr, filter);
    let rows = &page.rows;

    ui.columns(2, |cols| {
        cols[0].horizontal(|ui| {
            for (label, tab) in [
                ("Categories", SideTab::Categories),
                ("Types", SideTab::Kinds),
                ("Caution", SideTab::Danger),
                ("Tags", SideTab::Tags),
            ] {
                if ui.selectable_label(state.side_tab == tab, label).clicked() {
                    state.side_tab = tab;
                }
            }
        });

        egui::ScrollArea::vertical()
            .id_salt("rules_side")
            .max_height(420.0)
            .show(&mut cols[0], |ui| match state.side_tab {
                SideTab::Categories => {
                    let all_selected = state.category.is_none();
                    let all_resp = ui.selectable_label(
                        all_selected,
                        crate::tr!("All categories  ({} rules)", stats.host),
                    );
                    if all_resp.clicked() {
                        state.category = None;
                    }

                    for cat in cats {
                        ui.horizontal(|ui| {
                            let selected = state.category.as_deref() == Some(cat.name.as_str());
                            let policy_mark = match cat.policy {
                                CategoryPolicy::Default => "",
                                CategoryPolicy::ForceOn => t(" · always on"),
                                CategoryPolicy::ForceOff => t(" · always off"),
                            };
                            let label = format!(
                                "{}{}  {}/{}",
                                cat_title(&cat.name),
                                policy_mark,
                                cat.enabled,
                                cat.total
                            );
                            if ui.selectable_label(selected, label).clicked() {
                                state.category = Some(cat.name.clone());
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .small_button(t("Reset"))
                                    .on_hover_text(t("Reset this category to defaults"))
                                    .clicked()
                                {
                                    mgr.reset_category(&cat.name);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                }
                                if ui
                                    .small_button(t("Off"))
                                    .on_hover_text(t("Turn this whole category off"))
                                    .clicked()
                                {
                                    let _ = mgr.set_category_enabled(&cat.name, false);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                }
                                if ui
                                    .small_button(t("On"))
                                    .on_hover_text(t("Turn this whole category on"))
                                    .clicked()
                                {
                                    let _ = mgr.set_category_enabled(&cat.name, true);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                }
                            });
                        });
                    }
                }
                SideTab::Kinds => {
                    if ui
                        .selectable_label(
                            state.kind.is_none(),
                            crate::tr!("All types  ({})", kinds.len()),
                        )
                        .clicked()
                    {
                        state.kind = None;
                    }
                    for kind in kinds {
                        let parsed = JunkKind::parse(&kind.name);
                        let selected = state.kind == parsed;
                        let label = format!("{}  {}/{}", kind.name, kind.enabled, kind.total);
                        if ui.selectable_label(selected, label).clicked() {
                            state.kind = parsed;
                        }
                    }
                }
                SideTab::Danger => {
                    if ui
                        .selectable_label(state.risk.is_none(), t("All caution levels"))
                        .clicked()
                    {
                        state.risk = None;
                    }
                    for risk in risks {
                        let parsed = Risk::parse(&risk.name);
                        let selected = state.risk == parsed;
                        let label = format!(
                            "{} ({})  {}/{}",
                            risk.danger, risk.name, risk.enabled, risk.total
                        );
                        if ui.selectable_label(selected, label).clicked() {
                            state.risk = parsed;
                        }
                    }
                }
                SideTab::Tags => {
                    let all_selected = state.tag.is_none();
                    let all_resp =
                        ui.selectable_label(all_selected, crate::tr!("All tags  ({})", tags.len()));
                    if all_resp.clicked() {
                        state.tag = None;
                    }

                    for tag in tags {
                        ui.horizontal(|ui| {
                            let selected = state.tag.as_deref() == Some(tag.name.as_str());
                            let policy_mark = match tag.policy {
                                CategoryPolicy::Default => "",
                                CategoryPolicy::ForceOn => t(" · always on"),
                                CategoryPolicy::ForceOff => t(" · always off"),
                            };
                            let label = format!(
                                "{}{}  {}/{}",
                                tag.name, policy_mark, tag.enabled, tag.total
                            );
                            if ui.selectable_label(selected, label).clicked() {
                                state.tag = Some(tag.name.clone());
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .small_button(t("Reset"))
                                    .on_hover_text(t("Stop forcing this tag on or off"))
                                    .clicked()
                                {
                                    mgr.reset_tag(&tag.name);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                }
                                if ui
                                    .small_button(t("Off"))
                                    .on_hover_text(t("Turn off all rules with this tag"))
                                    .clicked()
                                {
                                    let _ = mgr.set_tag_enabled(&tag.name, false);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                }
                                if ui
                                    .small_button(t("On"))
                                    .on_hover_text(t("Turn on all rules with this tag"))
                                    .clicked()
                                {
                                    let _ = mgr.set_tag_enabled(&tag.name, true);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                }
                            });
                        });
                    }
                }
            });

        // --- rules ---
        cols[1].horizontal(|ui| {
            ui.label(RichText::new(t("Rules")).strong().color(TEXT));
            ui.label(
                RichText::new(format!("({})", page.total))
                    .size(12.0)
                    .color(MUTED),
            );
            if let Some(cat) = &state.category {
                ui.label(RichText::new(format!("· {cat}")).size(12.0).color(ACCENT));
            }
            if let Some(kind) = state.kind {
                ui.label(
                    RichText::new(format!("· {}", kind.as_str()))
                        .size(12.0)
                        .color(ACCENT),
                );
            }
            if let Some(risk) = state.risk {
                ui.label(
                    RichText::new(format!("· {}", risk.danger_label()))
                        .size(12.0)
                        .color(ACCENT),
                );
            }
            if let Some(tag) = &state.tag {
                ui.label(RichText::new(format!("· #{tag}")).size(12.0).color(ACCENT));
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("rules_list")
            .max_height(420.0)
            .show(&mut cols[1], |ui| {
                let shown = rows.iter().take(state.page_size);
                for row in shown {
                    draw_rule_row(ui, state, mgr, row, &mut action);
                }
                if page.total > state.page_size {
                    ui.add_space(6.0);
                    if ui
                        .button(crate::tr!(
                            "Show more ({} hidden)…",
                            page.total - state.page_size
                        ))
                        .clicked()
                    {
                        state.page_size = state.page_size.saturating_add(300);
                    }
                }
                if rows.is_empty() {
                    ui.label(RichText::new(t("No rules match.")).color(MUTED));
                }
            });
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(t("Close")).clicked() {
            action = RulesAction::Close;
        }
        if state.dirty {
            ui.label(RichText::new(t("Saved")).size(11.0).color(MUTED));
        }
    });

    action
}

fn draw_rule_row(
    ui: &mut egui::Ui,
    state: &mut RulesPanelState,
    mgr: &mut RuleManager,
    row: &RuleRow,
    action: &mut RulesAction,
) {
    ui.horizontal(|ui| {
        let mut enabled = row.effectively_enabled;
        if ui
            .checkbox(&mut enabled, "")
            .on_hover_text(if row.overridden {
                "Your setting (not the default)"
            } else {
                "Turn this rule on or off for junk cleaning"
            })
            .changed()
        {
            let _ = mgr.set_rule_enabled(&row.id, enabled);
            state.dirty = true;
            state.error = save(mgr);
            *action = RulesAction::PrefsChanged;
        }

        let color = if row.effectively_enabled { TEXT } else { MUTED };
        let name = if row.overridden {
            crate::tr!("Changed · {}", row.name)
        } else {
            row.name.clone()
        };
        let resp = ui
            .vertical(|ui| {
                ui.set_max_width(340.0);
                ui.label(RichText::new(name).size(12.5).color(color));
                ui.add(
                    egui::Label::new(RichText::new(&row.description).size(12.0).color(MUTED))
                        .truncate(),
                );
            })
            .response;
        resp.on_hover_ui(|ui| {
            ui.label(RichText::new(&row.id).strong());
            ui.label(crate::tr!(
                "{} · caution: {} ({})",
                row.kind,
                row.danger,
                row.risk
            ));
            if !row.description.is_empty() {
                ui.label(&row.description);
            }
            ui.separator();
            for detail in &row.details {
                ui.label(detail);
            }
            if !row.tags.is_empty() {
                ui.separator();
                ui.label(RichText::new(crate::tr!("tags: {}", row.tags.join(", "))).color(MUTED));
            }
            for p in row.paths.iter().take(6) {
                ui.monospace(p);
            }
            if row.paths.len() > 6 {
                ui.label(crate::tr!("… {} more paths", row.paths.len() - 6));
            }
            ui.label(crate::tr!(
                "skip if newer than {} days · default {}",
                row.min_age_days,
                if row.default_enabled {
                    t("on")
                } else {
                    t("off")
                }
            ));
        });

        ui.label(RichText::new(&row.kind).size(12.0).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button(t("Reset"))
                .on_hover_text(t("Use the default for this rule"))
                .clicked()
            {
                mgr.reset_rule(&row.id);
                state.dirty = true;
                state.error = save(mgr);
                *action = RulesAction::PrefsChanged;
            }
            if row.report_only {
                ui.label(RichText::new(t("list only")).size(12.0).color(MUTED));
            }
            let danger_color = match row.danger.as_str() {
                "dangerous" | "report-only" => DANGER,
                "caution" => ACCENT,
                _ => MUTED,
            };
            ui.label(RichText::new(t(&row.danger)).size(12.0).color(danger_color));
        });
    });
}

fn build_filter(state: &RulesPanelState) -> RuleFilter {
    let (enabled, overridden_only) = match state.enabled {
        EnabledFilter::All => (None, false),
        EnabledFilter::On => (Some(true), false),
        EnabledFilter::Off => (Some(false), false),
        EnabledFilter::Overridden => (None, true),
    };
    let tags = state
        .tag
        .as_ref()
        .map(|t| vec![t.clone()])
        .unwrap_or_default();
    RuleFilter {
        query: state.query.clone(),
        category: state.category.clone(),
        tags,
        kind: state.kind,
        risk: state.risk,
        max_risk: None,
        enabled,
        report_only: None,
        host_only: true,
        overridden_only,
    }
}

fn save(mgr: &RuleManager) -> Option<String> {
    mgr.save_prefs().err().map(|e| e.to_string())
}

fn cat_title(cat: &str) -> String {
    let mut s = cat.replace('-', " ");
    if let Some(c) = s.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_reuses_unchanged_data_and_invalidates_filters_prefs_and_pages() {
        let template: space_clean::Rule = serde_json::from_value(serde_json::json!({
            "id": "fixture", "name": "fixture", "description": "fixture",
            "paths": ["/tmp/space-rules-ui"], "platform": "any"
        }))
        .unwrap();
        let rules = (0..1000)
            .map(|i| {
                let mut rule = template.clone();
                rule.id = format!("rule-{i:04}");
                rule.name = rule.id.clone();
                rule
            })
            .collect();
        let mut mgr = RuleManager {
            pack: Arc::new(space_clean::RulePack {
                rules,
                sources: Vec::new(),
            }),
            prefs: space_clean::RulePrefs::default(),
        };
        let mut state = RulesPanelState::default();
        let summary = state.summary(&mgr);
        assert!(Arc::ptr_eq(&summary, &state.summary(&mgr)));
        let filter = build_filter(&state);
        let page = state.page(&mgr, filter.clone());
        assert_eq!(page.total, 1000);
        assert_eq!(page.rows.len(), 200);
        for _ in 0..100 {
            assert!(Arc::ptr_eq(&page, &state.page(&mgr, filter.clone())));
        }
        state.page_size = 500;
        let expanded = state.page(&mgr, filter.clone());
        assert_eq!(expanded.rows.len(), 500);
        assert_eq!(expanded.total, 1000);
        assert_eq!(expanded.rows[199].id, page.rows[199].id);
        mgr.prefs.rules.insert("rule-0000".into(), false);
        let updated = state.page(&mgr, filter);
        assert!(!updated.rows[0].effectively_enabled);
        assert!(!Arc::ptr_eq(&summary, &state.summary(&mgr)));
        state.query = "rule-0000".into();
        let filtered = state.page(&mgr, build_filter(&state));
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.rows[0].id, "rule-0000");
        mgr.pack = Arc::new(space_clean::RulePack {
            rules: Vec::new(),
            sources: Vec::new(),
        });
        assert_eq!(state.page(&mgr, build_filter(&state)).total, 0);
    }
}
