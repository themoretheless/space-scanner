//! Scan page: each skin draws its own start page (`start_skins.rs`) — disk
//! rings for Map, a disk sidebar and Continue card for Columns, a table of
//! disks and saved results for Table. Cleanup stays a quiet secondary entry.

use eframe::egui;

use crate::app::SpaceApp;

impl SpaceApp {
    pub(super) fn scan_page(&mut self, ctx: &egui::Context) {
        match crate::theme::skin() {
            crate::theme::Skin::Map => self.start_map(ctx),
            crate::theme::Skin::Columns => self.start_columns(ctx),
            crate::theme::Skin::Table => self.start_table(ctx),
        }
        // A folder dragged over the window can be dropped anywhere to scan it.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop_tint")))
                .rect_filled(ctx.screen_rect(), 0.0, crate::theme::ACCENT().gamma_multiply(0.11));
        }
    }
}
