//! Junk cleaner panel — category-first, Trash-first (ppduster rules).

use std::collections::BTreeMap;
use std::time::Instant;

use eframe::egui::{self, Align, Layout, RichText, Sense};
use space_clean::{CleanProgress, Finding, JunkScanProgress, ScanReport};
use space_core::format_bytes;

use crate::settings::FeatureFlags;
use crate::theme::{self, ACCENT, DANGER, MUTED, PANEL_2, SAFE, TEXT};

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
            MUTED,
            "Listed for information — never removed",
        );
    }
    match risk {
        0 => ("Safe", SAFE, "Rebuilt or re-downloaded automatically"),
        1 => (
            "Review",
            ACCENT,
            "Usually fine — check the list before trashing",
        ),
        _ => (
            "Careful",
            DANGER,
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
        // Pre-tick only low-risk rules: selection feeds the shared Trash queue.
        for (index, finding) in report.findings.iter().enumerate() {
            if risk_level(&finding.risk) > 0 {
                self.selection.set(index, finding, false);
            }
        }
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

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Caches, logs, and temp. Always to Trash — never deleted immediately.")
                .size(12.0)
                .color(MUTED),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add_enabled(
                    !state.loading && !state.cleaning,
                    egui::Button::new("Rescan"),
                )
                .clicked()
            {
                action = JunkAction::Rescan;
            }
            if ui
                .button("Rules…")
                .on_hover_text("Turn individual rules on or off")
                .clicked()
            {
                action = JunkAction::OpenRules;
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Also scan").small().color(MUTED));
        if ui
            .checkbox(&mut flags.scan_browser_cookies, "Cookies")
            .on_hover_text("Off by default — cleaning cookies signs you out of websites")
            .changed()
        {
            action = JunkAction::PrefsChanged;
        }
        if ui
            .checkbox(&mut flags.scan_browser_history, "History")
            .on_hover_text("Off by default")
            .changed()
        {
            action = JunkAction::PrefsChanged;
        }
    });
    ui.add_space(10.0);

    if let Some(err) = &state.error {
        ui.label(RichText::new(err).color(theme::DANGER));
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
                RichText::new(format!(
                    "Moving to Trash · {}/{} items · {} files · {} folders · {} · {} errors · {}s",
                    p.items_done,
                    p.items_total,
                    p.files_done,
                    p.folders_done,
                    format_bytes(p.bytes_processed),
                    p.error_count,
                    elapsed
                ))
                .color(MUTED),
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
                        .color(MUTED),
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
                RichText::new(format!(
                    "Scanning · {}/{} rules · {} removable files · {} folders · {} ({}) · {}s",
                    p.rules_done,
                    p.rules_total,
                    p.files_found,
                    p.folders_found,
                    format_bytes(p.bytes_found),
                    p.findings_found,
                    elapsed
                ))
                .color(MUTED),
            );
        });
        ui.add_space(6.0);
        ui.add(
            egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                .desired_width(ui.available_width())
                .show_percentage(),
        );
        if !p.current_rule.is_empty() {
            ui.label(
                RichText::new(format!("Looking at {}", p.current_rule))
                    .small()
                    .color(TEXT),
            );
        }
        if !p.current_path.as_os_str().is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(p.current_path.display().to_string())
                        .small()
                        .monospace()
                        .color(MUTED),
                )
                .truncate(),
            )
            .on_hover_text(p.current_path.display().to_string());
        }
        return action;
    }

    let Some(report) = state.report.as_ref() else {
        ui.label(
            RichText::new("Still scanning, or press Rescan if this is empty by mistake.")
                .color(MUTED),
        );
        ui.add_space(12.0);
        if ui.button("Close").clicked() {
            action = JunkAction::Close;
        }
        return action;
    };

    // Totals strip
    let cleanable_bytes = report.cleanable_bytes;
    let cleanable_n = report.cleanable_count;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format_bytes(cleanable_bytes))
                .size(22.0)
                .strong()
                .color(ACCENT),
        );
        ui.label(
            RichText::new(format!("reclaimable · {cleanable_n} items"))
                .size(13.0)
                .color(MUTED),
        );
    });
    ui.add_space(8.0);

    // Risk filter chips, with counts per group.
    ui.horizontal(|ui| {
        for (f, label) in [
            (RiskFilter::All, "All"),
            (RiskFilter::Safe, "Safe"),
            (RiskFilter::Review, "Review"),
            (RiskFilter::ReportOnly, "Report only"),
        ] {
            let n = report
                .categories
                .iter()
                .filter(|c| f == RiskFilter::All || RiskFilter::of(c) == f)
                .count();
            let on = state.risk_filter == f;
            let chip = egui::Button::new(RichText::new(format!("{label} · {n}")).color(if on {
                theme::BG
            } else {
                MUTED
            }))
            .fill(if on { TEXT } else { egui::Color32::TRANSPARENT })
            .stroke(if on {
                egui::Stroke::NONE
            } else {
                egui::Stroke::new(1.0, theme::BORDER)
            })
            .corner_radius(14);
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
    let max_bytes = cat_list.first().map(|c| c.bytes).unwrap_or(1).max(1);

    if cat_list.is_empty() {
        ui.label(
            RichText::new("Nothing to clean right now.")
                .size(14.0)
                .color(TEXT),
        );
        ui.label(
            RichText::new("Try Rules to turn more categories on, or Rescan.")
                .small()
                .color(MUTED),
        );
        ui.add_space(8.0);
    }

    egui::ScrollArea::vertical()
        .max_height(300.0)
        .id_salt("junk_cats")
        .show(ui, |ui| {
            for cat in cat_list {
                let open = state.expanded_cat.as_deref() == Some(cat.name.as_str());
                let frac = cat.bytes as f32 / max_bytes as f32;

                let row = egui::Frame::new()
                    .fill(PANEL_2)
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let chev = if open { "▾" } else { "▸" };
                            ui.label(RichText::new(chev).color(MUTED).monospace());
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(cat_title(&cat.name))
                                            .strong()
                                            .color(TEXT)
                                            .size(14.0),
                                    );
                                    let (label, color, hint) = badge(cat.cleanable > 0, cat.risk);
                                    egui::Frame::new()
                                        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.5)))
                                        .corner_radius(5)
                                        .inner_margin(egui::Margin::symmetric(6, 1))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new(label).size(11.0).color(color));
                                        })
                                        .response
                                        .on_hover_text(hint);
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(
                                            RichText::new(format_bytes(cat.bytes))
                                                .strong()
                                                .color(ACCENT)
                                                .monospace(),
                                        );
                                    });
                                });
                                // Size bar — animate fill when category appears.
                                let bar_w = ui.available_width().max(40.0);
                                let bar = egui::Rect::from_min_size(
                                    ui.cursor().min,
                                    egui::vec2(bar_w, 4.0),
                                );
                                ui.allocate_rect(bar, Sense::hover());
                                ui.painter().rect_filled(bar, 2.0, theme::BORDER);
                                let grow = ui.ctx().animate_bool_with_time(
                                    ui.id().with(("junk_bar", cat.name.as_str())),
                                    true,
                                    0.45,
                                );
                                let fill_w = bar_w * frac.clamp(0.04, 1.0) * grow;
                                let fill =
                                    egui::Rect::from_min_size(bar.min, egui::vec2(fill_w, 4.0));
                                ui.painter().rect_filled(fill, 2.0, ACCENT);
                                ui.add_space(2.0);
                                ui.label(
                                    RichText::new(format!(
                                        "{} removable{}",
                                        cat.cleanable,
                                        if cat.report_only > 0 {
                                            format!(
                                                " · {} listed only (not removable)",
                                                cat.report_only
                                            )
                                        } else {
                                            String::new()
                                        }
                                    ))
                                    .small()
                                    .color(MUTED),
                                );
                            });
                        });
                    });

                let cat_click = row.response.interact(Sense::click());
                cat_click.clone().on_hover_text(if open {
                    "Hide items"
                } else {
                    "Click to list items"
                });
                if cat_click.clicked() {
                    if open {
                        state.expanded_cat = None;
                    } else {
                        state.expanded_cat = Some(cat.name.clone());
                    }
                }

                if open {
                    ui.add_space(4.0);
                    ui.indent("junk_items", |ui| {
                        draw_finding_rows(ui, cat, &report.scan.findings, &mut state.selection);
                        ui.horizontal(|ui| {
                            if ui.small_button("Select all").clicked() {
                                state
                                    .selection
                                    .set_category(cat, &report.scan.findings, true);
                            }
                            if ui.small_button("Select none").clicked() {
                                state
                                    .selection
                                    .set_category(cat, &report.scan.findings, false);
                            }
                        });
                    });
                    ui.add_space(6.0);
                } else {
                    ui.add_space(6.0);
                }
            }
        });

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        if ui.button("Close").clicked() {
            action = JunkAction::Close;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let n = state.selection.count;
            ui.label(
                RichText::new(format!(
                    "{n} selected · {} — added to the Trash queue",
                    format_bytes(state.selected_bytes())
                ))
                .color(if n > 0 { ACCENT } else { MUTED }),
            );
        });
    });

    action
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
        .max(ui.fonts(|fonts| fonts.row_height(&egui::FontId::monospace(11.0))));
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
            let color = if enabled { TEXT } else { MUTED };
            ui.label(
                RichText::new(format!("{:>8}", format_bytes(finding.bytes)))
                    .monospace()
                    .size(11.0)
                    .color(color),
            );
            let path = finding.path.display().to_string();
            ui.add(
                egui::Label::new(RichText::new(&path).monospace().size(11.0).color(color))
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
        assert_eq!(state.selected_bytes(), 80);
        assert_eq!(state.selected_findings().len(), 3);
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
        assert!(texts.len() < 20, "only viewport rows should be painted");
        assert_eq!(texts[0].galley.text(), "/tmp/junk-panel/0/cache.bin");
        let first_row_y = texts[0].pos.y + texts[0].galley.size().y / 2.0;
        // The checkbox starts at the indented content edge (8 px panel + 18 px indent).
        let checkbox = egui::pos2(35.0, first_row_y);
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
            !state.selection.contains(0),
            "clicking the first visible checkbox deselects its finding"
        );
        assert_eq!(state.selection.count, 1_999);

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
        assert!(texts.len() < 20);
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
        assert!(!state.selection.contains(0));
        assert_eq!(state.selected_bytes(), 1_999 * 4_096);
        let final_row = texts
            .iter()
            .find(|text| text.galley.text() == "/tmp/junk-panel/1999/cache.bin")
            .unwrap();
        let checkbox = egui::pos2(35.0, final_row.pos.y + final_row.galley.size().y / 2.0);
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
            !state.selection.contains(1_999),
            "the final visible checkbox updates the correct finding after scrolling"
        );
        assert_eq!(state.selection.count, 1_998);
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
