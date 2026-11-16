//! Scan page: the same target and saved-result workflow in every palette.

use eframe::egui;

use crate::app::SpaceApp;

impl SpaceApp {
    pub(super) fn scan_page(&mut self, ctx: &egui::Context) {
        self.start_page(ctx);
        // A folder dragged over the window can be dropped anywhere to scan it.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_tint"),
            ))
            .rect_filled(
                ctx.screen_rect(),
                0.0,
                crate::theme::ACCENT().gamma_multiply(0.11),
            );
        }
    }
}
