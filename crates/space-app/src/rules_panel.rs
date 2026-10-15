//! Rule manager UI. The left column lists categories grouped by what they
//! clean, with the space the last check found; the table lists the rules of
//! the chosen category, largest finds first. List-only rules live in their own
//! "Manual review" section, and rules whose folders don't exist on this disk
//! stay hidden unless asked for.

use crate::i18n::t;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use eframe::egui::{self, Align, Layout, RichText};
use space_clean::{CategoryPolicy, JunkKind, Risk, RuleFilter, RuleManager, RuleRow, ScanReport};

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

/// Rules that clean automatically, or rules that only list what they find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Automatic,
    Manual,
}

pub struct RulesPanelState {
    pub open: bool,
    pub query: String,
    pub category: Option<String>,
    pub tag: Option<String>,
    pub kind: Option<JunkKind>,
    pub risk: Option<Risk>,
    pub enabled: EnabledFilter,
    pub(crate) section: Section,
    /// Also list rules none of whose folders exist on this disk.
    pub show_absent: bool,
    pub error: Option<String>,
    pub dirty: bool,
    /// Preferences changed; the app writes them after a quiet period.
    pub save_pending: Option<std::time::Instant>,
    /// Cap rows drawn per frame for responsiveness with many rules.
    pub page_size: usize,
    facts: Option<Arc<DiskFacts>>,
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
            section: Section::Automatic,
            show_absent: false,
            error: None,
            dirty: false,
            save_pending: None,
            page_size: 200,
            facts: None,
            summary: None,
            page: None,
        }
    }
}

/// What this disk says about the rules: which have a folder here, and how
/// much the last junk check found per rule.
struct DiskFacts {
    pack: Arc<space_clean::RulePack>,
    report_version: Option<u64>,
    present: HashSet<String>,
    by_rule: HashMap<String, u64>,
}

impl DiskFacts {
    fn shown(&self, show_absent: bool, rule: &space_clean::Rule) -> bool {
        show_absent || self.present.contains(&rule.id)
    }

    fn found(&self, id: &str) -> u64 {
        self.by_rule.get(id).copied().unwrap_or(0)
    }
}

/// One category in the left column, counted over the rules it shows.
struct CategorySide {
    name: String,
    total: usize,
    enabled: usize,
    bytes: u64,
    policy: CategoryPolicy,
}

struct RuleSummary {
    pack: Arc<space_clean::RulePack>,
    prefs: space_clean::RulePrefs,
    facts: Arc<DiskFacts>,
    section: Section,
    show_absent: bool,
    /// Automatic rules for this platform: all, and how many are on.
    host: usize,
    enabled: usize,
    categories: Vec<CategorySide>,
    /// Rules in the current section, and how many of them are listed.
    in_section: usize,
    listed: usize,
    manual: usize,
    bytes: u64,
}

struct RulePage {
    pack: Arc<space_clean::RulePack>,
    prefs: space_clean::RulePrefs,
    facts: Arc<DiskFacts>,
    show_absent: bool,
    filter: RuleFilter,
    limit: usize,
    total: usize,
    rows: Vec<RuleRow>,
}

impl RulesPanelState {
    /// Queue a preferences write; `app.rs` debounces and runs it off-thread.
    pub fn mark_save(&mut self) {
        self.error = None;
        self.save_pending = Some(std::time::Instant::now());
    }

    fn facts(&mut self, mgr: &RuleManager, report: Option<(u64, &ScanReport)>) -> Arc<DiskFacts> {
        let version = report.map(|(version, _)| version);
        if let Some(facts) = &self.facts {
            if Arc::ptr_eq(&facts.pack, &mgr.pack) && facts.report_version == version {
                return Arc::clone(facts);
            }
        }
        let index = mgr.index();
        let present = space_clean::rules_present_on_disk(
            index.host_positions().iter().map(|&position| &mgr.pack.rules[position]),
        );
        let mut by_rule: HashMap<String, u64> = HashMap::new();
        for finding in report.map(|(_, report)| report.findings.as_slice()).unwrap_or_default() {
            *by_rule.entry(finding.rule_id.clone()).or_default() += finding.bytes;
        }
        let facts = Arc::new(DiskFacts {
            pack: Arc::clone(&mgr.pack),
            report_version: version,
            present,
            by_rule,
        });
        self.facts = Some(Arc::clone(&facts));
        facts
    }

    fn summary(&mut self, mgr: &RuleManager, facts: &Arc<DiskFacts>) -> Arc<RuleSummary> {
        if let Some(summary) = &self.summary {
            if Arc::ptr_eq(&summary.pack, &mgr.pack)
                && Arc::ptr_eq(&summary.facts, facts)
                && summary.prefs == mgr.prefs
                && summary.section == self.section
                && summary.show_absent == self.show_absent
            {
                return Arc::clone(summary);
            }
        }
        let manual_section = self.section == Section::Manual;
        let policies: HashMap<String, CategoryPolicy> = mgr
            .categories(true)
            .into_iter()
            .map(|row| (row.name, row.policy))
            .collect();
        let mut sides: HashMap<&str, CategorySide> = HashMap::new();
        let (mut host, mut enabled, mut in_section, mut listed, mut manual) = (0, 0, 0, 0, 0);
        for &position in mgr.index().host_positions() {
            let rule = &mgr.pack.rules[position];
            let report_only = rule.is_report_only();
            let shown = facts.shown(self.show_absent, rule);
            let on = mgr.prefs.is_enabled(rule);
            if report_only {
                manual += usize::from(shown);
            } else {
                host += 1;
                enabled += usize::from(on);
            }
            if report_only != manual_section {
                continue;
            }
            in_section += 1;
            if !shown {
                continue;
            }
            listed += 1;
            let side = sides.entry(rule.category.as_str()).or_insert_with(|| CategorySide {
                name: rule.category.clone(),
                total: 0,
                enabled: 0,
                bytes: 0,
                policy: policies.get(&rule.category).copied().unwrap_or(CategoryPolicy::Default),
            });
            side.total += 1;
            side.enabled += usize::from(on);
            side.bytes += facts.found(&rule.id);
        }
        let mut categories: Vec<CategorySide> = sides.into_values().collect();
        categories.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
        let bytes = categories.iter().map(|side| side.bytes).sum();
        let summary = Arc::new(RuleSummary {
            pack: Arc::clone(&mgr.pack),
            prefs: mgr.prefs.clone(),
            facts: Arc::clone(facts),
            section: self.section,
            show_absent: self.show_absent,
            host,
            enabled,
            categories,
            in_section,
            listed,
            manual,
            bytes,
        });
        self.summary = Some(Arc::clone(&summary));
        summary
    }

    fn page(&mut self, mgr: &RuleManager, facts: &Arc<DiskFacts>, filter: RuleFilter) -> Arc<RulePage> {
        if let Some(page) = &self.page {
            if Arc::ptr_eq(&page.pack, &mgr.pack)
                && Arc::ptr_eq(&page.facts, facts)
                && page.prefs == mgr.prefs
                && page.show_absent == self.show_absent
                && page.filter == filter
                && page.limit == self.page_size
            {
                return Arc::clone(page);
            }
        }
        let show_absent = self.show_absent;
        let (total, rows) = mgr.list_page_ranked(
            &filter,
            self.page_size,
            |rule| facts.shown(show_absent, rule),
            |rule| facts.found(&rule.id),
        );
        let page = Arc::new(RulePage {
            pack: Arc::clone(&mgr.pack),
            prefs: mgr.prefs.clone(),
            facts: Arc::clone(facts),
            show_absent,
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

/// `report` is the last junk check with its version, for found sizes.
pub fn draw(
    ui: &mut egui::Ui,
    state: &mut RulesPanelState,
    mgr: &mut RuleManager,
    flags: &mut FeatureFlags,
    report: Option<(u64, &ScanReport)>,
) -> RulesAction {
    let mut action = RulesAction::None;
    let facts = state.facts(mgr, report);
    let summary = state.summary(mgr, &facts);
    let manual = state.section == Section::Manual;

    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(crate::tr!(
                "{} of {} on · {} off",
                summary.enabled,
                crate::i18n::count(summary.host, "rule"),
                summary.host - summary.enabled,
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
                state.mark_save();
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
                state.mark_save();
                action = RulesAction::PrefsChanged;
            }
        });
    });

    ui.horizontal(|ui| {
        ui.label(RichText::new(t("Privacy")).color(MUTED()));
        if ui
            .checkbox(&mut flags.scan_browser_cookies, t("Cookies"))
            .on_hover_text(t(
                "Scan/clean browser cookies. Off by default — signs you out of websites.",
            ))
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
        ui.add(
            egui::TextEdit::singleline(&mut state.query)
                .desired_width(200.0)
                .hint_text(t("Search name or description…")),
        );
        if !manual {
            ui.separator();
            for (label, mode, hint) in [
                ("All", EnabledFilter::All, "All rules"),
                ("Enabled rules", EnabledFilter::On, "Rules that will run"),
                ("Disabled rules", EnabledFilter::Off, "Rules that will not run"),
                ("Changed by you", EnabledFilter::Overridden, "Rules you turned on or off yourself"),
            ] {
                if crate::widgets::chip(ui, state.enabled == mode, label)
                    .on_hover_text(t(hint))
                    .clicked()
                {
                    state.enabled = mode;
                }
            }
            ui.separator();
            for (label, risk, hint) in [
                ("Any risk", None, "Rules of every caution level"),
                ("Safe", Some(Risk::Low), "Rebuilt or re-downloaded automatically"),
                ("Check", Some(Risk::Medium), "Usually fine — check the list before trashing"),
                ("Careful", Some(Risk::High), "May hold data you want — check each item"),
            ] {
                if crate::widgets::chip(ui, state.risk == risk, label)
                    .on_hover_text(t(hint))
                    .clicked()
                {
                    state.risk = risk;
                }
            }
        }
        ui.separator();
        egui::ComboBox::from_id_salt("rule_kind")
            .width(120.0)
            .selected_text(match state.kind {
                Some(kind) => t(kind_title(kind)).to_owned(),
                None => t("Any type").to_owned(),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.kind, None, t("Any type"));
                for &kind in JunkKind::all() {
                    ui.selectable_value(&mut state.kind, Some(kind), t(kind_title(kind)));
                }
            });
        if let Some(tag) = state.tag.clone() {
            if crate::widgets::chip(ui, true, &format!("#{tag}  ✕"))
                .on_hover_text(t("Clear the tag filter"))
                .clicked()
            {
                state.tag = None;
            }
        }
    });

    ui.add_space(8.0);

    let filter = build_filter(state);
    let page = state.page(mgr, &facts, filter);
    let rows = &page.rows;
    let cats = &summary.categories;

    let avail_h = (ui.available_height() - 64.0).max(160.0);
    ui.horizontal_top(|ui| {
        // Left: the two sections, then categories grouped by what they clean.
        ui.allocate_ui_with_layout(
            egui::vec2(260.0, avail_h),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(260.0);
                ui.horizontal(|ui| {
                    if crate::widgets::chip(ui, !manual, "Cleaned automatically")
                        .on_hover_text(t("Rules that move what they find to the Trash"))
                        .clicked()
                        && manual
                    {
                        state.section = Section::Automatic;
                        state.category = None;
                    }
                    if crate::widgets::chip(ui, manual, &crate::tr!("Manual review ({})", summary.manual))
                        .on_hover_text(t("Rules that only list what they find; you decide what to remove"))
                        .clicked()
                        && !manual
                    {
                        state.section = Section::Manual;
                        state.category = None;
                    }
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("rules_side")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let all = side_detail(summary.bytes, summary.listed);
                        if side_row(ui, state.category.is_none(), t("All categories"), &all, None).0 {
                            state.category = None;
                        }
                        for (group, members) in category_groups(cats) {
                            ui.add_space(6.0);
                            ui.label(widgets::caption(group).color(MUTED()));
                            for cat in members {
                                let selected = state.category.as_deref() == Some(cat.name.as_str());
                                let switch = (!manual).then_some((cat.enabled, cat.total));
                                let detail = side_detail(cat.bytes, cat.total);
                                let (pick, flip) = side_row(ui, selected, t(&category_title(&cat.name)), &detail, switch);
                                if flip {
                                    let _ = mgr.set_category_enabled(&cat.name, cat.enabled < cat.total);
                                    state.dirty = true;
                                    state.mark_save();
                                    action = RulesAction::PrefsChanged;
                                } else if pick {
                                    state.category = Some(cat.name.clone());
                                }
                            }
                        }
                        let hidden = summary.in_section - summary.listed;
                        ui.add_space(8.0);
                        if hidden > 0 || state.show_absent {
                            let label = if state.show_absent {
                                t("Hide rules not found on this disk").to_owned()
                            } else {
                                crate::tr!("Not found on this disk: {}", hidden)
                            };
                            if ui
                                .add(egui::Button::new(widgets::caption(label)).frame(false))
                                .on_hover_text(t("These rules look for folders that don't exist here, so they would find nothing"))
                                .clicked()
                            {
                                state.show_absent = !state.show_absent;
                            }
                        }
                    });
            },
        );

        // Right: the rules as a table, largest finds first.
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), avail_h),
            Layout::top_down(Align::Min),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t(if manual { "Manual review" } else { "Rules" })).strong().color(TEXT()));
                    ui.label(RichText::new(format!("({})", page.total)).size(12.0).color(MUTED()));
                    let current = cats
                        .iter()
                        .find(|cat| state.category.as_deref() == Some(cat.name.as_str()));
                    if let Some(cat) = current {
                        ui.label(RichText::new(format!("· {}", t(&category_title(&cat.name)))).size(12.0).color(ACCENT()));
                        if !manual && cat.enabled > 0 && cat.enabled < cat.total {
                            ui.label(RichText::new(crate::tr!("Partially enabled: {} of {}", cat.enabled, cat.total)).size(12.0).color(ACCENT()));
                        }
                        if !manual
                            && cat.policy == CategoryPolicy::ForceOff
                            && cat.enabled > cat.total / 2
                            && ui.button(t("Enable category")).on_hover_text(t("Enable defaults for this category, keeping individual exceptions. New rules will follow the category setting.")).clicked()
                        {
                            let _ = mgr.set_category_enabled(&cat.name, true);
                            state.dirty = true;
                            state.mark_save();
                            action = RulesAction::PrefsChanged;
                        }
                    }
                    if report.is_none() {
                        ui.label(widgets::caption("Run a check in Cleanup to see sizes").color(MUTED()));
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
                                    draw_rule_row(ui, state, mgr, row, facts.found(&row.id), &mut action);
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
    ui.horizontal(|ui| {
        if ui.button(t("Close")).clicked() {
            action = RulesAction::Close;
        }
        if state.dirty && state.error.is_none() {
            ui.label(RichText::new(t("Saved")).size(11.0).color(MUTED()));
        }
        if manual {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(widgets::caption("Lock: listed only — never removed automatically. Open it in Disk usage to remove by hand."));
            });
        }
    });

    action
}

/// Size found by the last check, or the rule count before any check.
fn side_detail(bytes: u64, rules: usize) -> String {
    if bytes > 0 {
        crate::i18n::bytes(bytes)
    } else {
        rules.to_string()
    }
}

fn draw_rule_row(
    ui: &mut egui::Ui,
    state: &mut RulesPanelState,
    mgr: &mut RuleManager,
    row: &RuleRow,
    found: u64,
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
                        state.mark_save();
                        *action = RulesAction::PrefsChanged;
                    }
                }

                let color = if row.effectively_enabled || locked { TEXT() } else { MUTED() };
                let action_w = widgets::text_width(
                    ui,
                    t(if locked { "Remove manually…" } else { "Reset" }),
                    egui::FontId::proportional(theme::type_scale::BODY),
                ) + 24.0;
                let size_w = if found > 0 { 80.0 } else { 0.0 };
                let text_w = (ui.available_width() - action_w - size_w - 10.0).max(80.0);
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
                        for tag in row.tags.iter().take(4) {
                            let chip = egui::Label::new(
                                RichText::new(format!("#{tag}")).size(theme::type_scale::CAPTION).color(MUTED()),
                            )
                            .sense(egui::Sense::click());
                            if ui.add(chip).on_hover_text(t("Show rules with this tag")).clicked() {
                                state.tag = Some(tag.clone());
                            }
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
                        state.mark_save();
                        *action = RulesAction::PrefsChanged;
                    }
                    if found > 0 {
                        ui.label(
                            RichText::new(crate::i18n::bytes(found))
                                .family(crate::theme::bold())
                                .color(TEXT()),
                        )
                        .on_hover_text(t("Found by the last check"));
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
    detail: &str,
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
    let mut right = if switch.is_some() {
        rect.right() - 48.0
    } else {
        rect.right() - 8.0
    };
    if !detail.is_empty() {
        let g = p.layout_no_wrap(detail.to_owned(), egui::FontId::proportional(12.0), MUTED());
        let at = egui::pos2(right - g.size().x, rect.center().y - g.size().y / 2.0);
        right = at.x - 8.0;
        p.galley(at, g, MUTED());
    }
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
        report_only: Some(state.section == Section::Manual),
        host_only: true,
        overridden_only,
    }
}


/// Left-column groups of catalog categories, in display order. Categories
/// missing here land under "Other"; names come from the Cleanup page.
const CATEGORY_GROUPS: &[(&str, &[&str])] = &[
    (
        "Caches",
        &[
            "app-cache", "caches", "browser-cache", "package-cache", "community-sources",
            "shader-cache", "media-cache", "game-cache", "ml-cache", "vm-cache", "wine-cache",
        ],
    ),
    ("Logs and leftovers", &["logs", "temp", "leftovers", "system", "report", "misc"]),
    (
        "Development",
        &["dev", "node-projects", "dotnet-projects", "language-projects", "databases"],
    ),
    (
        "Large content",
        &["ml-models", "vm-disk", "games-downloads", "media-library", "sample-library", "app-data"],
    ),
    ("Privacy", &["browser-privacy"]),
];

fn category_title(name: &str) -> String {
    crate::junk_panel::cat_title(name)
}

/// Categories split into the left-column groups; each group keeps the
/// incoming order (largest finds first) and empty groups are dropped.
fn category_groups(cats: &[CategorySide]) -> Vec<(&'static str, Vec<&CategorySide>)> {
    let mut groups: Vec<(&'static str, Vec<&CategorySide>)> = CATEGORY_GROUPS
        .iter()
        .map(|(group, _)| (*group, Vec::new()))
        .chain(std::iter::once(("Other", Vec::new())))
        .collect();
    for cat in cats {
        let at = CATEGORY_GROUPS
            .iter()
            .position(|(_, members)| members.contains(&cat.name.as_str()))
            .unwrap_or(CATEGORY_GROUPS.len());
        groups[at].1.push(cat);
    }
    groups.retain(|(_, members)| !members.is_empty());
    groups
}

fn kind_title(kind: JunkKind) -> &'static str {
    match kind {
        JunkKind::Cache => "Cache",
        JunkKind::Logs => "Logs",
        JunkKind::Temp => "Temp files",
        JunkKind::Downloads => "Downloads",
        JunkKind::Leftovers => "Leftovers",
        JunkKind::Packages => "Packages",
        JunkKind::Browser => "Browser",
        JunkKind::System => "System",
        JunkKind::AppData => "App data",
        JunkKind::Media => "Media",
        JunkKind::Databases => "Databases",
        JunkKind::Models => "Models",
        JunkKind::Vm => "Virtual machines",
        JunkKind::Dev => "Developer",
        JunkKind::Other => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(present: &std::path::Path) -> RuleManager {
        let template: space_clean::Rule = serde_json::from_value(serde_json::json!({
            "id": "fixture", "name": "fixture", "description": "fixture",
            "paths": [present.to_string_lossy()], "platform": "any", "category": "logs"
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
        RuleManager {
            pack: Arc::new(space_clean::RulePack {
                rules,
                sources: Vec::new(),
            }),
            prefs: space_clean::RulePrefs::default(),
        }
    }

    fn report(sizes: &[(&str, u64)]) -> ScanReport {
        ScanReport {
            findings: sizes
                .iter()
                .map(|&(id, bytes)| space_clean::Finding {
                    rule_id: id.into(),
                    category: "logs".into(),
                    risk: "low".into(),
                    path: format!("/tmp/{id}").into(),
                    bytes,
                    is_dir: true,
                    report_only: false,
                    root: None,
                    min_age_days: 0,
                })
                .collect(),
            total_bytes: 0,
            rules_matched: 0,
            rules_scanned: 0,
            skipped_roots: Vec::new(),
        }
    }

    #[test]
    fn panel_reuses_unchanged_data_and_invalidates_filters_prefs_and_pages() {
        let dir = std::env::temp_dir();
        let mut mgr = fixture(&dir);
        let mut state = RulesPanelState::default();
        let facts = state.facts(&mgr, None);
        assert!(Arc::ptr_eq(&facts, &state.facts(&mgr, None)));
        let summary = state.summary(&mgr, &facts);
        assert!(Arc::ptr_eq(&summary, &state.summary(&mgr, &facts)));
        let filter = build_filter(&state);
        let page = state.page(&mgr, &facts, filter.clone());
        assert_eq!(page.total, 1000);
        assert_eq!(page.rows.len(), 200);
        for _ in 0..100 {
            assert!(Arc::ptr_eq(&page, &state.page(&mgr, &facts, filter.clone())));
        }
        state.page_size = 500;
        let expanded = state.page(&mgr, &facts, filter.clone());
        assert_eq!(expanded.rows.len(), 500);
        assert_eq!(expanded.rows[199].id, page.rows[199].id);
        mgr.prefs.rules.insert("rule-0000".into(), false);
        let updated = state.page(&mgr, &facts, filter);
        assert!(!updated.rows[0].effectively_enabled);
        assert!(!Arc::ptr_eq(&summary, &state.summary(&mgr, &facts)));
        state.query = "rule-0000".into();
        let filtered = state.page(&mgr, &facts, build_filter(&state));
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.rows[0].id, "rule-0000");
    }

    #[test]
    fn largest_finds_lead_and_missing_folders_stay_hidden_until_asked() {
        let dir = std::env::temp_dir();
        let mut mgr = fixture(&dir);
        Arc::make_mut(&mut mgr.pack).rules[1].paths = vec!["/space-scanner-missing-root".into()];
        let mut state = RulesPanelState::default();
        let scan = report(&[("rule-0500", 10), ("rule-0700", 30)]);
        let facts = state.facts(&mgr, Some((1, &scan)));
        let summary = state.summary(&mgr, &facts);
        assert_eq!((summary.in_section, summary.listed), (1000, 999));
        assert_eq!(summary.bytes, 40);
        assert_eq!(summary.categories[0].bytes, 40);
        let page = state.page(&mgr, &facts, build_filter(&state));
        assert_eq!(page.total, 999);
        assert_eq!(page.rows[0].id, "rule-0700");
        assert_eq!(page.rows[1].id, "rule-0500");
        assert!(page.rows.iter().all(|row| row.id != "rule-0001"));
        state.show_absent = true;
        assert_eq!(state.page(&mgr, &facts, build_filter(&state)).total, 1000);
        assert_eq!(state.summary(&mgr, &facts).listed, 1000);
        state.section = Section::Manual;
        assert_eq!(state.page(&mgr, &facts, build_filter(&state)).total, 0);
        assert!(!Arc::ptr_eq(&facts, &state.facts(&mgr, Some((2, &scan)))));
    }

    #[test]
    fn every_catalog_category_has_a_group_and_a_translated_title() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules/categories");
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().replace(".yaml", "");
            assert!(
                CATEGORY_GROUPS.iter().any(|(_, members)| members.contains(&name.as_str())),
                "{name} is in no CATEGORY_GROUPS group"
            );
            let title = category_title(&name);
            assert!(
                crate::i18n::has_ru(&title),
                "{name}: {title:?} has no Russian translation"
            );
        }
    }
}
