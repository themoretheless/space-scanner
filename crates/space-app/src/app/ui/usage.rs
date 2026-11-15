//! Disk usage workspace: the size-ranked contents list is the dependable way to
//! navigate, the map explains proportions, details appear only on selection.

use crate::i18n::t;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense};
use space_core::{NodeId, SizeMode, SpaceTree as Tree};

use super::Density;
use crate::app::SpaceApp;
use crate::icons::{self, Icon};
use crate::review;
use crate::theme::metrics::{APP_BAR_H, S2, S3, S4, SIDE_W, TREE_INDENT, TREE_ROW_H, TREE_W};
use crate::theme::type_scale;
use crate::theme::{self, ACCENT, BG, BORDER, MUTED, PANEL, PANEL_2, TEXT};
use crate::views::{self, DrawCtx, ViewMode};
use crate::widgets::{
    badge, block, caption, divider, empty_state, ghost, paint_checkbox, primary, secondary,
    segmented, stat, text_width, CheckVisual,
};

impl SpaceApp {
    pub(super) fn usage_page(&mut self, ctx: &egui::Context, d: Density) {
        if self.choosing_target || (self.tree.is_none() && self.scan.is_none() && !self.busy) {
            self.scan_page(ctx);
            return;
        }
        if self.tree.is_none() {
            egui::CentralPanel::default().show(ctx, |ui| self.scan_wait(ui));
            return;
        }
        let split = d != Density::Narrow;
        if split {
            egui::SidePanel::left("contents")
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

    pub(super) fn details_drawer(&mut self, ctx: &egui::Context) {
        if self.selected.is_none() {
            return;
        }
        let screen = ctx.screen_rect();
        let w = (screen.width() - 2.0 * S3).clamp(240.0, 340.0);
        let top = APP_BAR_H + 48.0 + S3;
        egui::Area::new(egui::Id::new("details_drawer"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(screen.right() - w - S3, top))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL())
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::same(S4 as i8 + 2))
                    .shadow(egui::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: Color32::from_black_alpha(110),
                    })
                    .show(ui, |ui| {
                        ui.set_width(w - 36.0);
                        egui::ScrollArea::vertical()
                            .max_height((screen.height() - top - S3 - 36.0).max(160.0))
                            .show(ui, |ui| self.details(ui));
                    });
            });
    }

    /// While the first scan runs there is nothing to explore yet.
    pub(super) fn scan_wait(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(72.0);
            ui.spinner();
            ui.add_space(S3);
            ui.label(
                RichText::new(&self.status)
                    .size(type_scale::LEAD)
                    .color(TEXT()),
            );
            if let Some(p) = &self.progress {
                ui.label(caption(crate::tr!(
                    "{} files · {} so far",
                    p.files_seen,
                    crate::i18n::bytes(p.bytes_seen)
                )));
            }
            ui.add_space(S4);
            if ui
                .add(secondary("Cancel scan").min_size(egui::vec2(160.0, 36.0)))
                .clicked()
            {
                if let Some(s) = &self.scan {
                    s.cancel();
                }
            }
            ui.add_space(S2);
            ui.label(caption(
                "Wrong folder? Cancel, then choose another folder or disk.",
            ));
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
        let Some(focus_node) = tree.try_node(focus) else {
            return;
        };
        let folder_size = focus_node.size(mode);

        egui::TopBottomPanel::bottom("contents_footer")
            .frame(
                egui::Frame::new()
                    .fill(PANEL())
                    .inner_margin(egui::Margin::symmetric(S4 as i8, S3 as i8))
                    .stroke(egui::Stroke::new(1.0_f32, BORDER())),
            )
            .show_inside(ui, |ui| {
                if self.from_cache {
                    ui.horizontal_wrapped(|ui| {
                        if self.cache_offline {
                            badge(
                                ui,
                                "Offline disk",
                                crate::theme::MUTED(),
                                "This volume is not currently mounted — showing the last saved scan.",
                            );
                        } else if self.cache_stale {
                            badge(
                                ui,
                                "Out of date",
                                crate::theme::DANGER(),
                                "Saved result is older than your cache limit — files may have changed. Rescan to refresh (⌘R).",
                            );
                        } else {
                            badge(
                                ui,
                                "Saved result",
                                ACCENT(),
                                "Opened from cache — files may have changed. Rescan to refresh (⌘R).",
                            );
                        }
                        if !self.cache_offline
                            && !self.busy
                            && self.scan.is_none()
                            && ui
                                .add(crate::widgets::ghost("Rescan"))
                                .on_hover_text(t("Scan again now (⌘R)"))
                                .clicked()
                        {
                            self.rescan_current();
                        }
                    });
                    ui.add_space(4.0);
                }
                ui.add(egui::Label::new(caption(crate::tr!(
                    "Indexed {} items · {}",
                    tree.item_count,
                    crate::i18n::bytes(tree.total_size_mode(mode))
                )))
                .truncate());
                if let Some(gap) = self.scan_gap_label() {
                    if ui
                        .add(egui::Button::new(caption(gap).color(ACCENT())).frame(false))
                        .on_hover_text(t(
                            "Unindexed disk usage can include permissions, snapshots and filesystem accounting",
                        ))
                        .clicked()
                    {
                        self.open_tm_panel();
                    }
                }
                if !tree.skipped_roots.is_empty() {
                    ui.add_space(2.0);
                    let toggle_label = crate::tr!(
                        "Skipped / inaccessible ({})",
                        tree.skipped_roots.len()
                    );
                    let arrow = if self.skipped_expanded { "▾" } else { "▸" };
                    let resp = ui
                        .add(
                            egui::Button::new(
                                caption(format!("{arrow} {toggle_label}"))
                                    .color(crate::theme::DANGER()),
                            )
                            .frame(false)
                            .min_size(egui::vec2(24.0, 24.0)),
                        )
                        .on_hover_text(t(
                            "Folders that could not be opened due to permissions, exclusion rules, or filesystem boundaries.",
                        ));
                    if resp.has_focus() {
                        crate::theme::paint_focus_ring(ui.painter(), resp.rect);
                    }
                    if resp.clicked() {
                        self.skipped_expanded = !self.skipped_expanded;
                    }
                    if self.skipped_expanded {
                        egui::ScrollArea::vertical()
                            .id_salt("skipped_roots_list")
                            .max_height(110.0)
                            .show(ui, |ui| {
                                for path in &tree.skipped_roots {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(path)
                                                .monospace()
                                                .size(crate::theme::type_scale::CAPTION)
                                                .color(crate::theme::MUTED()),
                                        )
                                        .truncate(),
                                    );
                                }
                            });
                    }
                }
            });

        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 16,
                right: 16,
                top: 14,
                bottom: 10,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::Usage, 16.0, MUTED());
                    ui.label(RichText::new(t("Contents")).size(type_scale::LEAD).strong());
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
                if let Some(i) = segmented(
                    ui,
                    &["Folder contents", "Insights"],
                    self.show_largest as usize,
                ) {
                    self.show_largest = i == 1;
                }
                if self.show_largest {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for tab in crate::largest_files::InsightTab::ALL {
                            if ui
                                .selectable_label(self.insight_tab == tab, t(tab.label()))
                                .clicked()
                            {
                                self.insight_tab = tab;
                            }
                        }
                    });
                }
            });
        divider(ui);

        let mut ids: Vec<NodeId> = if self.show_largest {
            let data = self.largest.data();
            match self.insight_tab {
                crate::largest_files::InsightTab::Files => {
                    data.files.iter().map(|row| row.0).collect()
                }
                crate::largest_files::InsightTab::Folders => {
                    data.folders.iter().map(|row| row.0).collect()
                }
                crate::largest_files::InsightTab::Oldest => {
                    data.oldest.iter().map(|row| row.0).collect()
                }
                crate::largest_files::InsightTab::Duplicates => data
                    .duplicates
                    .iter()
                    .flat_map(|(_, _, nodes)| nodes.iter().copied())
                    .collect(),
                crate::largest_files::InsightTab::Types => Vec::new(),
            }
        } else {
            tree.children(focus).to_vec()
        };
        let before = ids.len();
        ids.retain(|&c| {
            let n = tree.node(c);
            self.filter.matches(&n.name, n.is_dir, n.size(mode))
        });
        if !self.show_largest || self.insight_tab != crate::largest_files::InsightTab::Oldest {
            ids.sort_by_key(|&c| std::cmp::Reverse(tree.node(c).size(mode)));
        }

        // Say what the filter is doing: scope, count, and a way out.
        if self.filter.is_active() || !self.filter_draft.is_empty() {
            egui::Frame::new()
                .fill(PANEL_2())
                .inner_margin(egui::Margin::symmetric(S4 as i8, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let scope = if self.show_largest {
                            t("largest files").to_string()
                        } else {
                            crate::tr!("in {}", focus_node.name)
                        };
                        ui.add(
                            egui::Label::new(caption(crate::tr!(
                                "{} of {} shown {}",
                                ids.len(),
                                before,
                                scope
                            )))
                            .truncate(),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(ghost("Clear").min_size(egui::vec2(0.0, 24.0)))
                                .clicked()
                            {
                                self.reset_filters();
                            }
                        });
                    });
                });
        }
        if self.show_largest {
            ui.horizontal(|ui| {
                ui.add_space(S4);
                if self.largest.is_loading() {
                    ui.spinner();
                }
                ui.label(caption("Up to 100 results · searches 8,000 folders"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(S2);
                    if ui
                        .add(ghost("Export CSV…").min_size(egui::vec2(0.0, 24.0)))
                        .clicked()
                    {
                        self.export_insights_csv(&ids);
                    }
                });
            });
            if self.insight_tab == crate::largest_files::InsightTab::Types {
                let types = self.largest.data().types.clone();
                egui::ScrollArea::vertical()
                    .id_salt("insight_types_list")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if types.is_empty() {
                            empty_state(
                                ui,
                                "No file types found",
                                "Select a folder with files to inspect extensions.",
                            );
                        }
                        for (ext, bytes, count) in types {
                            let is_active = self.filter.ext.as_deref().is_some_and(|e| e == ext);
                            let label = if ext.is_empty() {
                                t("(no ext)").to_string()
                            } else {
                                format!(".{ext}")
                            };
                            ui.horizontal(|ui| {
                                ui.add_space(S4);
                                if ui.selectable_label(is_active, &label).clicked() {
                                    self.filter.ext =
                                        if is_active { None } else { Some(ext.clone()) };
                                    self.geom = None;
                                }
                                ui.label(caption(crate::tr!("{} files", count)));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.add_space(S4);
                                    ui.label(
                                        RichText::new(crate::i18n::bytes(bytes))
                                            .size(type_scale::MONO)
                                            .monospace()
                                            .color(MUTED()),
                                    );
                                });
                            });
                        }
                    });
                return;
            }
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
        let flat = self.show_largest || self.filter.is_active();
        if !flat {
            self.reveal_in_tree(&tree);
        }
        let rows = tree_rows(&tree, &ids, &colors, &self.expanded, flat, mode);
        self.tree_rows = rows.iter().map(|r| r.id).collect();
        let scroll_to = std::mem::take(&mut self.tree_scroll);

        let queued_set: HashSet<NodeId> = self.review.iter().copied().collect();
        let mut partial_set = HashSet::new();
        for &qid in &self.review {
            let mut p = tree.try_node(qid).and_then(|n| n.parent);
            while let Some(pid) = p {
                partial_set.insert(pid);
                p = tree.try_node(pid).and_then(|n| n.parent);
            }
        }
        let (
            mut pick,
            mut shift_pick,
            mut enter,
            mut reveal,
            mut quick_look_id,
            mut term_id,
            mut queue,
            mut toggle,
            mut row_hover,
        ) = (None, false, None, None, None, None, None, None, None);
        let scroll_out = egui::ScrollArea::vertical()
            .id_salt("contents_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(4.0);
                if ids.is_empty() {
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
                for row in &rows {
                    let id = row.id;
                    let n = tree.node(id);
                    let selected = self.selected == Some(id) || self.selected_set.contains(&id);
                    let deletable = !n.synthetic && n.parent.is_some();
                    let queued = queued_set.contains(&id);
                    let check = if !deletable {
                        None
                    } else if queued {
                        Some(CheckVisual::Checked)
                    } else if has_queued_ancestor(&tree, &queued_set, id) {
                        Some(CheckVisual::CoveredByParent)
                    } else if partial_set.contains(&id) {
                        Some(CheckVisual::Partial)
                    } else {
                        Some(CheckVisual::Unchecked)
                    };
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), TREE_ROW_H),
                        Sense::click(),
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
                        RowVisual {
                            selected,
                            check,
                            linked_hover: self.hovered == Some(id),
                        },
                    );
                    if outcome.chevron {
                        toggle = Some(id);
                    }
                    if outcome.check {
                        queue = Some(id);
                    }
                    if resp.hovered() {
                        row_hover = Some(id);
                    }

                    let row_name = n.name.clone();
                    let resp = resp.on_hover_text(t(if n.is_dir {
                        "Click to select · double-click to open · check box to queue"
                    } else {
                        "Click to select · double-click to show in Finder · check box to queue"
                    }));
                    resp.widget_info(move || {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, row_name.as_str())
                    });
                    if resp.clicked() && !outcome.chevron && !outcome.check {
                        if ui.input(|i| i.modifiers.command) && deletable {
                            queue = Some(id);
                        }
                        shift_pick = ui.input(|i| i.modifiers.shift);
                        pick = Some(id);
                    }
                    if resp.double_clicked() && !outcome.check && !outcome.chevron {
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
                        if deletable {
                            let q = if queued {
                                "Remove from selection"
                            } else {
                                "Add to selection"
                            };
                            if ui.button(t(q)).clicked() {
                                queue = Some(id);
                                ui.close_menu();
                            }
                            if ui.button(t("Show in Finder")).clicked() {
                                reveal = Some(id);
                                ui.close_menu();
                            }
                        }
                        if !n.synthetic {
                            if ui.button(t("Quick Look")).clicked() {
                                quick_look_id = Some(id);
                                ui.close_menu();
                            }
                            if ui.button(t("Open in Terminal")).clicked() {
                                term_id = Some(id);
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
        if (row_hover.is_some() || ui.rect_contains_pointer(scroll_out.inner_rect))
            && self.hovered != row_hover
        {
            self.hovered = row_hover;
            ui.ctx().request_repaint();
        }
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
            if shift_pick {
                self.select_range_in_rows(id);
            } else {
                self.select_single(id);
            }
        }
        if let Some(id) = queue {
            review::toggle_in_tree(&tree, &mut self.review, id);
        }
        if let Some(id) = reveal {
            self.select_single(id);
            self.reveal_selected();
        }
        if let Some(id) = quick_look_id {
            self.select_single(id);
            self.quick_look_selected();
        }
        if let Some(id) = term_id {
            self.select_single(id);
            self.open_terminal_selected();
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
        if self.selected_set.len() > 1 {
            let mut count = 0usize;
            let mut sum_bytes = 0u64;
            let mut deletable_ids = Vec::new();
            for &sid in &self.selected_set {
                if let Some(n) = tree.try_node(sid) {
                    count += 1;
                    sum_bytes = sum_bytes.saturating_add(n.size(self.size_mode));
                    if !n.synthetic && n.parent.is_some() {
                        deletable_ids.push(sid);
                    }
                }
            }
            ui.horizontal(|ui| {
                icons::show(ui, Icon::Folder, 18.0, ACCENT());
                ui.label(caption("MULTI-SELECTION"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icons::button(ui, Icon::Close, 28.0, "Clear selection", true).clicked() {
                        self.clear_selection();
                    }
                });
            });
            ui.label(
                RichText::new(crate::tr!("{} items selected", count))
                    .size(type_scale::TITLE)
                    .strong(),
            );
            ui.add_space(S2);
            stat(ui, &crate::i18n::bytes(sum_bytes), t("total in selection"));
            ui.add_space(S3);
            if ui
                .add_enabled(
                    !deletable_ids.is_empty() && !self.busy,
                    block(ui, primary("Add selected to basket")),
                )
                .clicked()
            {
                for sid in deletable_ids {
                    if !self.review.contains(&sid) {
                        review::toggle_in_tree(&tree, &mut self.review, sid);
                    }
                }
            }
            ui.add_space(S2);
            if ui.add(block(ui, secondary("Clear selection"))).clicked() {
                self.clear_selection();
            }
            return;
        }
        let Some(id) = self.selected else {
            return;
        };
        let Some(node) = tree.try_node(id) else {
            self.clear_selection();
            return;
        };
        let size = node.size(self.size_mode);
        let path = tree.path_string(id);
        let queued = self.review.contains(&id);

        ui.horizontal(|ui| {
            icons::show(
                ui,
                if node.is_dir {
                    Icon::Folder
                } else {
                    Icon::File
                },
                18.0,
                MUTED(),
            );
            ui.label(caption(if node.is_dir { "FOLDER" } else { "FILE" }));
            if queued {
                badge(ui, "Selected", ACCENT(), "Marked to move to Trash");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icons::button(ui, Icon::Close, 28.0, "Close details", true).clicked() {
                    self.clear_selection();
                }
            });
        });
        ui.add(
            egui::Label::new(RichText::new(&node.name).size(type_scale::TITLE).strong()).truncate(),
        )
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
                &match self.percent_of_parent(id) {
                    Some(p) => crate::tr!("{}% of parent", format!("{p:.0}")),
                    None => t("whole scan").to_string(),
                },
            );
            stat(
                &mut cols[1],
                &node.file_count.to_string(),
                t(if node.is_dir { "files inside" } else { "file" }),
            );
        });
        ui.add_space(S3);

        if let Some((category, label, color, hint)) = self.junk.risk_hint(&PathBuf::from(&path)) {
            egui::Frame::new()
                .fill(PANEL_2())
                .stroke(egui::Stroke::new(1.0_f32, color.gamma_multiply(0.5)))
                .corner_radius(10)
                .inner_margin(egui::Margin::same(S3 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(format!("{} · {}", t(label), t(&category)))
                            .color(color)
                            .strong(),
                    );
                    ui.label(caption(crate::tr!(
                        "{}. A rule match is guidance, not a guarantee.",
                        t(hint)
                    )));
                });
            ui.add_space(S3);
        }

        let is_dir = node.is_dir && !node.synthetic;
        let deletable = !node.synthetic && node.parent.is_some();
        if is_dir {
            if ui.add(block(ui, primary("Open folder"))).clicked() {
                self.navigate(id, true);
            }
        } else if ui.add(block(ui, primary("Show in Finder"))).clicked() {
            self.reveal_selected();
        }
        ui.add_space(S2);
        let label = if queued {
            "Remove from selection"
        } else {
            "Add to selection"
        };
        if ui
            .add_enabled(deletable && !self.busy, block(ui, secondary(label)))
            .on_hover_text(t(
                "Selecting never removes anything — confirm in Move to Trash",
            ))
            .clicked()
        {
            review::toggle_in_tree(&tree, &mut self.review, id);
        }
        ui.add_space(S2);
        let w = (ui.available_width() - S2) * 0.5;
        let font = egui::FontId::proportional(type_scale::BODY);
        let fits = ["Show in Finder", "Rescan folder"]
            .iter()
            .all(|l| text_width(ui, t(l), font.clone()) + 28.0 <= w);
        let row = |ui: &mut egui::Ui| {
            let w = if fits { w } else { ui.available_width() };
            if is_dir
                && ui
                    .add_sized([w, 32.0], egui::Button::new(t("Show in Finder")))
                    .clicked()
            {
                self.reveal_selected();
            }
            if is_dir
                && ui
                    .add_enabled(
                        self.scan.is_none() && !self.busy,
                        egui::Button::new(t("Rescan folder")).min_size(egui::vec2(w, 32.0)),
                    )
                    .on_hover_text(t("Rebuild the index for the folder in view"))
                    .clicked()
            {
                self.incremental_rescan_focus();
            }
        };
        if fits {
            ui.horizontal(row);
        } else {
            ui.vertical(row);
        }
        if !node.synthetic {
            ui.add_space(S2);
            ui.horizontal(|ui| {
                if ui
                    .add_sized([w, 32.0], egui::Button::new(t("Quick Look")))
                    .clicked()
                {
                    self.quick_look_selected();
                }
                if ui
                    .add_sized([w, 32.0], egui::Button::new(t("Open in Terminal")))
                    .clicked()
                {
                    self.open_terminal_selected();
                }
            });
        }
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
                        selected_set: &self.selected_set,
                        hovered: self.hovered,
                        review: &self.review,
                        mode: self.size_mode,
                        filter: &self.filter,
                        filter_hash: fhash,
                        review_sig,
                        sunburst_rings: self.settings.sunburst_rings as u8,
                        treemap_depth: self.settings.treemap_depth as u8,
                        treemap_mode: self.settings.treemap_mode,
                        cascade_depth: self.settings.cascade_depth as u8,
                        cascade_row_height: self.settings.cascade_row_height,
                        geom: &mut self.geom,
                    },
                )
            })
            .inner;
        ui.set_opacity(1.0);

        if ctx.input(|i| !i.raw.hovered_files.is_empty()) && self.scan.is_none() && !self.busy {
            ui.painter()
                .rect_filled(map, 0.0, ACCENT().gamma_multiply(0.16));
            ui.painter().text(
                map.center(),
                egui::Align2::CENTER_CENTER,
                "Drop to scan this folder",
                egui::FontId::proportional(type_scale::TITLE),
                ACCENT(),
            );
        }
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

        let ptr = ctx.input(|i| i.pointer.hover_pos());
        let in_map = ptr.is_some_and(|p| map.contains(p));
        if action.hover.is_some() || in_map {
            if self.hovered != action.hover {
                self.hovered = action.hover;
                ctx.request_repaint();
            }
        } else if ptr.is_none() {
            self.hovered = None;
        }
        if let Some(id) = action.toggle_select {
            self.toggle_select_node(id);
        }
        if let Some(id) = action.select {
            self.select_single(id);
        }
        if let Some(id) = action.navigate {
            self.navigate(id, true);
        }
        if let Some(id) = action.toggle_review {
            review::toggle_in_tree(&tree, &mut self.review, id);
        }
        if let Some(id) = action.reveal {
            self.select_single(id);
            self.reveal_selected();
        }
        if let Some(id) = action.quick_look {
            self.select_single(id);
            self.quick_look_selected();
        }
        if let Some(id) = action.open_terminal {
            self.select_single(id);
            self.open_terminal_selected();
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
            self.reset_filters();
        }
    }

    /// Quiet workspace settings outside the map's interactive drawing area.
    fn map_controls(&mut self, ui: &mut egui::Ui) {
        ui.scope(|ui| {
            let widgets = &mut ui.visuals_mut().widgets;
            for visual in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
            ] {
                visual.bg_stroke = egui::Stroke::NONE;
                visual.weak_bg_fill = Color32::TRANSPARENT;
            }
            ui.horizontal(|ui| {
                let mut view = self.view;
                ui.menu_button(caption(format!("{} ▾", t(self.view.label()))), |ui| {
                    for mode in ViewMode::ALL {
                        if ui.selectable_label(view == mode, t(mode.label())).clicked() {
                            view = mode;
                            ui.close_menu();
                        }
                    }
                });
                if view != self.view {
                    self.set_view(view);
                }
                let mut size_mode = self.size_mode;
                let size_label = t(if size_mode == SizeMode::OnDisk {
                    "On disk"
                } else {
                    "File size"
                });
                ui.menu_button(caption(format!("{size_label} ▾")), |ui| {
                    for (mode, label) in [
                        (SizeMode::OnDisk, "On disk"),
                        (SizeMode::Apparent, "File size"),
                    ] {
                        if ui.selectable_label(size_mode == mode, t(label)).clicked() {
                            size_mode = mode;
                            ui.close_menu();
                        }
                    }
                });
                if size_mode != self.size_mode {
                    self.size_mode = size_mode;
                    if let Some(tree) = &mut self.tree {
                        Arc::make_mut(tree).sort_children_by(size_mode);
                    }
                    self.geom = None;
                    self.invalidate_largest();
                    self.schedule_largest();
                }
                if !matches!(self.view, ViewMode::Radial) {
                    let mut opts_changed = false;
                    ui.menu_button(caption(format!("{} ▾", t("Options"))), |ui| {
                        match self.view {
                            ViewMode::Sunburst => {
                                ui.label(caption("Rings"));
                                ui.horizontal(|ui| {
                                    for n in 1..=4usize {
                                        if ui
                                            .selectable_label(
                                                self.settings.sunburst_rings == n,
                                                format!("{n}"),
                                            )
                                            .clicked()
                                        {
                                            self.settings.sunburst_rings = n;
                                            opts_changed = true;
                                        }
                                    }
                                });
                            }
                            ViewMode::Container => {
                                ui.label(caption("Layout"));
                                ui.horizontal(|ui| {
                                    for (m, label) in [
                                        (crate::settings::TreemapMode::Squarify, "Squarified"),
                                        (crate::settings::TreemapMode::Strip, "Strip"),
                                    ] {
                                        if ui
                                            .selectable_label(
                                                self.settings.treemap_mode == m,
                                                t(label),
                                            )
                                            .clicked()
                                        {
                                            self.settings.treemap_mode = m;
                                            opts_changed = true;
                                        }
                                    }
                                });
                                ui.label(caption("Depth"));
                                ui.horizontal(|ui| {
                                    for n in 1..=4usize {
                                        if ui
                                            .selectable_label(
                                                self.settings.treemap_depth == n,
                                                format!("{n}"),
                                            )
                                            .clicked()
                                        {
                                            self.settings.treemap_depth = n;
                                            opts_changed = true;
                                        }
                                    }
                                });
                            }
                            ViewMode::Cascade => {
                                ui.label(caption("Depth"));
                                ui.horizontal(|ui| {
                                    for n in 2..=8usize {
                                        if ui
                                            .selectable_label(
                                                self.settings.cascade_depth == n,
                                                format!("{n}"),
                                            )
                                            .clicked()
                                        {
                                            self.settings.cascade_depth = n;
                                            opts_changed = true;
                                        }
                                    }
                                });
                                ui.label(caption("Row height"));
                                if ui
                                    .add(
                                        egui::Slider::new(
                                            &mut self.settings.cascade_row_height,
                                            24.0..=64.0,
                                        )
                                        .step_by(2.0)
                                        .suffix(" px"),
                                    )
                                    .changed()
                                {
                                    opts_changed = true;
                                }
                            }
                            ViewMode::Radial => {}
                        }
                    });
                    if opts_changed {
                        self.settings.clamp();
                        self.settings.save();
                        self.geom = None;
                    }
                }
            });
        });
        ui.add_space(4.0);
    }

    /// One dismissible hint, bottom-centre.
    fn map_hint(&mut self, ctx: &egui::Context, map: egui::Rect) {
        if !self.settings.flags.show_shortcuts_hint || self.scan.is_some() || self.busy {
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
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, 0.0])
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
    /// For each ancestor level: does a later sibling follow (draw a guide)?
    guides: Vec<bool>,
}

fn has_queued_ancestor(tree: &Tree, queued: &HashSet<NodeId>, id: NodeId) -> bool {
    let mut parent = tree.try_node(id).and_then(|n| n.parent);
    while let Some(pid) = parent {
        if queued.contains(&pid) {
            return true;
        }
        parent = tree.try_node(pid).and_then(|n| n.parent);
    }
    false
}

pub(super) fn tree_rows(
    tree: &Tree,
    top: &[NodeId],
    colors: &[Color32],
    expanded: &HashSet<NodeId>,
    flat: bool,
    mode: SizeMode,
) -> Vec<TreeRow> {
    struct WalkCtx<'a> {
        tree: &'a Tree,
        expanded: &'a HashSet<NodeId>,
        mode: SizeMode,
        parent_total: &'a dyn Fn(NodeId) -> f32,
        out: Vec<TreeRow>,
    }

    fn walk(ctx: &mut WalkCtx<'_>, id: NodeId, depth: usize, color: Color32, guides: Vec<bool>) {
        if ctx.out.len() >= MAX_TREE_ROWS {
            return;
        }
        let n = ctx.tree.node(id);
        let kids = if n.is_dir && !n.synthetic {
            ctx.tree.children(id)
        } else {
            &[]
        };
        let open = !kids.is_empty() && ctx.expanded.contains(&id);
        ctx.out.push(TreeRow {
            id,
            depth,
            color,
            size: n.size(ctx.mode),
            share: n.size(ctx.mode) as f32 / (ctx.parent_total)(id),
            has_kids: !kids.is_empty(),
            open,
            guides: guides.clone(),
        });
        if open {
            let mut kids = kids.to_vec();
            kids.sort_by_key(|&c| std::cmp::Reverse(ctx.tree.node(c).size(ctx.mode)));
            kids.truncate(MAX_KIDS);
            let last = kids.len().saturating_sub(1);
            for (i, k) in kids.into_iter().enumerate() {
                let mut g = guides.clone();
                g.push(i != last);
                let c = if ctx.tree.node(k).synthetic {
                    MUTED()
                } else {
                    color
                };
                walk(ctx, k, depth + 1, c, g);
            }
        }
    }

    let parent_total = |id: NodeId| {
        tree.node(id)
            .parent
            .map(|p| tree.node(p).size(mode))
            .unwrap_or(0)
            .max(1) as f32
    };
    let mut ctx = WalkCtx {
        tree,
        expanded,
        mode,
        parent_total: &parent_total,
        out: Vec::new(),
    };
    for (i, &id) in top.iter().enumerate() {
        let color = if tree.node(id).synthetic {
            MUTED()
        } else {
            colors[i]
        };
        if flat {
            ctx.out.push(TreeRow {
                id,
                depth: 0,
                color,
                size: tree.node(id).size(mode),
                share: tree.node(id).size(mode) as f32 / parent_total(id),
                has_kids: false,
                open: false,
                guides: Vec::new(),
            });
        } else {
            walk(&mut ctx, id, 0, color, Vec::new());
        }
        if ctx.out.len() >= MAX_TREE_ROWS {
            break;
        }
    }
    ctx.out
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
    check: bool,
}

struct RowVisual {
    selected: bool,
    check: Option<CheckVisual>,
    linked_hover: bool,
}

/// One tree row: checkbox · guides · chevron · swatch · name · share bar + % · size.
fn paint_tree_row(
    ui: &egui::Ui,
    rect: egui::Rect,
    resp: &egui::Response,
    row: &TreeRow,
    n: &space_core::Node,
    visual: RowVisual,
) -> RowOutcome {
    let p = ui.painter();
    if visual.selected {
        p.rect_filled(rect, 0.0, PANEL_2());
        p.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
            0.0,
            ACCENT(),
        );
    } else if resp.hovered() || visual.linked_hover {
        p.rect_filled(rect, 0.0, PANEL_2().gamma_multiply(0.6));
    }
    if resp.has_focus() {
        crate::theme::paint_focus_ring(p, rect);
    }
    let cy = rect.center().y;
    let mut check = false;
    if let Some(vis) = visual.check {
        let cb = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 17.0, cy),
            egui::vec2(14.0, 14.0),
        );
        let hit = cb.expand(5.0);
        let hot = resp.hovered()
            && ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|h| hit.contains(h));
        paint_checkbox(p, cb, vis, hot);
        check = resp.clicked()
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|h| hit.contains(h));
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
            && !check
            && ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|h| chev.expand(4.0).contains(h));
    }
    p.rect_filled(
        egui::Rect::from_center_size(egui::pos2(x0 + 24.0, cy), egui::vec2(10.0, 10.0)),
        3.0,
        row.color,
    );

    let queued = matches!(
        visual.check,
        Some(CheckVisual::Checked | CheckVisual::CoveredByParent)
    );
    let pad = S4;
    let size_g = p.layout_no_wrap(
        crate::i18n::bytes(row.size),
        egui::FontId::monospace(type_scale::MONO),
        if queued {
            ACCENT()
        } else if visual.selected {
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
        egui::FontId::monospace(type_scale::MONO),
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
    let mut job = egui::text::LayoutJob::simple_singleline(
        n.name.clone(),
        egui::FontId::proportional(type_scale::BODY),
        if n.synthetic { MUTED() } else { TEXT() },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(name_w);
    let g = p.layout_job(job);
    let name_end = (name_x + g.size().x).min(bar_x - 10.0);
    let underline_y = cy + g.size().y * 0.5 + 1.0;
    p.with_clip_rect(egui::Rect::from_min_max(
        rect.min,
        egui::pos2((bar_x - 10.0).max(rect.left()), rect.bottom()),
    ))
    .galley(egui::pos2(name_x, cy - g.size().y / 2.0), g, TEXT());
    if (resp.hovered() || visual.linked_hover) && name_end > name_x {
        p.line_segment(
            [
                egui::pos2(name_x, underline_y),
                egui::pos2(name_end, underline_y),
            ],
            egui::Stroke::new(1.0_f32, ACCENT()),
        );
    }
    RowOutcome { chevron, check }
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

    /// Arrow keys walk the visible tree. Returns true when the key was consumed.
    pub(in crate::app) fn tree_keys(
        &mut self,
        up: bool,
        down: bool,
        left: bool,
        right: bool,
        shift: bool,
    ) -> bool {
        let Some(tree) = self.tree.clone() else {
            return false;
        };
        if self.tree_rows.is_empty() || !(up || down || left || right) {
            return false;
        }
        let pos = self
            .selected
            .and_then(|s| self.tree_rows.iter().position(|&r| r == s));
        let Some(i) = pos else {
            if up || down {
                if let Some(&first) = self.tree_rows.first() {
                    self.select_single(first);
                    self.tree_revealed = self.selected;
                    self.tree_scroll = true;
                }
                return true;
            }
            return false;
        };
        let id = self.tree_rows[i];
        let Some(n) = tree.try_node(id) else {
            return false;
        };
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
        } else if up || down {
            // At an end of the list: consume so the map does not jump.
        } else {
            return left || right;
        }
        if let Some(x) = next {
            if shift && (up || down) {
                self.select_range_in_rows(x);
            } else {
                self.select_single(x);
            }
            self.tree_revealed = Some(x);
            self.tree_scroll = true;
        }
        true
    }
}
