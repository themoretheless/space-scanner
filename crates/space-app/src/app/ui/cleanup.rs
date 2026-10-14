//! Cleanup: find caches, logs and temporary files. Checking is an explicit
//! action (it takes a while), selecting is separate, and the Trash list beside
//! the table shows where the selection stands.

use eframe::egui::{self, Align, Layout, RichText};

use super::Density;
use crate::app::{Mode, SpaceApp};
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::junk_panel;
use crate::theme::metrics::{RADIUS_CARD, S2, S3, S4, SIDE_W};
use crate::theme::type_scale;
use crate::theme::{BG, BORDER, MUTED, PANEL, PANEL_2, SAFE};
use crate::widgets::{block, caption, ghost, primary, secondary, strong, title};

enum Act {
    Check,
    Rules,
    Prefs,
    SelectSafe(bool),
    Review,
    Snapshots,
}

impl SpaceApp {
    pub(super) fn cleanup_page(&mut self, ctx: &egui::Context, d: Density) {
        let mut act = None;
        let has_report = self.junk.report().is_some();
        let show_summary =
            has_report && ctx.available_rect().width() >= 820.0 && d != Density::Narrow;
        if show_summary {
            egui::SidePanel::right("cleanup_summary")
                .show_separator_line(false)
                .exact_width(SIDE_W + 12.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(PANEL())
                        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                        .inner_margin(egui::Margin::symmetric(20, 24)),
                )
                .show(ctx, |ui| self.selection_summary(ui, &mut act));
        }
        let margin = if d == Density::Narrow { S3 } else { 32.0 };
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()).inner_margin(egui::Margin {
                left: margin as i8,
                right: margin as i8,
                top: 24,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                if !has_report && !self.junk.loading && !self.junk.cleaning {
                    self.cleanup_intro(ui, &mut act);
                    return;
                }
                self.cleanup_header(ui, &mut act);
                ui.add_space(S4);
                if has_report && !self.junk.loading && !self.junk.cleaning {
                    self.safe_banner(ui, &mut act);
                    ui.add_space(S3);
                }
                egui::TopBottomPanel::bottom("cleanup_options")
                    .show_separator_line(false)
                    .frame(egui::Frame::new().inner_margin(egui::Margin {
                        left: 0,
                        right: 0,
                        top: 8,
                        bottom: 0,
                    }))
                    .show_inside(ui, |ui| {
                        self.cleanup_options(ui, &mut act, !show_summary);
                    });
                junk_panel::draw(ui, &mut self.junk);
            });

        match act {
            Some(Act::Check) => self.check_junk(),
            Some(Act::Rules) => self.open_rules_panel(),
            Some(Act::Prefs) => {
                self.mark_settings_dirty();
                if has_report {
                    self.check_junk();
                }
            }
            Some(Act::SelectSafe(on)) => {
                self.junk.select_safe(on);
                if on {
                    self.trash_result = None;
                    self.toasts.push(
                        super::ToastLevel::Success,
                        crate::tr!(
                            "Added {} of safe items to the Trash list",
                            crate::i18n::bytes(self.junk.safe_bytes())
                        ),
                        Some(super::ToastAction::OpenTrashList),
                    );
                }
            }
            Some(Act::Review) => self.set_mode(Mode::Review),
            Some(Act::Snapshots) => self.open_tm_panel(),
            None => {}
        }
    }

    /// Before the first check: what it does, how long it takes, and the button.
    fn cleanup_intro(&mut self, ui: &mut egui::Ui, act: &mut Option<Act>) {
        let h = ui.available_height();
        ui.vertical_centered(|ui| {
            ui.add_space((h * 0.5 - 190.0).max(S4));
            ui.set_max_width(520.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 24.0, PANEL_2());
            icons::paint(
                ui.painter(),
                Icon::Cleanup,
                egui::Rect::from_center_size(rect.center(), egui::vec2(24.0, 24.0)),
                SAFE(),
            );
            ui.add_space(S3);
            ui.label(strong("Find caches, logs and temporary files", type_scale::TITLE + 2.0));
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(t(
                        "Rules for the apps on this computer say what is safe to remove. The check takes up to a minute and removes nothing.",
                    ))
                    .color(MUTED()),
                )
                .wrap(),
            );
            ui.add_space(S4);
            if ui.add(primary("Check for junk  ⌘J").min_size(egui::vec2(180.0, 38.0))).clicked() {
                *act = Some(Act::Check);
            }
            ui.add_space(S2);
            if ui.add(ghost("Rules…")).clicked() {
                *act = Some(Act::Rules);
            }
            ui.add_space(S4);
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - 360.0) / 2.0).max(0.0));
                self.browser_options(ui, act);
            });
        });
    }

    fn cleanup_header(&mut self, ui: &mut egui::Ui, act: &mut Option<Act>) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(title("Cleanup"));
                let when = match self.junk.checked_at {
                    _ if self.junk.loading => {
                        t("Checking caches, logs and temporary files…").to_owned()
                    }
                    Some(at) => crate::tr!(
                        "Checked {} · nothing is removed until you confirm",
                        crate::i18n::age(at.elapsed().as_secs())
                    ),
                    None => t("Nothing is removed until you confirm").to_owned(),
                };
                ui.label(caption(when));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.junk.loading && !self.junk.cleaning,
                        secondary("Check again"),
                    )
                    .clicked()
                {
                    *act = Some(Act::Check);
                }
                if ui
                    .add(ghost("Rules…"))
                    .on_hover_text(t("Turn individual rules on or off"))
                    .clicked()
                {
                    *act = Some(Act::Rules);
                }
            });
        });
    }

    /// The fastest safe win as one button.
    fn safe_banner(&mut self, ui: &mut egui::Ui, act: &mut Option<Act>) {
        let total = self.junk.cleanable_bytes().unwrap_or(0);
        let safe = self.junk.safe_bytes();
        egui::Frame::new()
            .fill(PANEL())
            .stroke(egui::Stroke::new(1.0_f32, BORDER()))
            .corner_radius(RADIUS_CARD)
            .inner_margin(egui::Margin::symmetric(S4 as i8, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - 260.0).max(160.0));
                        ui.label(strong(
                            crate::tr!(
                                "{} in {} can go to the Trash",
                                crate::i18n::bytes(total),
                                crate::i18n::count(self.junk.cleanable_count(), "item")
                            ),
                            type_scale::LEAD,
                        ));
                        ui.label(caption(if safe > 0 {
                            crate::tr!(
                                "{} of it is rebuilt or downloaded again automatically",
                                crate::i18n::bytes(safe)
                            )
                        } else {
                            t("Nothing here is marked safe. Check each item before you select it.")
                                .to_owned()
                        }));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if safe == 0 {
                            return;
                        }
                        if self.junk.all_safe_selected() {
                            if selected_pill(ui, "All safe items selected")
                                .on_hover_text(t("Click to clear"))
                                .clicked()
                            {
                                *act = Some(Act::SelectSafe(false));
                            }
                        } else if ui
                            .add(
                                primary(&crate::tr!(
                                    "Select all safe · {}",
                                    crate::i18n::bytes(safe)
                                ))
                                .min_size(egui::vec2(0.0, 36.0)),
                            )
                            .clicked()
                        {
                            *act = Some(Act::SelectSafe(true));
                        }
                    });
                });
            });
    }

    fn browser_options(&mut self, ui: &mut egui::Ui, act: &mut Option<Act>) {
        let flags = &mut self.settings.flags;
        ui.label(caption("Also check"));
        if ui
            .checkbox(&mut flags.scan_browser_cookies, t("Browser cookies"))
            .on_hover_text(t(
                "Off by default — cleaning cookies signs you out of websites",
            ))
            .changed()
        {
            *act = Some(Act::Prefs);
        }
        if ui
            .checkbox(&mut flags.scan_browser_history, t("Browser history"))
            .changed()
        {
            *act = Some(Act::Prefs);
        }
    }

    fn cleanup_options(&mut self, ui: &mut egui::Ui, act: &mut Option<Act>, with_review: bool) {
        ui.horizontal_wrapped(|ui| {
            self.browser_options(ui, act);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if with_review && self.basket_len() > 0 {
                    let label = crate::tr!(
                        "Review {} · {}",
                        self.basket_len(),
                        crate::i18n::bytes(self.basket_bytes())
                    );
                    if ui.add(primary(&label)).clicked() {
                        *act = Some(Act::Review);
                    }
                }
            });
        });
    }

    /// Where the Trash list stands, with the way to check it.
    fn selection_summary(&mut self, ui: &mut egui::Ui, act: &mut Option<Act>) {
        ui.label(strong("Trash list", type_scale::LEAD));
        ui.add_space(S3);
        for (label, bytes) in [
            (
                crate::tr!("{} from Cleanup", self.junk.selected_count()),
                self.junk.selected_bytes(),
            ),
            (
                crate::tr!("{} from Disk usage", self.review.len()),
                self.review_bytes(),
            ),
        ] {
            egui::Frame::new()
                .fill(PANEL_2())
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(S3 as i8, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(crate::i18n::bytes(bytes)).monospace());
                        });
                    });
                });
            ui.add_space(6.0);
        }
        ui.add_space(S2);
        ui.horizontal(|ui| {
            ui.label(RichText::new(t("Total")).color(MUTED()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(strong(
                    crate::i18n::bytes(self.basket_bytes()),
                    type_scale::PAGE,
                ));
            });
        });
        ui.add_space(S3);
        let n = self.basket_len();
        if ui
            .add_enabled(n > 0, block(ui, primary(&crate::tr!("Review {} items", n))))
            .clicked()
        {
            *act = Some(Act::Review);
        }
        ui.add_space(S2);
        ui.add(
            egui::Label::new(caption(
                "Items go to the Trash. Space returns after you empty it.",
            ))
            .wrap(),
        );
        if cfg!(target_os = "macos") {
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::same(S3 as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.with_layout(Layout::top_down(Align::Min), |ui| {
                            ui.label(
                                RichText::new(t("Local snapshots")).family(crate::theme::medium()),
                            );
                            ui.add(
                                egui::Label::new(caption(
                                    "Time Machine snapshots can hold space a scan cannot see.",
                                ))
                                .wrap(),
                            );
                            ui.add_space(6.0);
                            if ui.add(secondary("Show snapshots…")).clicked() {
                                *act = Some(Act::Snapshots);
                            }
                        });
                    });
            });
        }
    }
}

/// A toggled-on state that can be clicked to turn off: check icon and label, tinted safe.
fn selected_pill(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let g = ui.painter().layout_no_wrap(
        t(label).to_owned(),
        egui::FontId::new(type_scale::BODY, crate::theme::medium()),
        SAFE(),
    );
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(g.size().x + 44.0, 36.0), egui::Sense::click());
    let p = ui.painter();
    p.rect(
        rect,
        crate::theme::metrics::RADIUS_CONTROL,
        SAFE().gamma_multiply(if resp.hovered() { 0.16 } else { 0.1 }),
        egui::Stroke::new(1.0_f32, SAFE().gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
    icons::paint(
        p,
        Icon::Check,
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 18.0, rect.center().y),
            egui::vec2(14.0, 14.0),
        ),
        SAFE(),
    );
    p.galley(
        egui::pos2(rect.left() + 32.0, rect.center().y - g.size().y / 2.0),
        g,
        SAFE(),
    );
    resp
}
