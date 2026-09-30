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
    pub report: Option<ScanReport>,
    pub progress: JunkScanProgress,
    pub scan_started: Option<Instant>,
    pub clean_progress: CleanProgress,
    pub clean_started: Option<Instant>,
    pub selected: Vec<usize>,
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
    fn of(cleanable: bool, risk: u8) -> Self {
        match (cleanable, risk) {
            (false, _) => Self::ReportOnly,
            (true, 0) => Self::Safe,
            _ => Self::Review,
        }
    }
}

impl JunkPanelState {
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
        self.selected.clear();
        self.expanded_cat = None;
    }

    pub fn set_report(&mut self, report: ScanReport) {
        self.loading = false;
        self.cleaning = false;
        self.scan_started = None;
        self.clean_started = None;
        self.selected = report
            .findings
            .iter()
            .enumerate()
            // Pre-tick only low-risk rules: selection feeds the shared Trash queue.
            .filter(|(_, f)| !f.report_only && risk_level(&f.risk) == 0)
            .map(|(i, _)| i)
            .collect();
        // Expand the largest cleanable category by default.
        self.expanded_cat = largest_category(&report);
        self.report = Some(report);
    }

    pub fn selected_findings(&self) -> Vec<Finding> {
        let Some(report) = &self.report else {
            return Vec::new();
        };
        self.selected
            .iter()
            .filter_map(|&i| report.findings.get(i).cloned())
            .filter(|f| !f.report_only)
            .collect()
    }

    pub fn selected_bytes(&self) -> u64 {
        self.selected_findings().iter().map(|f| f.bytes).sum()
    }
}

fn largest_category(report: &ScanReport) -> Option<String> {
    let mut map: BTreeMap<&str, u64> = BTreeMap::new();
    for f in &report.findings {
        if f.report_only {
            continue;
        }
        *map.entry(f.category.as_str()).or_default() += f.bytes;
    }
    map.into_iter()
        .max_by_key(|(_, b)| *b)
        .map(|(c, _)| c.to_string())
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

fn risk_badge(cat: &CatAgg) -> (&'static str, egui::Color32, &'static str) {
    badge(cat.cleanable > 0, cat.risk)
}

impl JunkPanelState {
    /// Junk rule covering `path` (the path itself or a parent), from the last junk scan.
    pub fn risk_hint(
        &self,
        path: &std::path::Path,
    ) -> Option<(String, &'static str, egui::Color32, &'static str)> {
        let report = self.report.as_ref()?;
        let f = report
            .findings
            .iter()
            .filter(|f| path.starts_with(&f.path))
            .max_by_key(|f| f.path.as_os_str().len())?;
        let (label, color, hint) = badge(!f.report_only, risk_level(&f.risk));
        Some((cat_title(&f.category), label, color, hint))
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
    let cleanable_bytes: u64 = report
        .findings
        .iter()
        .filter(|f| !f.report_only)
        .map(|f| f.bytes)
        .sum();
    let cleanable_n = report.findings.iter().filter(|f| !f.report_only).count();
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

    // Aggregate categories
    let mut cats: BTreeMap<String, CatAgg> = BTreeMap::new();
    for (i, f) in report.findings.iter().enumerate() {
        let e = cats.entry(f.category.clone()).or_insert_with(|| CatAgg {
            name: f.category.clone(),
            bytes: 0,
            cleanable: 0,
            report_only: 0,
            indices: Vec::new(),
            risk: 0,
        });
        e.indices.push(i);
        if f.report_only {
            e.report_only += 1;
        } else {
            e.bytes = e.bytes.saturating_add(f.bytes);
            e.cleanable += 1;
            e.risk = e.risk.max(risk_level(&f.risk));
        }
    }
    let mut cat_list: Vec<CatAgg> = cats.into_values().collect();
    cat_list.sort_by_key(|category| std::cmp::Reverse(category.bytes));

    // Risk filter chips, with counts per group.
    let group = |c: &CatAgg| RiskFilter::of(c.cleanable > 0, c.risk);
    ui.horizontal(|ui| {
        for (f, label) in [
            (RiskFilter::All, "All"),
            (RiskFilter::Safe, "Safe"),
            (RiskFilter::Review, "Review"),
            (RiskFilter::ReportOnly, "Report only"),
        ] {
            let n = cat_list
                .iter()
                .filter(|c| f == RiskFilter::All || group(c) == f)
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
    if state.risk_filter != RiskFilter::All {
        cat_list.retain(|c| group(c) == state.risk_filter);
    }
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
        .max_height((ui.available_height() - 56.0).max(200.0))
        .id_salt("junk_cats")
        .show(ui, |ui| {
            for cat in &cat_list {
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
                                    let (badge, color, hint) = risk_badge(cat);
                                    egui::Frame::new()
                                        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.5)))
                                        .corner_radius(5)
                                        .inner_margin(egui::Margin::symmetric(6, 1))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new(badge).size(11.0).color(color));
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
                        for &i in &cat.indices {
                            let Some(f) = report.findings.get(i) else {
                                continue;
                            };
                            let mut on = state.selected.contains(&i);
                            let enabled = !f.report_only;
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(enabled, egui::Checkbox::new(&mut on, ""))
                                    .changed()
                                {
                                    if on {
                                        if !state.selected.contains(&i) {
                                            state.selected.push(i);
                                        }
                                    } else {
                                        state.selected.retain(|&x| x != i);
                                    }
                                }
                                ui.label(
                                    RichText::new(format!("{:>8}", format_bytes(f.bytes)))
                                        .monospace()
                                        .size(11.0)
                                        .color(if enabled { TEXT } else { MUTED }),
                                );
                                let path = f.path.display().to_string();
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&path)
                                            .monospace()
                                            .size(11.0)
                                            .color(if enabled { TEXT } else { MUTED }),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&path);
                            });
                        }
                        ui.horizontal(|ui| {
                            if ui.small_button("Select all").clicked() {
                                for &i in &cat.indices {
                                    if report.findings.get(i).is_some_and(|f| !f.report_only)
                                        && !state.selected.contains(&i)
                                    {
                                        state.selected.push(i);
                                    }
                                }
                            }
                            if ui.small_button("Select none").clicked() {
                                state.selected.retain(|i| !cat.indices.contains(i));
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
            let n = state.selected.len();
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
