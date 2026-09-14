//! Flexible rule manager UI — browse / search / toggle categories & rules.

use eframe::egui::{self, Align, Layout, RichText};
use space_clean::{CategoryPolicy, RuleFilter, RuleManager, RuleRow};

use crate::theme::{ACCENT, MUTED, TEXT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnabledFilter {
    All,
    On,
    Off,
    Overridden,
}

pub struct RulesPanelState {
    pub open: bool,
    pub query: String,
    pub category: Option<String>,
    pub enabled: EnabledFilter,
    pub error: Option<String>,
    pub dirty: bool,
    /// Cap rows drawn per frame for responsiveness with 3k rules.
    pub page_size: usize,
}

impl Default for RulesPanelState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            category: None,
            enabled: EnabledFilter::All,
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

pub fn draw(ui: &mut egui::Ui, state: &mut RulesPanelState, mgr: &mut RuleManager) -> RulesAction {
    let mut action = RulesAction::None;
    let stats = mgr.stats();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} rules · {} on · {} off · {} overridden · {} cats",
                stats.host, stats.enabled, stats.disabled, stats.overridden, stats.categories
            ))
            .size(12.0)
            .color(MUTED),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .button("Reset all")
                .on_hover_text("Clear all overrides → YAML defaults")
                .clicked()
            {
                mgr.reset_all();
                state.dirty = true;
                state.error = save(mgr);
                action = RulesAction::PrefsChanged;
            }
            let mut all = mgr.prefs.enable_all_by_default;
            if ui
                .checkbox(&mut all, "Enable all")
                .on_hover_text("Treat every rule as on, then apply per-rule/category overrides")
                .changed()
            {
                mgr.prefs.enable_all_by_default = all;
                state.dirty = true;
                state.error = save(mgr);
                action = RulesAction::PrefsChanged;
            }
        });
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
                .hint_text("id, name, category…"),
        );
        if resp.changed() {
            // filter only
        }
        ui.separator();
        for (label, mode) in [
            ("All", EnabledFilter::All),
            ("On", EnabledFilter::On),
            ("Off", EnabledFilter::Off),
            ("Δ", EnabledFilter::Overridden),
        ] {
            if ui
                .selectable_label(state.enabled == mode, label)
                .on_hover_text(match mode {
                    EnabledFilter::All => "All rules",
                    EnabledFilter::On => "Effectively enabled",
                    EnabledFilter::Off => "Effectively disabled",
                    EnabledFilter::Overridden => "Has prefs override",
                })
                .clicked()
            {
                state.enabled = mode;
            }
        }
    });

    ui.add_space(8.0);

    let cats = mgr.categories(true);
    let filter = build_filter(state);
    let rows = mgr.list(&filter);

    ui.columns(2, |cols| {
        // --- categories ---
        cols[0].label(RichText::new("Categories").strong().color(TEXT));
        egui::ScrollArea::vertical()
            .id_salt("rules_cats")
            .max_height(420.0)
            .show(&mut cols[0], |ui| {
                let all_selected = state.category.is_none();
                let all_resp = ui.selectable_label(
                    all_selected,
                    format!("All categories  ({})", stats.host),
                );
                if all_resp.clicked() {
                    state.category = None;
                }

                for cat in &cats {
                    ui.horizontal(|ui| {
                        let selected = state.category.as_deref() == Some(cat.name.as_str());
                        let policy_mark = match cat.policy {
                            CategoryPolicy::Default => "",
                            CategoryPolicy::ForceOn => " ⊕",
                            CategoryPolicy::ForceOff => " ⊖",
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
                                .small_button("∅")
                                .on_hover_text("Reset category + its rule overrides")
                                .clicked()
                            {
                                mgr.reset_category(&cat.name);
                                state.dirty = true;
                                state.error = save(mgr);
                                action = RulesAction::PrefsChanged;
                            }
                            if ui
                                .small_button("Off")
                                .on_hover_text("Force category off")
                                .clicked()
                            {
                                let _ = mgr.set_category_enabled(&cat.name, false);
                                state.dirty = true;
                                state.error = save(mgr);
                                action = RulesAction::PrefsChanged;
                            }
                            if ui
                                .small_button("On")
                                .on_hover_text("Force category on")
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
            ui.label(RichText::new("Saved to rule-prefs.json").size(11.0).color(MUTED));
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
                "Override active"
            } else {
                "Override YAML default"
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
            format!("Δ {}", row.name)
        } else {
            row.name.clone()
        };
        let resp = ui.label(RichText::new(name).size(12.5).color(color));
        resp.on_hover_ui(|ui| {
            ui.label(RichText::new(&row.id).strong());
            if !row.description.is_empty() {
                ui.label(&row.description);
            }
            for p in row.paths.iter().take(6) {
                ui.monospace(p);
            }
            if row.paths.len() > 6 {
                ui.label(format!("… {} more paths", row.paths.len() - 6));
            }
            ui.label(format!(
                "min age {}d · yaml default {}",
                row.min_age_days,
                if row.default_enabled { "on" } else { "off" }
            ));
        });

        ui.label(RichText::new(&row.id).size(10.0).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button("↺")
                .on_hover_text("Clear rule override")
                .clicked()
            {
                mgr.reset_rule(&row.id);
                state.dirty = true;
                state.error = save(mgr);
                *action = RulesAction::PrefsChanged;
            }
            if row.report_only {
                ui.label(RichText::new("report").size(10.0).color(MUTED));
            }
            ui.label(RichText::new(&row.risk).size(10.0).color(MUTED));
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
    RuleFilter {
        query: state.query.clone(),
        category: state.category.clone(),
        risk: None,
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
