//! Junk cleaner panel — category-first, Trash-first (ppduster rules).

use std::collections::BTreeMap;

use eframe::egui::{self, Align, Layout, RichText, Sense};
use space_clean::{Finding, ScanReport};
use space_core::format_bytes;

use crate::theme::{self, ACCENT, MUTED, PANEL_2, TEXT};

pub struct JunkPanelState {
    pub open: bool,
    pub loading: bool,
    pub report: Option<ScanReport>,
    pub selected: Vec<usize>,
    pub confirm: bool,
    pub expanded_cat: Option<String>,
    pub error: Option<String>,
}

impl Default for JunkPanelState {
    fn default() -> Self {
        Self {
            open: false,
            loading: false,
            report: None,
            selected: Vec::new(),
            confirm: false,
            expanded_cat: None,
            error: None,
        }
    }
}

impl JunkPanelState {
    pub fn open_and_scan(&mut self) {
        self.open = true;
        self.loading = true;
        self.error = None;
        self.confirm = false;
        self.selected.clear();
        self.expanded_cat = None;
    }

    pub fn set_report(&mut self, report: ScanReport) {
        self.loading = false;
        self.selected = report
            .findings
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.report_only)
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
    Clean,
    Close,
}

struct CatAgg {
    name: String,
    bytes: u64,
    cleanable: usize,
    report_only: usize,
    indices: Vec<usize>,
}

pub fn draw(ui: &mut egui::Ui, state: &mut JunkPanelState) -> JunkAction {
    let mut action = JunkAction::None;

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Caches, logs, temp — regenerable only. Goes to Trash.")
                .size(12.0)
                .color(MUTED),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add_enabled(!state.loading, egui::Button::new("Rescan"))
                .clicked()
            {
                action = JunkAction::Rescan;
            }
        });
    });
    ui.add_space(10.0);

    if let Some(err) = &state.error {
        ui.label(RichText::new(err).color(theme::DANGER));
        ui.add_space(6.0);
    }

    if state.loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("Scanning…").color(MUTED));
        });
        return action;
    }

    let Some(report) = state.report.as_ref() else {
        ui.label(RichText::new("Nothing scanned yet.").color(MUTED));
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
        });
        e.bytes = e.bytes.saturating_add(f.bytes);
        e.indices.push(i);
        if f.report_only {
            e.report_only += 1;
        } else {
            e.cleanable += 1;
        }
    }
    let mut cat_list: Vec<CatAgg> = cats.into_values().collect();
    cat_list.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    let max_bytes = cat_list.first().map(|c| c.bytes).unwrap_or(1).max(1);

    egui::ScrollArea::vertical()
        .max_height(300.0)
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
                                let fill = egui::Rect::from_min_size(
                                    bar.min,
                                    egui::vec2(fill_w, 4.0),
                                );
                                ui.painter().rect_filled(fill, 2.0, ACCENT);
                                ui.add_space(2.0);
                                ui.label(
                                    RichText::new(format!(
                                        "{} cleanable{}",
                                        cat.cleanable,
                                        if cat.report_only > 0 {
                                            format!(" · {} report-only", cat.report_only)
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

                if row.response.interact(Sense::click()).clicked() {
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
            let bytes = state.selected_bytes();
            let can = n > 0 && !state.loading;
            if ui
                .add_enabled(
                    can,
                    egui::Button::new(
                        RichText::new(format!(
                            "Move to Trash  ·  {}  ·  {}",
                            n,
                            format_bytes(bytes)
                        ))
                        .color(ACCENT),
                    ),
                )
                .clicked()
            {
                state.confirm = true;
            }
        });
    });

    if state.confirm {
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(PANEL_2)
            .corner_radius(8.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!(
                        "Move {} items ({}) to Trash?",
                        state.selected.len(),
                        format_bytes(state.selected_bytes())
                    ))
                    .strong(),
                );
                ui.label(
                    RichText::new("You can restore from Trash if needed.")
                        .small()
                        .color(MUTED),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        state.confirm = false;
                    }
                    if ui
                        .button(RichText::new("Confirm").color(ACCENT).strong())
                        .clicked()
                    {
                        state.confirm = false;
                        action = JunkAction::Clean;
                    }
                });
            });
    }

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
        other => {
            let mut s = other.replace('-', " ");
            if let Some(c) = s.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            s
        }
    }
}
