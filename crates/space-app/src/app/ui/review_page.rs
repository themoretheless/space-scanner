//! Trash list: the one place where removal is checked. It lists both sources
//! with what each item is, says what will be skipped, and is the only route to
//! the Move to Trash confirmation. Afterwards it shows what actually happened.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Layout, RichText, Sense};
use space_core::NodeId;

use crate::app::{Mode, SpaceApp};
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::metrics::{RADIUS_CARD, S2, S3, S4, S6};
use crate::theme::type_scale;
use crate::theme::{ACCENT, BG, BORDER, DANGER, MUTED, PANEL, PANEL_2, SAFE, TEXT};
use crate::widgets::{
    caption, count_noun, empty_state, ghost, page_column, primary, secondary, section_label,
    strong, title,
};

const ROW_H: f32 = 52.0;
const HEAD_H: f32 = 40.0;

enum Source {
    Disk(NodeId),
    Junk,
}

struct Item {
    source: Source,
    path: PathBuf,
    size: u64,
    is_dir: bool,
    protected: bool,
    badge: Option<(String, egui::Color32)>,
}

enum Row {
    Header(&'static str, usize, u64),
    Item(usize),
}

enum Act {
    Drop(usize),
    DropProtected,
    Reveal(PathBuf),
    Clear,
    Confirm,
    Go(Mode),
    OpenTrash,
}

impl SpaceApp {
    pub(super) fn review_page(&mut self, ctx: &egui::Context) {
        let items = self.trash_items();
        let movable: Vec<&Item> = items.iter().filter(|i| !i.protected).collect();
        let move_bytes = movable.iter().fold(0_u64, |s, i| s.saturating_add(i.size));
        let skipped = items.len() - movable.len();
        let idle = !self.busy && self.scan.is_none() && !self.junk.cleaning;
        let mut act = None;

        if !items.is_empty() {
            egui::TopBottomPanel::bottom("trash_footer")
            .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(PANEL())
                        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                        .inner_margin(egui::Margin::symmetric(S6 as i8, 14)),
                )
                .show(ctx, |ui| {
                    page_column(ui, 920.0, |ui| {
                        ui.horizontal(|ui| {
                            let w = (ui.available_width() - 210.0).max(200.0);
                            ui.vertical(|ui| {
                                ui.set_width(w);
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(strong(crate::i18n::bytes(move_bytes), type_scale::PAGE));
                                    let mut line = crate::tr!(
                                        "{} will move",
                                        count_noun(movable.len(), "item")
                                    );
                                    if skipped > 0 {
                                        line.push_str(&crate::tr!(" · {} skipped", skipped));
                                    }
                                    ui.label(RichText::new(line).color(MUTED()));
                                });
                                ui.add(
                                    egui::Label::new(caption(
                                        "You can put items back from the Trash until you empty it. Space returns after that.",
                                    ))
                                    .wrap(),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add_enabled(
                                        !movable.is_empty() && idle,
                                        primary("Move to Trash…").min_size(egui::vec2(0.0, 40.0)),
                                    )
                                    .on_hover_text("⌘⌫")
                                    .clicked()
                                {
                                    act = Some(Act::Confirm);
                                }
                            });
                        });
                    });
                });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()))
            .show(ctx, |ui| {
                page_column(ui, 920.0, |ui| {
                    ui.add_space(24.0);
                    if items.is_empty() {
                        if let Some(result) = self.trash_result.clone() {
                            self.trash_result_view(ui, &result, &mut act);
                        } else {
                            ui.label(title("Trash list"));
                            empty_state(
                                ui,
                                "The Trash list is empty",
                                "Add items in Disk usage or Cleanup. Nothing moves until you confirm here.",
                            );
                            ui.add_space(S4);
                            ui.vertical_centered(|ui| {
                                ui.horizontal(|ui| {
                                    ui.add_space(((ui.available_width() - 290.0) / 2.0).max(0.0));
                                    if ui
                                        .add_enabled(self.tree.is_some(), secondary("Open Disk usage"))
                                        .clicked()
                                    {
                                        act = Some(Act::Go(Mode::Explore));
                                    }
                                    if ui.add(secondary("Open Cleanup")).clicked() {
                                        act = Some(Act::Go(Mode::Clean));
                                    }
                                });
                            });
                        }
                        return;
                    }

                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(title("Trash list"));
                            ui.label(caption(
                                "Check what will move to the Trash. Overlapping paths are counted once.",
                            ));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let armed_id = egui::Id::new("trash_clear_armed");
                            let armed = ui.data(|d| d.get_temp::<bool>(armed_id)).unwrap_or(false);
                            let label = if armed { "Click again to clear" } else { "Clear list" };
                            let b = if armed { secondary(label) } else { ghost(label) };
                            let resp = ui.add_enabled(idle, b);
                            if resp.clicked() {
                                if armed {
                                    act = Some(Act::Clear);
                                    ui.data_mut(|d| d.remove::<bool>(armed_id));
                                } else {
                                    ui.data_mut(|d| d.insert_temp(armed_id, true));
                                }
                            } else if armed && !resp.hovered() && ui.input(|i| i.pointer.any_click()) {
                                ui.data_mut(|d| d.remove::<bool>(armed_id));
                            }
                            if ui
                                .add_enabled(!items.is_empty() && !self.exporting, secondary("Export list…"))
                                .on_hover_text(t("Save the paths and sizes below as CSV"))
                                .clicked()
                            {
                                self.export_review_csv();
                            }
                            if ui
                                .add(secondary("Reveal in Trash"))
                                .on_hover_text(t("Open the Trash folder, where moved items wait"))
                                .clicked()
                            {
                                self.reveal_trash();
                            }
                        });
                    });
                    ui.add_space(S3);

                    if let Some(first) = items.iter().find(|i| i.protected) {
                        egui::Frame::new()
                            .fill(DANGER().gamma_multiply(0.08))
                            .stroke(egui::Stroke::new(1.0_f32, DANGER().gamma_multiply(0.5)))
                            .corner_radius(8)
                            .inner_margin(egui::Margin::symmetric(S3 as i8, 8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    icons::show(ui, Icon::Lock, 16.0, DANGER());
                                    let text = if skipped == 1 {
                                        crate::tr!(
                                            "{} is protected and will be skipped.",
                                            first.path.display()
                                        )
                                    } else {
                                        crate::tr!(
                                            "{} protected items will be skipped, such as {}.",
                                            skipped,
                                            first.path.display()
                                        )
                                    };
                                    ui.add(egui::Label::new(RichText::new(text).color(TEXT())).truncate());
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui.add(ghost("Remove from list")).clicked() {
                                            act = Some(Act::DropProtected);
                                        }
                                    });
                                });
                            });
                        ui.add_space(S3);
                    }

                    let mut rows = Vec::new();
                    for (title, disk) in [("From Disk usage", true), ("From Cleanup", false)] {
                        let idx: Vec<usize> = items
                            .iter()
                            .enumerate()
                            .filter(|(_, i)| matches!(i.source, Source::Disk(_)) == disk)
                            .map(|(n, _)| n)
                            .collect();
                        if idx.is_empty() {
                            continue;
                        }
                        let bytes = idx.iter().fold(0_u64, |s, &n| s.saturating_add(items[n].size));
                        rows.push(Row::Header(title, idx.len(), bytes));
                        rows.extend(idx.into_iter().map(Row::Item));
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("trash_rows")
                        .auto_shrink([false, false])
                        .show_rows(ui, ROW_H, rows.len(), |ui, range| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for row in &rows[range] {
                                match row {
                                    Row::Header(title, n, bytes) => header_row(ui, title, *n, *bytes),
                                    Row::Item(n) => {
                                        if let Some(a) = item_row(ui, &items[*n], *n) {
                                            act = Some(a);
                                        }
                                    }
                                }
                            }
                        });
                });
            });

        match act {
            Some(Act::Drop(n)) => self.drop_items(&items[n..=n]),
            Some(Act::DropProtected) => {
                let protected: Vec<&Item> = items.iter().filter(|i| i.protected).collect();
                self.drop_refs(&protected);
            }
            Some(Act::Reveal(path)) => crate::app::reveal_path(&path.display().to_string()),
            Some(Act::Clear) => {
                self.review.clear();
                self.junk.clear_selection();
            }
            Some(Act::Confirm) => self.confirm_delete = true,
            Some(Act::Go(mode)) => self.set_mode(mode),
            Some(Act::OpenTrash) => self.open_trash(),
            None => {}
        }
    }

    /// Items per source after nesting is resolved: (from Disk usage, from Cleanup).
    pub(super) fn trash_source_counts(&self) -> (usize, usize) {
        let items = self.trash_items();
        let disk = items
            .iter()
            .filter(|i| matches!(i.source, Source::Disk(_)))
            .count();
        (disk, items.len() - disk)
    }

    /// Both sources as one list, nested paths counted once.
    fn trash_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        if let Some(tree) = &self.tree {
            for &id in &self.review {
                let Some(node) = tree.try_node(id) else {
                    continue;
                };
                let path = PathBuf::from(tree.path_string(id));
                let protected = space_clean::safety::is_never_touch(&path);
                let badge = self
                    .junk
                    .risk_hint(&path)
                    .map(|(_, label, color, _)| (t(label).to_owned(), color));
                items.push(Item {
                    source: Source::Disk(id),
                    size: node.size(self.size_mode),
                    is_dir: node.is_dir,
                    protected,
                    badge,
                    path,
                });
            }
        }
        for f in self.junk.selected_findings() {
            let protected = space_clean::safety::is_never_touch(&f.path);
            let badge = self
                .junk
                .finding_badge(&f.path)
                .map(|(cat, label, color)| (format!("{} · {}", t(label), t(&cat)), color));
            items.push(Item {
                source: Source::Junk,
                size: f.bytes,
                is_dir: f.is_dir,
                protected,
                badge,
                path: f.path,
            });
        }
        // Drop items inside another listed folder: the folder already covers them.
        let paths: std::collections::HashSet<PathBuf> =
            items.iter().map(|i| i.path.clone()).collect();
        items.retain(|i| !i.path.ancestors().skip(1).any(|a| paths.contains(a)));
        items.sort_by_key(|i| std::cmp::Reverse(i.size));
        items
    }

    fn drop_items(&mut self, items: &[Item]) {
        let refs: Vec<&Item> = items.iter().collect();
        self.drop_refs(&refs);
    }

    fn drop_refs(&mut self, items: &[&Item]) {
        let mut junk = Vec::new();
        for item in items {
            match item.source {
                Source::Disk(id) => self.review.retain(|x| *x != id),
                Source::Junk => junk.push(item.path.clone()),
            }
        }
        // Exactly these findings: a parent path must not take its children along.
        for path in &junk {
            self.junk.exclude_exact(path);
        }
    }

    fn trash_result_view(
        &self,
        ui: &mut egui::Ui,
        result: &crate::app::TrashResult,
        act: &mut Option<Act>,
    ) {
        let ok = result.moved > 0;
        ui.add_space(32.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(52.0, 52.0), Sense::hover());
        let color = if ok { SAFE() } else { DANGER() };
        ui.painter()
            .circle_filled(rect.center(), 26.0, color.gamma_multiply(0.14));
        icons::paint(
            ui.painter(),
            if ok { Icon::Check } else { Icon::Alert },
            egui::Rect::from_center_size(rect.center(), egui::vec2(26.0, 26.0)),
            color,
        );
        ui.add_space(S4);
        if ok {
            ui.label(strong(
                crate::tr!("Moved {} to the Trash", count_noun(result.moved, "item")),
                26.0,
            ));
            ui.label(
                RichText::new(crate::tr!(
                    "{} is in the Trash now. Disk usage and Cleanup were updated.",
                    crate::i18n::bytes(result.bytes)
                ))
                .color(MUTED()),
            );
        } else {
            ui.label(strong("Nothing was moved", 26.0));
            ui.label(RichText::new(t("The items below could not go to the Trash.")).color(MUTED()));
        }
        ui.add_space(S6);

        if let Some(vol) = self.startup_volume() {
            egui::Frame::new()
                .fill(PANEL())
                .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                .corner_radius(RADIUS_CARD + 2)
                .inner_margin(egui::Margin::same(S4 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().min(640.0));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&vol.name).family(crate::theme::medium()));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if self.volumes_loading {
                                ui.spinner();
                            } else {
                                ui.label(caption("checked just now"));
                            }
                        });
                    });
                    ui.add_space(S2);
                    let total = vol.total.max(1) as f32;
                    let used = vol.used().min(vol.total);
                    let in_trash = result.bytes.min(used);
                    let (bar, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 10.0),
                        Sense::hover(),
                    );
                    let p = ui.painter();
                    p.rect_filled(bar, 5.0, PANEL_2());
                    let used_w = bar.width() * (used.saturating_sub(in_trash)) as f32 / total;
                    let trash_w = (bar.width() * in_trash as f32 / total).max(if in_trash > 0 {
                        3.0
                    } else {
                        0.0
                    });
                    p.rect_filled(
                        egui::Rect::from_min_size(bar.min, egui::vec2(used_w, bar.height())),
                        5.0,
                        MUTED().gamma_multiply(0.7),
                    );
                    p.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(bar.left() + used_w + 2.0, bar.top()),
                            egui::vec2(trash_w, bar.height()),
                        ),
                        5.0,
                        ACCENT(),
                    );
                    ui.add_space(S3);
                    ui.columns(2, |c| {
                        c[0].label(strong(
                            crate::i18n::bytes(vol.available),
                            type_scale::TITLE + 2.0,
                        ));
                        c[0].label(caption("free now"));
                        c[1].label(
                            strong(crate::i18n::bytes(result.bytes), type_scale::TITLE + 2.0)
                                .color(ACCENT()),
                        );
                        c[1].label(caption("in the Trash, until you empty it"));
                    });
                });
            ui.add_space(S4);
        }

        if !result.failures.is_empty() {
            egui::Frame::new()
                .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                .corner_radius(RADIUS_CARD + 2)
                .inner_margin(egui::Margin::symmetric(S4 as i8, S3 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().min(640.0));
                    egui::CollapsingHeader::new(
                        RichText::new(crate::tr!(
                            "{} not moved",
                            count_noun(result.failures.len(), "item")
                        ))
                        .family(crate::theme::medium()),
                    )
                    .default_open(result.failures.len() <= 5)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(180.0)
                            .show(ui, |ui| {
                                for f in &result.failures {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(failure_text(f))
                                                .monospace()
                                                .size(type_scale::CAPTION)
                                                .color(MUTED()),
                                        )
                                        .wrap(),
                                    );
                                }
                            });
                    });
                });
            ui.add_space(S4);
        }

        ui.horizontal(|ui| {
            if ok && ui.add(primary("Open Trash")).clicked() {
                *act = Some(Act::OpenTrash);
            }
            if self.tree.is_some() {
                if ui.add(secondary("Back to Disk usage")).clicked() {
                    *act = Some(Act::Go(Mode::Explore));
                }
            } else if ui.add(secondary("Overview")).clicked() {
                *act = Some(Act::Go(Mode::Home));
            }
        });
        ui.add_space(S3);
        ui.label(caption("Emptying the Trash removes the files for good."));
    }
}

/// "Protected: /x" from the worker reads better as "/x — protected folder".
fn failure_text(raw: &str) -> String {
    match raw.strip_prefix("Protected: ") {
        Some(path) => crate::tr!("{} — protected, skipped", path),
        None => raw.to_owned(),
    }
}

fn header_row(ui: &mut egui::Ui, title: &str, n: usize, bytes: u64) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ROW_H),
        Layout::left_to_right(Align::Max),
        |ui| {
            ui.set_height(HEAD_H.max(ROW_H));
            ui.label(section_label(title));
            ui.label(caption(format!("· {n}")));
            ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
                ui.add_space(80.0);
                ui.label(
                    RichText::new(crate::i18n::bytes(bytes))
                        .monospace()
                        .color(MUTED()),
                );
            });
        },
    );
}

/// One listed item: kind, name, folder, rule or protection, size, reveal, remove.
fn item_row(ui: &mut egui::Ui, item: &Item, n: usize) -> Option<Act> {
    let mut act = None;
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::hover());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 6.0, PANEL_2().gamma_multiply(0.5));
    }
    p.hline(
        rect.x_range(),
        rect.top() + 0.5,
        egui::Stroke::new(1.0_f32, BORDER()),
    );
    let cy = rect.center().y;
    let icon_box =
        egui::Rect::from_center_size(egui::pos2(rect.left() + 24.0, cy), egui::vec2(32.0, 32.0));
    p.rect_filled(icon_box, 8.0, PANEL());
    icons::paint(
        p,
        if item.protected {
            Icon::Lock
        } else if item.is_dir {
            Icon::Folder
        } else {
            Icon::File
        },
        icon_box.shrink(8.0),
        if item.protected { DANGER() } else { MUTED() },
    );

    // Right side: remove, reveal, size, badge.
    let mut x = rect.right() - 4.0;
    // Painted icon buttons with their own hit areas, so the row keeps its height.
    let mut button = |ui: &mut egui::Ui, icon: Icon, hint: &str, salt: &str| {
        let r = egui::Rect::from_min_size(egui::pos2(x - 30.0, cy - 15.0), egui::vec2(30.0, 30.0));
        x -= 34.0;
        let resp = ui
            .interact(r, ui.id().with((salt, n)), Sense::click())
            .on_hover_text(t(hint));
        if resp.hovered() {
            ui.painter().rect_filled(r, 8.0, PANEL_2());
        }
        icons::paint(
            ui.painter(),
            icon,
            r.shrink(7.0),
            if resp.hovered() { TEXT() } else { MUTED() },
        );
        resp.clicked()
    };
    if button(ui, Icon::Close, "Remove from Trash list", "drop") {
        act = Some(Act::Drop(n));
    }
    if button(ui, Icon::Reveal, "Show in Finder", "reveal") {
        act = Some(Act::Reveal(item.path.clone()));
    }
    let p = ui.painter();
    let size_g = p.layout_no_wrap(
        crate::i18n::bytes(item.size),
        egui::FontId::monospace(type_scale::MONO),
        TEXT(),
    );
    let size_x = x - 8.0 - size_g.size().x.max(64.0);
    p.galley(
        egui::pos2(x - 8.0 - size_g.size().x, cy - size_g.size().y / 2.0),
        size_g,
        TEXT(),
    );
    let mut text_right = size_x - 12.0;
    let badge = if item.protected {
        Some((t("Protected").to_owned(), DANGER()))
    } else {
        item.badge.clone()
    };
    if let Some((label, color)) = badge {
        let g = p.layout_no_wrap(
            label,
            egui::FontId::proportional(type_scale::CAPTION),
            color,
        );
        let w = g.size().x + 16.0;
        let pill =
            egui::Rect::from_min_size(egui::pos2(text_right - w, cy - 11.0), egui::vec2(w, 22.0));
        if pill.left() > rect.left() + 200.0 {
            p.rect_stroke(
                pill,
                11.0,
                egui::Stroke::new(1.0_f32, color.gamma_multiply(0.55)),
                egui::StrokeKind::Inside,
            );
            p.galley(
                egui::pos2(pill.left() + 8.0, cy - g.size().y / 2.0),
                g,
                color,
            );
            text_right = pill.left() - 12.0;
        }
    }

    let x0 = rect.left() + 52.0;
    let w = (text_right - x0).max(40.0);
    let name = item
        .path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| item.path.display().to_string());
    let folder = item.path.parent().map(home_relative).unwrap_or_default();
    let mut job = egui::text::LayoutJob::simple_singleline(
        name,
        egui::FontId::new(type_scale::BODY, crate::theme::medium()),
        if item.protected { MUTED() } else { TEXT() },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(w);
    let g = p.layout_job(job);
    let mut job = egui::text::LayoutJob::simple_singleline(
        folder,
        egui::FontId::monospace(type_scale::CAPTION),
        MUTED(),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(w);
    let g2 = p.layout_job(job);
    let top = cy - (g.size().y + g2.size().y) / 2.0;
    let gh = g.size().y;
    p.galley(egui::pos2(x0, top), g, TEXT());
    p.galley(egui::pos2(x0, top + gh), g2, MUTED());
    ui.interact(
        egui::Rect::from_min_max(
            egui::pos2(x0, rect.top()),
            egui::pos2(text_right, rect.bottom()),
        ),
        ui.id().with(("trash_row", n)),
        Sense::hover(),
    )
    .on_hover_text(item.path.display().to_string());
    act
}

fn home_relative(p: &Path) -> String {
    let s = p.display().to_string();
    if let Some(home) = std::env::var_os("HOME") {
        if let Some(rest) = s.strip_prefix(home.to_string_lossy().as_ref()) {
            return format!("~{rest}");
        }
    }
    s
}
