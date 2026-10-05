//! Skin-specific Scan (start) pages. Map shows disks as occupancy rings,
//! Columns a disk sidebar with a "Continue" card, Table one dense table of
//! disks and saved results. All of them keep the open scan one click away.

use crate::i18n::t;
use std::path::PathBuf;

use eframe::egui::{self, Align, Layout, RichText, Sense};

use crate::app::{Mode, SpaceApp};
use crate::theme::metrics::{S2, S3, S4, S6, S8};
use crate::theme::type_scale;
use crate::theme::{ACCENT, BG, BORDER, MUTED, PANEL, PANEL_2, TEXT};
use crate::widgets::{caption, format_age, ghost, primary, secondary};

/// What the user picked on a start page.
#[derive(Default)]
struct Pick {
    scan: Option<PathBuf>,
    refresh: bool,
    resume: bool,
    folder: bool,
    cleanup: bool,
}

impl SpaceApp {
    fn apply_pick(&mut self, pick: Pick) {
        if pick.refresh {
            self.refresh_volumes_async();
        }
        if pick.resume {
            self.set_mode(Mode::Explore);
        }
        if pick.cleanup {
            self.open_junk_panel();
        }
        if pick.folder {
            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                self.start_scan(p, false);
            }
        }
        if let Some(p) = pick.scan {
            self.start_scan(p, false);
        }
    }

    /// Reload the disk list; a spinner while it runs.
    fn refresh_disks(&self, ui: &mut egui::Ui, pick: &mut Pick) {
        if self.volumes_loading {
            ui.spinner();
        } else if ui.add(ghost("Refresh disks")).clicked() {
            pick.refresh = true;
        }
    }

    /// The open scan, if any: name, size, item count.
    fn open_session(&self) -> Option<(String, u64, u64)> {
        self.tree.as_ref().map(|tr| {
            (
                tr.root_path.clone(),
                tr.total_size_mode(self.size_mode),
                tr.item_count,
            )
        })
    }

    // ----------------------------------------------------------------- map

    pub(super) fn start_map(&mut self, ctx: &egui::Context) {
        let enabled = !self.busy && self.scan.is_none();
        let mut pick = Pick::default();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(S8 * 2.0);
                    ui.label(RichText::new(t("What should we look at?")).size(type_scale::DISPLAY).strong());
                    ui.label(caption("The ring is how full the disk is. A disk you scanned before opens instantly."));
                    ui.add_space(S8);
                });
                let ring = 150.0;
                let card_w = 200.0;
                let mut cards: Vec<(String, String, f32, Option<PathBuf>, bool)> = Vec::new();
                if let Some((root, size, _)) = self.open_session() {
                    cards.push((short(&root), crate::tr!("Continue · {}", crate::i18n::bytes(size)), 1.0, None, true));
                }
                if let Some(home) = std::env::var_os("HOME") {
                    cards.push((t("Home folder").to_string(), "~/".into(), -1.0, Some(PathBuf::from(home)), false));
                }
                for v in &self.volumes {
                    let frac = if v.total > 0 { v.used() as f32 / v.total as f32 } else { -1.0 };
                    cards.push((
                        v.name.clone(),
                        crate::tr!("{} free", crate::i18n::bytes(v.available)),
                        frac,
                        Some(v.mount.clone()),
                        false,
                    ));
                }
                let n = cards.len().min(5) as f32;
                let total_w = n * card_w + (n - 1.0).max(0.0) * S4;
                ui.horizontal_top(|ui| {
                    ui.add_space(((ui.available_width() - total_w) * 0.5).max(0.0));
                    ui.spacing_mut().item_spacing.x = S4;
                    for (name, note, frac, path, resume) in cards.iter().take(5) {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(card_w, 260.0), Sense::click());
                        let p = ui.painter();
                        let hot = resp.hovered() && enabled;
                        p.rect(
                            rect,
                            16.0,
                            if hot { PANEL_2() } else { PANEL() },
                            egui::Stroke::new(1.0_f32, if *resume { MUTED() } else { BORDER() }),
                            egui::StrokeKind::Inside,
                        );
                        let c = egui::pos2(rect.center().x, rect.top() + 24.0 + ring / 2.0);
                        // Occupancy ring: track, then the used arc.
                        p.circle_stroke(c, ring / 2.0 - 8.0, egui::Stroke::new(14.0_f32, PANEL_2()));
                        if *frac > 0.0 {
                            let col = if *resume { TEXT() } else if *frac > 0.85 { ACCENT() } else { crate::theme::color_for(name, true) };
                            let steps = (64.0 * frac.min(1.0)).max(2.0) as usize;
                            let pts: Vec<egui::Pos2> = (0..=steps)
                                .map(|i| {
                                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac.min(1.0) * i as f32 / steps as f32;
                                    c + egui::vec2(a.cos(), a.sin()) * (ring / 2.0 - 8.0)
                                })
                                .collect();
                            p.add(egui::Shape::line(pts, egui::Stroke::new(14.0_f32, col)));
                        }
                        let mid = if *frac >= 0.0 { format!("{:.0}%", frac * 100.0) } else { "~/".into() };
                        p.text(c, egui::Align2::CENTER_CENTER, mid, egui::FontId::monospace(22.0), TEXT());
                        p.text(
                            egui::pos2(rect.center().x, rect.bottom() - 52.0),
                            egui::Align2::CENTER_CENTER,
                            name,
                            egui::FontId::proportional(type_scale::LEAD),
                            TEXT(),
                        );
                        p.text(
                            egui::pos2(rect.center().x, rect.bottom() - 28.0),
                            egui::Align2::CENTER_CENTER,
                            note,
                            egui::FontId::proportional(type_scale::CAPTION),
                            if *resume { ACCENT() } else { MUTED() },
                        );
                        if resp.clicked() && (enabled || *resume) {
                            if *resume {
                                pick.resume = true;
                            } else {
                                pick.scan = path.clone();
                            }
                        }
                    }
                });
                ui.add_space(S8);
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(((ui.available_width() - 420.0) * 0.5).max(0.0));
                        if ui.add_enabled(enabled, secondary("Folder…  ⌘O")).clicked() {
                            pick.folder = true;
                        }
                        if ui.add_enabled(enabled, ghost("Review cleanup candidates")).clicked() {
                            pick.cleanup = true;
                        }
                        self.refresh_disks(ui, &mut pick);
                    });
                    ui.add_space(S4);
                    let recent: Vec<String> = self.recent.iter().take(3).map(|m| short(&m.root_path)).collect();
                    if !recent.is_empty() {
                        ui.label(caption(crate::tr!("Recent: {}", recent.join(" · "))));
                    }
                });
            });
        self.apply_pick(pick);
    }

    // ------------------------------------------------------------- columns

    pub(super) fn start_columns(&mut self, ctx: &egui::Context) {
        let enabled = !self.busy && self.scan.is_none();
        let mut pick = Pick::default();
        egui::SidePanel::left("start_sidebar")
            .exact_width(230.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL_2())
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .inner_margin(egui::Margin::symmetric(10, 16)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(caption("DISKS"));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| self.refresh_disks(ui, &mut pick));
                });
                for v in &self.volumes {
                    if side_item(ui, &v.name, enabled) {
                        pick.scan = Some(v.mount.clone());
                    }
                }
                ui.add_space(S4);
                ui.label(caption("RECENT FOLDERS"));
                for m in self.recent.iter().take(6) {
                    if side_item(ui, &short(&m.root_path), enabled) {
                        pick.scan = Some(PathBuf::from(&m.root_path));
                    }
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    if ui
                        .add_enabled(enabled, secondary("Other folder…  ⌘O").min_size(egui::vec2(ui.available_width(), 32.0)))
                        .clicked()
                    {
                        pick.folder = true;
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()).inner_margin(egui::Margin::symmetric(48, 40)))
            .show(ctx, |ui| {
                ui.label(RichText::new(t("Where to look for space?")).size(type_scale::DISPLAY).strong());
                ui.label(caption("The last result opens instantly. Rescan when you want fresh numbers."));
                ui.add_space(S6);
                if let Some((root, size, items)) = self.open_session() {
                    egui::Frame::new()
                        .fill(PANEL())
                        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                        .corner_radius(12)
                        .inner_margin(egui::Margin::same(20))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(short(&root)).size(type_scale::TITLE).strong());
                                    ui.label(caption(crate::tr!("{} · {} items", crate::i18n::bytes(size), items)));
                                    if self.basket_len() > 0 {
                                        ui.label(caption(crate::tr!("Selected for review: {}", crate::i18n::bytes(self.basket_bytes()))).color(ACCENT()));
                                    }
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add(primary("Continue")).clicked() {
                                        pick.resume = true;
                                    }
                                });
                            });
                        });
                    ui.add_space(S4);
                }
                let cols = ((ui.available_width() + S4) / (240.0 + S4)).floor().clamp(1.0, 3.0) as usize;
                let w = (ui.available_width() - S4 * (cols as f32 - 1.0)) / cols as f32;
                let mut targets: Vec<(String, String, f32, PathBuf)> = Vec::new();
                if let Some(home) = std::env::var_os("HOME") {
                    targets.push((t("Home folder").into(), "~/".into(), -1.0, PathBuf::from(home)));
                }
                for v in &self.volumes {
                    let frac = if v.total > 0 { v.used() as f32 / v.total as f32 } else { -1.0 };
                    targets.push((v.name.clone(), crate::tr!("{} free of {}", crate::i18n::bytes(v.available), crate::i18n::bytes(v.total)), frac, v.mount.clone()));
                }
                for row in targets.chunks(cols) {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = S4;
                        for (name, note, frac, path) in row {
                            egui::Frame::new()
                                .fill(PANEL())
                                .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                                .corner_radius(12)
                                .inner_margin(egui::Margin::same(16))
                                .show(ui, |ui| {
                                    ui.set_width(w - 34.0);
                                    ui.label(RichText::new(name).size(type_scale::LEAD).strong());
                                    if *frac >= 0.0 {
                                        crate::widgets::meter(ui, *frac, if *frac > 0.85 { ACCENT() } else { TEXT() }, 6.0);
                                    }
                                    ui.label(caption(note.clone()));
                                    ui.add_space(S2);
                                    if ui
                                        .add_enabled(enabled, secondary("Scan").min_size(egui::vec2(ui.available_width(), 30.0)))
                                        .clicked()
                                    {
                                        pick.scan = Some(path.clone());
                                    }
                                });
                        }
                    });
                    ui.add_space(S4);
                }
            });
        self.apply_pick(pick);
    }

    // --------------------------------------------------------------- table

    pub(super) fn start_table(&mut self, ctx: &egui::Context) {
        let enabled = !self.busy && self.scan.is_none();
        let mut pick = Pick::default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        egui::TopBottomPanel::bottom("start_table_foot")
            .frame(egui::Frame::new().fill(PANEL()).stroke(egui::Stroke::new(1.0_f32, BORDER())).inner_margin(egui::Margin::symmetric(S4 as i8, S2 as i8)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(caption("The open scan keeps its tree and selections. Rescan updates the tree and keeps what you marked."));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(enabled, ghost("Review cleanup candidates")).clicked() {
                            pick.cleanup = true;
                        }
                        if ui.add_enabled(enabled, secondary("Folder…  ⌘O")).clicked() {
                            pick.folder = true;
                        }
                        self.refresh_disks(ui, &mut pick);
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()))
            .show(ctx, |ui| {
                let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
                ui.painter().rect_filled(head, 0.0, PANEL_2());
                let cols = StartCols::new(head);
                let f = egui::FontId::proportional(type_scale::CAPTION);
                for (x, label, right) in [
                    (cols.name, t("DISK / FOLDER"), false),
                    (cols.used, t("USED"), false),
                    (cols.free_r, t("FREE"), true),
                    (cols.idx_r, t("INDEX"), true),
                ] {
                    let g = ui.painter().layout_no_wrap(label.to_string(), f.clone(), MUTED());
                    let px = if right { x - g.size().x } else { x };
                    ui.painter().galley(egui::pos2(px, head.center().y - g.size().y / 2.0), g, MUTED());
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let open_root = self.tree.as_ref().map(|t| t.root_path.clone());
                    let mut rows: Vec<(String, String, f32, String, String, PathBuf, bool, usize)> = Vec::new();
                    if let Some((root, size, _)) = self.open_session() {
                        rows.push((short(&root), t("open").into(), 1.0, crate::i18n::bytes(size), t("now").into(), PathBuf::from(&root), true, 0));
                    }
                    for v in &self.volumes {
                        let frac = if v.total > 0 { v.used() as f32 / v.total as f32 } else { 0.0 };
                        let idx = self
                            .recent
                            .iter()
                            .find(|m| PathBuf::from(&m.root_path) == v.mount)
                            .map(|m| format_age(now.saturating_sub(m.scanned_at_unix)))
                            .unwrap_or_else(|| t("none").into());
                        rows.push((v.name.clone(), t("disk").into(), frac, crate::i18n::bytes(v.available), idx, v.mount.clone(), false, 0));
                        for m in self.recent.iter().filter(|m| PathBuf::from(&m.root_path).starts_with(&v.mount) && PathBuf::from(&m.root_path) != v.mount && Some(&m.root_path) != open_root.as_ref()).take(4) {
                            rows.push((short(&m.root_path), t("folder").into(), -1.0, crate::i18n::bytes(m.total_size), format_age(now.saturating_sub(m.scanned_at_unix)), PathBuf::from(&m.root_path), false, 1));
                        }
                    }
                    for (name, kind, frac, free, idx, path, open, depth) in rows {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
                        let cols = StartCols::new(rect);
                        let p = ui.painter();
                        if open {
                            p.rect_filled(rect, 0.0, ACCENT().gamma_multiply(0.08));
                        } else if resp.hovered() {
                            p.rect_filled(rect, 0.0, PANEL_2().gamma_multiply(0.6));
                        }
                        p.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0_f32, BORDER().gamma_multiply(0.6)));
                        let cy = rect.center().y;
                        let x = cols.name + depth as f32 * 18.0;
                        let ng = p.layout_no_wrap(name, egui::FontId::proportional(type_scale::BODY), TEXT());
                        let nw = ng.size().x;
                        p.galley(egui::pos2(x, cy - ng.size().y / 2.0), ng, TEXT());
                        p.text(egui::pos2(x + nw + 8.0, cy), egui::Align2::LEFT_CENTER, kind, egui::FontId::proportional(type_scale::CAPTION), MUTED());
                        if frac >= 0.0 {
                            let track = egui::Rect::from_min_size(egui::pos2(cols.used, cy - 4.0), egui::vec2(120.0, 8.0));
                            p.rect_filled(track, 0.0, BORDER());
                            let mut fill = track;
                            fill.set_width(120.0 * frac.clamp(0.0, 1.0));
                            p.rect_filled(fill, 0.0, if frac > 0.85 { ACCENT() } else { MUTED() });
                        }
                        p.text(egui::pos2(cols.free_r, cy), egui::Align2::RIGHT_CENTER, free, egui::FontId::monospace(type_scale::MONO - 1.0), MUTED());
                        p.text(egui::pos2(cols.idx_r, cy), egui::Align2::RIGHT_CENTER, idx, egui::FontId::proportional(type_scale::CAPTION), MUTED());
                        let btn = egui::Rect::from_min_size(egui::pos2(cols.btn, cy - 14.0), egui::vec2(110.0, 28.0));
                        let label = if open { t("Continue") } else if depth > 0 { t("Open") } else { t("Scan") };
                        p.rect_filled(btn, 4.0, if open { TEXT() } else { PANEL() });
                        p.rect_stroke(btn, 4.0, egui::Stroke::new(1.0_f32, BORDER()), egui::StrokeKind::Inside);
                        p.text(btn.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(type_scale::CAPTION + 1.0), if open { BG() } else { TEXT() });
                        if resp.clicked() {
                            if open {
                                pick.resume = true;
                            } else if enabled {
                                pick.scan = Some(path);
                            }
                        }
                    }
                });
            });
        self.apply_pick(pick);
    }
}

struct StartCols {
    name: f32,
    used: f32,
    free_r: f32,
    idx_r: f32,
    btn: f32,
}

impl StartCols {
    fn new(r: egui::Rect) -> Self {
        let btn = r.right() - S4 - 110.0;
        let idx_r = btn - S6;
        let free_r = idx_r - 130.0;
        let used = free_r - 110.0 - 140.0;
        Self { name: r.left() + S4, used: used.max(r.left() + 280.0), free_r, idx_r, btn }
    }
}

fn side_item(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
    if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, 6.0, PANEL());
    }
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), egui::FontId::proportional(type_scale::BODY - 1.0), TEXT());
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - S3 * 2.0);
    let g = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(rect.left() + S3, rect.center().y - g.size().y / 2.0), g, TEXT());
    enabled && resp.on_hover_text(label).clicked()
}

/// `~/…` for paths under home, otherwise the path itself.
fn short(path: &str) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy().to_string();
        if let Some(rest) = path.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    path.to_owned()
}
