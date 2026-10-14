//! Dialogs: Settings, shortcuts, the Move to Trash confirmation, local snapshots
//! (all modal, over a scrim) and the Cleanup rules window.

use crate::i18n::t;
use std::time::Duration;

use eframe::egui::{self, Align, Layout, RichText};

use space_core::SizeMode;

use crate::app::SpaceApp;
use crate::i18n::Lang;
use crate::rules_panel::{self, RulesAction};
use crate::settings::{SizeUnits, WorkspaceLayout};
use crate::views::ViewMode;
use crate::theme::metrics::{S2, S3, S4, S6};
use crate::theme::type_scale;
use crate::theme::{self, Appearance, ACCENT, DANGER, MUTED, PANEL_2};
use crate::widgets::{
    caption, count_noun, danger, divider, ghost, path_size_row, primary, secondary, segmented,
    strong,
};

/// A modal frame: dims the window behind it and takes all input.
fn modal(id: &'static str, ctx: &egui::Context, width: f32) -> egui::Modal {
    let screen = ctx.screen_rect();
    let _ = width;
    egui::Modal::new(egui::Id::new(id))
        .backdrop_color(crate::theme::SCRIM())
        .frame(
            egui::Frame::window(&ctx.style())
                .corner_radius(crate::theme::metrics::RADIUS_DIALOG)
                .inner_margin(egui::Margin::same(S6 as i8)),
        )
        .area(
            egui::Modal::default_area(egui::Id::new(id).with("area"))
                .default_width((screen.width() - 64.0).min(width)),
        )
}

impl SpaceApp {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        self.settings_dialog(ctx);
        self.shortcuts_dialog(ctx);
        self.trash_dialog(ctx);
        self.snapshots_dialog(ctx);
        self.rules_dialog(ctx);
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut close = false;
        let mut open_rules = false;
        let mut open_keys = false;
        let resp = modal("settings", ctx, 520.0).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 112.0).min(472.0));
            ui.horizontal(|ui| {
                ui.label(strong("Settings", type_scale::TITLE));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if crate::icons::button(
                        ui,
                        crate::icons::Icon::Close,
                        28.0,
                        "Close · Esc",
                        true,
                    )
                    .clicked()
                    {
                        close = true;
                    }
                });
            });
            ui.add_space(S4);
            let row =
                |ui: &mut egui::Ui, label: &str, hint: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
                    ui.label(RichText::new(t(label)).family(crate::theme::medium()));
                    if !hint.is_empty() {
                        ui.label(caption(hint.to_owned()));
                    }
                    ui.add_space(4.0);
                    add(ui);
                    ui.add_space(S4);
                };

            let mut appearance = self.settings.appearance;
            row(ui, "Appearance", "", &mut |ui| {
                let labels: Vec<&str> = Appearance::ALL.iter().map(|a| a.label()).collect();
                let cur = Appearance::ALL
                    .iter()
                    .position(|&a| a == appearance)
                    .unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, cur) {
                    appearance = Appearance::ALL[i];
                }
            });
            if appearance != self.settings.appearance {
                self.settings.appearance = appearance;
                crate::theme::set_appearance(ui.ctx(), appearance);
                self.geom = None;
                self.mark_settings_dirty();
            }

            let mut layout = self.settings.layout;
            row(
                ui,
                "Disk usage layout",
                "Also on the Disk usage toolbar. Every layout works in both appearances.",
                &mut |ui| {
                    let labels: Vec<&str> =
                        WorkspaceLayout::ALL.iter().map(|l| l.label()).collect();
                    let cur = WorkspaceLayout::ALL
                        .iter()
                        .position(|&l| l == layout)
                        .unwrap_or(0);
                    if let Some(i) = segmented(ui, &labels, cur) {
                        layout = WorkspaceLayout::ALL[i];
                    }
                },
            );
            self.set_layout(layout);

            let mut size_mode = self.size_mode;
            row(
                ui,
                "Sizes",
                "On disk counts the space files take; file size is what apps report.",
                &mut |ui| {
                    if let Some(i) = segmented(
                        ui,
                        &["On disk", "File size"],
                        (size_mode == SizeMode::Apparent) as usize,
                    ) {
                        size_mode = if i == 0 {
                            SizeMode::OnDisk
                        } else {
                            SizeMode::Apparent
                        };
                    }
                },
            );
            if size_mode != self.size_mode {
                self.size_mode = size_mode;
                self.search_hits = None;
                self.invalidate_largest();
                self.schedule_largest();
            }

            let mut lang = self.settings.language;
            row(ui, "Language", "", &mut |ui| {
                let options = [None, Some(Lang::En), Some(Lang::Ru)];
                let labels = [
                    t("Automatic"),
                    Lang::En.native_name(),
                    Lang::Ru.native_name(),
                ];
                let cur = options.iter().position(|&o| o == lang).unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, cur) {
                    lang = options[i];
                }
            });
            if lang != self.settings.language {
                self.settings.language = lang;
                crate::i18n::set(lang.unwrap_or_else(crate::i18n::detect));
                self.mark_settings_dirty();
            }

            let mut units = self.settings.size_units;
            row(ui, "Size units", "", &mut |ui| {
                let labels: Vec<&str> = SizeUnits::ALL.iter().map(|u| u.label()).collect();
                let cur = SizeUnits::ALL.iter().position(|&u| u == units).unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, cur) {
                    units = SizeUnits::ALL[i];
                }
            });
            if units != self.settings.size_units {
                self.settings.size_units = units;
                crate::i18n::set_units(units);
                self.mark_settings_dirty();
            }

            row(ui, "Motion", "", &mut |ui| {
                let mut interaction = false;
                if ui
                    .checkbox(&mut self.settings.reduce_motion, t("Reduce motion"))
                    .on_hover_text(t("No zoom, crossfade or hover animation"))
                    .changed()
                {
                    interaction = true;
                }
                ui.horizontal(|ui| {
                    ui.label(caption("Hint delay"));
                    if ui
                        .add(
                            egui::Slider::new(&mut self.settings.tooltip_delay_ms, 0..=1500)
                                .step_by(50.0)
                                .suffix(" ms"),
                        )
                        .on_hover_text(t("How long the pointer rests before a hover hint appears"))
                        .changed()
                    {
                        interaction = true;
                    }
                });
                if interaction {
                    crate::theme::apply_interaction(
                        ui.ctx(),
                        self.settings.reduce_motion,
                        self.settings.tooltip_delay_ms,
                    );
                    self.mark_settings_dirty();
                }
            });

            let mut hint = self.settings.flags.show_shortcuts_hint;
            if ui
                .checkbox(&mut hint, t("Show the gesture hint on the map"))
                .changed()
            {
                self.settings.flags.show_shortcuts_hint = hint;
                self.mark_settings_dirty();
            }
            ui.add_space(S4);
            divider(ui);
            ui.add_space(S3);

            ui.label(RichText::new(t("Disk usage")).family(crate::theme::medium()));
            ui.add_space(4.0);
            let mut view = self.settings.default_view;
            row(ui, "Default view", "", &mut |ui| {
                let labels: Vec<&str> = ViewMode::ALL.iter().map(|m| m.label()).collect();
                let cur = ViewMode::ALL.iter().position(|&m| m == view).unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, cur) {
                    view = ViewMode::ALL[i];
                }
            });
            if view != self.settings.default_view {
                self.settings.default_view = view;
                self.mark_settings_dirty();
            }
            let mut default_size = self.settings.default_size_mode;
            row(ui, "Default size mode", "", &mut |ui| {
                if let Some(i) = segmented(
                    ui,
                    &["On disk", "File size"],
                    (default_size == SizeMode::Apparent) as usize,
                ) {
                    default_size = if i == 0 {
                        SizeMode::OnDisk
                    } else {
                        SizeMode::Apparent
                    };
                }
            });
            if default_size != self.settings.default_size_mode {
                self.settings.default_size_mode = default_size;
                self.mark_settings_dirty();
            }
            if ui
                .checkbox(
                    &mut self.settings.remember_last_view,
                    t("Open with the view and size mode last used"),
                )
                .changed()
            {
                self.mark_settings_dirty();
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t("Largest list"));
                if ui
                    .add(
                        egui::DragValue::new(&mut self.settings.largest_limit)
                            .range(10..=1000)
                            .speed(5),
                    )
                    .changed()
                {
                    self.mark_settings_dirty();
                    self.schedule_largest();
                }
                ui.label(caption("rows"));
            });
            ui.add_space(S3);

            ui.label(RichText::new(t("Map options")).family(crate::theme::medium()));
            ui.add_space(4.0);
            if super::usage::map_options_ui(ui, &mut self.settings.flags, Some(self.view)) {
                self.geom = None;
                self.mark_settings_dirty();
            }
            ui.add_space(S3);

            ui.label(RichText::new(t("Export")).family(crate::theme::medium()));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let can = self.tree.is_some() && !self.exporting;
                if ui
                    .add_enabled(can, secondary("Folder contents as CSV…"))
                    .clicked()
                {
                    self.export_children_csv();
                }
                if ui.add_enabled(can, secondary("Largest list as CSV…")).clicked() {
                    self.export_largest_csv();
                }
                if ui
                    .add_enabled(can, secondary("Folder as JSON (one level)…"))
                    .clicked()
                {
                    self.export_shallow_json();
                }
                if ui
                    .add_enabled(self.tree.is_some(), secondary("Copy visible list as TSV"))
                    .clicked()
                {
                    self.copy_listing_tsv();
                }
            });
            ui.add_space(S3);

            ui.label(RichText::new(t("Saved scans")).family(crate::theme::medium()));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t("Keep for"));
                let mut days = self.settings.cache_ttl_days;
                if ui
                    .add(egui::DragValue::new(&mut days).range(0..=3650).speed(1))
                    .on_hover_text(t("Scans older than this are removed at startup; 0 keeps them forever"))
                    .changed()
                {
                    self.settings.cache_ttl_days = days;
                    self.reconfigure_cache();
                    self.mark_settings_dirty();
                }
                ui.label(caption("days · 0 = forever"));
            });
            ui.horizontal(|ui| {
                ui.label(t("Size budget"));
                let mut mb = self.settings.cache_max_mb;
                if ui
                    .add(egui::DragValue::new(&mut mb).range(0..=100_000).speed(10))
                    .on_hover_text(t("Least recently opened scans are removed once the cache is larger; 0 = no limit"))
                    .changed()
                {
                    self.settings.cache_max_mb = mb;
                    self.reconfigure_cache();
                    self.mark_settings_dirty();
                }
                ui.label(caption("MB · 0 = no limit"));
            });
            ui.horizontal(|ui| {
                ui.label(t("On disk now"));
                match self.cache_bytes {
                    Some(bytes) => {
                        ui.label(crate::i18n::bytes(bytes));
                        ui.label(caption(crate::i18n::count(self.recent.len(), "result")));
                    }
                    None => {
                        ui.spinner();
                    }
                }
                if ui
                    .add_enabled(
                        self.cache_bytes.is_some_and(|b| b > 0) && self.scan.is_none() && !self.busy,
                        secondary("Clear saved scans"),
                    )
                    .on_hover_text(t("Removes every saved result; the open scan stays"))
                    .clicked()
                {
                    self.cache_bytes = None;
                    self.clear_cache();
                }
            });
            ui.add_space(S4);
            divider(ui);
            ui.add_space(S3);
            ui.horizontal(|ui| {
                if ui.add(secondary("Cleanup rules…")).clicked() {
                    open_rules = true;
                }
                if ui.add(ghost("Keyboard shortcuts  ?")).clicked() {
                    open_keys = true;
                }
            });
            ui.add_space(S3);
            let version = env!("CARGO_PKG_VERSION");
            let about = crate::tr!("Space Scanner {} · {}", version, space_clean::host_platform_name());
            ui.label(caption(about));
        });
        if close || resp.should_close() {
            self.show_settings = false;
        }
        if open_rules {
            self.show_settings = false;
            self.open_rules_panel();
        }
        if open_keys {
            self.show_settings = false;
            self.show_shortcuts = true;
        }
    }

    fn shortcuts_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut close = false;
        let resp = modal("shortcuts", ctx, 640.0).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 112.0).min(600.0));
            ui.label(strong("Keyboard shortcuts", type_scale::TITLE));
            ui.add_space(S3);
            egui::ScrollArea::vertical()
                .max_height((ctx.screen_rect().height() - 220.0).max(120.0))
                .show(ui, |ui| {
                    egui::Grid::new("shortcut_grid")
                        .num_columns(2)
                        .spacing([S6, S2])
                        .show(ui, |ui| {
                            for (keys, action) in [
                                ("⌘O", "Choose a folder to scan"),
                                ("⌘R", "Rescan"),
                                ("⌘⇧R", "Rescan the selected folder"),
                                ("⌘F  or  /", "Search"),
                                ("↑ ↓", "Move through the list"),
                                ("→ ←", "Open or close a folder in the list"),
                                ("Enter", "Open a folder, or show a file in Finder"),
                                ("Space", "Add to the Trash list, or take out"),
                                ("Shift-click", "Add a range to the Trash list"),
                                ("⌘-click", "Add one item to the Trash list"),
                                ("Esc", "Close details, clear the search, then go up"),
                                ("⌘← ⌘→", "Back / forward"),
                                ("Home", "Jump to the scan root"),
                                ("End", "Select the largest item"),
                                ("Option-click an arrow", "Expand a whole branch"),
                                ("Double-click", "Open a folder"),
                                ("⌘1–4", "Switch the map style"),
                                ("⌘J", "Cleanup"),
                                ("⌘⌫", "Open the Trash list with the selection"),
                                ("⌘⇧C", "Copy the path"),
                                ("⌘,", "Settings"),
                                ("?", "This list"),
                            ] {
                                ui.label(RichText::new(t(keys)).monospace().color(ACCENT()));
                                ui.label(t(action));
                                ui.end_row();
                            }
                        });
                });
            ui.add_space(S3);
            ui.label(caption(
                "Esc closes the innermost panel first. It never switches pages.",
            ));
            ui.add_space(S3);
            if ui.add(secondary("Close")).clicked() {
                close = true;
            }
        });
        if close || resp.should_close() {
            self.show_shortcuts = false;
        }
    }

    /// Confirmation lists every path that moves, with sizes, and says what is skipped.
    fn trash_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_delete {
            return;
        }
        let mut cancel = false;
        let mut go = false;
        let busy = self.delete_progress.is_some() || self.junk.cleaning;
        let resp = modal("trash_confirm", ctx, 600.0).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 112.0).min(552.0));
            if let Some(progress) = self
                .delete_progress
                .clone()
                .or_else(|| self.junk.cleaning.then(|| self.junk.clean_progress.clone()))
            {
                let fraction = if progress.items_total > 0 {
                    progress.items_done as f32 / progress.items_total as f32
                } else {
                    0.0
                };
                ui.label(strong(
                    crate::tr!(
                        "Moving to Trash · {}/{} items",
                        progress.items_done,
                        progress.items_total
                    ),
                    type_scale::LEAD,
                ));
                ui.label(caption(crate::tr!(
                    "{} files · {} folders · {} · {} errors",
                    progress.files_done,
                    progress.folders_done,
                    crate::i18n::bytes(progress.bytes_processed),
                    progress.error_count,
                )));
                ui.add_space(S3);
                ui.add(egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).show_percentage());
                if !progress.current_path.as_os_str().is_empty() {
                    let current = progress.current_path.display().to_string();
                    ui.add(
                        egui::Label::new(
                            RichText::new(&current)
                                .size(type_scale::CAPTION)
                                .monospace()
                                .color(MUTED()),
                        )
                        .truncate(),
                    )
                    .on_hover_text(current);
                }
                return;
            }

            let mut items: Vec<(std::path::PathBuf, u64)> = self.basket_items().as_ref().clone();
            items.sort_by_key(|item| std::cmp::Reverse(item.1));
            let (protected, items): (Vec<_>, Vec<_>) = items
                .into_iter()
                .partition(|(p, _)| space_clean::safety::is_never_touch(p));
            let bytes = items.iter().fold(0_u64, |s, i| s.saturating_add(i.1));
            ui.label(strong(
                crate::tr!("Move {} to the Trash?", count_noun(items.len(), "item")),
                type_scale::TITLE,
            ));
            let (from_disk, from_junk) = self.trash_source_counts();
            let mut sources = Vec::new();
            if from_disk > 0 {
                sources.push(crate::tr!("{} from Disk usage", from_disk));
            }
            if from_junk > 0 {
                sources.push(crate::tr!("{} from Cleanup", from_junk));
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(strong(crate::i18n::bytes(bytes), type_scale::BODY));
                ui.label(RichText::new(format!("· {}", sources.join(", "))).color(MUTED()));
            });
            if !protected.is_empty() {
                ui.add_space(S2);
                egui::Frame::new()
                    .fill(DANGER().gamma_multiply(0.1))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(S3 as i8, S2 as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            crate::icons::show(ui, crate::icons::Icon::Lock, 14.0, DANGER());
                            let first = protected[0].0.display().to_string();
                            ui.add(
                                egui::Label::new(
                                    RichText::new(crate::tr!(
                                        "{} protected skipped: {}",
                                        count_noun(protected.len(), "item"),
                                        first
                                    ))
                                    .color(DANGER()),
                                )
                                .truncate(),
                            );
                        });
                    });
            }
            ui.add_space(S3);
            egui::Frame::new()
                .fill(ctx.style().visuals.extreme_bg_color)
                .stroke(egui::Stroke::new(1.0_f32, crate::theme::BORDER()))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(S3 as i8, 4))
                .show(ui, |ui| {
                    let h = (ctx.screen_rect().height() - 340.0).clamp(96.0, 264.0);
                    egui::ScrollArea::vertical()
                        .max_height(h)
                        .auto_shrink([false, true])
                        .show_rows(ui, 30.0, items.len(), |ui, range| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (path, size) in &items[range] {
                                path_size_row(
                                    ui,
                                    &path.display().to_string(),
                                    &crate::i18n::bytes(*size),
                                    30.0,
                                    0.0,
                                );
                            }
                        });
                });
            ui.add_space(S2);
            ui.label(caption(crate::tr!(
                "All {} paths are listed. You can put them back from the Trash until you empty it.",
                items.len()
            )));
            ui.add_space(S4);
            ui.horizontal(|ui| {
                ui.label(caption("Enter moves · Esc cancels"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(!items.is_empty(), primary("Move to Trash"))
                        .clicked()
                    {
                        go = true;
                    }
                    if ui.add(secondary("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        });
        if !busy && (cancel || resp.should_close()) {
            self.confirm_delete = false;
        }
        if go {
            self.apply_basket();
        }
    }

    fn snapshots_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_tm_panel {
            return;
        }
        let mut open = true;
        let resp = modal("snapshots", ctx, 500.0)
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 80.0).min(460.0));
                ui.label(
                    strong("Local snapshots", type_scale::TITLE),
                );
                ui.add_space(S2);
                let tm_n = self.tm_snapshots.iter().filter(|s| s.is_tm()).count();
                let other_n = self.tm_snapshots.len().saturating_sub(tm_n);
                ui.label(caption(
                    "Snapshots hold disk space that never appears as ordinary files. Deleting old Time Machine copies can release it.",
                ));
                ui.add_space(S3);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tm_n.to_string())
                            .size(type_scale::TITLE)
                            .family(crate::theme::bold())
                            .color(ACCENT()),
                    );
                    ui.label("Time Machine");
                    if other_n > 0 {
                        ui.label(caption(crate::tr!("· {} macOS / other", other_n)));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.tm_loading && !self.busy, ghost("Refresh"))
                            .clicked()
                        {
                            self.refresh_snapshots();
                        }
                    });
                });
                ui.add_space(S2);
                if self.tm_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(caption("Loading…"));
                    });
                } else if self.tm_snapshots.is_empty() {
                    ui.label(caption("No local snapshots."));
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for snap in &self.tm_snapshots {
                                egui::Frame::new()
                                    .fill(PANEL_2())
                                    .corner_radius(8)
                                    .inner_margin(egui::Margin::symmetric(S3 as i8, S2 as i8))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal_wrapped(|ui| {
                                            ui.vertical(|ui| {
                                                ui.label(RichText::new(snap.label()).family(crate::theme::bold()));
                                                ui.label(caption(snap.kind_label()));
                                            });
                                            ui.with_layout(
                                                Layout::right_to_left(Align::Center),
                                                |ui| {
                                                    let (text, color) = if !snap.is_tm() {
                                                        ("left alone", MUTED())
                                                    } else if snap.purgeable {
                                                        ("macOS can drop this", MUTED())
                                                    } else {
                                                        ("keeps disk used", ACCENT())
                                                    };
                                                    ui.label(caption(text).color(color));
                                                },
                                            );
                                        });
                                    });
                                ui.add_space(4.0);
                            }
                        });
                }
                ui.add_space(S3);
                divider(ui);
                ui.add_space(S3);
                let keep = usize::from(tm_n > 1);
                let remove = tm_n.saturating_sub(keep);
                ui.horizontal(|ui| {
                    if ui.add(secondary("Close")).clicked() {
                        self.show_tm_panel = false;
                        self.confirm_tm_clean = false;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let can = remove > 0 && !self.busy && !self.tm_loading;
                        let label = if tm_n <= 1 {
                            "Delete snapshot…".to_string()
                        } else {
                            crate::tr!("Delete {} old…", remove)
                        };
                        if ui.add_enabled(can, secondary(&label)).clicked() {
                            self.confirm_tm_clean = true;
                        }
                    });
                });
                if self.confirm_tm_clean {
                    ui.add_space(S3);
                    egui::Frame::new()
                        .fill(PANEL_2())
                        .stroke(egui::Stroke::new(1.0_f32, DANGER().gamma_multiply(0.6)))
                        .corner_radius(10)
                        .inner_margin(egui::Margin::same(S3 as i8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(if keep == 0 {
                                    "Permanently delete this snapshot?".to_string()
                                } else {
                                    crate::tr!("Permanently delete {} oldest, keep the newest?", remove)
                                })
                                .family(crate::theme::bold()),
                            );
                            ui.label(caption(
                                "Snapshots do not go to Trash — this cannot be undone. macOS update snapshots are left alone.",
                            ));
                            ui.add_space(S2);
                            ui.horizontal(|ui| {
                                if ui.add(secondary("Cancel")).clicked() {
                                    self.confirm_tm_clean = false;
                                }
                                if ui
                                    .add(danger("Delete permanently"))
                                    .clicked()
                                {
                                    self.apply_tm_clean();
                                }
                            });
                        });
                }
            });
        if resp.should_close() {
            open = false;
        }
        if !open {
            self.show_tm_panel = false;
            self.confirm_tm_clean = false;
        }
    }

    fn rules_dialog(&mut self, ctx: &egui::Context) {
        if !self.rules.open {
            return;
        }
        let mut open = true;
        let mut reveal: Option<String> = None;
        egui::Window::new(t("Cleanup rules"))
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .default_size([
                (ctx.screen_rect().width() - 48.0).min(1040.0),
                (ctx.screen_rect().height() - 80.0).min(640.0),
            ])
            .min_width(640.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 12.0])
            .show(ctx, |ui| {
                if self.rule_mgr.is_none() && self.rules.error.is_none() {
                    self.start_rule_mgr_load();
                }
                if let Some(mgr) = self.rule_mgr.as_mut() {
                    match rules_panel::draw(ui, &mut self.rules, mgr, &mut self.settings.flags) {
                        RulesAction::Close => self.rules.open = false,
                        RulesAction::PrefsChanged => {
                            self.mark_settings_dirty();
                            if self.junk.open {
                                self.start_junk_scan();
                            }
                        }
                        RulesAction::Reveal(path) => reveal = Some(path),
                        RulesAction::None => {}
                    }
                } else if self.rule_mgr_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t("Loading rules…"));
                    });
                    ctx.request_repaint_after(Duration::from_millis(50));
                } else if let Some(err) = &self.rules.error {
                    ui.colored_label(theme::DANGER(), err);
                    if ui.add(secondary("Close")).clicked() {
                        self.rules.open = false;
                    }
                }
            });
        if !open {
            self.rules.open = false;
        }
        if let Some(path) = reveal {
            self.reveal_rule_path(&path);
        }
    }
}
