//! Global chrome: the sidebar with the four destinations, and the Disk usage
//! toolbar, filter bar and status footer.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense};

use super::{Density, SIDEBAR_LABELS_FROM};
use crate::app::{Mode, SpaceApp};
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::settings::WorkspaceLayout;
use crate::theme::metrics::{APP_BAR_H, CHIP_H, FILTER_BAR_H, RAIL_W, S3, S4, SIDEBAR_W};
use crate::theme::type_scale;
use crate::theme::{ACCENT, ACCENT_INK, BG, BORDER, DANGER, MUTED, PANEL, PANEL_2, TEXT};
use crate::views::EntryKindFilter;
use crate::volumes::VolumeInfo;
use crate::widgets::{caption, chip, ghost, icon_segmented, segmented, strong, text_width};

/// One sidebar destination.
struct NavItem {
    mode: Mode,
    icon: Icon,
    label: &'static str,
    sub: Option<String>,
    badge: usize,
    enabled: bool,
    /// Says "Empty" on the right when there is no count.
    empty_note: bool,
}

impl SpaceApp {
    pub(super) fn sidebar(&mut self, ctx: &egui::Context) {
        let wide = ctx.screen_rect().width() >= SIDEBAR_LABELS_FROM;
        let items = self.nav_items();
        let mut go = None;
        let mut settings = false;
        let mut cancel_scan = false;
        egui::SidePanel::left("nav")
            .show_separator_line(false)
            .exact_width(if wide { SIDEBAR_W } else { RAIL_W })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL())
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .inner_margin(if wide {
                        egui::Margin {
                            left: 12,
                            right: 12,
                            top: 16,
                            bottom: 12,
                        }
                    } else {
                        egui::Margin::symmetric(8, 14)
                    }),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                brand(ui, wide);
                ui.add_space(12.0);
                for item in &items {
                    if nav_item(ui, item, self.mode == item.mode, wide).clicked() && item.enabled {
                        go = Some(item.mode);
                    }
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let gear = NavItem {
                        mode: Mode::Home,
                        icon: Icon::Settings,
                        label: "Settings",
                        sub: None,
                        badge: 0,
                        enabled: true,
                        empty_note: false,
                    };
                    if nav_item(ui, &gear, self.show_settings, wide)
                        .on_hover_text("⌘,")
                        .clicked()
                    {
                        settings = true;
                    }
                    if wide {
                        if let Some(vol) = self.startup_volume() {
                            disk_card(ui, vol);
                        }
                    }
                    if self.scan.is_some() && self.tree.is_some() {
                        cancel_scan = scan_card(ui, self.progress.as_ref(), wide);
                    }
                });
            });
        if let Some(mode) = go {
            self.set_mode(mode);
        }
        if settings {
            if self.show_settings {
                self.show_settings = false;
            } else {
                self.open_settings();
            }
        }
        if cancel_scan {
            if let Some(s) = &self.scan {
                s.cancel();
            }
        }
    }

    fn nav_items(&self) -> Vec<NavItem> {
        let usage_sub = if self.scan.is_some() && self.tree.is_none() {
            t("Scanning…").to_owned()
        } else if let Some(tree) = &self.tree {
            format!(
                "{} · {}",
                short_root(&tree.root_path),
                crate::i18n::bytes(tree.total_size_mode(self.size_mode))
            )
        } else {
            t("No scan yet").to_owned()
        };
        let clean_sub = if self.junk.loading {
            t("Checking…").to_owned()
        } else if let Some(bytes) = self.junk.cleanable_bytes() {
            crate::tr!("{} found", crate::i18n::bytes(bytes))
        } else {
            t("Not checked yet").to_owned()
        };
        let n = self.basket_len();
        vec![
            NavItem {
                mode: Mode::Home,
                icon: Icon::Overview,
                label: "Overview",
                sub: None,
                badge: 0,
                enabled: true,
                empty_note: false,
            },
            NavItem {
                mode: Mode::Explore,
                icon: Icon::Usage,
                label: "Disk usage",
                sub: Some(usage_sub),
                badge: 0,
                enabled: self.tree.is_some()
                    || self.scan.is_some()
                    || self.busy
                    || self.scan_outcome.is_some(),
                empty_note: false,
            },
            NavItem {
                mode: Mode::Clean,
                icon: Icon::Cleanup,
                label: "Cleanup",
                sub: Some(clean_sub),
                badge: 0,
                enabled: true,
                empty_note: false,
            },
            NavItem {
                mode: Mode::Review,
                icon: Icon::Trash,
                label: "Trash list",
                sub: (n > 0).then(|| crate::i18n::bytes(self.basket_bytes())),
                badge: n,
                enabled: true,
                empty_note: true,
            },
        ]
    }

    /// The disk that holds the home folder, once its size is known.
    pub(super) fn startup_volume(&self) -> Option<&VolumeInfo> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        self.volumes
            .iter()
            .filter(|v| v.total > 0 && home.starts_with(&v.mount))
            .max_by_key(|v| v.mount.as_os_str().len())
    }

    // ------------------------------------------------------------- workspace

    /// Toolbar: history, location, search and layout.
    pub(super) fn workspace_bar(&mut self, ctx: &egui::Context, d: Density) {
        egui::TopBottomPanel::top("workspace_bar")
            .show_separator_line(false)
            .exact_height(APP_BAR_H)
            .frame(
                egui::Frame::new()
                    .fill(BG())
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .inner_margin(egui::Margin::symmetric(S3 as i8, 0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let narrow = d == Density::Narrow;
                    let split = self.settings.layout == WorkspaceLayout::Split;
                    let search_w = if narrow { 160.0 } else { 230.0 };
                    let right_w = search_w
                        + match (narrow, split) {
                            (true, true) => 120.0,
                            (true, false) => 0.0,
                            (false, _) => 290.0,
                        };
                    let budget = (ui.available_width() - right_w - S4).max(0.0);
                    self.location(ui, budget);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if narrow {
                            if split {
                                if let Some(i) =
                                    segmented(ui, &["Map", "List"], (!self.compact_map) as usize)
                                {
                                    self.compact_map = i == 0;
                                }
                            }
                        } else {
                            // Drawn right to left: Table, Columns, List and map.
                            let order = [
                                WorkspaceLayout::Table,
                                WorkspaceLayout::Columns,
                                WorkspaceLayout::Split,
                            ];
                            let current = order
                                .iter()
                                .position(|&l| l == self.settings.layout)
                                .unwrap_or(2);
                            if let Some(i) = icon_segmented(
                                ui,
                                &[
                                    (Icon::LayoutTable, "Table"),
                                    (Icon::LayoutColumns, "Columns"),
                                    (Icon::LayoutSplit, "List and map"),
                                ],
                                current,
                            ) {
                                self.set_layout(order[i]);
                            }
                            ui.add_space(4.0);
                            if let Some(i) = segmented(
                                ui,
                                &["Everywhere", "Folder"],
                                if self.search_everywhere { 0 } else { 1 },
                            ) {
                                self.search_everywhere = i == 0;
                                self.search_hits = None;
                            }
                        }
                        self.search_box(ui, search_w);
                    });
                });
            });
    }

    pub(super) fn set_layout(&mut self, layout: WorkspaceLayout) {
        if self.settings.layout != layout {
            self.settings.layout = layout;
            self.geom = None;
            self.mark_settings_dirty();
        }
    }

    /// ← → ↑ and breadcrumbs, shortened from the left so they never wrap.
    fn location(&mut self, ui: &mut egui::Ui, budget: f32) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let nav = |ui: &mut egui::Ui, enabled: bool, icon: Icon, hint: &str| {
            icons::button(ui, icon, 32.0, hint, enabled).clicked()
        };
        if nav(ui, !self.history_back.is_empty(), Icon::Back, "Back · ⌘←") {
            self.go_back();
        }
        if nav(
            ui,
            !self.history_fwd.is_empty(),
            Icon::Forward,
            "Forward · ⌘→",
        ) {
            self.go_forward();
        }
        let can_up = tree.node(self.focus).parent.is_some();
        if nav(ui, can_up, Icon::Up, "Parent folder · ↑") {
            self.go_up();
        }
        ui.add_space(6.0);

        let size_text = crate::i18n::bytes(tree.node(self.focus).size(self.size_mode));
        let size_font = egui::FontId::monospace(type_scale::MONO);
        let size_w = text_width(ui, &size_text, size_font.clone()) + 12.0;
        let avail = (budget - 3.0 * 36.0 - size_w - 6.0).max(0.0);
        let crumbs = tree.breadcrumb_names(self.focus);
        let chain = tree.path_to(self.focus);
        let font = egui::FontId::proportional(type_scale::BODY);
        let sep_w = 16.0;
        // Keep as many trailing segments as fit; the current folder always stays.
        let mut used = 0.0;
        let mut start = crumbs.len();
        for i in (0..crumbs.len()).rev() {
            let w = text_width(ui, &crumbs[i], font.clone()) + 12.0 + sep_w;
            if start < crumbs.len() && used + w > avail - 28.0 {
                break;
            }
            used += w;
            start = i;
        }
        let slash = BORDER().lerp_to_gamma(MUTED(), 0.5);
        let mut go = None;
        if start > 0 {
            if ui
                .add(egui::Button::new(RichText::new("…").color(MUTED())).frame(false))
                .on_hover_text(t("Show the path above"))
                .clicked()
            {
                go = Some(chain[start - 1]);
            }
            ui.label(RichText::new("/").color(slash));
        }
        let mut remaining = (avail - if start > 0 { 28.0 } else { 0.0 }).max(0.0);
        for i in start..crumbs.len() {
            if i > start {
                ui.label(RichText::new("/").color(slash));
            }
            let here = i + 1 == crumbs.len();
            let text = if here {
                RichText::new(&crumbs[i])
                    .size(type_scale::BODY)
                    .family(crate::theme::bold())
                    .color(TEXT())
            } else {
                RichText::new(&crumbs[i])
                    .size(type_scale::BODY)
                    .color(MUTED())
            };
            ui.scope(|ui| {
                ui.set_max_width(remaining.max(24.0));
                let resp = ui
                    .add(egui::Button::new(text).frame(false).truncate())
                    .on_hover_text(tree.path_string(chain[i]));
                if resp.clicked() && !here {
                    go = Some(chain[i]);
                }
            });
            remaining =
                (remaining - text_width(ui, &crumbs[i], font.clone()) - 12.0 - sep_w).max(0.0);
        }
        ui.label(RichText::new(size_text).font(size_font).color(MUTED()));
        if let Some(id) = go {
            self.navigate(id, true);
        }
    }

    fn search_box(&mut self, ui: &mut egui::Ui, width: f32) {
        let hint = if self.search_everywhere {
            t("Search the whole scan").to_owned()
        } else if let Some(tree) = &self.tree {
            crate::tr!("Search in {}", tree.node(self.focus).name)
        } else {
            t("Search").to_owned()
        };
        egui::Frame::new()
            .fill(PANEL())
            .stroke(egui::Stroke::new(1.0_f32, BORDER()))
            .corner_radius(crate::theme::metrics::RADIUS_CONTROL)
            .inner_margin(egui::Margin::symmetric(8, 0))
            .show(ui, |ui| {
                ui.set_width(width - 16.0);
                ui.set_height(CHIP_H + 2.0);
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    icons::show(ui, Icon::Search, 14.0, MUTED());
                    let clear_w = if self.filter_draft.is_empty() {
                        0.0
                    } else {
                        26.0
                    };
                    let edit = ui.add_sized(
                        [ui.available_width() - clear_w, CHIP_H - 2.0],
                        egui::TextEdit::singleline(&mut self.filter_draft)
                            .frame(false)
                            .margin(egui::vec2(0.0, 5.0))
                            .hint_text(RichText::new(hint).color(MUTED())),
                    );
                    if self.focus_filter {
                        edit.request_focus();
                        self.focus_filter = false;
                    }
                    self.search_focused = edit.has_focus();
                    if edit.changed() {
                        self.filter_edited_at = Some(std::time::Instant::now());
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(
                                self.settings.flags.debounce_filter_ms + 10,
                            ));
                    }
                    if !self.filter_draft.is_empty()
                        && icons::button(ui, Icon::Close, 22.0, "Clear search", true).clicked()
                    {
                        self.filter_draft.clear();
                        self.filter.query.clear();
                        self.filter_edited_at = None;
                        self.search_hits = None;
                        self.geom = None;
                    }
                });
            });
    }

    /// Kind and size chips, the extension picked on the map, the result count,
    /// and the batch action for whole-scan matches. Shared by every layout.
    pub(super) fn filter_bar(&mut self, ctx: &egui::Context, d: Density) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        egui::TopBottomPanel::top("filter_bar")
            .show_separator_line(false)
            .exact_height(FILTER_BAR_H)
            .frame(
                egui::Frame::new()
                    .fill(BG())
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .inner_margin(egui::Margin::symmetric(S3 as i8, 0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let kind = self.filter.kind;
                    let mut changed = false;
                    for (k, label) in [
                        (EntryKindFilter::All, "All"),
                        (EntryKindFilter::DirsOnly, "Folders"),
                        (EntryKindFilter::FilesOnly, "Files"),
                    ] {
                        if chip(ui, kind == k, label).clicked() {
                            self.filter.kind = k;
                            changed = true;
                        }
                    }
                    separator(ui);
                    for (bytes, label) in [(100_u64 << 20, "≥ 100 MB"), (1_u64 << 30, "≥ 1 GB")] {
                        let on = self.filter.min_size == bytes;
                        if chip(ui, on, label).clicked() {
                            self.filter.min_size = if on { 0 } else { bytes };
                            changed = true;
                        }
                    }
                    // Upper bound: presets, since typing bytes is rarely what anyone wants.
                    let max = self.filter.max_size;
                    let max_label = match max {
                        Some(bytes) => crate::tr!("≤ {}", crate::i18n::bytes(bytes)),
                        None => t("Max size").to_string(),
                    };
                    ui.menu_button(
                        RichText::new(format!("{max_label} ▾"))
                            .size(crate::theme::type_scale::CAPTION)
                            .color(if max.is_some() { TEXT() } else { MUTED() }),
                        |ui| {
                            for (label, value) in [
                                ("Any size", None),
                                ("≤ 1 MB", Some(1u64 << 20)),
                                ("≤ 10 MB", Some(10u64 << 20)),
                                ("≤ 100 MB", Some(100u64 << 20)),
                                ("≤ 1 GB", Some(1u64 << 30)),
                            ] {
                                if ui.selectable_label(max == value, t(label)).clicked() {
                                    self.filter.max_size = value;
                                    changed = true;
                                    ui.close_menu();
                                }
                            }
                        },
                    );
                    // One chip per file type picked on the map legend; clicking takes it out.
                    let exts: Vec<String> = self.filter.ext.iter().cloned().collect();
                    for ext in exts {
                        if chip(ui, true, &format!("*.{ext}  ×"))
                            .on_hover_text(t("Picked on the map legend. Click to clear."))
                            .clicked()
                        {
                            self.filter.toggle_ext(&ext);
                            changed = true;
                        }
                    }
                    if d == Density::Narrow {
                        separator(ui);
                        if let Some(i) = segmented(
                            ui,
                            &["Folder", "Everywhere"],
                            self.search_everywhere as usize,
                        ) {
                            self.search_everywhere = i == 1;
                            self.search_hits = None;
                        }
                    }
                    if changed {
                        self.geom = None;
                        self.search_hits = None;
                        self.settings.filter_kind = self.filter.kind;
                        self.mark_settings_dirty();
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let filtering = self.filter.is_active() || !self.filter_draft.is_empty();
                        if filtering && ui.add(ghost("Clear filters")).clicked() {
                            self.filter.clear();
                            self.filter_draft.clear();
                            self.filter_edited_at = None;
                            self.search_hits = None;
                            self.geom = None;
                        }
                        let mut add_all = None;
                        let summary = if let Some(hits) = &self.search_hits {
                            if !hits.ids.is_empty() {
                                let label =
                                    crate::tr!("Add all {} to Trash list", hits.ids.len());
                                let b = egui::Button::new(
                                    RichText::new(label)
                                        .color(ACCENT())
                                        .family(crate::theme::medium()),
                                )
                                .frame(false);
                                if ui
                                    .add(b)
                                    .on_hover_text(t(
                                        "Every match listed here. You still confirm on the Trash list.",
                                    ))
                                    .clicked()
                                {
                                    add_all = Some(hits.ids.clone());
                                }
                            }
                            if hits.truncated {
                                crate::tr!(
                                    "First {} matches in the whole scan · {}",
                                    hits.ids.len(),
                                    crate::i18n::bytes(hits.bytes)
                                )
                            } else {
                                crate::tr!(
                                    "{} in the whole scan · {}",
                                    crate::i18n::count(hits.ids.len(), "item"),
                                    crate::i18n::bytes(hits.bytes)
                                )
                            }
                        } else {
                            let kids = tree.children(self.focus);
                            let shown = kids
                                .iter()
                                .filter(|&&c| {
                                    let n = tree.node(c);
                                    self.filter.matches(&n.name, n.is_dir, n.size(self.size_mode))
                                })
                                .count();
                            if filtering {
                                crate::tr!("{} of {} shown", shown, kids.len())
                            } else {
                                crate::tr!(
                                    "{} · largest first",
                                    crate::i18n::count(kids.len(), "item")
                                )
                            }
                        };
                        ui.add(egui::Label::new(caption(summary)).truncate());
                        if let Some(ids) = add_all {
                            let missing: Vec<_> = ids
                                .into_iter()
                                .filter(|id| !self.review.contains(id))
                                .collect();
                            self.queue_toggle(&missing);
                        }
                    });
                });
            });
    }

    /// Bottom line of the workspace: what is indexed, how fresh it is, what is missing.
    pub(super) fn workspace_footer(&mut self, ctx: &egui::Context) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let mut rescan = false;
        let mut gap_clicked = false;
        egui::TopBottomPanel::bottom("workspace_footer")
            .show_separator_line(false)
            .exact_height(32.0)
            .frame(
                egui::Frame::new()
                    .fill(BG())
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .inner_margin(egui::Margin::symmetric(S4 as i8, 0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if self.scan.is_some() || self.busy {
                        ui.spinner();
                        ui.add(egui::Label::new(caption(&self.status)).truncate());
                        return;
                    }
                    ui.label(caption(crate::tr!(
                        "{} items · {}",
                        tree.item_count,
                        crate::i18n::bytes(tree.total_size_mode(self.size_mode))
                    )));
                    if self.from_cache {
                        let age = crate::i18n::age(
                            crate::app::now_unix().saturating_sub(tree.scanned_at_unix),
                        );
                        let (text, color) = if self.cache_stale {
                            (crate::tr!("Saved {} · files changed", age), ACCENT())
                        } else {
                            (crate::tr!("Saved {}", age), MUTED())
                        };
                        crate::widgets::badge(
                            ui,
                            &text,
                            color,
                            "Opened from a saved result. Rescan for current numbers.",
                        );
                    }
                    if ui
                        .add(egui::Button::new(caption("Rescan  ⌘R").color(TEXT())).frame(false))
                        .on_hover_text(t("Scan this folder again and keep the Trash list"))
                        .clicked()
                    {
                        rescan = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(gap) = self.scan_gap_label() {
                            if ui
                                .add(egui::Button::new(caption(gap).underline()).frame(false))
                                .on_hover_text(t(
                                    "Unindexed disk usage can include permissions, snapshots and filesystem accounting",
                                ))
                                .clicked()
                            {
                                gap_clicked = true;
                            }
                        }
                    });
                });
            });
        if rescan {
            self.rescan_current();
        }
        if gap_clicked {
            self.open_tm_panel();
        }
    }
}

/// The last folder name, with the home folder shown as ~.
pub(super) fn short_root(root: &str) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if home.as_deref() == Some(Path::new(root)) {
        return "~".into();
    }
    Path::new(root)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_owned())
}

fn separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 18.0), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        egui::Stroke::new(1.0_f32, BORDER()),
    );
}

/// A destination: icon, label, optional second line and count. Icons only in the rail.
fn nav_item(ui: &mut egui::Ui, item: &NavItem, on: bool, wide: bool) -> egui::Response {
    let h = if wide && item.sub.is_some() {
        48.0
    } else {
        44.0
    };
    let w = ui.available_width();
    let sense = if item.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), sense);
    let hovered = resp.hovered() && item.enabled;
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 8.0, PANEL_2());
    } else if hovered {
        p.rect_filled(rect, 8.0, PANEL_2().gamma_multiply(0.55));
    }
    let fg = if item.enabled {
        TEXT()
    } else {
        MUTED().gamma_multiply(0.6)
    };
    let icon_color = if on {
        ACCENT()
    } else if item.enabled {
        MUTED()
    } else {
        MUTED().gamma_multiply(0.6)
    };
    let label = t(item.label);
    if wide {
        icons::paint(
            p,
            item.icon,
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 19.0, rect.center().y),
                egui::vec2(18.0, 18.0),
            ),
            icon_color,
        );
        let x = rect.left() + 40.0;
        let right = rect.right() - 8.0;
        let badge_w = if item.badge > 0 || item.empty_note {
            44.0
        } else {
            0.0
        };
        let max_w = (right - x - badge_w).max(20.0);
        let mut job = egui::text::LayoutJob::simple_singleline(
            label.to_owned(),
            egui::FontId::new(type_scale::BODY, crate::theme::medium()),
            fg,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        let g = p.layout_job(job);
        if let Some(sub) = &item.sub {
            let mut sj = egui::text::LayoutJob::simple_singleline(
                sub.clone(),
                egui::FontId::proportional(type_scale::CAPTION),
                MUTED(),
            );
            sj.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
            let sg = p.layout_job(sj);
            let total = g.size().y + sg.size().y;
            let top = rect.center().y - total / 2.0;
            let gh = g.size().y;
            p.galley(egui::pos2(x, top), g, fg);
            p.galley(egui::pos2(x, top + gh), sg, MUTED());
        } else {
            p.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
        }
        if item.badge > 0 {
            paint_count(p, egui::pos2(right, rect.center().y), item.badge, false);
        } else if item.empty_note {
            let e = p.layout_no_wrap(
                t("Empty").to_owned(),
                egui::FontId::proportional(type_scale::CAPTION),
                MUTED(),
            );
            p.galley(
                egui::pos2(right - e.size().x, rect.center().y - e.size().y / 2.0),
                e,
                MUTED(),
            );
        }
        resp
    } else {
        icons::paint(
            p,
            item.icon,
            egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)),
            icon_color,
        );
        if item.badge > 0 {
            paint_count(
                p,
                egui::pos2(rect.right() + 2.0, rect.top() + 11.0),
                item.badge,
                true,
            );
        }
        let hover = match &item.sub {
            Some(sub) => format!("{label} · {sub}"),
            None => label.to_owned(),
        };
        resp.on_hover_text(hover)
    }
}

/// Accent pill with a number, right edge at `anchor.x`, centred on `anchor.y`.
fn paint_count(p: &egui::Painter, anchor: egui::Pos2, n: usize, small: bool) {
    let text = if n > 999 {
        "999+".to_owned()
    } else {
        n.to_string()
    };
    let g = p.layout_no_wrap(
        text,
        egui::FontId::new(
            if small { 11.0 } else { type_scale::CAPTION },
            crate::theme::bold(),
        ),
        ACCENT_INK(),
    );
    let h = if small { 18.0 } else { 22.0 };
    let w = (g.size().x + 12.0).max(h);
    let rect = egui::Rect::from_min_size(
        egui::pos2(anchor.x - w, anchor.y - h / 2.0),
        egui::vec2(w, h),
    );
    p.rect_filled(rect, h / 2.0, ACCENT());
    p.galley(rect.center() - g.size() * 0.5, g, ACCENT_INK());
}

/// Free space of the startup disk, always in view.
fn disk_card(ui: &mut egui::Ui, vol: &VolumeInfo) {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), Sense::hover());
    let p = ui.painter();
    p.rect(
        rect,
        8.0,
        BG(),
        egui::Stroke::new(1.0_f32, BORDER()),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink2(egui::vec2(10.0, 10.0));
    let free = p.layout_no_wrap(
        crate::tr!("{} free", crate::i18n::bytes(vol.available)),
        egui::FontId::proportional(type_scale::CAPTION),
        TEXT(),
    );
    let mut job = egui::text::LayoutJob::simple_singleline(
        vol.name.clone(),
        egui::FontId::proportional(type_scale::CAPTION),
        MUTED(),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(
        (inner.width() - free.size().x - 8.0).max(20.0),
    );
    let name = p.layout_job(job);
    p.galley(inner.left_top(), name, MUTED());
    p.galley(
        egui::pos2(inner.right() - free.size().x, inner.top()),
        free,
        TEXT(),
    );
    let used = vol.used() as f32 / vol.total.max(1) as f32;
    let track = egui::Rect::from_min_size(
        egui::pos2(inner.left(), inner.bottom() - 4.0),
        egui::vec2(inner.width(), 4.0),
    );
    p.rect_filled(track, 2.0, PANEL_2());
    let mut fill = track;
    fill.set_width(track.width() * used.clamp(0.0, 1.0));
    p.rect_filled(fill, 2.0, fill_for(used));
    resp.on_hover_text(crate::tr!(
        "{} free of {}",
        crate::i18n::bytes(vol.available),
        crate::i18n::bytes(vol.total)
    ));
}

/// Meter colour: calm until the disk is nearly full.
pub(super) fn fill_for(used: f32) -> Color32 {
    if used >= 0.9 {
        DANGER()
    } else if used >= 0.75 {
        ACCENT()
    } else {
        MUTED()
    }
}

/// Progress of a rescan running behind an open result. Returns true on Cancel.
fn scan_card(ui: &mut egui::Ui, progress: Option<&space_core::ScanProgress>, wide: bool) -> bool {
    if !wide {
        ui.spinner().on_hover_text(t("Rescanning"));
        return false;
    }
    let mut cancel = false;
    egui::Frame::new()
        .fill(BG())
        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spinner();
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(t("Rescanning"))
                            .size(type_scale::CAPTION)
                            .color(TEXT()),
                    );
                    if let Some(p) = progress {
                        ui.label(caption(crate::tr!(
                            "{} files · {}",
                            p.files_seen,
                            crate::i18n::bytes(p.bytes_seen)
                        )));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(ghost("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        });
    cancel
}

/// Two-tone ring + name.
fn brand(ui: &mut egui::Ui, with_name: bool) {
    ui.horizontal(|ui| {
        if with_name {
            ui.add_space(8.0);
        } else {
            ui.add_space(((ui.available_width() - 24.0) / 2.0).max(0.0));
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 40.0), Sense::hover());
        let c = rect.center();
        let p = ui.painter();
        p.circle_stroke(c, 8.0, egui::Stroke::new(4.0_f32, ACCENT()));
        let blue = if crate::theme::is_dark() {
            Color32::from_rgb(0x6E, 0x9B, 0xE8)
        } else {
            Color32::from_rgb(0x3D, 0x6F, 0xD1)
        };
        let arc: Vec<egui::Pos2> = (-90..=0)
            .step_by(6)
            .map(|deg| {
                let a = (deg as f32).to_radians();
                c + egui::vec2(a.cos(), a.sin()) * 8.0
            })
            .collect();
        p.add(egui::Shape::line(arc, egui::Stroke::new(4.0_f32, blue)));
        if with_name {
            ui.add_space(2.0);
            ui.label(strong("Space", type_scale::LEAD));
        }
    });
}
