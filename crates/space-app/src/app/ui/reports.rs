//! Oldest, Duplicates and Types reports drawn in the contents pane.

use eframe::egui::{self, Sense};
use space_core::{now_unix, NodeId, SpaceTree as Tree};

use crate::app::SpaceApp;
use crate::i18n::t;
use crate::reports::{self, ReportData, ReportKind, OLDEST_PRESETS_MB};
use crate::theme::metrics::{S2, S3, S4, TREE_ROW_H};
use crate::theme::type_scale;
use crate::theme::{ACCENT, MUTED, PANEL_2, TEXT};
use crate::widgets::{caption, chip, empty_state, ghost};

enum Act {
    Select(NodeId),
    SelectMany(Vec<NodeId>),
    Queue(Vec<NodeId>),
    Reveal(NodeId),
}

impl SpaceApp {
    pub(super) fn report_view(&mut self, ui: &mut egui::Ui, tree: &Tree, kind: ReportKind) {
        let key = self.report_key(tree, kind);
        let data = self.reports.get(&key);
        let mut act = None;

        ui.horizontal_wrapped(|ui| {
            ui.add_space(S4);
            ui.spacing_mut().item_spacing.x = 6.0;
            match kind {
                ReportKind::Oldest => {
                    ui.label(caption("At least"));
                    for mb in OLDEST_PRESETS_MB {
                        let label = crate::i18n::bytes(mb * 1024 * 1024);
                        if chip(ui, self.settings.oldest_min_mb == mb, &label).clicked() {
                            self.settings.oldest_min_mb = mb;
                            self.mark_settings_dirty();
                            self.schedule_report();
                        }
                    }
                    ui.label(caption("· whole scan"));
                }
                ReportKind::Duplicates => {
                    ui.label(caption("Same name and size · whole scan"));
                    if let Some(ReportData::Duplicates(groups)) = data.as_deref() {
                        if !groups.is_empty()
                            && ui
                                .add(ghost("Add extras to Trash list"))
                                .on_hover_text(t("Keeps the newest copy of each group"))
                                .clicked()
                        {
                            act = Some(Act::Queue(
                                groups.iter().flat_map(|g| reports::extras(tree, g)).collect(),
                            ));
                        }
                    }
                }
                ReportKind::Types => {
                    ui.label(caption("By extension · this folder"));
                }
            }
        });
        ui.add_space(S2);

        let Some(data) = data else {
            ui.add_space(S4);
            ui.vertical_centered(|ui| ui.spinner());
            return;
        };
        let empty = match &*data {
            ReportData::Oldest(r) => r.is_empty(),
            ReportData::Duplicates(r) => r.is_empty(),
            ReportData::Types(r) => r.is_empty(),
        };
        if empty {
            empty_state(ui, "Nothing to show", "No file in this scan fits this report.");
            return;
        }

        let now = now_unix();
        ui.spacing_mut().item_spacing.y = 0.0;
        let area = egui::ScrollArea::vertical()
            .id_salt(("report_list", kind as u8))
            .auto_shrink([false, false]);
        match &*data {
            ReportData::Oldest(rows) => {
                area.show_rows(ui, TREE_ROW_H, rows.len(), |ui, range| {
                    for &(id, size, mtime) in &rows[range] {
                        let Some(n) = tree.try_node(id) else { continue };
                        let when = crate::i18n::date_time(u64::from(mtime));
                        let age = crate::i18n::age(now.saturating_sub(u64::from(mtime)));
                        let resp = report_row(
                            ui,
                            self.is_selected(id),
                            self.review.contains(&id),
                            &n.name,
                            &age,
                            &crate::i18n::bytes(size),
                            None,
                        )
                        .on_hover_text(format!("{}\n{}", tree.path_string(id), when));
                        row_actions(&resp, id, &mut act);
                    }
                });
            }
            ReportData::Duplicates(groups) => {
                area.show_rows(ui, TREE_ROW_H, groups.len(), |ui, range| {
                    for g in &groups[range] {
                        let selected = g.members.iter().all(|&id| self.is_selected(id));
                        let queued = g.members.iter().any(|id| self.review.contains(id));
                        let detail = crate::tr!(
                            "{} copies · {} each",
                            g.members.len(),
                            crate::i18n::bytes(g.size)
                        );
                        let wasted = crate::tr!("{} extra", crate::i18n::bytes(g.wasted));
                        let paths: Vec<String> = g
                            .members
                            .iter()
                            .filter(|&&id| tree.try_node(id).is_some())
                            .map(|&id| tree.path_string(id))
                            .collect();
                        let resp = report_row(ui, selected, queued, &g.name, &detail, &wasted, None)
                            .on_hover_text(paths.join("\n"));
                        if resp.clicked() {
                            act = Some(Act::SelectMany(g.members.clone()));
                        }
                        resp.context_menu(|ui| {
                            if ui.button(t("Add extras to Trash list")).clicked() {
                                act = Some(Act::Queue(reports::extras(tree, g)));
                                ui.close_menu();
                            }
                        });
                    }
                });
            }
            ReportData::Types(rows) => {
                let total: u64 = rows.iter().map(|r| r.1).sum::<u64>().max(1);
                area.show_rows(ui, TREE_ROW_H, rows.len(), |ui, range| {
                    for (ext, bytes, files) in &rows[range] {
                        let name = match ext.as_str() {
                            "(none)" => t("No extension").to_string(),
                            "(small files)" => t("Small files").to_string(),
                            e => format!(".{e}"),
                        };
                        let share = *bytes as f32 / total as f32;
                        report_row(
                            ui,
                            false,
                            false,
                            &name,
                            &crate::i18n::count(*files as usize, "file"),
                            &crate::i18n::bytes(*bytes),
                            Some(share),
                        )
                        .on_hover_text(format!("{:.1}%", share * 100.0));
                    }
                });
            }
        }

        match act {
            Some(Act::Select(id)) => {
                self.select_one(Some(id));
                self.tree_scroll = true;
            }
            Some(Act::SelectMany(ids)) => {
                let live: Vec<NodeId> = ids.into_iter().filter(|&id| tree.try_node(id).is_some()).collect();
                if let Some(&first) = live.first() {
                    self.select_one(Some(first));
                    if live.len() > 1 {
                        self.multi = live.iter().copied().collect();
                    }
                    self.tree_scroll = true;
                }
            }
            Some(Act::Queue(ids)) => {
                // Only add: a group already on the list is not taken off by a second click.
                let ids: Vec<NodeId> = ids.into_iter().filter(|id| !self.review.contains(id)).collect();
                if !ids.is_empty() {
                    self.queue_toggle(&ids);
                }
            }
            Some(Act::Reveal(id)) => {
                self.select_one(Some(id));
                self.reveal_selected();
            }
            None => {}
        }
    }
}

fn row_actions(resp: &egui::Response, id: NodeId, act: &mut Option<Act>) {
    if resp.clicked() {
        *act = Some(Act::Select(id));
    }
    if resp.double_clicked() {
        *act = Some(Act::Reveal(id));
    }
    resp.context_menu(|ui| {
        if ui.button(t("Add to Trash list")).clicked() {
            *act = Some(Act::Queue(vec![id]));
            ui.close_menu();
        }
        if ui.button(t("Show in Finder")).clicked() {
            *act = Some(Act::Reveal(id));
            ui.close_menu();
        }
    });
}

/// One report line: name, a muted detail, a right-aligned figure and an
/// optional share bar along the bottom.
fn report_row(
    ui: &mut egui::Ui,
    selected: bool,
    queued: bool,
    name: &str,
    detail: &str,
    figure: &str,
    share: Option<f32>,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), TREE_ROW_H), Sense::click());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 4.0, ACCENT().gamma_multiply(0.18));
    } else if resp.hovered() {
        painter.rect_filled(rect, 4.0, PANEL_2());
    }
    if let Some(share) = share {
        let w = (rect.width() - 2.0 * S4) * share.clamp(0.0, 1.0);
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + S4, rect.bottom() - 4.0),
            egui::vec2(w.max(1.0), 2.0),
        );
        painter.rect_filled(bar, 1.0, ACCENT());
    }
    let mid = rect.center().y;
    let name_font = egui::FontId::proportional(type_scale::BODY);
    let small = egui::FontId::monospace(type_scale::MONO);
    let figure_w = painter
        .layout_no_wrap(figure.to_string(), small.clone(), MUTED())
        .size()
        .x;
    painter.text(
        egui::pos2(rect.right() - S4, mid),
        egui::Align2::RIGHT_CENTER,
        figure,
        small.clone(),
        if queued { ACCENT() } else { TEXT() },
    );
    let detail_galley = painter.layout_no_wrap(detail.to_string(), small.clone(), MUTED());
    let detail_x = rect.right() - S4 - figure_w - S3 - detail_galley.size().x;
    painter.galley(
        egui::pos2(detail_x, mid - detail_galley.size().y / 2.0),
        detail_galley,
        MUTED(),
    );
    let max_name = (detail_x - S3 - rect.left() - S4).max(20.0);
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), name_font, TEXT());
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_name);
    let galley = painter.layout_job(job);
    painter.galley(egui::pos2(rect.left() + S4, mid - galley.size().y / 2.0), galley, TEXT());
    resp
}
