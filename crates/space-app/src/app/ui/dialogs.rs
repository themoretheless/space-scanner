//! Modal windows: shortcuts, the Move to Trash confirmation, local snapshots, rules.

use crate::i18n::t;
use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{self, Align, Layout, RichText};

use crate::app::SpaceApp;
use crate::rules_panel::{self, RulesAction};
use crate::theme::metrics::{S2, S3, S4, S6};
use crate::theme::type_scale;
use crate::theme::{self, ACCENT, DANGER, MUTED, PANEL_2};
use crate::widgets::{
    caption, count_noun, danger, divider, ghost, path_size_row, primary, secondary,
};

fn window(title: &'static str, ctx: &egui::Context) -> egui::Window<'static> {
    egui::Window::new(title)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(
            egui::Frame::window(&ctx.style())
                .corner_radius(8)
                .inner_margin(egui::Margin::same(S6 as i8)),
        )
}

impl SpaceApp {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        self.shortcuts_dialog(ctx);
        self.goto_dialog(ctx);
        self.trash_dialog(ctx);
        self.snapshots_dialog(ctx);
        self.rules_dialog(ctx);
        self.settings_dialog(ctx);
        self.about_dialog(ctx);
    }

    fn goto_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_goto {
            return;
        }
        let mut close = false;
        let mut go = false;
        window("Go to folder", ctx).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 80.0).min(480.0));
            ui.label(
                RichText::new(t("Go to folder"))
                    .size(type_scale::TITLE)
                    .strong(),
            );
            ui.add_space(S2);
            ui.label(caption(
                "Enter an indexed path or folder on disk (supports ~)",
            ));
            ui.add_space(S2);
            let edit = ui.add_sized(
                [ui.available_width(), theme::metrics::CONTROL_H],
                egui::TextEdit::singleline(&mut self.goto_draft)
                    .hint_text(t("/Users/you/Library/Caches")),
            );
            if self.focus_goto {
                edit.request_focus();
                self.focus_goto = false;
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            ui.add_space(S3);
            ui.horizontal(|ui| {
                if ui.add(secondary("Cancel")).clicked() {
                    close = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(primary("Go")).clicked() {
                        go = true;
                    }
                });
            });
        });
        if go {
            let mut raw = self.goto_draft.trim().to_string();
            if let (Some(rest), Some(home)) = (raw.strip_prefix('~'), std::env::var_os("HOME")) {
                raw = format!("{}{rest}", home.to_string_lossy());
            }
            let path = PathBuf::from(&raw);
            if let Some(id) = self
                .tree
                .as_ref()
                .and_then(|t| crate::app::find_path(t, &path))
            {
                let is_dir = self
                    .tree
                    .as_ref()
                    .and_then(|t| t.try_node(id))
                    .is_some_and(|n| n.is_dir);
                if is_dir {
                    self.navigate(id, true);
                } else {
                    if let Some(parent) = self
                        .tree
                        .as_ref()
                        .and_then(|t| t.try_node(id))
                        .and_then(|n| n.parent)
                    {
                        self.navigate(parent, true);
                    }
                    self.select_single(id);
                }
                self.choosing_target = false;
                self.mode = crate::app::Mode::Explore;
                close = true;
            } else if path.is_dir() {
                self.choosing_target = false;
                self.start_scan(path, false);
                close = true;
            } else {
                self.status = crate::tr!("Folder not found: {}", path.display());
            }
        }
        if close {
            self.show_goto = false;
        }
    }

    fn shortcuts_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut close = false;
        window("Keyboard shortcuts", ctx).show(ctx, |ui| {
            ui.label(
                RichText::new(t("Keyboard shortcuts"))
                    .size(type_scale::TITLE)
                    .strong(),
            );
            ui.set_width((ctx.screen_rect().width() - 80.0).min(620.0));
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
                        ("⌘L", "Go to folder"),
                        ("⌘R", "Rescan everything"),
                        ("⌘⇧R", "Rescan this folder"),
                        ("⌘F  or  /", "Search this scan"),
                        ("↑ ↓", "Move through the tree"),
                        ("⇧↑↓  or  ⇧-click", "Select a range of items"),
                        ("→ ←", "Expand or collapse a folder; ← on a closed one goes to its parent"),
                        ("Space  or  ⌘Y", "Quick Look preview"),
                        ("Delete / ⌫", "Add or remove from Trash basket"),
                        ("⌘⌫", "Move basket to Trash"),
                        ("⌥⌘⌫", "Delete permanently (bypass Trash)"),
                        ("Enter", "Open a folder, show a file in Finder, or confirm"),
                        ("Esc", "Go to the parent folder"),
                        ("⌘← ⌘→", "Back / forward"),
                        ("Home", "Jump to the scan root"),
                        ("⌥-click ▸", "Expand a whole branch"),
                        ("Double-click", "Open a folder"),
                        ("⌘-click", "Select or clear item"),
                        ("⌘1–4", "Switch the map style"),
                        ("⌘J", "Quick clean"),
                        ("⌘⇧C", "Copy the folder path"),
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
                "Esc cancels a scan, closes a panel, clears search, then goes up. It never switches tabs.",
            ));
            ui.add_space(S3);
            if ui.add(secondary("Close")).clicked() {
                close = true;
            }
        });
        if close {
            self.show_shortcuts = false;
        }
    }

    /// Confirmation lists exactly what moves, with sizes and where it goes.
    fn trash_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_delete {
            return;
        }
        let permanent = self.delete_method == space_clean::DeleteMethod::Permanent;
        let title = if permanent {
            "Delete permanently"
        } else {
            "Move to Trash"
        };
        window(title, ctx).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 80.0).min(520.0));
            if let Some(progress) = &self.delete_progress {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                let fraction = progress.fraction();
                let elapsed = self
                    .delete_started
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                let headline = if permanent {
                    crate::tr!(
                        "Deleting permanently · {}/{} items",
                        progress.items_done,
                        progress.items_total
                    )
                } else {
                    crate::tr!(
                        "Moving to Trash · {}/{} items",
                        progress.items_done,
                        progress.items_total
                    )
                };
                ui.label(RichText::new(headline).size(type_scale::LEAD).strong());
                ui.label(caption(crate::tr!(
                    "{} files · {} folders · {} · {} errors · {}s",
                    progress.files_done,
                    progress.folders_done,
                    crate::i18n::bytes(progress.displayed_bytes()),
                    progress.error_count,
                    elapsed
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

            let items = self.active_delete_items();
            let total_bytes = crate::app::basket_total_bytes(&items);
            let prompt = if permanent {
                crate::tr!(
                    "Permanently delete {} ({})?",
                    count_noun(items.len(), "item"),
                    crate::i18n::bytes(total_bytes)
                )
            } else {
                crate::tr!(
                    "Move {} ({}) to Trash?",
                    count_noun(items.len(), "item"),
                    crate::i18n::bytes(total_bytes)
                )
            };
            ui.label(RichText::new(prompt).size(type_scale::LEAD).strong());
            if permanent {
                ui.label(
                    RichText::new(t(
                        "Items bypass Trash and are removed from disk immediately. This cannot be undone.",
                    ))
                    .size(type_scale::CAPTION)
                    .color(crate::theme::DANGER()),
                );
            } else {
                ui.label(caption(
                    "Items go to the macOS Trash and can be restored from there. Disk space is only released after you empty Trash in Finder.",
                ));
            }
            ui.add_space(S2);
            if !self.is_direct_delete() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(S3, 4.0);
                    for (label, n, bytes) in [
                        (
                            "From Quick clean",
                            self.junk.selected_count(),
                            self.junk.selected_bytes(),
                        ),
                        (
                            "From Downloads",
                            self.downloads.selected_count(),
                            self.downloads.selected_bytes(),
                        ),
                        ("From Folders", self.review.len(), self.review_bytes()),
                    ] {
                        if n > 0 {
                            ui.label(caption(crate::tr!(
                                "{}: {} · {}",
                                t(label),
                                n,
                                crate::i18n::bytes(bytes)
                            )));
                        }
                    }
                });
                ui.add_space(S2);
            }
            const ROW_H: f32 = 34.0;
            let mut remove_path: Option<PathBuf> = None;
            egui::Frame::new()
                .fill(PANEL_2())
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(S3 as i8, S2 as i8))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("trash_confirm_list")
                        .max_height((ctx.screen_rect().height() - 280.0).clamp(120.0, 280.0))
                        .show_rows(ui, ROW_H, items.len(), |ui, range| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for item in &items[range] {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), ROW_H),
                                    Layout::right_to_left(Align::Center),
                                    |ui| {
                                        if crate::icons::button(
                                            ui,
                                            crate::icons::Icon::Close,
                                            24.0,
                                            if permanent {
                                                "Remove from delete list"
                                            } else {
                                                "Remove from Trash list"
                                            },
                                            true,
                                        )
                                        .clicked()
                                        {
                                            remove_path = Some(item.path.clone());
                                        }
                                        ui.with_layout(
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                path_size_row(
                                                    ui,
                                                    &item.path.display().to_string(),
                                                    &crate::i18n::bytes(item.bytes),
                                                    ROW_H,
                                                    0.0,
                                                );
                                            },
                                        );
                                    },
                                );
                            }
                        });
                });
            if let Some(path) = remove_path {
                self.remove_from_active_delete(&path);
                if self.active_delete_items().is_empty() {
                    self.confirm_delete = false;
                    self.delete_source = crate::app::DeleteSource::Basket;
                }
            }
            ui.add_space(S4);
            ui.horizontal(|ui| {
                if ui.add(secondary("Cancel")).clicked() {
                    self.confirm_delete = false;
                    self.delete_source = crate::app::DeleteSource::Basket;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if permanent {
                        let label = crate::tr!(
                            "Delete {} permanently",
                            count_noun(items.len(), "item")
                        );
                        if ui.add_enabled(!items.is_empty(), danger(&label)).clicked() {
                            self.apply_basket();
                        }
                    } else if ui
                        .add_enabled(
                            !items.is_empty(),
                            primary(&crate::tr!(
                                "Move {} to Trash",
                                count_noun(items.len(), "item")
                            )),
                        )
                        .clicked()
                    {
                        self.apply_basket();
                    }
                });
            });
        });
    }

    fn snapshots_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_tm_panel {
            return;
        }
        let mut open = true;
        window("Local snapshots", ctx)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 80.0).min(460.0));
                ui.label(
                    RichText::new(t("Local snapshots"))
                        .size(type_scale::TITLE)
                        .strong(),
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
                            .strong()
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
                                                ui.label(RichText::new(snap.label()).strong());
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
                                .strong(),
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
                                    .add(
                                        egui::Button::new(
                                            RichText::new(t("Delete permanently"))
                                                .color(egui::Color32::WHITE)
                                                .strong(),
                                        )
                                        .fill(DANGER())
                                        .corner_radius(theme::metrics::RADIUS_CONTROL)
                                        .min_size(egui::vec2(0.0, theme::metrics::CONTROL_H)),
                                    )
                                    .clicked()
                                {
                                    self.apply_tm_clean();
                                }
                            });
                        });
                }
            });
        if !open {
            self.show_tm_panel = false;
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
                    let report = self
                        .junk
                        .report()
                        .map(|report| (self.junk.report_version(), report));
                    match rules_panel::draw(
                        ui,
                        &mut self.rules,
                        mgr,
                        &mut self.settings.flags,
                        report,
                    ) {
                        RulesAction::Close => self.rules.open = false,
                        RulesAction::PrefsChanged => {
                            self.settings.save();
                            self.start_junk_scan();
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

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut close = false;
        let mut clear_cache = false;
        let mut apply_excludes = false;
        let mut changed = false;

        window("Settings", ctx).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 80.0).min(580.0));
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t("Settings"))
                        .size(type_scale::TITLE)
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(secondary("Close")).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(S3);

            egui::ScrollArea::vertical()
                .id_salt("settings_scroll")
                .max_height((ctx.screen_rect().height() - 200.0).max(160.0))
                .show(ui, |ui| {
                    ui.label(RichText::new(t("Appearance & Accessibility")).strong());
                    ui.add_space(4.0);
                    egui::Grid::new("settings_a11y_grid")
                        .num_columns(2)
                        .spacing([S4, S2])
                        .show(ui, |ui| {
                            ui.label(t("Default map view"));
                            egui::ComboBox::from_id_salt("set_def_view")
                                .selected_text(t(self.settings.default_view.label()))
                                .show_ui(ui, |ui| {
                                    for mode in crate::views::ViewMode::ALL {
                                        if ui
                                            .selectable_label(
                                                self.settings.default_view == mode,
                                                t(mode.label()),
                                            )
                                            .clicked()
                                        {
                                            self.settings.default_view = mode;
                                            changed = true;
                                        }
                                    }
                                });
                            ui.end_row();

                            ui.label(t("Default size accounting"));
                            ui.horizontal(|ui| {
                                for (mode, label) in [
                                    (space_core::SizeMode::OnDisk, "On disk"),
                                    (space_core::SizeMode::Apparent, "File size"),
                                ] {
                                    if ui
                                        .selectable_label(
                                            self.settings.default_size_mode == mode,
                                            t(label),
                                        )
                                        .clicked()
                                    {
                                        self.settings.default_size_mode = mode;
                                        changed = true;
                                    }
                                }
                            });
                            ui.end_row();

                            ui.label(t("Byte units"));
                            ui.horizontal(|ui| {
                                for (unit, label) in [
                                    (crate::settings::ByteUnit::Binary, "IEC (KiB, MiB, GiB)"),
                                    (crate::settings::ByteUnit::Si, "Decimal (KB, MB, GB)"),
                                ] {
                                    if ui
                                        .selectable_label(self.settings.byte_unit == unit, t(label))
                                        .clicked()
                                    {
                                        self.settings.byte_unit = unit;
                                        crate::i18n::set_byte_unit(unit);
                                        self.geom = None;
                                        changed = true;
                                    }
                                }
                            });
                            ui.end_row();

                            ui.label(t("Language"));
                            ui.horizontal(|ui| {
                                for (choice, label) in [
                                    (None, t("Automatic")),
                                    (
                                        Some(crate::i18n::Lang::En),
                                        crate::i18n::Lang::En.native_name(),
                                    ),
                                    (
                                        Some(crate::i18n::Lang::Ru),
                                        crate::i18n::Lang::Ru.native_name(),
                                    ),
                                ] {
                                    if ui
                                        .selectable_label(self.settings.language == choice, label)
                                        .clicked()
                                    {
                                        self.settings.language = choice;
                                        crate::i18n::set(
                                            choice.unwrap_or_else(crate::i18n::detect),
                                        );
                                        changed = true;
                                    }
                                }
                            });
                            ui.end_row();

                            ui.label(t("High contrast"));
                            if ui
                                .checkbox(
                                    &mut self.settings.high_contrast,
                                    t("Higher text and border contrast"),
                                )
                                .changed()
                            {
                                theme::set_high_contrast(self.settings.high_contrast);
                                theme::set_skin(ctx, self.settings.skin);
                                self.geom = None;
                                changed = true;
                            }
                            ui.end_row();

                            ui.label(t("Reduce motion"));
                            if ui
                                .checkbox(
                                    &mut self.settings.reduce_motion,
                                    t("Disable zoom and hover transitions"),
                                )
                                .changed()
                            {
                                theme::apply_motion(ctx, self.settings.reduce_motion);
                                if self.settings.reduce_motion {
                                    self.transition = None;
                                }
                                changed = true;
                            }
                            ui.end_row();
                        });

                    ui.add_space(S3);
                    divider(ui);
                    ui.add_space(S3);

                    ui.label(RichText::new(t("Scanning & Cache")).strong());
                    ui.add_space(4.0);
                    egui::Grid::new("settings_scan_grid")
                        .num_columns(2)
                        .spacing([S4, S2])
                        .show(ui, |ui| {
                            ui.label(t("Small-file rollup threshold (KB)"));
                            let mut kb = self.settings.small_file_threshold_kb as i32;
                            if ui
                                .add(egui::DragValue::new(&mut kb).range(0..=16_384).speed(16))
                                .changed()
                            {
                                self.settings.small_file_threshold_kb = kb.max(0) as u64;
                                changed = true;
                            }
                            ui.end_row();

                            ui.label(t("Scan threads"));
                            ui.horizontal(|ui| {
                                let auto = self.settings.scan_threads == 0;
                                if ui.selectable_label(auto, t("Automatic")).clicked() {
                                    self.settings.scan_threads = 0;
                                    changed = true;
                                }
                                let mut threads = if auto {
                                    4u8
                                } else {
                                    self.settings.scan_threads
                                };
                                if ui.add(egui::Slider::new(&mut threads, 1..=16)).changed() {
                                    self.settings.scan_threads = threads;
                                    changed = true;
                                }
                            });
                            ui.end_row();

                            ui.label(t("Hidden files"));
                            if ui
                                .checkbox(
                                    &mut self.settings.skip_hidden,
                                    t("Skip dotfiles during scan"),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                            ui.end_row();

                            ui.label(t("Cross-mount volumes"));
                            if ui
                                .checkbox(
                                    &mut self.settings.cross_mount,
                                    t("Traverse mounted sub-filesystems"),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                            ui.end_row();

                            ui.label(t("Excluded folder names"));
                            ui.horizontal(|ui| {
                                let edit = ui.add(
                                    egui::TextEdit::singleline(&mut self.exclude_draft)
                                        .desired_width(220.0)
                                        .hint_text("node_modules, target"),
                                );
                                if edit.lost_focus() || ui.add(secondary("Apply")).clicked() {
                                    apply_excludes = true;
                                }
                            });
                            ui.end_row();

                            ui.label(t("Insights row limit"));
                            let mut limit = self.settings.largest_limit as i32;
                            if ui.add(egui::Slider::new(&mut limit, 10..=500)).changed() {
                                self.settings.largest_limit = limit.max(10) as usize;
                                changed = true;
                            }
                            ui.end_row();

                            ui.label(t("Cache lifetime (days)"));
                            ui.horizontal(|ui| {
                                let mut ttl = self.settings.cache_ttl_days as i32;
                                if ui.add(egui::Slider::new(&mut ttl, 1..=365)).changed() {
                                    self.settings.cache_ttl_days = ttl.max(1) as u32;
                                    changed = true;
                                }
                                if ui.add(secondary("Clear all")).clicked() {
                                    clear_cache = true;
                                }
                            });
                            ui.end_row();

                            ui.label(t("Trash confirmation"));
                            if ui
                                .checkbox(
                                    &mut self.settings.confirm_trash,
                                    t("Ask before moving items to Trash"),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                            ui.end_row();
                            ui.label(t("Permanent delete"));
                            if ui
                                .checkbox(
                                    &mut self.settings.confirm_permanent_delete,
                                    t("Ask before permanently deleting items"),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                            ui.end_row();
                        });
                });

            ui.add_space(S3);
            ui.label(caption("Scan options apply on your next scan or rescan."));
        });

        if apply_excludes {
            self.settings.exclude_names = self
                .exclude_draft
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            changed = true;
        }
        if clear_cache {
            self.clear_all_cache();
        }
        if changed {
            self.settings.clamp();
            self.settings.save();
        }
        if close {
            self.show_settings = false;
        }
    }

    fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut close = false;
        window("About Space Scanner", ctx).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 80.0).min(420.0));
            ui.label(
                RichText::new("Space Scanner")
                    .size(type_scale::DISPLAY)
                    .strong(),
            );
            ui.label(caption(crate::tr!("Version {}", env!("CARGO_PKG_VERSION"))));
            ui.add_space(S3);
            ui.label(t(
                "Fast disk space analyzer and safe cleanup tool for macOS.",
            ));
            ui.add_space(S2);
            ui.label(caption(t("License: MIT")));
            ui.label(caption(t(
                "Moves items to Trash by default, with optional fast permanent deletion.",
            )));
            ui.add_space(S4);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(secondary("Close")).clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.show_about = false;
        }
    }
}
