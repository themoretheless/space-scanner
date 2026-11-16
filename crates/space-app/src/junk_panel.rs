//! Quick Clean junk groups: human-readable Safe vs Review cards with
//! automatic selection for Safe (`low` risk) findings only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense};
use space_clean::{Finding, JunkScanProgress, ScanReport};

use crate::i18n::t;
use crate::theme::type_scale;
use crate::theme::{self, ACCENT, DANGER, MUTED, PANEL_2, SAFE, TEXT};
use crate::widgets::{self, CheckVisual};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JunkGroup {
    Safe,
    Review,
}

#[derive(Default)]
pub struct JunkPanelState {
    pub open: bool,
    pub loading: bool,
    report: Option<JunkReport>,
    pub progress: JunkScanProgress,
    pub scan_started: Option<Instant>,
    selection: Selection,
    pub safe_open: bool,
    pub review_open: bool,
    pub expanded_cat: Option<(JunkGroup, String)>,
    pub error: Option<String>,
}

/// Only explicit low-risk non-report-only rules qualify for automatic selection.
pub fn is_safe_finding(finding: &Finding) -> bool {
    !finding.report_only
        && matches!(
            finding.risk.trim().to_ascii_lowercase().as_str(),
            "low" | "safe"
        )
}

fn risk_level(risk: &str) -> u8 {
    match risk.trim().to_ascii_lowercase().as_str() {
        "high" => 2,
        "medium" | "caution" => 1,
        _ => 0,
    }
}

/// Short risk badge for category headers.
fn badge(risk: u8) -> (&'static str, Color32, &'static str) {
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

    pub fn selected_bytes(&self) -> u64 {
        self.selection.bytes()
    }

    pub fn selection_version(&self) -> u64 {
        self.selection.version
    }

    /// Junk rule covering `path` (the path itself or a parent), from the last junk scan.
    pub fn risk_hint(&self, path: &Path) -> Option<(String, &'static str, Color32, &'static str)> {
        let f = self
            .report()?
            .findings
            .iter()
            .filter(|f| path.starts_with(&f.path))
            .max_by_key(|f| f.path.as_os_str().len())?;
        let (label, color, hint) = if f.report_only {
            (
                "Report only",
                MUTED(),
                "Listed for information — never removed",
            )
        } else {
            badge(risk_level(&f.risk))
        };
        Some((cat_title(&f.category), label, color, hint))
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn open_and_scan(&mut self) {
        self.open = true;
        self.loading = true;
        self.error = None;
        self.progress = JunkScanProgress::default();
        self.scan_started = Some(Instant::now());
        self.clear_selection();
        self.safe_open = false;
        self.review_open = false;
        self.expanded_cat = None;
    }

    /// Replace the active scan report, preselecting only `Safe` findings.
    pub fn set_report(&mut self, report: ScanReport) {
        self.loading = false;
        self.scan_started = None;
        self.safe_open = false;
        self.review_open = false;
        self.expanded_cat = None;
        let next_version = self.selection.version.wrapping_add(1);
        let mut selection = Selection::from_safe_findings(&report.findings);
        selection.version = next_version;
        self.selection = selection;
        self.report = Some(JunkReport::new(report));
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

    pub fn exclude_paths(&mut self, paths: &[PathBuf]) {
        let Some(report) = &self.report else { return };
        for (index, finding) in report.scan.findings.iter().enumerate() {
            if paths.iter().any(|path| finding.path.starts_with(path)) {
                self.selection.set(index, finding, false);
            }
        }
    }

    /// Prune trashed/missing paths in-place, preserving user selection choices on survivors.
    pub fn remove_paths(&mut self, removed: &[PathBuf]) {
        if removed.is_empty() {
            return;
        }
        let Some(old) = self.report.take() else {
            return;
        };
        let mut kept_findings = Vec::with_capacity(old.scan.findings.len());
        let mut kept_selected = Vec::with_capacity(old.scan.findings.len());
        let mut total_bytes = 0_u64;
        let mut changed = false;

        for (index, finding) in old.scan.findings.into_iter().enumerate() {
            if removed.iter().any(|path| finding.path.starts_with(path)) {
                changed = true;
                continue;
            }
            let was_selected = self.selection.contains(index) && !finding.report_only;
            total_bytes = total_bytes.saturating_add(finding.bytes);
            kept_findings.push(finding);
            kept_selected.push(was_selected);
        }

        let scan = ScanReport {
            findings: kept_findings,
            total_bytes,
            rules_matched: old.scan.rules_matched,
            rules_scanned: old.scan.rules_scanned,
            skipped_roots: old.scan.skipped_roots,
        };

        let mut next_selection = Selection {
            included: vec![false; scan.findings.len()],
            ..Selection::default()
        };
        for (index, (finding, on)) in scan.findings.iter().zip(kept_selected).enumerate() {
            if on {
                next_selection.set(index, finding, true);
            }
        }
        next_selection.version = if changed {
            self.selection.version.wrapping_add(1)
        } else {
            self.selection.version
        };
        let next_report = JunkReport::new(scan);

        if let Some((group, cat_name)) = &self.expanded_cat {
            let exists = next_report
                .categories_for(*group)
                .iter()
                .any(|c| c.name == *cat_name);
            if !exists {
                self.expanded_cat = None;
            }
        }

        self.selection = next_selection;
        self.report = Some(next_report);
    }

    pub fn has_cleanable_items(&self) -> bool {
        self.report
            .as_ref()
            .is_some_and(|r| r.safe_count > 0 || r.review_count > 0)
    }

    pub fn total_cleanable_bytes(&self) -> u64 {
        self.report
            .as_ref()
            .map(|r| r.safe_bytes.saturating_add(r.review_bytes))
            .unwrap_or(0)
    }

    /// Return `(total_count, total_bytes, selected_count, selected_bytes)` for `group`.
    pub fn group_stats(&self, group: JunkGroup) -> (usize, u64, usize, u64) {
        let Some(report) = &self.report else {
            return (0, 0, 0, 0);
        };
        let (total_count, total_bytes) = match group {
            JunkGroup::Safe => (report.safe_count, report.safe_bytes),
            JunkGroup::Review => (report.review_count, report.review_bytes),
        };
        let mut sel_count = 0usize;
        let mut sel_bytes = 0u64;
        for cat in report.categories_for(group) {
            for &idx in &cat.indices {
                if self.selection.contains(idx) {
                    sel_count += 1;
                    sel_bytes = sel_bytes.saturating_add(report.scan.findings[idx].bytes);
                }
            }
        }
        (total_count, total_bytes, sel_count, sel_bytes)
    }

    pub fn set_group_selected(&mut self, group: JunkGroup, on: bool) {
        let Some(report) = &self.report else { return };
        for cat in report.categories_for(group) {
            self.selection.set_category(cat, &report.scan.findings, on);
        }
    }
}

/// Keeps checkbox membership and its displayed totals in sync.
#[derive(Default)]
struct Selection {
    included: Vec<bool>,
    count: usize,
    // Retain the exact sum so deselection stays correct even if the display saturates.
    bytes: u128,
    version: u64,
}

impl Selection {
    fn from_safe_findings(findings: &[Finding]) -> Self {
        let mut selection = Self {
            included: vec![false; findings.len()],
            ..Self::default()
        };
        for (index, finding) in findings.iter().enumerate() {
            if is_safe_finding(finding) {
                selection.set(index, finding, true);
            }
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
        self.version = self.version.wrapping_add(1);
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
        if self.count > 0 {
            self.version = self.version.wrapping_add(1);
        }
        self.included.fill(false);
        self.count = 0;
        self.bytes = 0;
    }

    fn bytes(&self) -> u64 {
        self.bytes.min(u128::from(u64::MAX)) as u64
    }
}

struct CatAgg {
    name: String,
    bytes: u64,
    indices: Vec<usize>,
    /// Highest rule risk among removable items in this bucket: 0 low, 1 medium, 2 high.
    risk: u8,
}

struct JunkReport {
    scan: ScanReport,
    safe_categories: Vec<CatAgg>,
    review_categories: Vec<CatAgg>,
    safe_bytes: u64,
    safe_count: usize,
    review_bytes: u64,
    review_count: usize,
    #[allow(dead_code)]
    report_only_bytes: u64,
    #[allow(dead_code)]
    report_only_count: usize,
}

impl JunkReport {
    fn new(scan: ScanReport) -> Self {
        let mut grouped = BTreeMap::<(JunkGroup, String), CatAgg>::new();
        let mut safe_bytes = 0_u64;
        let mut safe_count = 0_usize;
        let mut review_bytes = 0_u64;
        let mut review_count = 0_usize;
        let mut report_only_bytes = 0_u64;
        let mut report_only_count = 0_usize;

        for (index, finding) in scan.findings.iter().enumerate() {
            if finding.report_only {
                report_only_bytes = report_only_bytes.saturating_add(finding.bytes);
                report_only_count += 1;
                continue;
            }
            let group = if is_safe_finding(finding) {
                safe_bytes = safe_bytes.saturating_add(finding.bytes);
                safe_count += 1;
                JunkGroup::Safe
            } else {
                review_bytes = review_bytes.saturating_add(finding.bytes);
                review_count += 1;
                JunkGroup::Review
            };
            let key = (group, finding.category.clone());
            let cat = grouped.entry(key).or_insert_with(|| CatAgg {
                name: finding.category.clone(),
                bytes: 0,
                indices: Vec::new(),
                risk: 0,
            });
            cat.bytes = cat.bytes.saturating_add(finding.bytes);
            cat.risk = cat.risk.max(risk_level(&finding.risk));
            cat.indices.push(index);
        }

        let mut safe_categories = Vec::new();
        let mut review_categories = Vec::new();
        for ((group, _), mut cat) in grouped {
            cat.indices
                .sort_by_key(|&idx| std::cmp::Reverse(scan.findings[idx].bytes));
            match group {
                JunkGroup::Safe => safe_categories.push(cat),
                JunkGroup::Review => review_categories.push(cat),
            }
        }
        let sort_cats = |cats: &mut Vec<CatAgg>| {
            cats.sort_by(|a, b| {
                std::cmp::Reverse(a.bytes)
                    .cmp(&std::cmp::Reverse(b.bytes))
                    .then_with(|| a.name.cmp(&b.name))
            });
        };
        sort_cats(&mut safe_categories);
        sort_cats(&mut review_categories);

        Self {
            scan,
            safe_categories,
            review_categories,
            safe_bytes,
            safe_count,
            review_bytes,
            review_count,
            report_only_bytes,
            report_only_count,
        }
    }

    fn categories_for(&self, group: JunkGroup) -> &[CatAgg] {
        match group {
            JunkGroup::Safe => &self.safe_categories,
            JunkGroup::Review => &self.review_categories,
        }
    }
}

/// Shared collapsible group card used by Quick Clean (Safe junk, Downloads, App data).
#[allow(clippy::too_many_arguments)]
pub(crate) fn group_card(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    badge_label: &str,
    badge_color: Color32,
    badge_hint: &str,
    total_count: usize,
    total_bytes: u64,
    selected_count: usize,
    open: &mut bool,
    on_toggle_all: impl FnOnce(bool),
    body: impl FnOnce(&mut egui::Ui),
) {
    let visual = if total_count > 0 && selected_count == total_count {
        CheckVisual::Checked
    } else if selected_count > 0 {
        CheckVisual::Partial
    } else {
        CheckVisual::Unchecked
    };
    let check_target = !(total_count > 0 && selected_count == total_count);

    egui::Frame::new()
        .fill(theme::PANEL())
        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER()))
        .corner_radius(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), Sense::click());
            let p = ui.painter();
            if selected_count > 0 {
                p.rect_filled(rect, 10.0, ACCENT().gamma_multiply(0.05));
            } else if resp.hovered() {
                p.rect_filled(rect, 10.0, PANEL_2().gamma_multiply(0.5));
            }
            if resp.has_focus() {
                crate::theme::paint_focus_ring(p, rect);
            }

            let cy = rect.center().y;
            let cb = egui::Rect::from_center_size(
                egui::pos2(rect.left() + 24.0, cy),
                egui::vec2(16.0, 16.0),
            );
            let cb_hover = resp.hovered()
                && ui
                    .input(|i| i.pointer.hover_pos())
                    .is_some_and(|h| cb.expand(6.0).contains(h));
            widgets::paint_checkbox(p, cb, visual, cb_hover);

            // Right side: chevron + total bytes + count caption
            let chev = egui::Rect::from_center_size(
                egui::pos2(rect.right() - 22.0, cy),
                egui::vec2(14.0, 14.0),
            );
            crate::icons::paint(
                p,
                if *open {
                    crate::icons::Icon::ChevronDown
                } else {
                    crate::icons::Icon::ChevronRight
                },
                chev,
                MUTED(),
            );

            let right_edge = rect.right() - 40.0;
            let size_g = p.layout_no_wrap(
                crate::i18n::bytes(total_bytes),
                egui::FontId::monospace(type_scale::LEAD),
                if selected_count > 0 { ACCENT() } else { TEXT() },
            );
            let count_text = if selected_count > 0 && selected_count < total_count {
                crate::tr!("{} of {} selected", selected_count, total_count)
            } else {
                widgets::count_noun(total_count, "item")
            };
            let count_g = p.layout_no_wrap(
                count_text,
                egui::FontId::proportional(type_scale::CAPTION),
                if selected_count > 0 {
                    ACCENT()
                } else {
                    MUTED()
                },
            );
            let right_w = size_g.size().x.max(count_g.size().x);
            p.galley(
                egui::pos2(right_edge - size_g.size().x, rect.top() + 12.0),
                size_g,
                TEXT(),
            );
            p.galley(
                egui::pos2(right_edge - count_g.size().x, rect.top() + 36.0),
                count_g,
                MUTED(),
            );

            // Left text column: title + badge pill, subtitle below.
            let text_left = rect.left() + 48.0;
            let text_max_w = (right_edge - right_w - 16.0 - text_left).max(60.0);

            let badge_g = p.layout_no_wrap(
                badge_label.to_owned(),
                egui::FontId::proportional(type_scale::CAPTION),
                badge_color,
            );
            let badge_w = badge_g.size().x + 14.0;
            let title_max_w = (text_max_w - badge_w - 8.0).max(40.0);
            let mut title_job = egui::text::LayoutJob::simple_singleline(
                title.to_owned(),
                egui::FontId::proportional(type_scale::LEAD),
                TEXT(),
            );
            title_job.wrap = egui::text::TextWrapping::truncate_at_width(title_max_w);
            let title_g = p.layout_job(title_job);
            let title_w = title_g.size().x;
            p.galley(egui::pos2(text_left, rect.top() + 12.0), title_g, TEXT());

            let pill_rect = egui::Rect::from_min_size(
                egui::pos2(text_left + title_w + 8.0, rect.top() + 12.0),
                egui::vec2(badge_w, 20.0),
            );
            p.rect_stroke(
                pill_rect,
                6.0,
                egui::Stroke::new(1.0_f32, badge_color.gamma_multiply(0.55)),
                egui::StrokeKind::Inside,
            );
            p.galley(
                egui::pos2(
                    pill_rect.left() + 7.0,
                    pill_rect.center().y - badge_g.size().y * 0.5,
                ),
                badge_g,
                badge_color,
            );

            let mut sub_job = egui::text::LayoutJob::simple_singleline(
                subtitle.to_owned(),
                egui::FontId::proportional(type_scale::CAPTION),
                MUTED(),
            );
            sub_job.wrap = egui::text::TextWrapping::truncate_at_width(text_max_w);
            let sub_g = p.layout_job(sub_job);
            p.galley(egui::pos2(text_left, rect.top() + 36.0), sub_g, MUTED());

            let a11y_title = title.to_owned();
            let resp = resp.on_hover_text(badge_hint);
            resp.widget_info(move || {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, a11y_title.as_str())
            });
            if resp.clicked() {
                let on_cb = ui
                    .input(|i| i.pointer.interact_pos())
                    .is_some_and(|h| cb.expand(8.0).contains(h));
                if on_cb && total_count > 0 {
                    on_toggle_all(check_target);
                } else {
                    *open = !*open;
                }
            }

            if *open {
                widgets::divider(ui);
                egui::Frame::new()
                    .fill(theme::BG())
                    .corner_radius(egui::CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: 10,
                        se: 10,
                    })
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        body(ui);
                    });
            }
        });
}

/// Draw either the `Safe` card or `Review` card from `JunkPanelState`.
/// Sets `reveal` on "Show in Finder" or `delete_now` on permanent deletion context actions.
pub(crate) fn draw_group_card(
    ui: &mut egui::Ui,
    state: &mut JunkPanelState,
    group: JunkGroup,
    reveal: &mut Option<PathBuf>,
    delete_now: &mut Option<Vec<Finding>>,
) {
    let (total_count, total_bytes, selected_count, _selected_bytes) = state.group_stats(group);
    if total_count == 0 {
        return;
    }
    let (title, subtitle, badge_label, badge_color, badge_hint) = match group {
        JunkGroup::Safe => (
            t("Safe junk"),
            t("App and browser caches, logs and temporary files — recreated automatically"),
            t("Safe"),
            SAFE(),
            t("Rebuilt or re-downloaded automatically"),
        ),
        JunkGroup::Review => (
            t("App & developer data"),
            t("Build caches, dev tools, virtual machines and app leftovers — review before trashing"),
            t("Check"),
            ACCENT(),
            t("Usually fine — check the list before trashing"),
        ),
    };

    let mut open = match group {
        JunkGroup::Safe => state.safe_open,
        JunkGroup::Review => state.review_open,
    };
    let mut toggle_all = None;

    group_card(
        ui,
        title,
        subtitle,
        badge_label,
        badge_color,
        badge_hint,
        total_count,
        total_bytes,
        selected_count,
        &mut open,
        |on| toggle_all = Some(on),
        |ui| {
            draw_group_categories(ui, state, group, reveal, delete_now);
        },
    );

    match group {
        JunkGroup::Safe => state.safe_open = open,
        JunkGroup::Review => state.review_open = open,
    }
    if let Some(on) = toggle_all {
        state.set_group_selected(group, on);
    }
}

fn draw_group_categories(
    ui: &mut egui::Ui,
    state: &mut JunkPanelState,
    group: JunkGroup,
    reveal: &mut Option<PathBuf>,
    delete_now: &mut Option<Vec<Finding>>,
) {
    let Some(report) = state.report.as_ref() else {
        return;
    };
    let findings = &report.scan.findings;
    let cats = report.categories_for(group);
    let mut next_expanded = state.expanded_cat.clone();
    let mut toggle_cat: Option<(&[usize], bool)> = None;

    ui.spacing_mut().item_spacing.y = 2.0;
    for cat in cats {
        let is_open = state
            .expanded_cat
            .as_ref()
            .is_some_and(|(g, name)| *g == group && name == &cat.name);
        let total = cat.indices.len();
        let picked = cat
            .indices
            .iter()
            .filter(|&&i| state.selection.contains(i))
            .count();
        let all = total > 0 && picked == total;
        let visual = if all {
            CheckVisual::Checked
        } else if picked > 0 {
            CheckVisual::Partial
        } else {
            CheckVisual::Unchecked
        };

        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
        let p = ui.painter();
        if picked > 0 {
            p.rect_filled(rect, 6.0, ACCENT().gamma_multiply(0.06));
        } else if resp.hovered() {
            p.rect_filled(rect, 6.0, PANEL_2().gamma_multiply(0.55));
        }
        if resp.has_focus() {
            crate::theme::paint_focus_ring(p, rect);
        }
        let cy = rect.center().y;
        let cb = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 14.0, cy),
            egui::vec2(15.0, 15.0),
        );
        let cb_hot = resp.hovered()
            && ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|h| cb.expand(5.0).contains(h));
        widgets::paint_checkbox(p, cb, visual, cb_hot);

        crate::icons::paint(
            p,
            if is_open {
                crate::icons::Icon::ChevronDown
            } else {
                crate::icons::Icon::ChevronRight
            },
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 34.0, cy),
                egui::vec2(12.0, 12.0),
            ),
            MUTED(),
        );

        let size_g = p.layout_no_wrap(
            crate::i18n::bytes(cat.bytes),
            egui::FontId::monospace(type_scale::MONO),
            TEXT(),
        );
        let size_x = rect.right() - 10.0 - size_g.size().x;
        p.galley(
            egui::pos2(size_x, cy - size_g.size().y * 0.5),
            size_g,
            TEXT(),
        );

        let mut right_limit = size_x - 10.0;
        if group == JunkGroup::Review {
            let (risk_label, risk_color, _) = badge(cat.risk);
            let risk_g = p.layout_no_wrap(
                t(risk_label).to_owned(),
                egui::FontId::proportional(type_scale::CAPTION),
                risk_color,
            );
            let rx = right_limit - risk_g.size().x - 4.0;
            p.galley(
                egui::pos2(rx, cy - risk_g.size().y * 0.5),
                risk_g,
                risk_color,
            );
            right_limit = rx - 10.0;
        }

        let name_x = rect.left() + 48.0;
        let max_w = (right_limit - name_x).max(40.0);
        let cat_title_str = cat_title(&cat.name);
        let cat_name = t(&cat_title_str);
        let src = sources(cat, findings);
        let line = if src.is_empty() {
            format!("{cat_name} · {total}")
        } else {
            format!("{cat_name} · {src} · {total}")
        };
        let mut job = egui::text::LayoutJob::simple_singleline(
            line,
            egui::FontId::proportional(type_scale::BODY),
            TEXT(),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        let g = p.layout_job(job);
        p.galley(egui::pos2(name_x, cy - g.size().y * 0.5), g, TEXT());

        let a11y_cat = cat_name.to_owned();
        let resp = resp.on_hover_text(t(if is_open { "Hide items" } else { "Show items" }));
        resp.widget_info(move || {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, a11y_cat.as_str())
        });
        let cat_deletable: Vec<Finding> = cat
            .indices
            .iter()
            .map(|&idx| &findings[idx])
            .filter(|f| !f.report_only)
            .cloned()
            .collect();
        if !cat_deletable.is_empty() {
            resp.context_menu(|ui| {
                if ui
                    .button(
                        RichText::new(t("Delete category permanently…"))
                            .color(crate::theme::DANGER()),
                    )
                    .clicked()
                {
                    *delete_now = Some(cat_deletable.clone());
                    ui.close_menu();
                }
            });
        }
        if resp.clicked() {
            let on_cb = ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|h| cb.expand(6.0).contains(h));
            if on_cb {
                toggle_cat = Some((&cat.indices, !all));
            } else if is_open {
                next_expanded = None;
            } else {
                next_expanded = Some((group, cat.name.clone()));
            }
        }

        if is_open {
            let (_, _, hint) = badge(cat.risk);
            ui.indent(("cat_rows", &cat.name), |ui| {
                if group == JunkGroup::Review {
                    ui.label(widgets::caption(hint));
                    ui.add_space(2.0);
                }
                draw_finding_rows(
                    ui,
                    group,
                    cat,
                    findings,
                    &mut state.selection,
                    reveal,
                    delete_now,
                );
                ui.add_space(4.0);
            });
        }
    }

    if let Some((indices, on)) = toggle_cat {
        for &idx in indices {
            state.selection.set(idx, &findings[idx], on);
        }
    }
    state.expanded_cat = next_expanded;
}

/// Which apps or tools the category's rules belong to ("xcode.derived-data" -> "xcode").
fn sources(cat: &CatAgg, findings: &[Finding]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for &i in &cat.indices {
        let id = findings[i].rule_id.as_str();
        let head = id.split(['.', '/', ':']).next().unwrap_or(id);
        if !names.contains(&head) {
            names.push(head);
        }
        if names.len() >= 3 {
            break;
        }
    }
    match names.len() {
        0 => String::new(),
        1 => names[0].to_owned(),
        2 => format!("{}, {}", names[0], names[1]),
        _ => format!("{}, {}…", names[0], names[1]),
    }
}

fn draw_finding_rows(
    ui: &mut egui::Ui,
    group: JunkGroup,
    category: &CatAgg,
    findings: &[Finding],
    selection: &mut Selection,
    reveal: &mut Option<PathBuf>,
    delete_now: &mut Option<Vec<Finding>>,
) {
    let spacing = ui.spacing().item_spacing.y;
    let row_height = ui
        .spacing()
        .interact_size
        .y
        .max(ui.spacing().icon_width)
        .max(24.0)
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
            .id_salt(("junk_finding_rows", group, &category.name))
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
            if ui.checkbox(&mut included, "").changed() {
                selection.set(index, finding, included);
            }
            ui.label(
                RichText::new(format!("{:>8}", crate::i18n::bytes(finding.bytes)))
                    .monospace()
                    .size(type_scale::MONO)
                    .color(TEXT()),
            );
            let path = finding.path.display().to_string();
            let path_resp = ui
                .add(
                    egui::Label::new(
                        RichText::new(&path)
                            .monospace()
                            .size(type_scale::MONO)
                            .color(MUTED()),
                    )
                    .truncate()
                    .sense(Sense::click()),
                )
                .on_hover_text(&path);
            path_resp.context_menu(|ui| {
                if !finding.report_only
                    && ui
                        .button(
                            RichText::new(t("Delete permanently…")).color(crate::theme::DANGER()),
                        )
                        .clicked()
                {
                    *delete_now = Some(vec![finding.clone()]);
                    ui.close_menu();
                }
                if ui.button(t("Show in Finder")).clicked() {
                    *reveal = Some(finding.path.clone());
                    ui.close_menu();
                }
            });
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
    use space_core::format_bytes;
    use std::hint::black_box;

    fn finding_with_risk(
        category: &str,
        risk: &str,
        bytes: u64,
        report_only: bool,
        index: usize,
    ) -> Finding {
        Finding {
            rule_id: "benchmark-rule".into(),
            category: category.into(),
            risk: risk.into(),
            path: format!("/tmp/junk-panel/{index}/cache.bin").into(),
            bytes,
            is_dir: false,
            report_only,
            root: None,
            min_age_days: 0,
        }
    }

    fn finding(category: &str, bytes: u64, report_only: bool, index: usize) -> Finding {
        finding_with_risk(category, "low", bytes, report_only, index)
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

    fn draw_test_groups(
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
                egui::ScrollArea::vertical()
                    .id_salt("junk_test_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut reveal = None;
                        let mut delete_now = None;
                        draw_group_card(ui, state, JunkGroup::Safe, &mut reveal, &mut delete_now);
                        draw_group_card(ui, state, JunkGroup::Review, &mut reveal, &mut delete_now);
                        black_box((reveal, delete_now));
                    });
            });
        })
    }

    #[test]
    fn safe_findings_are_preselected_while_medium_high_and_report_only_stay_unselected() {
        let mut state = JunkPanelState::default();
        state.set_report(report(vec![
            finding_with_risk("cache", "low", 40, false, 0),
            finding_with_risk("cache", "safe", 20, false, 1),
            finding_with_risk("cache", "medium", 80, false, 2),
            finding_with_risk("dev", "high", 100, false, 3),
            finding_with_risk("cache", "low", 9_000, true, 4),
        ]));

        // Only items 0 and 1 (low/safe, !report_only) are auto-selected.
        assert_eq!(state.selected_count(), 2);
        assert_eq!(state.selected_bytes(), 60);
        assert_eq!(state.group_stats(JunkGroup::Safe), (2, 60, 2, 60));
        assert_eq!(state.group_stats(JunkGroup::Review), (2, 180, 0, 0));

        // Same category string splits cleanly by group and excludes report_only.
        let r = state.report.as_ref().unwrap();
        assert_eq!(r.safe_categories.len(), 1);
        assert_eq!(r.safe_categories[0].indices, vec![0, 1]);
        assert_eq!(r.review_categories.len(), 2);
        assert_eq!(r.report_only_count, 1);
    }

    #[test]
    fn remove_paths_prunes_deleted_findings_in_place_and_preserves_manual_choices() {
        let mut state = JunkPanelState::default();
        state.set_report(report(vec![
            finding_with_risk("cache", "low", 40, false, 0),
            finding_with_risk("cache", "low", 20, false, 1),
            finding_with_risk("dev", "medium", 50, false, 2),
        ]));
        // Manually uncheck item 0, keep item 1 selected, and check item 2.
        state.exclude_paths(&["/tmp/junk-panel/0".into()]);
        state.set_group_selected(JunkGroup::Review, true);
        assert_eq!(state.selected_count(), 2);
        assert_eq!(state.selected_bytes(), 70);

        // Removing item 1 drops it without re-selecting manually unchecked item 0.
        state.remove_paths(&["/tmp/junk-panel/1".into()]);
        assert_eq!(state.report().unwrap().findings.len(), 2);
        assert_eq!(state.selected_count(), 1);
        assert_eq!(state.selected_bytes(), 50);
        assert_eq!(
            state.selected_findings()[0].path,
            std::path::Path::new("/tmp/junk-panel/2/cache.bin")
        );
    }

    #[test]
    fn selection_saturation_can_be_reversed_and_new_reports_replace_cache() {
        let mut state = JunkPanelState::default();
        state.set_report(report(vec![
            finding("cache", u64::MAX, false, 0),
            finding("temp", 7, false, 1),
        ]));
        assert_eq!(state.selected_bytes(), u64::MAX);
        state.exclude_paths(&["/tmp/junk-panel/0".into()]);
        assert_eq!(state.selected_bytes(), 7);
        state.set_report(report(vec![finding("report", 100, true, 2)]));
        assert_eq!(state.selected_bytes(), 0);
        assert!(state.selected_findings().is_empty());
        assert!(state.expanded_cat.is_none());
        assert_eq!(state.report.as_ref().unwrap().report_only_count, 1);
        state.set_report(report(vec![]));
        assert!(!state.has_cleanable_items());
        state.open_and_scan();
        assert!(state.loading);
        assert_eq!(state.selected_count(), 0);
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
        state.clear_selection();
        state.safe_open = true;
        state.expanded_cat = Some((JunkGroup::Safe, "cache".into()));

        let ctx = egui::Context::default();
        let first = draw_test_groups(&ctx, &mut state, Vec::new());
        let texts = path_texts(&first);
        assert!(!texts.is_empty());
        assert!(texts.len() < 40, "only viewport rows should be painted");
        assert_eq!(texts[0].galley.text(), "/tmp/junk-panel/0/cache.bin");
        let first_row_y = texts[0].pos.y + texts[0].galley.size().y / 2.0;
        let size_label = first
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text)
                    if text.galley.text().trim() == format_bytes(4_096)
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
        draw_test_groups(
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
        draw_test_groups(
            &ctx,
            &mut state,
            vec![egui::Event::PointerButton {
                pos: checkbox,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(state.selection.contains(0));
        assert_eq!(state.selection.count, 1);

        draw_test_groups(
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
        let mut last = draw_test_groups(&ctx, &mut state, Vec::new());
        for _ in 0..30 {
            if path_texts(&last)
                .iter()
                .any(|text| text.galley.text() == "/tmp/junk-panel/1999/cache.bin")
            {
                break;
            }
            last = draw_test_groups(&ctx, &mut state, Vec::new());
        }
        let texts = path_texts(&last);
        assert!(texts.len() < 40);
        assert!(texts
            .iter()
            .any(|text| text.galley.text() == "/tmp/junk-panel/1999/cache.bin"));
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
        draw_test_groups(
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
        draw_test_groups(
            &ctx,
            &mut state,
            vec![egui::Event::PointerButton {
                pos: checkbox,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(state.selection.contains(1_999));
        assert_eq!(state.selection.count, 2);
    }

    #[test]
    fn selection_version_tracks_report_and_selection_updates() {
        let mut state = JunkPanelState::default();
        let v0 = state.selection_version();
        state.set_report(ScanReport {
            findings: vec![Finding {
                rule_id: "r1".into(),
                category: "Caches".into(),
                risk: "low".into(),
                path: PathBuf::from("/tmp/c1"),
                bytes: 500,
                is_dir: true,
                report_only: false,
                root: None,
                min_age_days: 0,
            }],
            total_bytes: 500,
            rules_matched: 1,
            rules_scanned: 1,
            skipped_roots: Vec::new(),
        });
        let v1 = state.selection_version();
        assert_ne!(v1, v0);

        state.clear_selection();
        let v2 = state.selection_version();
        assert_ne!(v2, v1);

        state.clear_selection();
        assert_eq!(state.selection_version(), v2);

        state.remove_paths(&[PathBuf::from("/tmp/c1")]);
        assert_ne!(state.selection_version(), v2);
    }
}
