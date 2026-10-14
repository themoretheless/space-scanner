//! Skin-specific Disk usage workspaces. Map keeps the original list + map
//! layout (`usage.rs`); Columns is Finder-style column browsing with an
//! inspector; Table is one dense tree table with checkboxes for review.
//! All three share the same state: focus, selection, expansion, review.

use crate::i18n::t;

use eframe::egui::{self, Align, Layout, RichText, Sense};
use space_core::NodeId;

use super::usage::{expand_all, tree_rows, TreeRow, MAX_TREE_ROWS};
use super::Density;
use crate::app::SpaceApp;
use crate::icons::{self, Icon};
use crate::theme::metrics::{S2, S3, S4, SIDE_W, TREE_INDENT};
use crate::theme::type_scale;
use crate::theme::{self, ACCENT, ACCENT_INK, BG, BORDER, MUTED, PANEL, PANEL_2, TEXT};
use crate::widgets::{caption, empty_state};

const COL_W: f32 = 300.0;
const COL_ROW_H: f32 = 40.0;
const TABLE_ROW_H: f32 = 28.0;

impl SpaceApp {
    // ------------------------------------------------------------- columns

    pub(super) fn columns_page(&mut self, ctx: &egui::Context, d: Density) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        self.tree_rows.clear();
        if d == Density::Narrow {
            self.details_drawer(ctx);
        }
        if d != Density::Narrow {
            egui::SidePanel::right("columns_inspector")
                .show_separator_line(false)
                .exact_width(SIDE_W + 20.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(BG())
                        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                        .inner_margin(egui::Margin::same(S4 as i8 + 2)),
                )
                .show(ctx, |ui| {
                    if self.selected.is_some() {
                        self.details(ui);
                    } else {
                        empty_state(
                            ui,
                            "Nothing selected",
                            "Click an item in a column to see what it is.",
                        );
                    }
                });
        }

        // Columns: the focus, then every folder on the way to the selection.
        let mut dirs = vec![self.focus];
        if let Some(sel) = self.selected {
            let path = tree.path_to(sel);
            if let Some(at) = path.iter().position(|&x| x == self.focus) {
                for &x in &path[at + 1..] {
                    let n = tree.node(x);
                    if n.is_dir && !n.synthetic && !tree.children(x).is_empty() {
                        dirs.push(x);
                    }
                }
            }
        }
        let on_path = |id: NodeId, sel: Option<NodeId>| {
            sel.is_some_and(|s| s == id || tree.path_to(s).contains(&id))
        };
        let mode = self.size_mode;
        let (mut pick, mut enter, mut queue) = (None, None, None);

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PANEL()))
            .show(ctx, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("columns")
                    .auto_shrink([false, false])
                    .stick_to_right(true)
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for &dir in &dirs {
                                let total = tree.node(dir).size(mode).max(1) as f32;
                                let mut kids = tree.children(dir).to_vec();
                                kids.retain(|&c| {
                                    let n = tree.node(c);
                                    self.filter.matches(&n.name, n.is_dir, n.size(mode))
                                });
                                kids.sort_by_key(|&c| std::cmp::Reverse(tree.node(c).size(mode)));
                                let items: Vec<(&str, bool)> = kids
                                    .iter()
                                    .map(|&c| (tree.node(c).name.as_str(), tree.node(c).is_dir))
                                    .collect();
                                let colors = theme::sibling_palette(&items);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(COL_W, ui.available_height()),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        ui.set_width(COL_W);
                                        let head = ui.available_rect_before_wrap();
                                        ui.painter().vline(
                                            head.right(),
                                            head.top()..=head.top() + ui.available_height(),
                                            egui::Stroke::new(1.0_f32, BORDER()),
                                        );
                                        ui.horizontal(|ui| {
                                            ui.add_space(S3);
                                            ui.label(
                                                RichText::new(&tree.node(dir).name)
                                                    .family(crate::theme::bold()),
                                            );
                                            ui.add_space(6.0);
                                            ui.label(caption(crate::i18n::bytes(total as u64)));
                                        });
                                        ui.add_space(4.0);
                                        egui::ScrollArea::vertical()
                                            .id_salt(("column", dir.0))
                                            .auto_shrink([false, false])
                                            .show(ui, |ui| {
                                                ui.spacing_mut().item_spacing.y = 0.0;
                                                for (i, &id) in kids.iter().enumerate() {
                                                    let n = tree.node(id);
                                                    let hot = on_path(id, self.selected);
                                                    let size = n.size(mode);
                                                    let (rect, resp) = ui.allocate_exact_size(
                                                        egui::vec2(COL_W - 1.0, COL_ROW_H),
                                                        Sense::CLICK,
                                                    );
                                                    let p = ui.painter();
                                                    let fg =
                                                        if hot { ACCENT_INK() } else { TEXT() };
                                                    if hot {
                                                        p.rect_filled(
                                                            rect.shrink2(egui::vec2(4.0, 1.0)),
                                                            6.0,
                                                            ACCENT(),
                                                        );
                                                    } else if resp.hovered() {
                                                        p.rect_filled(
                                                            rect.shrink2(egui::vec2(4.0, 1.0)),
                                                            6.0,
                                                            PANEL_2(),
                                                        );
                                                    }
                                                    let swatch = if n.synthetic {
                                                        MUTED()
                                                    } else {
                                                        colors[i]
                                                    };
                                                    let cy = rect.top() + 16.0;
                                                    p.rect_filled(
                                                        egui::Rect::from_center_size(
                                                            egui::pos2(rect.left() + 18.0, cy),
                                                            egui::vec2(10.0, 10.0),
                                                        ),
                                                        3.0,
                                                        swatch,
                                                    );
                                                    let size_g = p.layout_no_wrap(
                                                        crate::i18n::bytes(size),
                                                        egui::FontId::monospace(
                                                            type_scale::MONO - 1.0,
                                                        ),
                                                        if hot { ACCENT_INK() } else { MUTED() },
                                                    );
                                                    let right = rect.right() - 26.0;
                                                    let sw = size_g.size().x;
                                                    p.galley(
                                                        egui::pos2(
                                                            right - sw,
                                                            cy - size_g.size().y / 2.0,
                                                        ),
                                                        size_g,
                                                        MUTED(),
                                                    );
                                                    if n.is_dir && !n.synthetic {
                                                        icons::paint(
                                                            p,
                                                            Icon::ChevronRight,
                                                            egui::Rect::from_center_size(
                                                                egui::pos2(rect.right() - 14.0, cy),
                                                                egui::vec2(12.0, 12.0),
                                                            ),
                                                            if hot {
                                                                ACCENT_INK()
                                                            } else {
                                                                MUTED()
                                                            },
                                                        );
                                                    }
                                                    if self.review.contains(&id) {
                                                        p.circle_filled(
                                                            egui::pos2(rect.left() + 8.0, cy),
                                                            3.0,
                                                            if hot {
                                                                ACCENT_INK()
                                                            } else {
                                                                ACCENT()
                                                            },
                                                        );
                                                    }
                                                    let mut job =
                                                        egui::text::LayoutJob::simple_singleline(
                                                            n.name.clone(),
                                                            egui::FontId::proportional(
                                                                type_scale::BODY,
                                                            ),
                                                            if n.synthetic && !hot {
                                                                MUTED()
                                                            } else {
                                                                fg
                                                            },
                                                        );
                                                    job.wrap =
                                                        egui::text::TextWrapping::truncate_at_width(
                                                            (right
                                                                - sw
                                                                - 10.0
                                                                - (rect.left() + 30.0))
                                                                .max(20.0),
                                                        );
                                                    let g = p.layout_job(job);
                                                    p.galley(
                                                        egui::pos2(
                                                            rect.left() + 30.0,
                                                            cy - g.size().y / 2.0,
                                                        ),
                                                        g,
                                                        fg,
                                                    );
                                                    // Share of this column's folder.
                                                    let track = egui::Rect::from_min_max(
                                                        egui::pos2(
                                                            rect.left() + 30.0,
                                                            rect.bottom() - 9.0,
                                                        ),
                                                        egui::pos2(
                                                            rect.right() - 14.0,
                                                            rect.bottom() - 6.0,
                                                        ),
                                                    );
                                                    p.rect_filled(
                                                        track,
                                                        1.5,
                                                        if hot {
                                                            ACCENT_INK().gamma_multiply(0.25)
                                                        } else {
                                                            BORDER()
                                                        },
                                                    );
                                                    let mut fill = track;
                                                    fill.set_width(
                                                        track.width()
                                                            * (size as f32 / total).clamp(0.0, 1.0),
                                                    );
                                                    p.rect_filled(
                                                        fill,
                                                        1.5,
                                                        if hot { ACCENT_INK() } else { swatch },
                                                    );

                                                    if resp.clicked() {
                                                        if ui.input(|i| i.modifiers.command)
                                                            && !n.synthetic
                                                        {
                                                            queue = Some(id);
                                                        }
                                                        pick = Some(id);
                                                    }
                                                    if resp.double_clicked() && !n.synthetic {
                                                        enter = Some(id);
                                                    }
                                                    resp.context_menu(|ui| {
                                                        if !n.synthetic {
                                                            let q = if self.review.contains(&id) {
                                                                "Take out of Trash list"
                                                            } else {
                                                                "Add to Trash list"
                                                            };
                                                            if ui.button(t(q)).clicked() {
                                                                queue = Some(id);
                                                                ui.close_menu();
                                                            }
                                                        }
                                                    });
                                                }
                                                if kids.is_empty() {
                                                    ui.add_space(S4);
                                                    ui.horizontal(|ui| {
                                                        ui.add_space(S3);
                                                        ui.label(caption("Empty"));
                                                    });
                                                }
                                            });
                                    },
                                );
                            }
                        });
                    });
            });
        if let Some(id) = pick {
            self.selected = Some(id);
        }
        if let Some(id) = queue {
            self.queue_toggle(&[id]);
        }
        if let Some(id) = enter {
            if tree.node(id).is_dir {
                self.navigate(id, true);
            } else {
                self.selected = Some(id);
                self.reveal_selected();
            }
        }
    }

    // --------------------------------------------------------------- table

    pub(super) fn table_page(&mut self, ctx: &egui::Context, d: Density) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        if d != Density::Wide {
            self.details_drawer(ctx);
        }
        if d == Density::Wide && self.selected.is_some() {
            egui::SidePanel::right("table_inspector")
                .show_separator_line(false)
                .exact_width(SIDE_W + 10.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(PANEL())
                        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                        .inner_margin(egui::Margin::same(S4 as i8 + 2)),
                )
                .show(ctx, |ui| self.details(ui));
        }

        let mode = self.size_mode;
        let focus = self.focus;
        let mut ids: Vec<NodeId> = tree.children(focus).to_vec();
        ids.retain(|&c| {
            let n = tree.node(c);
            self.filter.matches(&n.name, n.is_dir, n.size(mode))
        });
        ids.sort_by_key(|&c| std::cmp::Reverse(tree.node(c).size(mode)));
        let items: Vec<(&str, bool)> = ids
            .iter()
            .map(|&c| (tree.node(c).name.as_str(), tree.node(c).is_dir))
            .collect();
        let colors = theme::sibling_palette(&items);
        let flat = self.filter.is_active();
        let rows = tree_rows(&tree, &ids, &colors, &self.expanded, flat, mode);
        self.tree_rows = rows.iter().map(|r| r.id).collect();
        let scroll_to = std::mem::take(&mut self.tree_scroll);

        let (mut pick, mut enter, mut queue, mut toggle) = (None, None, None, None);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()))
            .show(ctx, |ui| {
                // Header.
                let (head, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
                ui.painter().rect_filled(head, 0.0, PANEL_2());
                let cols = TableCols::new(head);
                let hp = ui.painter();
                let f = egui::FontId::proportional(type_scale::CAPTION);
                for (x, label, right) in [
                    (cols.name, t("NAME"), false),
                    (cols.size_r, t("SIZE"), true),
                    (cols.share, t("SHARE"), false),
                    (cols.files_r, t("FILES"), true),
                ] {
                    let g = hp.layout_no_wrap(label.to_string(), f.clone(), MUTED());
                    let px = if right { x - g.size().x } else { x };
                    hp.galley(
                        egui::pos2(px, head.center().y - g.size().y / 2.0),
                        g,
                        MUTED(),
                    );
                }

                egui::ScrollArea::vertical()
                    .id_salt("table_rows")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        if rows.is_empty() {
                            empty_state(
                                ui,
                                "Nothing here",
                                "This folder is empty or the filter hides everything.",
                            );
                        }
                        for row in &rows {
                            let id = row.id;
                            let n = tree.node(id);
                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), TABLE_ROW_H),
                                Sense::CLICK,
                            );
                            if self.selected == Some(id) && scroll_to {
                                resp.scroll_to_me(None);
                            }
                            let out = paint_table_row(
                                ui,
                                rect,
                                &resp,
                                row,
                                n,
                                self.selected == Some(id),
                                self.review.contains(&id),
                            );
                            if out.check && !n.synthetic {
                                queue = Some(id);
                            } else if out.chevron {
                                toggle = Some(id);
                            } else if resp.clicked() {
                                pick = Some(id);
                            }
                            if resp.double_clicked() && !n.synthetic {
                                enter = Some(id);
                            }
                        }
                        if rows.len() >= MAX_TREE_ROWS {
                            ui.add_space(S2);
                            ui.horizontal(|ui| {
                                ui.add_space(S4);
                                ui.label(caption("Collapse folders to see the rest"));
                            });
                        }
                    });
            });
        if let Some(id) = toggle {
            if !self.expanded.remove(&id) {
                if ctx.input(|i| i.modifiers.alt) {
                    expand_all(&tree, id, &mut self.expanded);
                } else {
                    self.expanded.insert(id);
                }
            }
        }
        if let Some(id) = pick {
            self.selected = Some(id);
        }
        if let Some(id) = queue {
            self.queue_toggle(&[id]);
        }
        if let Some(id) = enter {
            if tree.node(id).is_dir {
                self.navigate(id, true);
            } else {
                self.selected = Some(id);
                self.reveal_selected();
            }
        }
    }
}

struct TableCols {
    left: f32,
    name: f32,
    size_r: f32,
    share: f32,
    files_r: f32,
}

impl TableCols {
    fn new(r: egui::Rect) -> Self {
        let files_r = r.right() - S4;
        let share = files_r - 90.0 - 150.0;
        let size_r = share - 16.0;
        Self {
            left: r.left() + S4,
            name: r.left() + S4 + 28.0,
            size_r: size_r.max(r.left() + 300.0),
            share,
            files_r,
        }
    }
}

struct TableOutcome {
    check: bool,
    chevron: bool,
}

/// ☐ · indent · chevron · swatch · name | size | share bar % | files.
fn paint_table_row(
    ui: &egui::Ui,
    rect: egui::Rect,
    resp: &egui::Response,
    row: &TreeRow,
    n: &space_core::Node,
    selected: bool,
    queued: bool,
) -> TableOutcome {
    let p = ui.painter();
    let cols = TableCols::new(rect);
    if selected {
        p.rect_filled(rect, 0.0, PANEL_2());
        p.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            0.0,
            ACCENT(),
        );
    } else if queued {
        p.rect_filled(rect, 0.0, ACCENT().gamma_multiply(0.08));
    } else if resp.hovered() {
        p.rect_filled(rect, 0.0, PANEL_2().gamma_multiply(0.6));
    }
    p.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0_f32, BORDER().gamma_multiply(0.5)),
    );
    let cy = rect.center().y;
    let hit = |r: egui::Rect| {
        resp.clicked()
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|h| r.expand(4.0).contains(h))
    };

    // Checkbox = in review.
    let cb = egui::Rect::from_center_size(egui::pos2(cols.left + 7.0, cy), egui::vec2(14.0, 14.0));
    if !n.synthetic {
        if queued {
            p.rect_filled(cb, 3.0, ACCENT());
            icons::paint(p, Icon::Check, cb.shrink(2.0), ACCENT_INK());
        } else {
            p.rect_stroke(
                cb,
                3.0,
                egui::Stroke::new(1.5_f32, MUTED()),
                egui::StrokeKind::Inside,
            );
        }
    }

    let x0 = cols.name + row.depth as f32 * TREE_INDENT;
    let chev = egui::Rect::from_center_size(egui::pos2(x0 + 6.0, cy), egui::vec2(14.0, 14.0));
    if row.has_kids {
        icons::paint(
            p,
            if row.open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            chev.shrink(2.0),
            MUTED(),
        );
    }
    p.rect_filled(
        egui::Rect::from_center_size(egui::pos2(x0 + 22.0, cy), egui::vec2(12.0, 10.0)),
        2.0,
        row.color,
    );

    let mono = egui::FontId::monospace(type_scale::MONO - 1.0);
    let size_g = p.layout_no_wrap(crate::i18n::bytes(row.size), mono.clone(), TEXT());
    let name_w = (cols.size_r - size_g.size().x - 16.0 - (x0 + 34.0)).max(20.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        n.name.clone(),
        egui::FontId::proportional(type_scale::BODY - 1.0),
        if n.synthetic { MUTED() } else { TEXT() },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(name_w);
    let g = p.layout_job(job);
    p.galley(egui::pos2(x0 + 34.0, cy - g.size().y / 2.0), g, TEXT());
    p.galley(
        egui::pos2(cols.size_r - size_g.size().x, cy - size_g.size().y / 2.0),
        size_g,
        TEXT(),
    );

    let track = egui::Rect::from_min_size(egui::pos2(cols.share, cy - 4.0), egui::vec2(90.0, 8.0));
    p.rect_filled(track, 0.0, BORDER());
    let mut fill = track;
    fill.set_width(90.0 * row.share.clamp(0.0, 1.0));
    p.rect_filled(fill, 0.0, row.color);
    let pct = (row.share * 100.0).round() as u32;
    let pct_g = p.layout_no_wrap(
        if pct == 0 {
            "<1%".into()
        } else {
            format!("{pct}%")
        },
        mono.clone(),
        MUTED(),
    );
    p.galley(
        egui::pos2(track.right() + 8.0, cy - pct_g.size().y / 2.0),
        pct_g,
        MUTED(),
    );

    let files_g = p.layout_no_wrap(n.file_count.to_string(), mono, MUTED());
    p.galley(
        egui::pos2(cols.files_r - files_g.size().x, cy - files_g.size().y / 2.0),
        files_g,
        MUTED(),
    );

    TableOutcome {
        check: hit(cb),
        chevron: row.has_kids && hit(chev),
    }
}
