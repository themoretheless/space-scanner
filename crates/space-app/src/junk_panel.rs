//! Junk cleaner panel — category-first, Trash-first (ppduster rules).

use crate::i18n::t;
use std::collections::BTreeMap;
use std::time::Instant;

use eframe::egui::{self, Align, Layout, RichText, Sense};
use space_clean::{CleanProgress, Finding, JunkScanProgress, ScanReport};

use crate::settings::FeatureFlags;
use crate::theme::type_scale;
use crate::theme::{self, ACCENT, DANGER, MUTED, PANEL_2, SAFE, TEXT};
use crate::widgets;

#[derive(Default)]
pub struct JunkPanelState {
    pub open: bool,
    pub loading: bool,
    pub cleaning: bool,
    report: Option<JunkReport>,
    pub progress: JunkScanProgress,
    pub scan_started: Option<Instant>,
    pub clean_progress: CleanProgress,
    pub clean_started: Option<Instant>,
    selection: Selection,
    pub confirm: bool,
    pub expanded_cat: Option<String>,
    pub error: Option<String>,
    pub risk_filter: RiskFilter,
}

/// Which risk group the category list shows.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum RiskFilter {
    #[default]
    All,
    Safe,
    Review,
    ReportOnly,
}

impl RiskFilter {
    fn of(category: &CatAgg) -> Self {
        match (category.cleanable > 0, category.risk) {
            (false, _) => Self::ReportOnly,
            (true, 0) => Self::Safe,
            _ => Self::Review,
        }
    }
}

fn risk_level(risk: &str) -> u8 {
    match risk {
        "high" => 2,
        "medium" => 1,
        _ => 0,
    }
}

/// Short badge: what the user should do before trashing it.
fn badge(cleanable: bool, risk: u8) -> (&'static str, egui::Color32, &'static str) {
    if !cleanable {
        return (
            "Report only",
            MUTED(),
            "Listed for information — never removed",
        );
    }
    match risk {
        0 => ("Safe", SAFE(), "Rebuilt or re-downloaded automatically"),
        1 => (
            "Check",
            ACCENT(),
            "Usually fine — check the list before trashing",
        ),
        _ => (
            "Careful",
            DANGER(),
            "May hold data you want — check each item",
        ),
    }
}

impl JunkPanelState {
    pub fn report(&self) -> Option<&ScanReport> {
        self.report.as_ref().map(|report| &report.scan)
    }

    pub fn selected_count(&self) -> usize {
        self.selection.count
    }

    /// Junk rule covering `path` (the path itself or a parent), from the last junk scan.
    pub fn risk_hint(
        &self,
        path: &std::path::Path,
    ) -> Option<(String, &'static str, egui::Color32, &'static str)> {
        let f = self
            .report()?
            .findings
            .iter()
            .filter(|f| path.starts_with(&f.path))
            .max_by_key(|f| f.path.as_os_str().len())?;
        let (label, color, hint) = badge(!f.report_only, risk_level(&f.risk));
        Some((cat_title(&f.category), label, color, hint))
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn open_and_scan(&mut self) {
        self.open = true;
        self.loading = true;
        self.cleaning = false;
        self.error = None;
        self.progress = JunkScanProgress::default();
        self.scan_started = Some(Instant::now());
        self.clean_progress = CleanProgress::default();
        self.clean_started = None;
        self.confirm = false;
        self.clear_selection();
        self.expanded_cat = None;
    }

    pub fn set_report(&mut self, report: ScanReport) {
        self.loading = false;
        self.cleaning = false;
        self.scan_started = None;
        self.clean_started = None;
        self.confirm = false;
        self.selection = Selection::from_findings(&report.findings);
        // Discovery never authorizes removal, including low-risk candidates.
        self.selection.clear();
        let report = JunkReport::new(report);
        // Expand the largest cleanable category by default.
        self.expanded_cat = report
            .categories
            .iter()
            .filter(|category| category.cleanable > 0)
            .max_by_key(|category| (category.bytes, &category.name))
            .map(|category| category.name.clone());
        self.report = Some(report);
    }

    pub fn selected_findings(&self) -> Vec<Finding> {
        let Some(report) = &self.report else {
            return Vec::new();
        };
        report
            .scan
            .findings
            .iter()
            .enumerate()
            .filter(|(index, finding)| self.selection.contains(*index) && !finding.report_only)
            .map(|(_, finding)| finding.clone())
            .collect()
    }

    pub fn exclude_paths(&mut self, paths: &[std::path::PathBuf]) {
        let Some(report) = &self.report else { return };
        for (index, finding) in report.scan.findings.iter().enumerate() {
            if paths.iter().any(|path| finding.path.starts_with(path)) {
                self.selection.set(index, finding, false);
            }
        }
    }

    pub fn selected_bytes(&self) -> u64 {
        self.selection.bytes()
    }
}

/// Keeps checkbox membership and its displayed totals in sync.
#[derive(Default)]
struct Selection {
    included: Vec<bool>,
    count: usize,
    // Retain the exact sum so deselection stays correct even if the display saturates.
    bytes: u128,
}

impl Selection {
    fn from_findings(findings: &[Finding]) -> Self {
        let mut selection = Self {
            included: vec![false; findings.len()],
            ..Self::default()
        };
        for (index, finding) in findings.iter().enumerate() {
            selection.set(index, finding, true);
        }
        selection
    }

    fn contains(&self, index: usize) -> bool {
        self.included.get(index).copied().unwrap_or(false)
    }

    fn set(&mut self, index: usize, finding: &Finding, included: bool) {
        let Some(current) = self.included.get_mut(index) else {
            return;
        };
        let included = included && !finding.report_only;
        if *current == included {
            return;
        }
        *current = included;
        if included {
            self.count += 1;
            self.bytes += u128::from(finding.bytes);
        } else {
            self.count -= 1;
            self.bytes -= u128::from(finding.bytes);
        }
    }

    fn set_category(&mut self, category: &CatAgg, findings: &[Finding], included: bool) {
        for &index in &category.indices {
            self.set(index, &findings[index], included);
        }
    }

    fn clear(&mut self) {
        self.included.fill(false);
        self.count = 0;
        self.bytes = 0;
    }

    fn bytes(&self) -> u64 {
        self.bytes.min(u128::from(u64::MAX)) as u64
    }
}

pub enum JunkAction {
    None,
    Rescan,
    Close,
    OpenRules,
    PrefsChanged,
}

struct CatAgg {
    name: String,
    bytes: u64,
    cleanable: usize,
    report_only: usize,
    indices: Vec<usize>,
    /// Highest rule risk among removable items: 0 low, 1 medium, 2 high.
    risk: u8,
}

/// Immutable report data, prepared once when a scan completes.
struct JunkReport {
    scan: ScanReport,
    categories: Vec<CatAgg>,
    cleanable_bytes: u64,
    cleanable_count: usize,
}

impl JunkReport {
    fn new(scan: ScanReport) -> Self {
        let mut categories = BTreeMap::new();
        let mut cleanable_bytes = 0_u64;
        let mut cleanable_count = 0;
        for (index, finding) in scan.findings.iter().enumerate() {
            let category = categories
                .entry(finding.category.as_str())
                .or_insert_with(|| CatAgg {
                    name: finding.category.clone(),
                    bytes: 0,
                    cleanable: 0,
                    report_only: 0,
                    indices: Vec::new(),
                    risk: 0,
                });
            category.indices.push(index);
            if finding.report_only {
                category.report_only += 1;
            } else {
                category.bytes = category.bytes.saturating_add(finding.bytes);
                category.cleanable += 1;
                category.risk = category.risk.max(risk_level(&finding.risk));
                cleanable_bytes = cleanable_bytes.saturating_add(finding.bytes);
                cleanable_count += 1;
            }
        }
        let mut categories: Vec<_> = categories.into_values().collect();
        categories.sort_by_key(|category| std::cmp::Reverse(category.bytes));
        Self {
            scan,
            categories,
            cleanable_bytes,
            cleanable_count,
        }
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut JunkPanelState, flags: &mut FeatureFlags) -> JunkAction {
    let mut action = JunkAction::None;

    // One header row: what this is on the left, the two actions on the right.
    ui.vertical(|ui| {
        ui.vertical(|ui| {
            let (note, total) = match state.report.as_ref() {
                _ if state.loading => ("Looking through caches, logs and temp…".to_string(), None),
                Some(r) => (
                    if state.selection.count == 0 {
                        crate::tr!(
                            "{} removable items in {} categories · nothing selected",
                            r.cleanable_count,
                            r.categories.len()
                        )
                    } else {
                        crate::tr!(
                            "{} removable items in {} categories · {} selected ({})",
                            r.cleanable_count,
                            r.categories.len(),
                            state.selection.count,
                            crate::i18n::bytes(state.selection.bytes())
                        )
                    },
                    Some(r.cleanable_bytes),
                ),
                None => ("Caches, logs and temporary files".to_string(), None),
            };
            ui.label(widgets::caption(note));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    RichText::new(t("Cleanup candidates"))
                        .size(type_scale::DISPLAY)
                        .strong(),
                );
                if let Some(bytes) = total {
                    ui.label(
                        RichText::new(crate::i18n::bytes(bytes))
                            .size(type_scale::DISPLAY)
                            .strong()
                            .color(ACCENT()),
                    );
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(widgets::secondary("Rules…"))
                .on_hover_text(t("Turn individual rules on or off"))
                .clicked()
            {
                action = JunkAction::OpenRules;
            }
            if ui
                .add_enabled(
                    !state.loading && !state.cleaning,
                    widgets::secondary("Search again"),
                )
                .clicked()
            {
                action = JunkAction::Rescan;
            }
        });
    });
    ui.add_space(theme::metrics::S2);
    ui.horizontal_wrapped(|ui| {
        ui.label(widgets::caption("Also scan"));
        if ui
            .checkbox(&mut flags.scan_browser_cookies, "Cookies")
            .on_hover_text(t(
                "Off by default — cleaning cookies signs you out of websites",
            ))
            .changed()
        {
            action = JunkAction::PrefsChanged;
        }
        if ui
            .checkbox(&mut flags.scan_browser_history, "History")
            .on_hover_text(t("Off by default"))
            .changed()
        {
            action = JunkAction::PrefsChanged;
        }
    });
    ui.add_space(10.0);

    if let Some(err) = &state.error {
        ui.label(RichText::new(err).color(theme::DANGER()));
        ui.add_space(6.0);
    }

    if state.cleaning {
        let p = &state.clean_progress;
        let fraction = if p.items_total > 0 {
            p.items_done as f32 / p.items_total as f32
        } else {
            0.0
        };
        let elapsed = state
            .clean_started
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(
                RichText::new(crate::tr!(
                    "Moving to Trash · {}/{} items · {} files · {} folders · {} · {} errors · {}s",
                    p.items_done,
                    p.items_total,
                    p.files_done,
                    p.folders_done,
                    crate::i18n::bytes(p.bytes_processed),
                    p.error_count,
                    elapsed
                ))
                .color(MUTED()),
            );
        });
        ui.add_space(6.0);
        ui.add(
            egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                .desired_width(ui.available_width())
                .show_percentage(),
        );
        if !p.current_path.as_os_str().is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(p.current_path.display().to_string())
                        .small()
                        .monospace()
                        .color(MUTED()),
                )
                .truncate(),
            )
            .on_hover_text(p.current_path.display().to_string());
        }
        return action;
    }

    if state.loading {
        let p = &state.progress;
        let fraction = if p.roots_total > 0 {
            p.roots_done as f32 / p.roots_total as f32
        } else if p.rules_total > 0 {
            p.rules_done as f32 / p.rules_total as f32
        } else {
            0.0
        };
        let elapsed = state
            .scan_started
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(
                RichText::new(crate::tr!(
                    "{} · {} removable files · {} folders · {} ({}) · {}s",
                    if p.rules_total == 0 {
                        t("Loading rules…").to_string()
                    } else {
                        crate::tr!("Scanning {}/{} rules", p.rules_done, p.rules_total)
                    },
                    p.files_found,
                    p.folders_found,
                    crate::i18n::bytes(p.bytes_found),
                    p.findings_found,
                    elapsed
                ))
                .color(MUTED()),
            );
        });
        ui.add_space(6.0);
        ui.add(
            egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                .desired_width(ui.available_width())
                .animate(p.rules_total == 0),
        );
        if !p.current_rule.is_empty() {
            ui.label(
                RichText::new(crate::tr!("Looking at {}", p.current_rule))
                    .small()
                    .color(TEXT()),
            );
        }
        if !p.current_path.as_os_str().is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(p.current_path.display().to_string())
                        .small()
                        .monospace()
                        .color(MUTED()),
                )
                .truncate(),
            )
            .on_hover_text(p.current_path.display().to_string());
        }
        return action;
    }

    let Some(report) = state.report.as_ref() else {
        ui.label(
            RichText::new(t(
                "Still scanning, or press Rescan if this is empty by mistake.",
            ))
            .color(MUTED()),
        );
        ui.add_space(12.0);
        if ui.button(t("Close")).clicked() {
            action = JunkAction::Close;
        }
        return action;
    };

    // Risk filter chips, with counts per group.
    ui.horizontal_wrapped(|ui| {
        for (f, label) in [
            (RiskFilter::All, "All"),
            (RiskFilter::Safe, "Safe"),
            (RiskFilter::Review, "Check"),
            (RiskFilter::ReportOnly, "Report only"),
        ] {
            let n = report
                .categories
                .iter()
                .filter(|c| f == RiskFilter::All || RiskFilter::of(c) == f)
                .count();
            let on = state.risk_filter == f;
            let chip =
                egui::Button::new(RichText::new(format!("{} · {n}", t(label))).color(if on {
                    theme::BG()
                } else {
                    MUTED()
                }))
                .fill(if on {
                    TEXT()
                } else {
                    egui::Color32::TRANSPARENT
                })
                .stroke(if on {
                    egui::Stroke::NONE
                } else {
                    egui::Stroke::new(1.0_f32, theme::BORDER())
                })
                .corner_radius(theme::metrics::RADIUS_CONTROL);
            if ui.add(chip).clicked() {
                state.risk_filter = f;
            }
        }
    });
    ui.add_space(8.0);
    let filter = state.risk_filter;
    let cat_list: Vec<&CatAgg> = report
        .categories
        .iter()
        .filter(|c| filter == RiskFilter::All || RiskFilter::of(c) == filter)
        .collect();

    if cat_list.is_empty() {
        ui.label(
            RichText::new(t("Nothing to clean right now."))
                .size(14.0)
                .color(TEXT()),
        );
        ui.label(
            RichText::new(t("Try Rules to turn more categories on, or Rescan."))
                .small()
                .color(MUTED()),
        );
        ui.add_space(8.0);
    }

    let findings = &report.scan.findings;
    // Density follows the skin: Table is a tight grid, the others breathe.
    let row_h = 46.0;
    let radius: u8 = 6;
    egui::Frame::new()
        .fill(theme::PANEL())
        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER()))
        .corner_radius(radius)
        .show(ui, |ui| {
            // Column header.
            let (head, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::hover());
            ui.painter().rect_filled(head, 0.0, PANEL_2());
            let cols = CatCols::new(head);
            let f = egui::FontId::proportional(type_scale::CAPTION);
            for (x, label, right) in [
                (cols.name, t("CATEGORY"), false),
                (cols.source, t("SOURCE"), false),
                (cols.risk, t("RISK"), false),
                (cols.size_r, t("SIZE"), true),
            ] {
                let g = ui
                    .painter()
                    .layout_no_wrap(label.to_string(), f.clone(), MUTED());
                let px = if right { x - g.size().x } else { x };
                ui.painter().galley(
                    egui::pos2(px, head.center().y - g.size().y / 2.0),
                    g,
                    MUTED(),
                );
            }

            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - 40.0).max(160.0))
                .id_salt("junk_cats")
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for cat in cat_list.iter().copied() {
                        let open = state.expanded_cat.as_deref() == Some(cat.name.as_str());
                        let picked = cat
                            .indices
                            .iter()
                            .filter(|&&i| state.selection.contains(i))
                            .count();
                        let all = cat.cleanable > 0 && picked == cat.cleanable;
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), row_h),
                            Sense::click(),
                        );
                        let cols = CatCols::new(rect);
                        let p = ui.painter();
                        if picked > 0 {
                            p.rect_filled(rect, 0.0, ACCENT().gamma_multiply(0.08));
                        } else if resp.hovered() {
                            p.rect_filled(rect, 0.0, PANEL_2().gamma_multiply(0.6));
                        }
                        p.hline(
                            rect.x_range(),
                            rect.bottom() - 0.5,
                            egui::Stroke::new(1.0_f32, theme::BORDER()),
                        );
                        let cy = rect.center().y;

                        // Checkbox: the whole category in or out of review.
                        let cb = egui::Rect::from_center_size(
                            egui::pos2(cols.left + 8.0, cy),
                            egui::vec2(16.0, 16.0),
                        );
                        if cat.cleanable > 0 {
                            if all {
                                p.rect_filled(cb, 4.0, ACCENT());
                                crate::icons::paint(
                                    p,
                                    crate::icons::Icon::Check,
                                    cb.shrink(2.5),
                                    theme::ACCENT_INK(),
                                );
                            } else {
                                p.rect_stroke(
                                    cb,
                                    4.0,
                                    egui::Stroke::new(1.5_f32, MUTED()),
                                    egui::StrokeKind::Inside,
                                );
                                if picked > 0 {
                                    p.rect_filled(cb.shrink(4.5), 1.0, ACCENT());
                                }
                            }
                        }
                        crate::icons::paint(
                            p,
                            if open {
                                crate::icons::Icon::ChevronDown
                            } else {
                                crate::icons::Icon::ChevronRight
                            },
                            egui::Rect::from_center_size(
                                egui::pos2(cols.name - 12.0, cy),
                                egui::vec2(12.0, 12.0),
                            ),
                            MUTED(),
                        );

                        let (label, color, hint) = badge(cat.cleanable > 0, cat.risk);
                        let title_w = (cols.source - cols.name - 12.0).max(20.0);
                        let mut title_job = egui::text::LayoutJob::simple_singleline(
                            t(&cat_title(&cat.name)).to_owned(),
                            egui::FontId::proportional(type_scale::BODY),
                            TEXT(),
                        );
                        title_job.wrap = egui::text::TextWrapping::truncate_at_width(title_w);
                        let title_g = p.layout_job(title_job);
                        let mut sub = crate::tr!("{} removable", cat.cleanable);
                        if picked > 0 && picked < cat.cleanable {
                            sub.push_str(&crate::tr!(" · {} selected", picked));
                        }
                        if cat.report_only > 0 {
                            sub.push_str(&crate::tr!(" · {} listed only", cat.report_only));
                        }
                        p.galley(egui::pos2(cols.name, rect.top() + 5.0), title_g, TEXT());
                        let mut job = egui::text::LayoutJob::simple_singleline(
                            sub,
                            egui::FontId::proportional(type_scale::CAPTION),
                            MUTED(),
                        );
                        job.wrap = egui::text::TextWrapping::truncate_at_width(title_w);
                        let sub_g = p.layout_job(job);
                        p.galley(
                            egui::pos2(cols.name, rect.bottom() - sub_g.size().y - 5.0),
                            sub_g,
                            MUTED(),
                        );

                        let mut job = egui::text::LayoutJob::simple_singleline(
                            sources(cat, findings),
                            egui::FontId::proportional(type_scale::CAPTION + 1.0),
                            MUTED(),
                        );
                        job.wrap = egui::text::TextWrapping::truncate_at_width(
                            cols.risk - cols.source - 12.0,
                        );
                        let src_g = p.layout_job(job);
                        p.galley(
                            egui::pos2(cols.source, cy - src_g.size().y / 2.0),
                            src_g,
                            MUTED(),
                        );

                        let risk_g = p.layout_no_wrap(
                            t(label).to_owned(),
                            egui::FontId::proportional(type_scale::CAPTION),
                            color,
                        );
                        let pill = egui::Rect::from_min_size(
                            egui::pos2(cols.risk, cy - 10.0),
                            egui::vec2(risk_g.size().x + 16.0, 20.0),
                        );
                        p.galley(
                            egui::pos2(pill.left() + 8.0, cy - risk_g.size().y / 2.0),
                            risk_g,
                            color,
                        );

                        let size_g = p.layout_no_wrap(
                            crate::i18n::bytes(cat.bytes),
                            egui::FontId::monospace(type_scale::MONO + 1.0),
                            TEXT(),
                        );
                        p.galley(
                            egui::pos2(cols.size_r - size_g.size().x, cy - size_g.size().y / 2.0),
                            size_g,
                            TEXT(),
                        );

                        let resp =
                            resp.on_hover_text(t(if open { "Hide items" } else { "Show items" }));
                        if resp.clicked() {
                            let on_cb = ui
                                .input(|i| i.pointer.interact_pos())
                                .is_some_and(|h| cb.expand(6.0).contains(h));
                            if on_cb && cat.cleanable > 0 {
                                state.selection.set_category(cat, findings, !all);
                            } else {
                                state.expanded_cat =
                                    if open { None } else { Some(cat.name.clone()) };
                            }
                        }
                        if open {
                            egui::Frame::new()
                                .fill(theme::BG())
                                .inner_margin(egui::Margin {
                                    left: 52,
                                    right: 16,
                                    top: 8,
                                    bottom: 10,
                                })
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(widgets::caption(t(hint).to_owned()));
                                    ui.add_space(4.0);
                                    draw_finding_rows(ui, cat, findings, &mut state.selection);
                                });
                        }
                    }
                });
        });

    action
}

/// Column x positions for the category table.
struct CatCols {
    left: f32,
    name: f32,
    source: f32,
    risk: f32,
    size_r: f32,
}

impl CatCols {
    fn new(r: egui::Rect) -> Self {
        let size_r = r.right() - 18.0;
        let risk = size_r - 100.0 - 120.0;
        let source = (risk - 170.0).max(r.left() + 260.0);
        Self {
            left: r.left() + 16.0,
            name: r.left() + 62.0,
            source,
            risk,
            size_r,
        }
    }
}

/// Which apps or tools the category's rules belong to, from rule ids
/// ("xcode.derived-data" → "xcode"); at most two, then a count.
fn sources(cat: &CatAgg, findings: &[Finding]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for &i in &cat.indices {
        let id = findings[i].rule_id.as_str();
        let head = id.split(['.', '/', ':']).next().unwrap_or(id);
        if !names.contains(&head) {
            names.push(head);
        }
    }
    match names.len() {
        0 => String::new(),
        1 | 2 => names.join(", "),
        n => format!("{}, {} +{}", names[0], names[1], n - 2),
    }
}

fn draw_finding_rows(
    ui: &mut egui::Ui,
    category: &CatAgg,
    findings: &[Finding],
    selection: &mut Selection,
) {
    let spacing = ui.spacing().item_spacing.y;
    let row_height = ui
        .spacing()
        .interact_size
        .y
        .max(ui.spacing().icon_width)
        .max(ui.fonts(|fonts| fonts.row_height(&egui::FontId::monospace(type_scale::MONO))));
    let stride = row_height + spacing;
    let total_height = (category.indices.len() as f32 * stride - spacing).max(0.0);
    let (_, rows_rect) = ui.allocate_space(egui::vec2(ui.available_width(), total_height));
    let visible = visible_rows(rows_rect, ui.clip_rect(), stride, category.indices.len());
    let visible_rect = egui::Rect::from_min_max(
        egui::pos2(
            rows_rect.left(),
            rows_rect.top() + visible.start as f32 * stride,
        ),
        rows_rect.right_bottom(),
    );
    let mut rows_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("junk_finding_rows", &category.name))
            .max_rect(visible_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    // One horizontal container per row, with IDs stable while scrolling.
    rows_ui.skip_ahead_auto_ids(visible.start);
    for row in visible {
        let index = category.indices[row];
        let finding = &findings[index];
        rows_ui.horizontal(|ui| {
            let mut included = selection.contains(index);
            let enabled = !finding.report_only;
            if ui
                .add_enabled(enabled, egui::Checkbox::new(&mut included, ""))
                .changed()
            {
                selection.set(index, finding, included);
            }
            let color = if enabled { TEXT() } else { MUTED() };
            ui.label(
                RichText::new(format!("{:>8}", crate::i18n::bytes(finding.bytes)))
                    .monospace()
                    .size(type_scale::MONO)
                    .color(color),
            );
            let path = finding.path.display().to_string();
            ui.add(
                egui::Label::new(
                    RichText::new(&path)
                        .monospace()
                        .size(type_scale::MONO)
                        .color(color),
                )
                .truncate(),
            )
            .on_hover_text(&path);
        });
    }
}

fn visible_rows(
    rows: egui::Rect,
    clip: egui::Rect,
    stride: f32,
    count: usize,
) -> std::ops::Range<usize> {
    if !rows.intersects(clip) {
        return 0..0;
    }
    let start = ((clip.top() - rows.top()).max(0.0) / stride).floor() as usize;
    let end = ((clip.bottom() - rows.top()).max(0.0) / stride).ceil() as usize;
    start.min(count)..end.min(count)
}

fn cat_title(cat: &str) -> String {
    match cat {
        "caches" | "cache" => "Caches".into(),
        "logs" | "app-logs" => "Logs".into(),
        "temp" => "Temp files".into(),
        "leftovers" => "Leftovers".into(),
        "dev" => "Developer".into(),
        "package-cache" => "Package caches".into(),
        "browser-cache" => "Browser caches".into(),
        "app-cache" => "App caches".into(),
        "media-cache" => "Media caches".into(),
        "ml-cache" => "ML caches".into(),
        "ml-models" => "ML models".into(),
        "vm-cache" => "VM caches".into(),
        "vm-disk" => "VM disks".into(),
        "game-cache" => "Game caches".into(),
        "game-downloads" => "Game downloads".into(),
        "shader-cache" => "Shader caches".into(),
        "wine-cache" => "Wine caches".into(),
        "databases" => "Databases".into(),
        "app-data" => "App data".into(),
        "system" => "System".into(),
        "report" => "Report only".into(),
        "media-library" => "Media libraries".into(),
        "sample-library" => "Sample libraries".into(),
        other => {
            let mut s = other.replace('-', " ");
            if let Some(c) = s.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    fn finding(category: &str, bytes: u64, report_only: bool, index: usize) -> Finding {
        Finding {
            rule_id: "benchmark-rule".into(),
            category: category.into(),
            risk: "safe".into(),
            path: format!("/tmp/junk-panel/{index}/cache.bin").into(),
            bytes,
            is_dir: false,
            report_only,
        }
    }

    fn report(findings: Vec<Finding>) -> ScanReport {
        ScanReport {
            total_bytes: findings
                .iter()
                .map(|f| f.bytes)
                .fold(0, u64::saturating_add),
            findings,
            rules_matched: 1,
            rules_scanned: 1,
            skipped_roots: Vec::new(),
        }
    }

    fn draw_frame(ctx: &egui::Context, state: &mut JunkPanelState) -> egui::FullOutput {
        draw_input(ctx, state, Vec::new())
    }

    fn draw_input(
        ctx: &egui::Context,
        state: &mut JunkPanelState,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                black_box(draw(ui, state, &mut FeatureFlags::default()));
            });
        })
    }

    #[test]
    fn discovery_requires_selection_and_successful_removals_exclude_only_covered_paths() {
        let mut state = JunkPanelState::default();
        state.set_report(report(vec![
            finding("cache", 40, false, 0),
            finding("cache", 20, false, 1),
        ]));
        assert_eq!(state.selected_count(), 0);
        for index in 0..2 {
            let finding = state.report().unwrap().findings[index].clone();
            state.selection.set(index, &finding, true);
        }
        state.exclude_paths(&["/tmp/junk-panel/0".into()]);
        assert_eq!(state.selected_count(), 1);
        assert_eq!(state.selected_bytes(), 20);
        assert_eq!(
            state.selected_findings()[0].path,
            std::path::Path::new("/tmp/junk-panel/1/cache.bin")
        );
    }

    #[test]
    fn report_aggregates_preserve_order_and_exclude_report_only_bytes() {
        let mut state = JunkPanelState::default();
        state.set_report(report(vec![
            finding("temp", 20, false, 0),
            finding("cache", 40, false, 1),
            finding("cache", 9_000, true, 2),
            finding("temp", 20, false, 3),
            finding("report", 99_000, true, 4),
        ]));
        let report = state.report.as_ref().unwrap();
        assert_eq!(report.cleanable_bytes, 80);
        assert_eq!(report.cleanable_count, 3);
        let names: Vec<_> = report
            .categories
            .iter()
            .map(|category| category.name.as_str())
            .collect();
        assert_eq!(names, ["cache", "temp", "report"]);
        assert_eq!(report.categories[0].indices, [1, 2]);
        assert_eq!(report.categories[0].report_only, 1);
        // The original largest-category tie break selects the last alphabetical name.
        assert_eq!(state.expanded_cat.as_deref(), Some("temp"));
        assert_eq!(state.selected_bytes(), 0);
        assert_eq!(state.selected_findings().len(), 0);
    }

    #[test]
    fn selection_updates_totals_without_duplicates_or_report_only_items() {
        let findings = vec![
            finding("cache", 40, false, 0),
            finding("cache", 50, true, 1),
            finding("temp", 20, false, 2),
        ];
        let report = JunkReport::new(report(findings));
        let mut selection = Selection::from_findings(&report.scan.findings);
        let cache = &report.categories[0];
        selection.set_category(cache, &report.scan.findings, true);
        selection.set_category(cache, &report.scan.findings, true);
        assert_eq!((selection.count, selection.bytes()), (2, 60));
        assert!(!selection.contains(1));
        selection.set_category(cache, &report.scan.findings, false);
        selection.set_category(cache, &report.scan.findings, false);
        assert_eq!((selection.count, selection.bytes()), (1, 20));
        assert!(selection.contains(2));
        selection.set(0, &report.scan.findings[0], true);
        assert_eq!((selection.count, selection.bytes()), (2, 60));
        selection.clear();
        assert_eq!((selection.count, selection.bytes()), (0, 0));
        assert!(selection.included.iter().all(|included| !included));
    }

    #[test]
    fn selection_saturation_can_be_reversed_and_new_reports_replace_cache() {
        let mut state = JunkPanelState::default();
        state.set_report(report(vec![
            finding("cache", u64::MAX, false, 0),
            finding("temp", 7, false, 1),
        ]));
        for index in 0..2 {
            let finding = state.report().unwrap().findings[index].clone();
            state.selection.set(index, &finding, true);
        }
        assert_eq!(state.selected_bytes(), u64::MAX);
        state
            .selection
            .set(0, &state.report().unwrap().findings[0].clone(), false);
        assert_eq!(state.selected_bytes(), 7);
        state.confirm = true;
        state.set_report(report(vec![finding("report", 100, true, 2)]));
        assert_eq!(state.selected_bytes(), 0);
        assert!(state.selected_findings().is_empty());
        assert!(state.expanded_cat.is_none());
        assert!(!state.confirm);
        assert_eq!(state.report.as_ref().unwrap().categories.len(), 1);
        state.set_report(report(vec![]));
        assert!(state.report.as_ref().unwrap().categories.is_empty());
        state.open_and_scan();
        assert!(state.loading);
        assert_eq!(state.selection.count, 0);
    }

    #[test]
    fn visible_rows_include_partial_rows_and_clamp_to_content() {
        let rows = egui::Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(200.0, 100.0));
        let clip =
            |top, bottom| egui::Rect::from_min_max(egui::pos2(0.0, top), egui::pos2(200.0, bottom));
        assert_eq!(visible_rows(rows, clip(90.0, 119.0), 20.0, 5), 0..1);
        assert_eq!(visible_rows(rows, clip(121.0, 161.0), 20.0, 5), 1..4);
        assert_eq!(visible_rows(rows, clip(181.0, 230.0), 20.0, 5), 4..5);
        assert_eq!(visible_rows(rows, clip(0.0, 90.0), 20.0, 5), 0..0);
        assert_eq!(visible_rows(rows, clip(210.0, 300.0), 20.0, 5), 0..0);
        assert_eq!(visible_rows(rows, clip(90.0, 230.0), 20.0, 0), 0..0);
    }

    fn path_texts(output: &egui::FullOutput) -> Vec<&egui::epaint::TextShape> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().starts_with("/tmp/junk-panel/") => {
                    Some(text)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn large_panel_scrolls_to_last_finding_and_keeps_row_selection() {
        let mut state = JunkPanelState::default();
        state.set_report(report(
            (0..2_000)
                .map(|index| finding("cache", 4_096, false, index))
                .collect(),
        ));
        let ctx = egui::Context::default();
        let first = draw_frame(&ctx, &mut state);
        let texts = path_texts(&first);
        assert!(!texts.is_empty());
        // The list fills the Clean screen; still only the rows in its viewport are painted.
        assert!(texts.len() < 40, "only viewport rows should be painted");
        assert_eq!(texts[0].galley.text(), "/tmp/junk-panel/0/cache.bin");
        let first_row_y = texts[0].pos.y + texts[0].galley.size().y / 2.0;
        // The checkbox sits just left of the row's size label.
        let size_label = first
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text)
                    if text.galley.text().trim() == space_core::format_bytes(4_096)
                        && (text.pos.y + text.galley.size().y / 2.0 - first_row_y).abs() < 4.0 =>
                {
                    Some(text.pos.x)
                }
                _ => None,
            })
            .next()
            .expect("size label of the first row");
        let checkbox_x = size_label - 15.0;
        let checkbox = egui::pos2(checkbox_x, first_row_y);
        draw_input(
            &ctx,
            &mut state,
            vec![
                egui::Event::PointerMoved(checkbox),
                egui::Event::PointerButton {
                    pos: checkbox,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        draw_input(
            &ctx,
            &mut state,
            vec![egui::Event::PointerButton {
                pos: checkbox,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            state.selection.contains(0),
            "clicking the first visible checkbox explicitly selects its finding"
        );
        assert_eq!(state.selection.count, 1);

        // Simulate a fast wheel gesture over the category scroller.
        draw_input(
            &ctx,
            &mut state,
            vec![
                egui::Event::PointerMoved(egui::pos2(400.0, 300.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -100_000.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let mut last = draw_frame(&ctx, &mut state);
        for _ in 0..30 {
            if path_texts(&last)
                .iter()
                .any(|text| text.galley.text() == "/tmp/junk-panel/1999/cache.bin")
            {
                break;
            }
            last = draw_frame(&ctx, &mut state);
        }
        let texts = path_texts(&last);
        assert!(texts.len() < 40);
        assert!(
            texts
                .iter()
                .any(|text| text.galley.text() == "/tmp/junk-panel/1999/cache.bin"),
            "visible paths after scrolling: {:?}",
            texts
                .iter()
                .map(|text| text.galley.text())
                .collect::<Vec<_>>()
        );
        assert!(state.selection.contains(0));
        assert_eq!(state.selected_bytes(), 4_096);
        let final_row = texts
            .iter()
            .find(|text| text.galley.text() == "/tmp/junk-panel/1999/cache.bin")
            .unwrap();
        let checkbox = egui::pos2(
            checkbox_x,
            final_row.pos.y + final_row.galley.size().y / 2.0,
        );
        draw_input(
            &ctx,
            &mut state,
            vec![
                egui::Event::PointerMoved(checkbox),
                egui::Event::PointerButton {
                    pos: checkbox,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        draw_input(
            &ctx,
            &mut state,
            vec![egui::Event::PointerButton {
                pos: checkbox,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            state.selection.contains(1_999),
            "the final visible checkbox updates the correct finding after scrolling"
        );
        assert_eq!(state.selection.count, 2);
    }

    #[test]
    #[ignore = "manual release benchmark; run with --release --ignored --nocapture"]
    fn benchmark_large_junk_panel() {
        for count in [1_000, 20_000] {
            let mut state = JunkPanelState::default();
            state.set_report(report(
                (0..count)
                    .map(|i| finding("cache", 4_096, false, i))
                    .collect(),
            ));
            let ctx = egui::Context::default();
            black_box(draw_frame(&ctx, &mut state));
            let started = Instant::now();
            for _ in 0..100 {
                black_box(black_box(&state).selected_bytes());
            }
            let selected = started.elapsed();
            let started = Instant::now();
            for _ in 0..20 {
                black_box(draw_frame(&ctx, &mut state));
            }
            eprintln!(
                "junk-panel findings={count}: selected_bytes={:.2}ns/call frame={:.2}us/frame",
                selected.as_secs_f64() * 1e9 / 100.0,
                started.elapsed().as_secs_f64() * 1e6 / 20.0
            );
        }
    }
}
