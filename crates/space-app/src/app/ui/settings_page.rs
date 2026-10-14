//! Settings window: everything persisted in `settings.rs` that has no other
//! home, grouped by what it affects. Every control applies at once.

use crate::i18n::t;

use eframe::egui::{self, Align, Layout, RichText};
use space_core::SizeMode;

use crate::app::SpaceApp;
use crate::i18n::Lang;
use crate::settings::SizeUnits;
use crate::theme::metrics::{S2, S3, S4, S6};
use crate::theme::type_scale;
use crate::theme::MUTED;
use crate::views::ViewMode;
use crate::widgets::{caption, secondary};

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(S3);
    ui.label(
        RichText::new(t(text).to_uppercase())
            .size(type_scale::CAPTION)
            .color(MUTED()),
    );
    ui.add_space(S2);
}

impl SpaceApp {
    pub(super) fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut dirty = false;
        egui::Window::new(t("Settings"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ctx.style())
                    .corner_radius(8)
                    .inner_margin(egui::Margin::same(S6 as i8)),
            )
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 80.0).min(560.0));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t("Settings")).size(type_scale::TITLE).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if crate::icons::button(ui, crate::icons::Icon::Close, 28.0, "Close", true)
                            .clicked()
                        {
                            self.show_settings = false;
                        }
                    });
                });
                egui::ScrollArea::vertical()
                    .max_height((ctx.screen_rect().height() - 200.0).max(160.0))
                    .show(ui, |ui| {
                        dirty |= self.settings_body(ui);
                    });
                ui.add_space(S3);
                if ui.add(secondary("Close")).clicked() {
                    self.show_settings = false;
                }
            });
        if !open {
            self.show_settings = false;
        }
        if dirty {
            self.mark_settings_dirty();
        }
    }

    /// The controls; returns true when something changed.
    fn settings_body(&mut self, ui: &mut egui::Ui) -> bool {
        let mut dirty = false;
        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([S4, S2])
            .min_col_width(150.0)
            .show(ui, |ui| {
                // --- interface
                ui.label(t("Language"));
                ui.horizontal(|ui| {
                    for (choice, label) in [
                        (None, t("Automatic")),
                        (Some(Lang::En), Lang::En.native_name()),
                        (Some(Lang::Ru), Lang::Ru.native_name()),
                    ] {
                        if ui
                            .selectable_label(self.settings.language == choice, label)
                            .clicked()
                        {
                            self.settings.language = choice;
                            crate::i18n::set(choice.unwrap_or_else(crate::i18n::detect));
                            dirty = true;
                        }
                    }
                });
                ui.end_row();

                ui.label(t("Size units"));
                ui.horizontal(|ui| {
                    for units in SizeUnits::ALL {
                        if ui
                            .selectable_label(self.settings.size_units == units, t(units.label()))
                            .clicked()
                        {
                            self.settings.size_units = units;
                            crate::i18n::set_units(units);
                            dirty = true;
                        }
                    }
                });
                ui.end_row();

                ui.label(t("Skin"));
                ui.horizontal(|ui| {
                    for skin in crate::theme::Skin::ALL {
                        if ui
                            .selectable_label(self.settings.skin == skin, t(skin.label()))
                            .clicked()
                        {
                            self.settings.skin = skin;
                            crate::theme::set_skin(ui.ctx(), skin);
                            self.geom = None;
                            dirty = true;
                        }
                    }
                });
                ui.end_row();

                ui.label(t("Motion"));
                if ui
                    .checkbox(&mut self.settings.reduce_motion, t("Reduce motion"))
                    .on_hover_text(t("No zoom, crossfade or hover animation"))
                    .changed()
                {
                    crate::theme::apply_interaction(
                        ui.ctx(),
                        self.settings.reduce_motion,
                        self.settings.tooltip_delay_ms,
                    );
                    dirty = true;
                }
                ui.end_row();

                ui.label(t("Hint delay"));
                if ui
                    .add(
                        egui::Slider::new(&mut self.settings.tooltip_delay_ms, 0..=1500)
                            .step_by(50.0)
                            .suffix(" ms"),
                    )
                    .on_hover_text(t("How long the pointer rests before a hover hint appears"))
                    .changed()
                {
                    crate::theme::apply_interaction(
                        ui.ctx(),
                        self.settings.reduce_motion,
                        self.settings.tooltip_delay_ms,
                    );
                    dirty = true;
                }
                ui.end_row();
            });

        heading(ui, "Disk usage");
        egui::Grid::new("settings_usage")
            .num_columns(2)
            .spacing([S4, S2])
            .min_col_width(150.0)
            .show(ui, |ui| {
                ui.label(t("Default view"));
                ui.horizontal(|ui| {
                    for mode in ViewMode::ALL {
                        if ui
                            .selectable_label(self.settings.default_view == mode, t(mode.label()))
                            .clicked()
                        {
                            self.settings.default_view = mode;
                            dirty = true;
                        }
                    }
                });
                ui.end_row();

                ui.label(t("Default size mode"));
                ui.horizontal(|ui| {
                    for (mode, label) in [
                        (SizeMode::OnDisk, "On disk"),
                        (SizeMode::Apparent, "File size"),
                    ] {
                        if ui
                            .selectable_label(self.settings.default_size_mode == mode, t(label))
                            .clicked()
                        {
                            self.settings.default_size_mode = mode;
                            dirty = true;
                        }
                    }
                });
                ui.end_row();

                ui.label("");
                if ui
                    .checkbox(
                        &mut self.settings.remember_last_view,
                        t("Open with the view and size mode last used"),
                    )
                    .changed()
                {
                    dirty = true;
                }
                ui.end_row();

                ui.label(t("Largest list"));
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::DragValue::new(&mut self.settings.largest_limit)
                                .range(10..=1000)
                                .speed(5),
                        )
                        .changed()
                    {
                        dirty = true;
                        self.schedule_largest();
                    }
                    ui.label(caption("rows"));
                });
                ui.end_row();
            });

        heading(ui, "Map options");
        if super::usage::map_options_ui(ui, &mut self.settings.flags, Some(self.view)) {
            self.geom = None;
            dirty = true;
        }

        heading(ui, "Saved scans");
        egui::Grid::new("settings_cache")
            .num_columns(2)
            .spacing([S4, S2])
            .min_col_width(150.0)
            .show(ui, |ui| {
                ui.label(t("Keep for"));
                ui.horizontal(|ui| {
                    let mut days = self.settings.cache_ttl_days;
                    if ui
                        .add(egui::DragValue::new(&mut days).range(0..=3650).speed(1))
                        .on_hover_text(t("Scans older than this are removed at startup; 0 keeps them forever"))
                        .changed()
                    {
                        self.settings.cache_ttl_days = days;
                        self.reconfigure_cache();
                        dirty = true;
                    }
                    ui.label(caption("days · 0 = forever"));
                });
                ui.end_row();

                ui.label(t("Size budget"));
                ui.horizontal(|ui| {
                    let mut mb = self.settings.cache_max_mb;
                    if ui
                        .add(egui::DragValue::new(&mut mb).range(0..=100_000).speed(10))
                        .on_hover_text(t("Least recently opened scans are removed once the cache is larger; 0 = no limit"))
                        .changed()
                    {
                        self.settings.cache_max_mb = mb;
                        self.reconfigure_cache();
                        dirty = true;
                    }
                    ui.label(caption("MB · 0 = no limit"));
                });
                ui.end_row();

                ui.label(t("On disk now"));
                ui.horizontal(|ui| {
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
                ui.end_row();
            });
        ui.add_space(S4);
        let version = env!("CARGO_PKG_VERSION");
        ui.label(caption(crate::tr!(
            "Space Scanner {} · {}",
            version,
            space_clean::host_platform_name()
        )));
        dirty
    }
}
