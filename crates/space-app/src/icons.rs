//! Small outline icons drawn with the painter, so they do not depend on glyphs the
//! bundled font lacks and scale with the layout. Drawn on a 24-unit grid.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Scan,
    Usage,
    Cleanup,
    Review,
    Folder,
    File,
    Disk,
    Home,
    Back,
    Forward,
    Up,
    Close,
    ChevronRight,
    ChevronDown,
    Check,
    Lock,
}

/// Draw `icon` filling `rect` (a square) with a 1.7-unit stroke.
pub fn paint(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    let unit = rect.width().min(rect.height()) / 24.0;
    let origin = rect.center() - egui::vec2(12.0, 12.0) * unit;
    let p = |x: f32, y: f32| -> Pos2 { origin + egui::vec2(x, y) * unit };
    let stroke = Stroke::new(1.7 * unit.max(0.6), color);
    let line = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(
            pts.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    let closed = |pts: &[(f32, f32)]| {
        painter.add(Shape::closed_line(
            pts.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    match icon {
        Icon::Scan => {
            painter.circle_stroke(p(12.0, 12.0), 8.0 * unit, stroke);
            painter.circle_stroke(p(12.0, 12.0), 3.0 * unit, stroke);
            line(&[(12.0, 1.5), (12.0, 5.0)]);
            line(&[(12.0, 19.0), (12.0, 22.5)]);
            line(&[(1.5, 12.0), (5.0, 12.0)]);
            line(&[(19.0, 12.0), (22.5, 12.0)]);
        }
        Icon::Usage => {
            painter.circle_stroke(p(12.0, 12.0), 9.0 * unit, stroke);
            painter.circle_stroke(p(12.0, 12.0), 3.5 * unit, stroke);
            line(&[(12.0, 3.0), (12.0, 8.5)]);
            line(&[(15.1, 13.8), (19.8, 16.5)]);
        }
        Icon::Cleanup => {
            // Four-point sparkle plus a small one.
            closed(&[
                (10.0, 3.0),
                (12.0, 9.0),
                (18.0, 11.0),
                (12.0, 13.0),
                (10.0, 19.0),
                (8.0, 13.0),
                (2.0, 11.0),
                (8.0, 9.0),
            ]);
            line(&[(19.0, 3.0), (19.0, 8.0)]);
            line(&[(16.5, 5.5), (21.5, 5.5)]);
        }
        Icon::Review => {
            closed(&[(4.0, 3.0), (20.0, 3.0), (20.0, 21.0), (4.0, 21.0)]);
            line(&[(8.0, 8.0), (16.0, 8.0)]);
            line(&[(8.0, 12.0), (12.0, 12.0)]);
            line(&[(8.0, 16.5), (10.5, 19.0), (16.0, 13.5)]);
        }
        Icon::Folder => {
            closed(&[
                (2.5, 5.0),
                (9.0, 5.0),
                (11.5, 8.0),
                (21.5, 8.0),
                (21.5, 19.5),
                (2.5, 19.5),
            ]);
        }
        Icon::File => {
            closed(&[
                (5.0, 2.5),
                (14.0, 2.5),
                (19.0, 7.5),
                (19.0, 21.5),
                (5.0, 21.5),
            ]);
            line(&[(14.0, 2.5), (14.0, 7.5), (19.0, 7.5)]);
        }
        Icon::Disk => {
            closed(&[(3.0, 14.0), (21.0, 14.0), (21.0, 20.0), (3.0, 20.0)]);
            line(&[(3.0, 14.0), (6.0, 5.0), (18.0, 5.0), (21.0, 14.0)]);
            painter.circle_filled(p(17.0, 17.0), 1.2 * unit, color);
        }
        Icon::Back => {
            line(&[(20.0, 12.0), (4.5, 12.0)]);
            line(&[(11.0, 5.5), (4.5, 12.0), (11.0, 18.5)]);
        }
        Icon::Forward => {
            line(&[(4.0, 12.0), (19.5, 12.0)]);
            line(&[(13.0, 5.5), (19.5, 12.0), (13.0, 18.5)]);
        }
        Icon::Up => {
            line(&[(12.0, 20.0), (12.0, 4.5)]);
            line(&[(5.5, 11.0), (12.0, 4.5), (18.5, 11.0)]);
        }
        Icon::Close => {
            line(&[(6.0, 6.0), (18.0, 18.0)]);
            line(&[(18.0, 6.0), (6.0, 18.0)]);
        }
        Icon::ChevronRight => line(&[(9.0, 5.0), (16.0, 12.0), (9.0, 19.0)]),
        Icon::ChevronDown => line(&[(5.0, 9.0), (12.0, 16.0), (19.0, 9.0)]),
        Icon::Check => line(&[(4.5, 12.5), (10.0, 18.0), (19.5, 6.5)]),
        Icon::Lock => {
            closed(&[(5.0, 11.0), (19.0, 11.0), (19.0, 21.0), (5.0, 21.0)]);
            line(&[(8.0, 11.0), (8.0, 7.5), (9.5, 4.5), (12.0, 3.5), (14.5, 4.5), (16.0, 7.5), (16.0, 11.0)]);
        }
        Icon::Home => {
            line(&[(3.0, 11.5), (12.0, 3.5), (21.0, 11.5)]);
            line(&[(5.5, 10.0), (5.5, 20.5), (18.5, 20.5), (18.5, 10.0)]);
            line(&[(10.0, 20.5), (10.0, 14.5), (14.0, 14.5), (14.0, 20.5)]);
        }
    }
}

/// An icon as a sized widget that reserves its own space.
pub fn show(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    paint(ui.painter(), icon, rect, color);
}

/// A square icon-only button: quiet until hovered. Always give it a hover hint.
pub fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    hint: &str,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), sense);
    let hovered = resp.hovered() && enabled;
    if hovered {
        ui.painter().rect_filled(rect, 8.0, crate::theme::PANEL_2());
    }
    let color = if !enabled {
        crate::theme::MUTED().gamma_multiply(0.45)
    } else if hovered {
        crate::theme::TEXT()
    } else {
        crate::theme::MUTED()
    };
    paint(ui.painter(), icon, rect.shrink(size * 0.22), color);
    resp.on_hover_text(hint)
}
