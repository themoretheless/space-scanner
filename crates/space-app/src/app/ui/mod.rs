//! Interface layer. `app.rs` owns state, workers and keys; this module only draws.
//!
//! Layout contract (see docs/UX.md):
//!
//! - a sidebar holds the four destinations (Overview, Disk usage, Cleanup,
//!   Trash list) and Settings; below 1000 pt it becomes an icon rail;
//! - Disk usage has a toolbar (location, search, layout), a filter bar shared by
//!   every layout, and a status footer;
//! - wide (>= 1240 pt): contents + map + details side by side;
//! - medium (960–1240): contents + map, details as a drawer;
//! - narrow (< 960): one pane at a time (List | Map), details as a bottom sheet.
//!
//! Appearance (dark / light) and the Disk usage layout are independent settings.

mod cleanup;
mod dialogs;
mod layouts;
mod overview;
mod review_page;
mod scan;
mod shell;
mod toast;
mod usage;

pub(super) use toast::{ToastAction, ToastLevel, Toasts};

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
        if width >= 1240.0 {
            Self::Wide
        } else if width >= 960.0 {
            Self::Medium
        } else {
            Self::Narrow
        }
    }
}

/// The sidebar shows labels from this window width; below it, icons only.
pub(super) const SIDEBAR_LABELS_FROM: f32 = 1000.0;

impl SpaceApp {
    /// Draw one frame of the whole interface.
    pub(super) fn ui(&mut self, ctx: &egui::Context) {
        let density = Density::of(ctx.screen_rect().width());
        self.dialogs(ctx);
        self.toasts(ctx);
        self.sidebar(ctx);
        match self.mode {
            Mode::Home => self.overview_page(ctx),
            Mode::Explore => self.usage_page(ctx, density),
            Mode::Clean => self.cleanup_page(ctx, density),
            Mode::Review => self.review_page(ctx),
        }
        self.drop_tint(ctx);
    }
}
