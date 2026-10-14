//! Flexible rule manager UI — browse / search / toggle categories, tags & rules.

use crate::i18n::t;
use std::sync::Arc;

use eframe::egui::{self, Align, Layout, RichText};
use space_clean::{CategoryPolicy, JunkKind, Risk, RuleFilter, RuleManager, RuleRow};

use crate::settings::FeatureFlags;
use crate::theme::{self, ACCENT, DANGER, MUTED, TEXT};
use crate::widgets;

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
    /// Open this rule path in Disk usage (or Finder when it isn't indexed),
    /// the manual route for list-only matches.
    Reveal(String),
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

    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(crate::tr!(
                "{} of {} rules on · {} off",
                stats.enabled,
                stats.host,
                stats.disabled,
            ))
            .size(12.0)
            .color(MUTED()),
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
        ui.label(RichText::new(t("Privacy")).color(MUTED()));
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
        ui.colored_label(crate::theme::DANGER(), err);
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(t("Search")).color(MUTED()));
        let resp = ui.add(
            egui::TextEdit::singleline(&mut state.query)
                .desired_width(220.0)
                .hint_text(t("name or description…")),
        );
        let _ = resp;
        ui.separator();
        for (label, mode) in [
            ("All", EnabledFilter::All),
            ("Enabled rules", EnabledFilter::On),
            ("Disabled rules", EnabledFilter::Off),
            ("Changed by you", EnabledFilter::Overridden),
        ] {
            if crate::widgets::chip(ui, state.enabled == mode, label)
                .on_hover_text(t(match mode {
                    EnabledFilter::All => "All rules",
                    EnabledFilter::On => "Rules that will run",
                    EnabledFilter::Off => "Rules that will not run",
                    EnabledFilter::Overridden => "Rules you turned on or off yourself",
                }))
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

    let avail_h = (ui.available_height() - 64.0).max(160.0);
    ui.horizontal_top(|ui| {
        // Left: categories (with a switch each), then the other ways to slice.
        ui.allocate_ui_with_layout(
            egui::vec2(250.0, avail_h),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(250.0);
                egui::ComboBox::from_id_salt("rule_filter_dimension")
                    .width(230.0)
                    .selected_text(t(match state.side_tab {
                        SideTab::Categories => "Categories",
                        SideTab::Kinds => "Types",
                        SideTab::Danger => "Caution",
                        SideTab::Tags => "Tags",
                    }))
                    .show_ui(ui, |ui| {
                    for (label, tab) in [
                        ("Categories", SideTab::Categories),
                        ("Types", SideTab::Kinds),
                        ("Caution", SideTab::Danger),
                        ("Tags", SideTab::Tags),
                    ] {
                        if ui.selectable_label(state.side_tab == tab, t(label)).clicked() {
                            state.side_tab = tab;
                        }
                    }
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("rules_side")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match state.side_tab {
                        SideTab::Categories => {
                            if side_row(ui, state.category.is_none(), &crate::tr!("All categories  ({} rules)", stats.host), None).0 {
                                state.category = None;
                            }
                            for cat in cats {
                                let selected = state.category.as_deref() == Some(cat.name.as_str());
                                let on = cat.enabled == cat.total;
                                let label = format!("{}  {}/{}", t(&cat_title(&cat.name)), cat.enabled, cat.total);
                                let (pick, flip) = side_row(ui, selected, &label, Some((cat.enabled, cat.total)));
                                if flip {
                                    let _ = mgr.set_category_enabled(&cat.name, !on);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                } else if pick {
                                    state.category = Some(cat.name.clone());
                                }
                            }
                        }
                        SideTab::Kinds => {
                            if side_row(ui, state.kind.is_none(), &crate::tr!("All types  ({})", kinds.len()), None).0 {
                                state.kind = None;
                            }
                            for kind in kinds {
                                let parsed = JunkKind::parse(&kind.name);
                                let label = format!("{}  {}/{}", kind.name, kind.enabled, kind.total);
                                if side_row(ui, state.kind == parsed, &label, None).0 {
                                    state.kind = parsed;
                                }
                            }
                        }
                        SideTab::Danger => {
                            if side_row(ui, state.risk.is_none(), t("All caution levels"), None).0 {
                                state.risk = None;
                            }
                            for risk in risks {
                                let parsed = Risk::parse(&risk.name);
                                let label = format!("{}  {}/{}", t(&risk.danger), risk.enabled, risk.total);
                                if side_row(ui, state.risk == parsed, &label, None).0 {
                                    state.risk = parsed;
                                }
                            }
                        }
                        SideTab::Tags => {
                            if side_row(ui, state.tag.is_none(), &crate::tr!("All tags  ({})", tags.len()), None).0 {
                                state.tag = None;
                            }
                            for tag in tags {
                                let selected = state.tag.as_deref() == Some(tag.name.as_str());
                                let on = tag.enabled == tag.total;
                                let label = format!("{}  {}/{}", tag.name, tag.enabled, tag.total);
                                let (pick, flip) = side_row(ui, selected, &label, Some((tag.enabled, tag.total)));
                                if flip {
                                    let _ = mgr.set_tag_enabled(&tag.name, !on);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                } else if pick {
                                    state.tag = Some(tag.name.clone());
                                }
                            }
                        }
                    });
            },
        );

        // Right: the rules as a table, descriptions visible.
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), avail_h),
            Layout::top_down(Align::Min),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t("Rules")).family(crate::theme::bold()).color(TEXT()));
                    ui.label(RichText::new(format!("({})", page.total)).size(12.0).color(MUTED()));
                    if let Some(cat) = &state.category {
                        ui.label(RichText::new(format!("· {}", t(&cat_title(cat)))).size(12.0).color(ACCENT()));
                    }
                    if let Some(cat) = cats.iter().find(|cat| {
                        state.category.as_deref() == Some(cat.name.as_str())
                            && cat.enabled > 0 && cat.enabled < cat.total
                    }) {
                        ui.label(RichText::new(crate::tr!("Partially enabled: {} of {}", cat.enabled, cat.total)).size(12.0).color(ACCENT()));
                    }
                    if let Some(cat) = cats.iter().find(|cat| {
                        state.category.as_deref() == Some(cat.name.as_str())
                            && cat.policy == CategoryPolicy::ForceOff
                            && cat.enabled > cat.total / 2
                    }) {
                        if ui.button(t("Enable category")).on_hover_text(t("Enable defaults for this category, keeping individual exceptions. New rules will follow the category setting.")).clicked() {
                            let _ = mgr.set_category_enabled(&cat.name, true);
                            state.dirty = true;
                            state.error = save(mgr);
                            action = RulesAction::PrefsChanged;
                        }
                    }
                    if let Some(kind) = state.kind {
                        ui.label(RichText::new(format!("· {}", kind.as_str())).size(12.0).color(ACCENT()));
                    }
                    if let Some(risk) = state.risk {
                        ui.label(RichText::new(format!("· {}", t(risk.danger_label()))).size(12.0).color(ACCENT()));
                    }
                    if let Some(tag) = &state.tag {
                        ui.label(RichText::new(format!("· #{tag}")).size(12.0).color(ACCENT()));
                    }
                });
                egui::Frame::new()
                    .fill(theme::PANEL())
                    .stroke(egui::Stroke::new(1.0_f32, theme::BORDER()))
                    .corner_radius(theme::metrics::RADIUS_CARD)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("rules_list")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                for row in rows.iter().take(state.page_size) {
                                    draw_rule_row(ui, state, mgr, row, &mut action);
                                }
                                if page.total > state.page_size {
                                    ui.add_space(6.0);
                                    if ui
                                        .button(crate::tr!("Show more ({} hidden)…", page.total - state.page_size))
                                        .clicked()
                                    {
                                        state.page_size = state.page_size.saturating_add(300);
                                    }
                                }
                                if rows.is_empty() {
                                    ui.add_space(12.0);
                                    ui.label(RichText::new(t("No rules match.")).color(MUTED()));
                                }
                            });
                    });
            },
        );
    });

    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if ui.add(widgets::secondary("Close")).clicked() {
            action = RulesAction::Close;
        }
        if state.dirty && state.error.is_none() {
            ui.label(RichText::new(t("Saved")).size(theme::type_scale::CAPTION).color(MUTED()));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(widgets::caption("Lock: listed only — never removed automatically. Open it in Disk usage to remove by hand."));
        });
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
    let locked = row.report_only;
    let frame = egui::Frame::new()
        .fill(if locked {
            theme::BG()
        } else {
            egui::Color32::TRANSPARENT
        })
        .inner_margin(egui::Margin::symmetric(10, 8));
    let resp = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                // On/off switch, or a lock for list-only rules: those are
                // shown but never removed automatically, whatever the switch.
                let (slot, _) = ui.allocate_exact_size(egui::vec2(38.0, 22.0), egui::Sense::hover());
                if locked {
                    crate::icons::paint(
                        ui.painter(),
                        crate::icons::Icon::Lock,
                        egui::Rect::from_center_size(slot.center(), egui::vec2(16.0, 16.0)),
                        MUTED(),
                    );
                    ui.interact(slot, ui.id().with(("lock", &row.id)), egui::Sense::hover())
                        .on_hover_text(t("Listed only — never removed automatically"));
                } else {
                    let r = ui.interact(slot, ui.id().with(("sw", &row.id)), egui::Sense::click());
                    paint_switch(ui, slot, row.effectively_enabled, r.hovered());
                    if r.on_hover_text(t("Turn this rule on or off for junk cleaning")).clicked() {
                        let _ = mgr.set_rule_enabled(&row.id, !row.effectively_enabled);
                        state.dirty = true;
                        state.error = save(mgr);
                        *action = RulesAction::PrefsChanged;
                    }
                }

                let color = if row.effectively_enabled || locked { TEXT() } else { MUTED() };
                let action_w = widgets::text_width(
                    ui,
                    t(if locked { "Remove manually…" } else { "Reset" }),
                    egui::FontId::proportional(theme::type_scale::BODY),
                ) + 24.0;
                let text_w = (ui.available_width() - action_w - 10.0).max(80.0);
                ui.vertical(|ui| {
                    ui.set_width(text_w);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(RichText::new(&row.name).size(14.0).family(crate::theme::bold()).color(color)).truncate())
                        .on_hover_text(&row.name);
                    if !row.description.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&row.description).size(12.0).color(MUTED())).truncate()).on_hover_text(&row.description);
                    }
                    if let Some(p) = row.paths.first() {
                        let more = if row.paths.len() > 1 { crate::tr!("  +{} more", row.paths.len() - 1) } else { String::new() };
                        ui.add(
                            egui::Label::new(RichText::new(format!("{p}{more}")).size(theme::type_scale::CAPTION).monospace().color(MUTED()))
                                .truncate(),
                        )
                        .on_hover_text(row.paths.join("\n"));
                    }
                    ui.horizontal_wrapped(|ui| {
                        let (label, color) = if locked {
                            ("Manual removal", MUTED())
                        } else {
                            match row.danger.as_str() {
                                "dangerous" => ("Careful", DANGER()),
                                "caution" => ("Check", ACCENT()),
                                _ => ("Safe", theme::SAFE()),
                            }
                        };
                        ui.label(widgets::caption(label).color(color));
                        ui.label(widgets::caption(rule_action(row)));
                        if row.overridden {
                            ui.label(widgets::caption("changed by you").color(ACCENT()));
                        }
                    });

                });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if locked {
                        if let Some(p) = row.paths.first() {
                            if ui
                                .add(widgets::secondary("Remove manually…"))
                                .on_hover_text(t("Find it in Disk usage and add it to the Trash list by hand"))
                                .clicked()
                            {
                                *action = RulesAction::Reveal(p.clone());
                            }
                        }
                    } else if row.overridden
                        && ui
                            .add(widgets::secondary("Reset"))
                            .on_hover_text(t("Use the default for this rule"))
                            .clicked()
                    {
                        mgr.reset_rule(&row.id);
                        state.dirty = true;
                        state.error = save(mgr);
                        *action = RulesAction::PrefsChanged;
                    }

                });
            });
        })
        .response;
    ui.painter().hline(
        resp.rect.x_range(),
        resp.rect.bottom(),
        egui::Stroke::new(1.0_f32, theme::BORDER()),
    );
    resp.on_hover_ui(|ui| {
        ui.label(RichText::new(&row.id).family(crate::theme::bold()));
        for detail in &row.details {
            ui.label(detail);
        }
        if !row.tags.is_empty() {
            ui.label(RichText::new(crate::tr!("tags: {}", row.tags.join(", "))).color(MUTED()));
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
}

/// What the rule does with a match, in a few words.
fn rule_action(row: &RuleRow) -> &'static str {
    if row.report_only {
        "Automatic cleaning disabled"
    } else if row
        .details
        .iter()
        .any(|d| d.contains("preserving the configured root"))
    {
        "Empties the folder"
    } else {
        "Moves to Trash"
    }
}

/// A list row in the left column: label, optional on/off switch.
/// Returns (row clicked, switch clicked).
fn side_row(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    switch: Option<(usize, usize)>,
) -> (bool, bool) {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 6.0, theme::PANEL_2());
    } else if resp.hovered() {
        p.rect_filled(rect, 6.0, theme::PANEL_2().gamma_multiply(0.5));
    }
    let right = if switch.is_some() {
        rect.right() - 48.0
    } else {
        rect.right() - 8.0
    };
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_owned(),
        egui::FontId::proportional(13.0),
        if switch.is_some_and(|(enabled, _)| enabled == 0) {
            MUTED()
        } else {
            TEXT()
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(right - rect.left() - 18.0);
    let g = p.layout_job(job);
    p.galley(
        egui::pos2(rect.left() + 10.0, rect.center().y - g.size().y / 2.0),
        g,
        TEXT(),
    );
    let mut flip = false;
    if let Some((enabled, total)) = switch {
        let slot = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 26.0, rect.center().y),
            egui::vec2(34.0, 20.0),
        );
        let hot = resp.hovered()
            && ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|h| slot.contains(h));
        let partial = enabled > 0 && enabled < total;
        paint_switch(ui, slot, enabled == total, hot);
        if partial {
            p.rect_filled(slot, 10.0, ACCENT().gamma_multiply(0.45));
            p.circle_filled(slot.center(), 8.0, egui::Color32::WHITE);
        }
        resp.clone().on_hover_text(if partial {
            crate::tr!("Partially enabled: {} of {}. Click the switch to enable the group; individual exceptions stay unchanged.", enabled, total)
        } else if enabled == 0 {
            t("Disabled. Click the switch to enable the group.").to_owned()
        } else {
            t("Enabled. Individual exceptions stay unchanged.").to_owned()
        });
        flip = resp.clicked()
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|h| slot.expand(4.0).contains(h));
    }
    (resp.clicked() && !flip, flip)
}

fn paint_switch(ui: &egui::Ui, slot: egui::Rect, on: bool, hot: bool) {
    let p = ui.painter();
    let r = slot.height() / 2.0;
    let mut track = if on { ACCENT() } else { theme::BORDER() };
    if hot {
        track = track.gamma_multiply(0.85);
    }
    p.rect_filled(slot, r, track);
    let x = if on {
        slot.right() - r
    } else {
        slot.left() + r
    };
    p.circle_filled(
        egui::pos2(x, slot.center().y),
        r - 2.0,
        egui::Color32::WHITE,
    );
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
