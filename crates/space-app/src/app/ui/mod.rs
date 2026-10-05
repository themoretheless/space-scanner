//! Interface layer. `app.rs` owns state, workers and keys; this module only draws.
//!
//! Layout contract (see docs/UX.md):
//!
//! - wide (>= 1180 pt): contents list + map + inspector side by side;
//! - medium (900–1180): contents list + map, details as a drawer over the map;
//! - narrow (< 900): one pane at a time (List | Map), location and search on a sub-bar.
//!
//! Global chrome is one row; it never wraps.

mod cleanup;
mod dialogs;
mod layouts;
mod review_page;
mod scan;
mod shell;
mod start_skins;
mod usage;

use eframe::egui;

use super::{Mode, SpaceApp};

/// Window-width class that decides which panes are shown together.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Density {
    Narrow,
    Medium,
    Wide,
}

impl Density {
    pub(super) fn of(width: f32) -> Self {
        if width >= 1180.0 {
            Self::Wide
        } else if width >= 900.0 {
            Self::Medium
        } else {
            Self::Narrow
        }
    }
}

impl SpaceApp {
    /// Draw one frame of the whole interface.
    pub(super) fn ui(&mut self, ctx: &egui::Context) {
        let density = Density::of(ctx.screen_rect().width());
        self.dialogs(ctx);
        self.app_bar(ctx, density);
        match self.mode {
            Mode::Home => self.scan_page(ctx),
            Mode::Explore => self.usage_page(ctx, density),
            Mode::Clean => self.cleanup_page(ctx, density),
            Mode::Review => self.review_page(ctx),
        }
    }
}
