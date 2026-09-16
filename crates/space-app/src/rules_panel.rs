//! Flexible rule manager UI — browse / search / toggle categories, tags & rules.

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
        }
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
    let stats = mgr.stats();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} of {} rules on · {} off",
                stats.enabled, stats.host, stats.disabled,
            ))
            .size(12.0)
            .color(MUTED),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .button("Reset all")
                .on_hover_text("Restore built-in defaults for every rule")
                .clicked()
            {
                mgr.reset_all();
                state.dirty = true;
                state.error = save(mgr);
                action = RulesAction::PrefsChanged;
            }
            let mut all = mgr.prefs.enable_all_by_default;
            if ui
                .checkbox(&mut all, "Turn every rule on")
                .on_hover_text("Then you can still turn categories, tags, or single rules off")
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
        ui.label(RichText::new("Privacy").color(MUTED));
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
            .checkbox(&mut flags.scan_browser_history, "History")
            .on_hover_text("Scan/clean browser history. Off by default.")
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
        ui.label(RichText::new("Search").color(MUTED));
        let resp = ui.add(
            egui::TextEdit::singleline(&mut state.query)
                .desired_width(220.0)
                .hint_text("name or description…"),
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

    let cats = mgr.categories(true);
    let tags = mgr.tags(true);
    let kinds = mgr.kinds(true);
    let risks = mgr.risks(true);
    let filter = build_filter(state);
    let rows = mgr.list(&filter);

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
                        format!("All categories  ({} rules)", stats.host),
                    );
                    if all_resp.clicked() {
                        state.category = None;
                    }

                    for cat in &cats {
                        ui.horizontal(|ui| {
                            let selected = state.category.as_deref() == Some(cat.name.as_str());
                            let policy_mark = match cat.policy {
                                CategoryPolicy::Default => "",
                                CategoryPolicy::ForceOn => " · always on",
                                CategoryPolicy::ForceOff => " · always off",
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
                                    .small_button("Reset")
                                    .on_hover_text("Reset this category to defaults")
                                    .clicked()
                                {
                                    mgr.reset_category(&cat.name);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                }
                                if ui
                                    .small_button("Off")
                                    .on_hover_text("Turn this whole category off")
                                    .clicked()
                                {
                                    let _ = mgr.set_category_enabled(&cat.name, false);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                }
                                if ui
                                    .small_button("On")
                                    .on_hover_text("Turn this whole category on")
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
                            format!("All types  ({})", kinds.len()),
                        )
                        .clicked()
                    {
                        state.kind = None;
                    }
                    for kind in &kinds {
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
                        .selectable_label(state.risk.is_none(), "All caution levels")
                        .clicked()
                    {
                        state.risk = None;
                    }
                    for risk in &risks {
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
                        ui.selectable_label(all_selected, format!("All tags  ({})", tags.len()));
                    if all_resp.clicked() {
                        state.tag = None;
                    }

                    for tag in &tags {
                        ui.horizontal(|ui| {
                            let selected = state.tag.as_deref() == Some(tag.name.as_str());
                            let policy_mark = match tag.policy {
                                CategoryPolicy::Default => "",
                                CategoryPolicy::ForceOn => " · always on",
                                CategoryPolicy::ForceOff => " · always off",
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
                                    .small_button("Reset")
                                    .on_hover_text("Stop forcing this tag on or off")
                                    .clicked()
                                {
                                    mgr.reset_tag(&tag.name);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                }
                                if ui
                                    .small_button("Off")
                                    .on_hover_text("Turn off all rules with this tag")
                                    .clicked()
                                {
                                    let _ = mgr.set_tag_enabled(&tag.name, false);
                                    state.dirty = true;
                                    state.error = save(mgr);
                                    action = RulesAction::PrefsChanged;
                                }
                                if ui
                                    .small_button("On")
                                    .on_hover_text("Turn on all rules with this tag")
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
            ui.label(RichText::new("Rules").strong().color(TEXT));
            ui.label(
                RichText::new(format!("({})", rows.len()))
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
                if rows.len() > state.page_size {
                    ui.add_space(6.0);
                    if ui
                        .button(format!(
                            "Show more ({} hidden)…",
                            rows.len() - state.page_size
                        ))
                        .clicked()
                    {
                        state.page_size = state.page_size.saturating_add(300);
                    }
                }
                if rows.is_empty() {
                    ui.label(RichText::new("No rules match.").color(MUTED));
                }
            });
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Close").clicked() {
            action = RulesAction::Close;
        }
        if state.dirty {
            ui.label(RichText::new("Saved").size(11.0).color(MUTED));
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
            format!("Changed · {}", row.name)
        } else {
            row.name.clone()
        };
        let resp = ui
            .vertical(|ui| {
                ui.set_max_width(340.0);
                ui.label(RichText::new(name).size(12.5).color(color));
                ui.add(
                    egui::Label::new(RichText::new(&row.description).size(10.5).color(MUTED))
                        .truncate(),
                );
            })
            .response;
        resp.on_hover_ui(|ui| {
            ui.label(RichText::new(&row.id).strong());
            ui.label(format!(
                "{} · caution: {} ({})",
                row.kind, row.danger, row.risk
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
                ui.label(RichText::new(format!("tags: {}", row.tags.join(", "))).color(MUTED));
            }
            for p in row.paths.iter().take(6) {
                ui.monospace(p);
            }
            if row.paths.len() > 6 {
                ui.label(format!("… {} more paths", row.paths.len() - 6));
            }
            ui.label(format!(
                "skip if newer than {} days · default {}",
                row.min_age_days,
                if row.default_enabled { "on" } else { "off" }
            ));
        });

        ui.label(RichText::new(&row.kind).size(10.0).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button("Reset")
                .on_hover_text("Use the default for this rule")
                .clicked()
            {
                mgr.reset_rule(&row.id);
                state.dirty = true;
                state.error = save(mgr);
                *action = RulesAction::PrefsChanged;
            }
            if row.report_only {
                ui.label(RichText::new("list only").size(10.0).color(MUTED));
            }
            let danger_color = match row.danger.as_str() {
                "dangerous" | "report-only" => DANGER,
                "caution" => ACCENT,
                _ => MUTED,
            };
            ui.label(RichText::new(&row.danger).size(10.0).color(danger_color));
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
