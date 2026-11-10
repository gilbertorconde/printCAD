//! Drawings: a schematic with its measures and callouts, and plotted lines.

use super::*;

/// How wide a hint with a figure grows, in points.
pub(super) const HINT_FIGURE_WIDTH: f32 = 260.0;

/// What the pointer did on a diagram this frame, in the diagram's own
/// coordinates: a press, each move of a drag begun on it, and where the
/// drag or click ended.
pub(super) fn diagram_press(ui: &Ui, wid: &str, drawn: &DiagramResponse) -> Option<PanelEvent> {
    let response = &drawn.response;
    let pressed = response.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed());
    let released = response.drag_stopped() || response.clicked();
    let moved = response.dragged() && response.drag_delta() != egui::Vec2::ZERO;
    if !(pressed || released || moved) {
        return None;
    }
    let pos = response
        .interact_pointer_pos()
        .or_else(|| ui.input(|i| i.pointer.latest_pos()))?;
    let [x, y] = drawn.to_diagram(pos);
    Some(PanelEvent::DiagramPress {
        id: wid.to_string(),
        x,
        y,
        dragging: moved && !pressed,
        released,
    })
}

/// A drawn diagram: what the pointer did on it, and how its space maps
/// to the screen.
pub(super) struct DiagramResponse {
    pub(super) response: egui::Response,
    pub(super) origin: egui::Pos2,
    pub(super) scale: f32,
}

impl DiagramResponse {
    fn to_diagram(&self, pos: egui::Pos2) -> [f32; 2] {
        [
            (pos.x - self.origin.x) / self.scale,
            (self.origin.y - pos.y) / self.scale,
        ]
    }
}

/// The tallest a diagram grows, in pixels; a wide one takes the panel's
/// width instead.
pub(super) const DIAGRAM_MAX_HEIGHT: f32 = 220.0;

/// Draw a [`Widget::Diagram`]: its space fitted to the panel's width,
/// strokes, arrows and text at their pixel sizes whatever the fit.
pub(super) fn diagram(
    ui: &mut Ui,
    width: f32,
    height: f32,
    shapes: &[DiagramShape],
    dimensions: &[Dimension],
    callouts: &[Callout],
) -> DiagramResponse {
    if width <= 0.0 || height <= 0.0 {
        let (rect, response) = ui.allocate_exact_size(egui::Vec2::ZERO, egui::Sense::hover());
        return DiagramResponse {
            response,
            origin: rect.min,
            scale: 1.0,
        };
    }
    let available = ui.available_width().max(1.0);
    let scale = (available / width).min(DIAGRAM_MAX_HEIGHT / height);
    let size = egui::vec2(width * scale, height * scale);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(available, size.y), egui::Sense::click_and_drag());
    let origin = egui::pos2(rect.center().x - size.x / 2.0, rect.bottom());
    let at = |p: [f32; 2]| egui::pos2(origin.x + p[0] * scale, origin.y - p[1] * scale);
    let painter = ui.painter_at(rect);
    let stroke_of = |stroke: DiagramStroke| match stroke {
        DiagramStroke::Outline => egui::Stroke::new(1.5, TEXT1),
        DiagramStroke::Hidden => egui::Stroke::new(1.0, TEXT2),
        DiagramStroke::Axis | DiagramStroke::Thin => egui::Stroke::new(1.0, TEXT3),
        DiagramStroke::Accent => egui::Stroke::new(1.5, ACCENT),
    };
    let dashes = |stroke: DiagramStroke| match stroke {
        DiagramStroke::Hidden => Some((4.0, 3.0)),
        DiagramStroke::Axis => Some((10.0, 4.0)),
        _ => None,
    };
    for shape in shapes {
        match shape {
            DiagramShape::Path {
                points,
                closed,
                stroke,
                fill,
            } => {
                let mut points: Vec<egui::Pos2> = points.iter().map(|p| at(*p)).collect();
                if points.len() < 2 {
                    continue;
                }
                if *fill && points.len() >= 3 {
                    painter.add(egui::Shape::convex_polygon(
                        points.clone(),
                        BG3,
                        egui::Stroke::NONE,
                    ));
                }
                if *closed {
                    points.push(points[0]);
                }
                match dashes(*stroke) {
                    Some((dash, gap)) => {
                        painter.extend(egui::Shape::dashed_line(
                            &points,
                            stroke_of(*stroke),
                            dash,
                            gap,
                        ));
                    }
                    None => {
                        painter.add(egui::Shape::line(points, stroke_of(*stroke)));
                    }
                }
            }
            DiagramShape::Circle {
                center,
                radius,
                stroke,
                fill,
            } => {
                let fill = if *fill {
                    BG3
                } else {
                    egui::Color32::TRANSPARENT
                };
                painter.circle(at(*center), radius * scale, fill, stroke_of(*stroke));
            }
            DiagramShape::Text { at: p, text, mono } => {
                let font = if *mono {
                    ui_kit::theme::mono(FONT_XS)
                } else {
                    ui_kit::theme::sans(FONT_XS)
                };
                painter.text(at(*p), egui::Align2::CENTER_CENTER, text, font, TEXT2);
            }
        }
    }
    for dimension in dimensions {
        let from = at(dimension.from);
        let to = at(dimension.to);
        let run = to - from;
        if run.length() < 0.5 {
            continue;
        }
        let dir = run.normalized();
        // The drawing's left of `from → to` is the screen's right, as y
        // flips; the offset is in the drawing's space.
        let normal = egui::vec2(dir.y, -dir.x);
        let offset = normal * dimension.offset * scale;
        let (p1, p2) = (from + offset, to + offset);
        let color = if dimension.emphasis { ACCENT } else { TEXT2 };
        let stroke = egui::Stroke::new(1.0, color);
        let overshoot = normal * 4.0 * dimension.offset.signum();
        painter.line_segment([from, p1 + overshoot], egui::Stroke::new(1.0, TEXT3));
        painter.line_segment([to, p2 + overshoot], egui::Stroke::new(1.0, TEXT3));
        painter.line_segment([p1, p2], stroke);
        arrow(&painter, p1, dir, color);
        arrow(&painter, p2, -dir, color);
        pill(
            &painter,
            p1 + (p2 - p1) / 2.0,
            &dimension.text,
            dimension.emphasis,
        );
    }
    for callout in callouts {
        let anchor = at(callout.anchor);
        let label = at(callout.at);
        let color = if callout.emphasis { ACCENT } else { TEXT2 };
        painter.line_segment([anchor, label], egui::Stroke::new(1.0, color));
        painter.circle_filled(anchor, 2.5, color);
        pill(&painter, label, &callout.text, callout.emphasis);
    }
    DiagramResponse {
        response,
        origin,
        scale,
    }
}

/// An arrowhead at `tip`, pointing along `dir`.
pub(super) fn arrow(
    painter: &egui::Painter,
    tip: egui::Pos2,
    dir: egui::Vec2,
    color: egui::Color32,
) {
    let side = egui::vec2(-dir.y, dir.x);
    let base = tip + dir * 7.0;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, base + side * 2.5, base - side * 2.5],
        color,
        egui::Stroke::NONE,
    ));
}

/// Text centred at `center` on a rounded backing, so it reads over lines.
pub(super) fn pill(painter: &egui::Painter, center: egui::Pos2, text: &str, emphasis: bool) {
    let color = if emphasis { ACCENT } else { TEXT1 };
    let font = if emphasis {
        ui_kit::theme::mono_medium(FONT_XS)
    } else {
        ui_kit::theme::mono(FONT_XS)
    };
    let galley = painter.layout_no_wrap(text.to_string(), font, color);
    let pad = egui::vec2(PILL_PAD[0], PILL_PAD[1]);
    let rect = egui::Rect::from_center_size(center, galley.size() + pad * 2.0);
    painter.rect_filled(rect, RADIUS_SM, if emphasis { ACCENT_DIM } else { BG2 });
    painter.galley(rect.min + pad, galley, color);
}

/// A [`Widget::Plot`]: a strip of the panel's width, each series a line
/// scaled to its own peak, on the x range of all of them from zero.
pub(super) fn plot(ui: &mut Ui, plot: &bench_api::Plot) {
    let width = ui.available_width();
    let (rect, _) =
        ui.allocate_exact_size(egui::Vec2::new(width, plot.height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, BG1);
    let end = plot
        .series
        .iter()
        .flat_map(|s| s.points.iter().map(|p| p[0]))
        .fold(0.0, f64::max)
        .max(1e-9);
    for series in &plot.series {
        let peak = series.points.iter().map(|p| p[1]).fold(0.0, f64::max);
        if peak <= 0.0 {
            continue;
        }
        let points: Vec<egui::Pos2> = series
            .points
            .iter()
            .map(|[x, y]| {
                egui::pos2(
                    rect.left() + rect.width() * (x / end) as f32,
                    rect.bottom() - 4.0 - (rect.height() - 8.0) * (y / peak) as f32,
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(1.5, tone_color(series.tone)),
        ));
    }
}
