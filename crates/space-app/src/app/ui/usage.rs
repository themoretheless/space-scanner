//! Disk usage workspace: the size-ranked contents list is the dependable way to
//! navigate, the map explains proportions, details appear only on selection.

use crate::i18n::t;
use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense};
use space_core::{NodeId, SizeMode, SpaceTree as Tree};

use super::Density;
use crate::app::SpaceApp;
use crate::icons::{self, Icon};
use crate::settings::WorkspaceLayout;
use crate::theme::metrics::{S2, S3, S4, SIDE_W, TREE_INDENT, TREE_ROW_H, TREE_W};
use crate::theme::type_scale;
use crate::theme::{self, ACCENT, BG, BORDER, MUTED, PANEL, PANEL_2, TEXT};
use crate::views::{self, DrawCtx, ViewMode};
use crate::widgets::{
    block, caption, divider, empty_state, ghost, primary, secondary, segmented, stat,
};

impl SpaceApp {
    pub(super) fn usage_page(&mut self, ctx: &egui::Context, d: Density) {
        if self.tree.is_none() {
            self.scan_state_page(ctx);
            return;
        }
        // Toolbar, filters and footer are the same in every layout.
        self.workspace_bar(ctx, d);
        self.filter_bar(ctx, d);
        self.workspace_footer(ctx);
        match self.settings.layout {
            WorkspaceLayout::Columns => return self.columns_page(ctx, d),
            WorkspaceLayout::Table => return self.table_page(ctx, d),
            WorkspaceLayout::Split => {}
        }
        let split = d != Density::Narrow;
        if split {
            egui::SidePanel::left("contents")
                .show_separator_line(false)
                .exact_width(if d == Density::Medium { 320.0 } else { TREE_W })
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(PANEL())
                        .stroke(egui::Stroke::new(1.0_f32, BORDER())),
                )
                .show(ctx, |ui| self.contents(ui));
        }
        if self.selected.is_some() {
            if d == Density::Wide {
                egui::SidePanel::right("details")
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
            } else {
                self.details_drawer(ctx);
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()))
            .show(ctx, |ui| {
                if !split && !self.compact_map {
                    egui::Frame::new().fill(PANEL()).show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        self.contents(ui);
                    });
                } else {
                    self.map_area(ctx, ui);
                }
            });
    }

    /// Details without a side panel: a drawer on the right at medium widths,
    /// a bottom sheet over a scrim when the window is narrow. Esc closes both.
    pub(super) fn details_drawer(&mut self, ctx: &egui::Context) {
        if self.selected.is_none() {
            return;
        }
        let screen = ctx.screen_rect();
        let left = ctx.available_rect().left();
        let narrow = Density::of(screen.width()) == Density::Narrow;
        if narrow {
            let mut close = false;
            let scrim = egui::Rect::from_min_max(egui::pos2(left, screen.top()), screen.max);
            egui::Area::new(egui::Id::new("details_scrim"))
                .order(egui::Order::Middle)
                .fixed_pos(scrim.min)
                .show(ctx, |ui| {
                    let (r, resp) = ui.allocate_exact_size(scrim.size(), Sense::click());
                    ui.painter().rect_filled(r, 0.0, crate::theme::SCRIM());
                    close = resp.clicked();
                });
            let h = (screen.height() * 0.55).clamp(220.0, 420.0);
            egui::Area::new(egui::Id::new("details_sheet"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(left, screen.bottom() - h))
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(ctx.style().visuals.window_fill)
                        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                        .corner_radius(egui::CornerRadius {
                            nw: 14,
                            ne: 14,
                            sw: 0,
                            se: 0,
                        })
                        .inner_margin(egui::Margin::symmetric(20, 14))
                        .shadow(ctx.style().visuals.window_shadow)
                        .show(ui, |ui| {
                            ui.set_width(screen.right() - left - 40.0);
                            ui.set_height(h - 28.0);
                            egui::ScrollArea::vertical().show(ui, |ui| self.details(ui));
                        });
                });
            if close {
                self.selected = None;
            }
            return;
        }
        let w = (screen.width() - 2.0 * S3).clamp(240.0, 320.0);
        let top = ctx.available_rect().top() + S3;
        egui::Area::new(egui::Id::new("details_drawer"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(screen.right() - w - S3, top))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(ctx.style().visuals.window_fill)
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .corner_radius(crate::theme::metrics::RADIUS_CARD)
                    .inner_margin(egui::Margin::same(S4 as i8 + 2))
                    .shadow(ctx.style().visuals.window_shadow)
                    .show(ui, |ui| {
                        ui.set_width(w - 36.0);
                        egui::ScrollArea::vertical()
                            .max_height((screen.bottom() - top - S3 - 70.0).max(160.0))
                            .show(ui, |ui| self.details(ui));
                    });
            });
    }

    // ---------------------------------------------------------------- contents

    fn contents(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        self.schedule_largest();
        let focus = self.focus;
        let mode = self.size_mode;
        let folder_size = tree.node(focus).size(mode);

        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 16,
                right: 16,
                top: 14,
                bottom: 10,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let title = if self.search_hits.is_some() {
                        "Matches"
                    } else {
                        "Contents"
                    };
                    ui.label(crate::widgets::strong(title, type_scale::LEAD));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(crate::i18n::bytes(folder_size))
                                .size(type_scale::MONO)
                                .monospace()
                                .color(MUTED()),
                        );
                    });
                });
                ui.add_space(S2);
                if self.search_hits.is_none() {
                    if let Some(i) = segmented(
                        ui,
                        &["Folder contents", "Largest files"],
                        self.show_largest as usize,
                    ) {
                        self.show_largest = i == 1;
                    }
                }
            });
        divider(ui);

        let everywhere = self.search_hits.as_ref().map(|h| h.ids.clone());
        let mut ids: Vec<NodeId> = if let Some(hits) = &everywhere {
            hits.clone()
        } else if self.show_largest {
            self.largest.rows().iter().map(|row| row.0).collect()
        } else {
            tree.children(focus).to_vec()
        };
        let before = ids.len();
        ids.retain(|&c| {
            let n = tree.node(c);
            self.filter.matches(&n.name, n.is_dir, n.size(mode))
        });
        ids.sort_by_key(|&c| std::cmp::Reverse(tree.node(c).size(mode)));

        if self.show_largest && everywhere.is_none() {
            ui.horizontal(|ui| {
                ui.add_space(S4);
                if self.largest.is_loading() {
                    ui.spinner();
                }
                ui.label(caption("The 100 largest files found under this folder"));
            });
        }

        let items: Vec<(&str, bool)> = ids
            .iter()
            .map(|&c| {
                let n = tree.node(c);
                (n.name.as_str(), n.is_dir)
            })
            .collect();
        let colors = theme::sibling_palette(&items);

        // A flat list while filtering or browsing largest files; otherwise a
        // tree rooted at the focus, with folders opened inline.
        let flat = self.show_largest || self.filter.is_active() || everywhere.is_some();
        if !flat {
            self.reveal_in_tree(&tree);
        }
        let mut rows = tree_rows(&tree, &ids, &colors, &self.expanded, flat, mode);
        if everywhere.is_some() || self.show_largest {
            // Flat results come from many folders: say which one.
            for row in &mut rows {
                if let Some(parent) = tree.node(row.id).parent {
                    row.note = Some(short_path(&tree, parent));
                }
            }
        }
        self.tree_rows = rows.iter().map(|r| r.id).collect();
        let scroll_to = std::mem::take(&mut self.tree_scroll);

        let (mut pick, mut enter, mut reveal, mut queue, mut toggle) =
            (None, None, None, None::<Vec<NodeId>>, None);
        egui::ScrollArea::vertical()
            .id_salt("contents_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(4.0);
                if ids.is_empty() {
                    if self.show_largest && self.largest.is_loading() {
                        ui.add_space(S4);
                        ui.vertical_centered(|ui| ui.spinner());
                    } else if everywhere.is_some() {
                        empty_state(
                            ui,
                            "Nothing matches",
                            "No item in the whole scan has this name.",
                        );
                    } else {
                        empty_state(
                            ui,
                            if before == 0 {
                                "This folder is empty"
                            } else {
                                "Nothing matches"
                            },
                            if before == 0 {
                                "No indexed items here."
                            } else {
                                "Change or clear the filter."
                            },
                        );
                    }
                }
                for row in &rows {
                    let id = row.id;
                    let n = tree.node(id);
                    let selected = self.selected == Some(id);
                    let queued = self.review.contains(&id);
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), TREE_ROW_H),
                        Sense::CLICK,
                    );
                    if selected && scroll_to {
                        resp.scroll_to_me(None);
                    }
                    let outcome = paint_tree_row(
                        ui,
                        rect,
                        &resp,
                        row,
                        n,
                        selected,
                        queued,
                        self.hovered == Some(id),
                    );
                    if outcome.chevron {
                        toggle = Some(id);
                    }
                    if outcome.check {
                        queue = Some(vec![id]);
                    }
                    if resp.clicked() && !outcome.chevron && !outcome.check {
                        let mods = ui.input(|i| i.modifiers);
                        if mods.shift && !n.synthetic {
                            // Shift-click adds the whole range to the Trash list.
                            let from = self
                                .range_anchor
                                .and_then(|a| self.tree_rows.iter().position(|&r| r == a));
                            let to = self.tree_rows.iter().position(|&r| r == id);
                            if let (Some(a), Some(b)) = (from, to) {
                                let (lo, hi) = (a.min(b), a.max(b));
                                let range: Vec<NodeId> = self.tree_rows[lo..=hi]
                                    .iter()
                                    .copied()
                                    .filter(|r| !self.review.contains(r))
                                    .collect();
                                queue = Some(range);
                            } else {
                                queue = Some(vec![id]);
                            }
                        } else if mods.command && !n.synthetic {
                            queue = Some(vec![id]);
                        }
                        self.range_anchor = Some(id);
                        pick = Some(id);
                    }
                    if resp.double_clicked() {
                        if n.is_dir && !n.synthetic {
                            enter = Some(id);
                        } else {
                            reveal = Some(id);
                        }
                    }
                    resp.context_menu(|ui| {
                        if row.has_kids
                            && ui
                                .button(t(if row.open { "Collapse" } else { "Expand" }))
                                .clicked()
                        {
                            toggle = Some(id);
                            ui.close_menu();
                        }
                        if n.is_dir && !n.synthetic && ui.button(t("Open folder")).clicked() {
                            enter = Some(id);
                            ui.close_menu();
                        }
                        if !n.synthetic {
                            let q = if queued {
                                "Take out of Trash list"
                            } else {
                                "Add to Trash list"
                            };
                            if ui.button(t(q)).clicked() {
                                queue = Some(vec![id]);
                                ui.close_menu();
                            }
                            if ui.button(t("Show in Finder")).clicked() {
                                reveal = Some(id);
                                ui.close_menu();
                            }
                        }
                    });
                }
                if rows.len() >= MAX_TREE_ROWS {
                    ui.add_space(S2);
                    ui.horizontal(|ui| {
                        ui.add_space(S4);
                        ui.label(caption("Collapse folders to see the rest"));
                    });
                }
            });
        if let Some(id) = toggle {
            if !self.expanded.remove(&id) {
                if ui.input(|i| i.modifiers.alt) {
                    expand_all(&tree, id, &mut self.expanded);
                } else {
                    self.expanded.insert(id);
                }
            }
        }
        if let Some(id) = pick {
            self.selected = Some(id);
        }
        if let Some(ids) = queue {
            self.queue_toggle(&ids);
        }
        if let Some(id) = reveal {
            self.selected = Some(id);
            self.reveal_selected();
        }
        if let Some(id) = enter {
            self.navigate(id, true);
        }
    }

    // ----------------------------------------------------------------- details

    pub(super) fn details(&mut self, ui: &mut egui::Ui) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let Some(id) = self.selected else {
            return;
        };
        let Some(node) = tree.try_node(id) else {
            self.selected = None;
            return;
        };
        let size = node.size(self.size_mode);
        let path = tree.path_string(id);
        let queued = self.review.contains(&id);
        let protected = space_clean::safety::is_never_touch(std::path::Path::new(&path));

        ui.horizontal(|ui| {
            icons::show(
                ui,
                if node.is_dir {
                    Icon::Folder
                } else {
                    Icon::File
                },
                16.0,
                MUTED(),
            );
            ui.label(crate::widgets::section_label(if node.synthetic {
                "Small files"
            } else if node.is_dir {
                "Folder"
            } else {
                "File"
            }));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icons::button(ui, Icon::Close, 28.0, "Close details · Esc", true).clicked() {
                    self.selected = None;
                }
            });
        });
        ui.add(egui::Label::new(crate::widgets::strong(node.name.clone(), 20.0)).truncate())
            .on_hover_text(&node.name);
        ui.add(
            egui::Label::new(
                RichText::new(&path)
                    .size(type_scale::CAPTION)
                    .monospace()
                    .color(MUTED()),
            )
            .wrap(),
        );
        ui.add_space(S3);

        ui.columns(2, |cols| {
            stat(
                &mut cols[0],
                &crate::i18n::bytes(size),
                &match (self.percent_of_parent(id), node.parent) {
                    (Some(p), Some(parent)) => {
                        crate::tr!("{}% of {}", format!("{p:.0}"), tree.node(parent).name)
                    }
                    _ => t("whole scan").to_string(),
                },
            );
            stat(
                &mut cols[1],
                &node.file_count.to_string(),
                t(if node.is_dir { "files inside" } else { "file" }),
            );
        });
        ui.add_space(S3);

        if protected {
            note_card(
                ui,
                crate::theme::DANGER(),
                Icon::Lock,
                "Protected",
                "The system or your personal data lives here. It cannot go to the Trash list.",
            );
            ui.add_space(S3);
        } else if let Some((category, label, color, hint)) =
            self.junk.risk_hint(&PathBuf::from(&path))
        {
            note_card(
                ui,
                color,
                Icon::Check,
                &format!("{} · {}", t(label), t(&category)),
                &crate::tr!("{}. A rule match is guidance, not a guarantee.", t(hint)),
            );
            ui.add_space(S3);
        }

        let is_dir = node.is_dir && !node.synthetic;
        let listable = !node.synthetic && node.parent.is_some() && !protected;
        if is_dir {
            if ui.add(block(ui, primary("Open folder"))).clicked() {
                self.navigate(id, true);
            }
        } else if ui.add(block(ui, primary("Show in Finder"))).clicked() {
            self.reveal_selected();
        }
        ui.add_space(S2);
        let label = if queued {
            "Take out of Trash list"
        } else {
            "Add to Trash list"
        };
        if ui
            .add_enabled(listable && !self.busy, block(ui, secondary(label)))
            .on_hover_text(t("Nothing moves until you confirm on the Trash list"))
            .on_disabled_hover_text(t(if node.synthetic {
                "Grouped small files can't be listed one by one"
            } else {
                "This item can't go to the Trash"
            }))
            .clicked()
        {
            self.queue_toggle(&[id]);
        }
        ui.add_space(S2);
        let mut copy = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            if is_dir && ui.add(ghost("Show in Finder")).clicked() {
                self.reveal_selected();
            }
            if is_dir
                && ui
                    .add_enabled(self.scan.is_none() && !self.busy, ghost("Rescan"))
                    .on_hover_text(t("Scan this folder again · ⌘⇧R"))
                    .clicked()
            {
                self.incremental_rescan(id);
            }
            if ui.add(ghost("Copy path")).on_hover_text("⌘⇧C").clicked() {
                copy = true;
            }
        });
        if copy {
            self.copy_text(path);
        }
        ui.add_space(S3);
        ui.label(caption(if is_dir {
            "Enter opens · Space adds to the Trash list · Shift-click adds a range"
        } else {
            "Space adds to the Trash list · Shift-click adds a range"
        }));
    }

    // --------------------------------------------------------------------- map

    fn map_area(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        self.map_controls(ui);
        let map = ui.available_rect_before_wrap();
        let fhash = self.filter.hash64();
        let review_sig = views::review_sig(&self.review);

        if let Some(tr) = &self.transition {
            if tr.done() {
                self.transition = None;
            } else {
                ctx.request_repaint();
            }
        }
        let (opacity, scale) = self
            .transition
            .as_ref()
            .map(|t| (t.opacity(), t.scale()))
            .unwrap_or((1.0, 1.0));
        let draw_rect = egui::Rect::from_center_size(map.center(), map.size() * scale);

        ui.set_opacity(opacity);
        let action = ui
            .allocate_new_ui(egui::UiBuilder::new().max_rect(draw_rect), |ui| {
                views::draw(
                    ui,
                    self.view,
                    &mut DrawCtx {
                        tree: &tree,
                        focus: self.focus,
                        selected: self.selected,
                        hovered: self.hovered,
                        review: &self.review,
                        mode: self.size_mode,
                        filter: &self.filter,
                        filter_hash: fhash,
                        review_sig,
                        geom: &mut self.geom,
                    },
                )
            })
            .inner;
        ui.set_opacity(1.0);

        if self.scan.is_some() || self.busy {
            ui.painter().rect_filled(map, 0.0, BG().gamma_multiply(0.6));
            ui.painter().text(
                map.center(),
                egui::Align2::CENTER_CENTER,
                &self.status,
                egui::FontId::proportional(type_scale::LEAD),
                TEXT(),
            );
        }

        self.map_hint(ctx, map);

        if self.hovered != action.hover {
            self.hovered = action.hover;
            ctx.request_repaint();
        }
        if let Some(id) = action.select {
            self.selected = Some(id);
        }
        if let Some(id) = action.navigate {
            self.navigate(id, true);
        }
        if let Some(id) = action.toggle_review {
            self.queue_toggle(&[id]);
        }
        if let Some(id) = action.reveal {
            self.selected = Some(id);
            self.reveal_selected();
        }
        if let Some(ext) = action.filter_ext {
            self.filter.ext = if self.filter.ext.as_deref() == Some(ext.as_str()) {
                None
            } else {
                Some(ext)
            };
            self.geom = None;
        }
        if action.clear_filter {
            self.filter.clear();
            self.filter_draft.clear();
            self.filter_edited_at = None;
            self.geom = None;
        }
    }

    /// Map style and size accounting, above the map and outside its drawing area.
    /// Segmented when there is room, compact pickers when the map is narrow.
    fn map_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let roomy = ui.available_width() >= 600.0;
        let mut view = self.view;
        let mut size_mode = self.size_mode;
        let size_label = |m: SizeMode| {
            if m == SizeMode::OnDisk {
                "On disk"
            } else {
                "File size"
            }
        };
        ui.horizontal(|ui| {
            ui.add_space(S2);
            if roomy {
                let labels: Vec<&str> = ViewMode::ALL.iter().map(|m| m.label()).collect();
                let current = ViewMode::ALL.iter().position(|&m| m == view).unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, current) {
                    view = ViewMode::ALL[i];
                }
            } else {
                egui::ComboBox::from_id_salt("map_view")
                    .selected_text(t(view.label()))
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for m in ViewMode::ALL {
                            ui.selectable_value(&mut view, m, t(m.label()));
                        }
                    });
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(S2);
                if roomy {
                    // Drawn right to left.
                    if let Some(i) = segmented(
                        ui,
                        &["File size", "On disk"],
                        (size_mode == SizeMode::OnDisk) as usize,
                    ) {
                        size_mode = if i == 1 {
                            SizeMode::OnDisk
                        } else {
                            SizeMode::Apparent
                        };
                    }
                } else {
                    egui::ComboBox::from_id_salt("map_size")
                        .selected_text(t(size_label(size_mode)))
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            for m in [SizeMode::OnDisk, SizeMode::Apparent] {
                                ui.selectable_value(&mut size_mode, m, t(size_label(m)));
                            }
                        });
                }
            });
        });
        if view != self.view {
            self.set_view(view);
        }
        if size_mode != self.size_mode {
            self.size_mode = size_mode;
            self.search_hits = None;
            self.invalidate_largest();
            self.schedule_largest();
        }
        ui.add_space(6.0);
    }

    /// One dismissible hint, bottom-centre.
    fn map_hint(&mut self, ctx: &egui::Context, map: egui::Rect) {
        if !self.settings.flags.show_shortcuts_hint
            || self.scan.is_some()
            || self.busy
            || self.hovered.is_some()
        {
            return;
        }
        let hint = t(self.view.click_hint());
        let text_w = ctx.fonts(|f| {
            f.layout_no_wrap(
                hint.to_owned(),
                egui::FontId::proportional(type_scale::CAPTION),
                Color32::WHITE,
            )
            .size()
            .x
        });
        // Needs the text, a close button and padding; otherwise stay out of the way.
        if text_w + 80.0 > map.width() {
            return;
        }
        let mut dismissed = false;
        egui::Area::new(egui::Id::new("map_hint"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(
                map.center().x - (text_w + 64.0) * 0.5,
                map.bottom() - 52.0,
            ))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL().gamma_multiply(0.92))
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .corner_radius(crate::theme::metrics::RADIUS_CONTROL)
                    .inner_margin(egui::Margin::symmetric(S4 as i8, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(caption(hint));
                            if icons::button(ui, Icon::Close, 24.0, "Hide this hint", true)
                                .clicked()
                            {
                                dismissed = true;
                            }
                        });
                    });
            });
        if dismissed {
            self.settings.flags.show_shortcuts_hint = false;
            self.settings.save();
        }
    }
}

// -------------------------------------------------------------------- tree

/// Hard cap on drawn rows: an opened `node_modules` must not stall a frame.
pub(super) const MAX_TREE_ROWS: usize = 2_000;
/// Children shown per folder before the rest fold into "N more".
const MAX_KIDS: usize = 200;

pub(super) struct TreeRow {
    pub(super) id: NodeId,
    pub(super) depth: usize,
    pub(super) color: Color32,
    pub(super) size: u64,
    /// Share of the parent folder, 0..=1.
    pub(super) share: f32,
    pub(super) has_kids: bool,
    pub(super) open: bool,
    /// Where a flat result lives, drawn after the name.
    pub(super) note: Option<String>,
    /// For each ancestor level: does a later sibling follow (draw a guide)?
    guides: Vec<bool>,
}

pub(super) fn tree_rows(
    tree: &Tree,
    top: &[NodeId],
    colors: &[Color32],
    expanded: &HashSet<NodeId>,
    flat: bool,
    mode: SizeMode,
) -> Vec<TreeRow> {
    let mut out = Vec::new();
    let parent_total = |id: NodeId| {
        tree.node(id)
            .parent
            .map(|p| tree.node(p).size(mode))
            .unwrap_or(0)
            .max(1) as f32
    };
    fn walk(
        tree: &Tree,
        id: NodeId,
        depth: usize,
        color: Color32,
        guides: Vec<bool>,
        expanded: &HashSet<NodeId>,
        mode: SizeMode,
        parent_total: &dyn Fn(NodeId) -> f32,
        out: &mut Vec<TreeRow>,
    ) {
        if out.len() >= MAX_TREE_ROWS {
            return;
        }
        let n = tree.node(id);
        let kids = if n.is_dir && !n.synthetic {
            tree.children(id)
        } else {
            &[]
        };
        let open = !kids.is_empty() && expanded.contains(&id);
        out.push(TreeRow {
            id,
            depth,
            color,
            size: n.size(mode),
            share: n.size(mode) as f32 / parent_total(id),
            has_kids: !kids.is_empty(),
            open,
            note: None,
            guides: guides.clone(),
        });
        if open {
            let mut kids = kids.to_vec();
            kids.sort_by_key(|&c| std::cmp::Reverse(tree.node(c).size(mode)));
            kids.truncate(MAX_KIDS);
            let last = kids.len().saturating_sub(1);
            for (i, k) in kids.into_iter().enumerate() {
                let mut g = guides.clone();
                g.push(i != last);
                let c = if tree.node(k).synthetic {
                    MUTED()
                } else {
                    color
                };
                walk(tree, k, depth + 1, c, g, expanded, mode, parent_total, out);
            }
        }
    }
    for (i, &id) in top.iter().enumerate() {
        let color = if tree.node(id).synthetic {
            MUTED()
        } else {
            colors[i]
        };
        if flat {
            out.push(TreeRow {
                id,
                depth: 0,
                color,
                size: tree.node(id).size(mode),
                share: tree.node(id).size(mode) as f32 / parent_total(id),
                has_kids: false,
                open: false,
                note: None,
                guides: Vec::new(),
            });
        } else {
            walk(
                tree,
                id,
                0,
                color,
                Vec::new(),
                expanded,
                mode,
                &parent_total,
                &mut out,
            );
        }
        if out.len() >= MAX_TREE_ROWS {
            break;
        }
    }
    out
}

pub(super) fn expand_all(tree: &Tree, id: NodeId, expanded: &mut HashSet<NodeId>) {
    let mut stack = vec![id];
    let mut budget = 500;
    while let Some(x) = stack.pop() {
        if budget == 0 {
            break;
        }
        let n = tree.node(x);
        if n.is_dir && !n.synthetic && !tree.children(x).is_empty() {
            expanded.insert(x);
            budget -= 1;
            stack.extend_from_slice(tree.children(x));
        }
    }
}

struct RowOutcome {
    chevron: bool,
    /// The Trash list checkbox was clicked.
    check: bool,
}

/// One tree row: guides · chevron · swatch · name · share bar + % · size · review mark.
#[allow(clippy::too_many_arguments)]
fn paint_tree_row(
    ui: &egui::Ui,
    rect: egui::Rect,
    resp: &egui::Response,
    row: &TreeRow,
    n: &space_core::Node,
    selected: bool,
    queued: bool,
    linked_hover: bool,
) -> RowOutcome {
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 0.0, PANEL_2());
        p.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
            0.0,
            ACCENT(),
        );
    } else if resp.hovered() || linked_hover {
        p.rect_filled(rect, 0.0, PANEL_2().gamma_multiply(0.6));
    }
    let cy = rect.center().y;
    // Trash list checkbox, in the same place on every row.
    let mut check = false;
    if !n.synthetic && n.parent.is_some() {
        let cb = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 18.0, cy),
            egui::vec2(14.0, 14.0),
        );
        let hot = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|h| cb.expand(5.0).contains(h));
        let state = if queued {
            crate::widgets::Check::On
        } else {
            crate::widgets::Check::Off
        };
        if queued || hot || resp.hovered() || selected {
            crate::widgets::paint_check(p, cb, state);
        }
        check = resp.clicked()
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|h| cb.expand(5.0).contains(h));
    }
    let left = rect.left() + 32.0;
    // Indent guides: a hairline per ancestor level that still has siblings below.
    for (lvl, &more) in row.guides.iter().enumerate() {
        let x = left + lvl as f32 * TREE_INDENT + 8.0;
        let last = lvl + 1 == row.guides.len();
        if more || last {
            let bottom = if more { rect.bottom() } else { cy };
            p.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, bottom)],
                egui::Stroke::new(1.0_f32, BORDER()),
            );
        }
    }
    let x0 = left + row.depth as f32 * TREE_INDENT;
    let chev = egui::Rect::from_center_size(egui::pos2(x0 + 8.0, cy), egui::vec2(16.0, 16.0));
    let mut chevron = false;
    if row.has_kids {
        let hot = resp.hovered()
            && ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|h| chev.expand(4.0).contains(h));
        icons::paint(
            p,
            if row.open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            chev.shrink(2.0),
            if hot { TEXT() } else { MUTED() },
        );
        chevron = resp.clicked()
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|h| chev.expand(4.0).contains(h));
    }
    p.rect_filled(
        egui::Rect::from_center_size(egui::pos2(x0 + 24.0, cy), egui::vec2(10.0, 10.0)),
        3.0,
        row.color,
    );

    let pad = S4;
    let size_g = p.layout_no_wrap(
        crate::i18n::bytes(row.size),
        egui::FontId::monospace(type_scale::MONO),
        if queued {
            ACCENT()
        } else if selected {
            TEXT()
        } else {
            MUTED()
        },
    );
    let size_x = rect.right() - pad - size_g.size().x;
    p.galley(
        egui::pos2(size_x, cy - size_g.size().y / 2.0),
        size_g,
        MUTED(),
    );

    // Reserve the measured widths of both numbers, including 100% and TiB sizes.
    let pct = (row.share * 100.0).round().clamp(0.0, 100.0) as u32;
    let pct_g = p.layout_no_wrap(
        if pct == 0 {
            "<1%".into()
        } else {
            format!("{pct}%")
        },
        egui::FontId::monospace(type_scale::MONO - 1.0),
        MUTED(),
    );
    let pct_x = size_x - 10.0 - pct_g.size().x;
    let bar_w = 28.0;
    let bar_x = pct_x - 8.0 - bar_w;
    let track = egui::Rect::from_min_size(egui::pos2(bar_x, cy - 2.0), egui::vec2(bar_w, 4.0));
    p.rect_filled(track, 2.0, BORDER());
    let mut fill = track;
    fill.set_width(bar_w * row.share.clamp(0.0, 1.0));
    p.rect_filled(fill, 2.0, row.color);
    p.galley(egui::pos2(pct_x, cy - pct_g.size().y / 2.0), pct_g, MUTED());

    let name_x = x0 + 36.0;
    let name_w = (bar_x - 10.0 - name_x).max(20.0);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &n.name,
        0.0,
        egui::TextFormat::simple(
            egui::FontId::proportional(type_scale::BODY),
            if n.synthetic { MUTED() } else { TEXT() },
        ),
    );
    if let Some(note) = &row.note {
        job.append(
            note,
            8.0,
            egui::TextFormat::simple(egui::FontId::proportional(type_scale::CAPTION), MUTED()),
        );
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(name_w);
    let g = p.layout_job(job);
    p.with_clip_rect(egui::Rect::from_min_max(
        rect.min,
        egui::pos2((bar_x - 10.0).max(rect.left()), rect.bottom()),
    ))
    .galley(egui::pos2(name_x, cy - g.size().y / 2.0), g, TEXT());
    RowOutcome { chevron, check }
}

/// A folder path for a flat result row: "~/code/app".
pub(super) fn short_path(tree: &Tree, id: NodeId) -> String {
    let full = tree.path_string(id);
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        if let Some(rest) = full.strip_prefix(home.as_ref()) {
            return format!("~{rest}");
        }
    }
    full
}

/// A tinted note in the inspector: rule match, protection.
fn note_card(ui: &mut egui::Ui, color: Color32, icon: Icon, title: &str, body: &str) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(egui::Stroke::new(1.0_f32, color.gamma_multiply(0.5)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(S3 as i8, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                icons::show(ui, icon, 14.0, color);
                ui.label(
                    RichText::new(t(title))
                        .color(color)
                        .family(crate::theme::medium()),
                );
            });
            ui.add(egui::Label::new(caption(body.to_owned())).wrap());
        });
}

impl SpaceApp {
    /// Open every ancestor of a selection made elsewhere (map, search) so the
    /// tree shows it.
    fn reveal_in_tree(&mut self, tree: &Tree) {
        if self.selected == self.tree_revealed {
            return;
        }
        self.tree_revealed = self.selected;
        let Some(sel) = self.selected else { return };
        let path = tree.path_to(sel);
        if let Some(at) = path.iter().position(|&x| x == self.focus) {
            for &a in &path[at + 1..path.len().saturating_sub(1)] {
                self.expanded.insert(a);
            }
            self.tree_scroll = true;
        }
    }

    /// Arrow keys walk the visible tree; Space toggles review. Returns true
    /// when the key was consumed.
    pub(in crate::app) fn tree_keys(
        &mut self,
        up: bool,
        down: bool,
        left: bool,
        right: bool,
        space: bool,
    ) -> bool {
        let Some(tree) = self.tree.clone() else {
            return false;
        };
        if self.tree_rows.is_empty() || !(up || down || left || right || space) {
            return false;
        }
        let pos = self
            .selected
            .and_then(|s| self.tree_rows.iter().position(|&r| r == s));
        let Some(i) = pos else {
            if up || down {
                self.selected = self.tree_rows.first().copied();
                self.tree_revealed = self.selected;
                self.tree_scroll = true;
                return true;
            }
            return false;
        };
        let id = self.tree_rows[i];
        let n = tree.node(id);
        let can_open = n.is_dir && !n.synthetic && !tree.children(id).is_empty();
        let mut next = None;
        if up && i > 0 {
            next = Some(self.tree_rows[i - 1]);
        } else if down && i + 1 < self.tree_rows.len() {
            next = Some(self.tree_rows[i + 1]);
        } else if right && can_open {
            if !self.expanded.insert(id) {
                next = self.tree_rows.get(i + 1).copied();
            }
        } else if left {
            if !self.expanded.remove(&id) {
                next = n.parent.filter(|p| self.tree_rows.contains(p));
            }
        } else if space && !n.synthetic {
            self.queue_toggle(&[id]);
        } else if up || down {
            // At an end of the list: consume so the map does not jump.
        } else {
            return left || right;
        }
        if let Some(x) = next {
            self.selected = Some(x);
            self.tree_revealed = Some(x);
            self.tree_scroll = true;
        }
        true
    }
}
