//! Notifications in the bottom-right corner: what just happened, with an undo
//! or a next step. Every result and error that used to go to a status line
//! nobody could see lands here, at every width and on every page.

use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};
use space_core::NodeId;

use crate::app::{Mode, SpaceApp};
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::type_scale;
use crate::theme::{ACCENT, BORDER, DANGER, MUTED, PANEL_2, SAFE, TEXT};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum ToastLevel {
    Success,
    Info,
    Error,
}

/// The one button a notification may carry.
#[derive(Clone, Debug)]
pub(in crate::app) enum ToastAction {
    /// Flip these Trash list memberships back.
    UndoQueue(Vec<NodeId>),
    /// Rescan the open result.
    Rescan,
    /// Go to the Trash list page.
    OpenTrashList,
}

impl ToastAction {
    fn label(&self) -> &'static str {
        match self {
            Self::UndoQueue(_) => "Undo",
            Self::Rescan => "Rescan",
            Self::OpenTrashList => "Open Trash list",
        }
    }
}

struct Toast {
    id: u64,
    level: ToastLevel,
    text: String,
    action: Option<ToastAction>,
    shown: Instant,
}

impl Toast {
    fn lifetime(&self) -> Duration {
        match self.level {
            ToastLevel::Error => Duration::from_secs(12),
            _ if self.action.is_some() => Duration::from_secs(7),
            _ => Duration::from_secs(4),
        }
    }
}

#[derive(Default)]
pub(in crate::app) struct Toasts {
    items: Vec<Toast>,
    next: u64,
}

/// At most this many stay on screen; older ones give way.
const MAX_TOASTS: usize = 3;

impl Toasts {
    pub(in crate::app) fn push(
        &mut self,
        level: ToastLevel,
        text: String,
        action: Option<ToastAction>,
    ) {
        // Repeating the same message refreshes it instead of stacking copies.
        self.items.retain(|t| t.text != text);
        self.next += 1;
        self.items.push(Toast {
            id: self.next,
            level,
            text,
            action,
            shown: Instant::now(),
        });
        if self.items.len() > MAX_TOASTS {
            self.items.remove(0);
        }
    }
}

impl SpaceApp {
    pub(super) fn toasts(&mut self, ctx: &egui::Context) {
        // Pointer over a toast keeps it; time only runs while it is not being read.
        let hovering = ctx
            .layer_id_at(ctx.pointer_hover_pos().unwrap_or_default())
            .is_some_and(|l| l.id == egui::Id::new("toasts"));
        if hovering {
            for toast in &mut self.toasts.items {
                toast.shown = Instant::now();
            }
        }
        self.toasts
            .items
            .retain(|t| t.shown.elapsed() < t.lifetime());
        if self.toasts.items.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(250));

        let screen = ctx.screen_rect();
        let width = (screen.width() - 32.0).clamp(240.0, 460.0);
        let mut run: Option<ToastAction> = None;
        let mut close: Option<u64> = None;
        egui::Area::new(egui::Id::new("toasts"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -44.0])
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_max_width(width);
                ui.spacing_mut().item_spacing.y = 8.0;
                for toast in &self.toasts.items {
                    let (icon, color) = match toast.level {
                        ToastLevel::Success => (Icon::Check, SAFE()),
                        ToastLevel::Info => (Icon::Alert, MUTED()),
                        ToastLevel::Error => (Icon::Alert, DANGER()),
                    };
                    egui::Frame::new()
                        .fill(PANEL_2())
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            if toast.level == ToastLevel::Error {
                                DANGER().gamma_multiply(0.6)
                            } else {
                                BORDER()
                            },
                        ))
                        .corner_radius(crate::theme::metrics::RADIUS_CARD)
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 6,
                            top: 8,
                            bottom: 8,
                        })
                        .shadow(ctx.style().visuals.popup_shadow)
                        .show(ui, |ui| {
                            ui.set_width(width - 18.0);
                            ui.horizontal(|ui| {
                                icons::show(ui, icon, 16.0, color);
                                let reserve = if toast.action.is_some() { 120.0 } else { 36.0 };
                                ui.scope(|ui| {
                                    ui.set_max_width((ui.available_width() - reserve).max(80.0));
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&toast.text)
                                                .size(type_scale::BODY)
                                                .color(TEXT()),
                                        )
                                        .wrap(),
                                    );
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if icons::button(ui, Icon::Close, 26.0, "Dismiss", true)
                                            .clicked()
                                        {
                                            close = Some(toast.id);
                                        }
                                        if let Some(action) = &toast.action {
                                            let b = egui::Button::new(
                                                RichText::new(t(action.label()))
                                                    .color(ACCENT())
                                                    .family(crate::theme::medium()),
                                            )
                                            .frame(false);
                                            if ui.add(b).clicked() {
                                                run = Some(action.clone());
                                                close = Some(toast.id);
                                            }
                                        }
                                    },
                                );
                            });
                        });
                }
            });
        if let Some(id) = close {
            self.toasts.items.retain(|t| t.id != id);
        }
        match run {
            Some(ToastAction::UndoQueue(ids)) => self.undo_queue(&ids),
            Some(ToastAction::Rescan) => self.rescan_current(),
            Some(ToastAction::OpenTrashList) => self.set_mode(Mode::Review),
            None => {}
        }
    }
}
