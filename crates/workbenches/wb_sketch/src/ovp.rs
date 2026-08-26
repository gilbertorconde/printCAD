//! On-view parameters: live dimension readouts near the cursor and
//! type-to-constrain numeric entry while a drawing tool is mid-flight.

use core_document::{KeyCode, ScreenSpaceLabel};

use crate::geom2d;
use crate::sketch::{AxisDirection, Circle, ConstraintKind, GeometryElement, Point, Sketch, Vec2D};
use crate::snap::SnapTarget;
use crate::tools::ToolState;

/// Untyped (cursor-implied) readout.
const COLOR_READOUT: [f32; 3] = [0.72, 0.72, 0.72];
/// Field with typed input (overrides the cursor on commit).
const COLOR_TYPED: [f32; 3] = [1.0, 0.82, 0.3];
const READOUT_SIZE: f32 = 13.0;
const READOUT_OFFSET_PX: [f32; 2] = [22.0, -12.0];
const READOUT_ROW_PX: f32 = 18.0;

/// A dimension field the active tool exposes in its current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Length,
    Angle,
    Width,
    Height,
    Diameter,
    Radius,
    MajorRadius,
    MinorRadius,
}

impl FieldKind {
    fn short(self) -> &'static str {
        match self {
            FieldKind::Length => "L",
            FieldKind::Angle => "∠",
            FieldKind::Width => "W",
            FieldKind::Height => "H",
            FieldKind::Diameter => "⌀",
            FieldKind::Radius => "R",
            FieldKind::MajorRadius => "a",
            FieldKind::MinorRadius => "b",
        }
    }

    fn is_angle(self) -> bool {
        matches!(self, FieldKind::Angle)
    }
}

/// The dimension fields a tool state exposes (empty when nothing is
/// pending, i.e. the tool is between shapes).
pub fn fields_for(state: &ToolState) -> &'static [FieldKind] {
    match state {
        ToolState::LineFrom { .. } => &[FieldKind::Length, FieldKind::Angle],
        ToolState::RectFrom { .. } | ToolState::RectCenterAt { .. } => {
            &[FieldKind::Width, FieldKind::Height]
        }
        ToolState::CircleFrom { .. } | ToolState::Circle3Two { .. } => &[FieldKind::Diameter],
        ToolState::ArcCenter { .. } | ToolState::PolygonCenter { .. } => &[FieldKind::Radius],
        ToolState::EllipseCenter { .. } => &[FieldKind::MajorRadius],
        ToolState::EllipseMajor { .. } => &[FieldKind::MinorRadius],
        ToolState::SlotFrom { .. } | ToolState::ArcSlotCenter { .. } => &[FieldKind::Length],
        _ => &[],
    }
}

/// A typed value whose geometry (and constraint target) only materializes
/// on a later click of the same tool.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PendingDim {
    ArcRadius(f32),
    ArcSlotLength(f32),
}

/// Per-field typed buffers + focus for the active tool's dimension fields.
#[derive(Debug, Default)]
pub struct DimCapture {
    fields: Vec<FieldKind>,
    buffers: Vec<String>,
    focus: usize,
    pending: Option<PendingDim>,
}

impl DimCapture {
    /// Align the capture with the tool state: a different field set drops
    /// the typed buffers; returning to Idle drops any deferred value too.
    pub fn sync(&mut self, state: &ToolState) {
        let fields = fields_for(state);
        if self.fields != fields {
            self.fields = fields.to_vec();
            self.buffers = vec![String::new(); fields.len()];
            self.focus = 0;
        }
        if state.is_idle() {
            self.pending = None;
        }
    }

    pub fn is_active(&self) -> bool {
        !self.fields.is_empty()
    }

    pub fn has_typed_input(&self) -> bool {
        self.buffers.iter().any(|b| !b.is_empty())
    }

    pub fn clear_buffers(&mut self) {
        for b in &mut self.buffers {
            b.clear();
        }
        self.focus = 0;
    }

    fn cycle_focus(&mut self) {
        if !self.fields.is_empty() {
            self.focus = (self.focus + 1) % self.fields.len();
        }
    }

    /// Route a key into the focused buffer; `true` when consumed.
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        if self.fields.is_empty() {
            return false;
        }
        if key == KeyCode::Tab {
            self.cycle_focus();
            return true;
        }
        let buffer = &mut self.buffers[self.focus];
        match key {
            KeyCode::Backspace => {
                if buffer.is_empty() {
                    false
                } else {
                    buffer.pop();
                    true
                }
            }
            KeyCode::Minus => {
                if let Some(rest) = buffer.strip_prefix('-') {
                    *buffer = rest.to_string();
                } else {
                    buffer.insert(0, '-');
                }
                true
            }
            KeyCode::Period | KeyCode::Comma => {
                if !buffer.contains('.') {
                    buffer.push('.');
                }
                true
            }
            _ => match digit_of(key) {
                Some(c) => {
                    buffer.push(c);
                    true
                }
                None => false,
            },
        }
    }

    /// Parsed value of every non-empty buffer (display units).
    pub fn typed(&self) -> Vec<(FieldKind, f32)> {
        self.fields
            .iter()
            .zip(&self.buffers)
            .filter_map(|(f, b)| b.trim().parse::<f32>().ok().map(|v| (*f, v)))
            .collect()
    }
}

fn digit_of(key: KeyCode) -> Option<char> {
    Some(match key {
        KeyCode::Key0 => '0',
        KeyCode::Key1 => '1',
        KeyCode::Key2 => '2',
        KeyCode::Key3 => '3',
        KeyCode::Key4 => '4',
        KeyCode::Key5 => '5',
        KeyCode::Key6 => '6',
        KeyCode::Key7 => '7',
        KeyCode::Key8 => '8',
        KeyCode::Key9 => '9',
        _ => return None,
    })
}

fn typed_value(typed: &[(FieldKind, f32)], kind: FieldKind) -> Option<f32> {
    typed.iter().find(|(f, _)| *f == kind).map(|(_, v)| *v)
}

/// Live value of `field` implied by the cursor position (display units:
/// degrees for angles, sketch units otherwise).
pub fn implied_value(
    state: &ToolState,
    sketch: &Sketch,
    cursor: Vec2D,
    field: FieldKind,
) -> Option<f32> {
    let pos = |t: &SnapTarget| t.position(sketch);
    match (state, field) {
        (ToolState::LineFrom { from, .. }, FieldKind::Length) => {
            Some((cursor - pos(from)?).to_glam().length())
        }
        (ToolState::LineFrom { from, .. }, FieldKind::Angle) => {
            let d = (cursor - pos(from)?).to_glam();
            (d.length() > 1e-6).then(|| d.y.atan2(d.x).to_degrees())
        }
        (ToolState::RectFrom { corner }, FieldKind::Width) => {
            Some((cursor.x - pos(corner)?.x).abs())
        }
        (ToolState::RectFrom { corner }, FieldKind::Height) => {
            Some((cursor.y - pos(corner)?.y).abs())
        }
        (ToolState::RectCenterAt { center }, FieldKind::Width) => {
            Some(2.0 * (cursor.x - center.x).abs())
        }
        (ToolState::RectCenterAt { center }, FieldKind::Height) => {
            Some(2.0 * (cursor.y - center.y).abs())
        }
        (ToolState::CircleFrom { center }, FieldKind::Diameter) => {
            Some(2.0 * (cursor - pos(center)?).to_glam().length())
        }
        (ToolState::Circle3Two { a, b }, FieldKind::Diameter) => {
            let c = geom2d::circumcenter(a.to_glam(), b.to_glam(), cursor.to_glam())?;
            Some(2.0 * (a.to_glam() - c).length())
        }
        (ToolState::ArcCenter { center }, FieldKind::Radius)
        | (ToolState::PolygonCenter { center }, FieldKind::Radius)
        | (ToolState::EllipseCenter { center }, FieldKind::MajorRadius)
        | (ToolState::ArcSlotCenter { center }, FieldKind::Length) => {
            Some((cursor - pos(center)?).to_glam().length())
        }
        (ToolState::EllipseMajor { center, major_pos }, FieldKind::MinorRadius) => {
            let c = pos(center)?;
            let major = (*major_pos - c).to_glam();
            (major.length() > 1e-6).then(|| {
                (major / major.length())
                    .perp_dot((cursor - c).to_glam())
                    .abs()
            })
        }
        (ToolState::SlotFrom { from }, FieldKind::Length) => {
            Some((cursor - pos(from)?).to_glam().length())
        }
        _ => None,
    }
}

/// Anchor + radial direction from the cursor, at typed distance `r`.
fn radial(anchor: Vec2D, cursor: Vec2D, r: f32) -> Vec2D {
    let d = (cursor - anchor).to_glam();
    let dir = if d.length() > 1e-6 {
        d.normalize()
    } else {
        glam::Vec2::X
    };
    Vec2D::from_glam(anchor.to_glam() + dir * r)
}

fn sign_or(v: f32, fallback: f32) -> f32 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        fallback
    }
}

/// The click position with typed values substituted for the cursor-implied
/// ones (directions and quadrant signs still follow the cursor).
pub fn override_cursor(
    state: &ToolState,
    sketch: &Sketch,
    cursor: Vec2D,
    typed: &[(FieldKind, f32)],
) -> Vec2D {
    let get = |k: FieldKind| typed_value(typed, k);
    let pos = |t: &SnapTarget| t.position(sketch);
    match state {
        ToolState::LineFrom { from, .. } => {
            let Some(a) = pos(from) else { return cursor };
            let d = (cursor - a).to_glam();
            let dir = match get(FieldKind::Angle) {
                Some(deg) => glam::Vec2::from_angle(deg.to_radians()),
                None if d.length() > 1e-6 => d.normalize(),
                None => glam::Vec2::X,
            };
            let length = get(FieldKind::Length)
                .map(f32::abs)
                .unwrap_or_else(|| d.length());
            Vec2D::from_glam(a.to_glam() + dir * length)
        }
        ToolState::RectFrom { corner } => {
            let Some(a) = pos(corner) else { return cursor };
            Vec2D::new(
                get(FieldKind::Width)
                    .map(|w| a.x + sign_or(cursor.x - a.x, 1.0) * w.abs())
                    .unwrap_or(cursor.x),
                get(FieldKind::Height)
                    .map(|h| a.y + sign_or(cursor.y - a.y, 1.0) * h.abs())
                    .unwrap_or(cursor.y),
            )
        }
        ToolState::RectCenterAt { center } => Vec2D::new(
            get(FieldKind::Width)
                .map(|w| center.x + sign_or(cursor.x - center.x, 1.0) * 0.5 * w.abs())
                .unwrap_or(cursor.x),
            get(FieldKind::Height)
                .map(|h| center.y + sign_or(cursor.y - center.y, 1.0) * 0.5 * h.abs())
                .unwrap_or(cursor.y),
        ),
        ToolState::CircleFrom { center } => match (pos(center), get(FieldKind::Diameter)) {
            (Some(c), Some(d)) => radial(c, cursor, 0.5 * d.abs()),
            _ => cursor,
        },
        ToolState::ArcCenter { center } | ToolState::PolygonCenter { center } => {
            match (pos(center), get(FieldKind::Radius)) {
                (Some(c), Some(r)) => radial(c, cursor, r.abs()),
                _ => cursor,
            }
        }
        ToolState::EllipseCenter { center } => match (pos(center), get(FieldKind::MajorRadius)) {
            (Some(c), Some(r)) => radial(c, cursor, r.abs()),
            _ => cursor,
        },
        ToolState::EllipseMajor { center, major_pos } => {
            let (Some(c), Some(minor)) = (pos(center), get(FieldKind::MinorRadius)) else {
                return cursor;
            };
            let major = (*major_pos - c).to_glam();
            if major.length() < 1e-6 {
                return cursor;
            }
            let perp = major.perp() / major.length();
            let side = sign_or(perp.dot((cursor - c).to_glam()), 1.0);
            Vec2D::from_glam(c.to_glam() + perp * side * minor.abs())
        }
        ToolState::SlotFrom { from } => match (pos(from), get(FieldKind::Length)) {
            (Some(a), Some(l)) => radial(a, cursor, l.abs()),
            _ => cursor,
        },
        ToolState::ArcSlotCenter { center } => match (pos(center), get(FieldKind::Length)) {
            (Some(c), Some(l)) => radial(c, cursor, l.abs()),
            _ => cursor,
        },
        _ => cursor,
    }
}

/// Wrap degrees into (-180, 180].
fn normalize_deg(deg: f32) -> f32 {
    let mut d = deg % 360.0;
    if d <= -180.0 {
        d += 360.0;
    }
    if d > 180.0 {
        d -= 360.0;
    }
    d
}

fn last_of<'a, T>(
    sketch: &'a Sketch,
    back: usize,
    pick: impl Fn(&'a GeometryElement) -> Option<&'a T>,
) -> Option<&'a T> {
    let n = sketch.geometry.len();
    sketch.geometry.get(n.checked_sub(back)?).and_then(pick)
}

/// After a tool click, add driving constraints for the typed fields (or
/// record a deferred value for shapes that only finish on a later click).
/// `changed` is the tool effect's changed flag. Returns how many
/// constraints were added.
pub fn apply_typed_constraints(
    capture: &mut DimCapture,
    sketch: &mut Sketch,
    state_before: &ToolState,
    state_after: &ToolState,
    changed: bool,
    typed: &[(FieldKind, f32)],
) -> usize {
    let get = |k: FieldKind| typed_value(typed, k);
    let mut added = 0;
    let mut add = |sketch: &mut Sketch, kind: ConstraintKind| {
        sketch.add_constraint(kind);
        added += 1;
    };
    match state_before {
        ToolState::LineFrom { .. } if changed => {
            let Some(line) = last_of(sketch, 1, |g| match g {
                GeometryElement::Line(l) => Some(l),
                _ => None,
            })
            .cloned() else {
                return 0;
            };
            if let Some(len) = get(FieldKind::Length) {
                add(
                    sketch,
                    ConstraintKind::Distance {
                        point1: line.start,
                        point2: line.end,
                        distance: len.abs(),
                    },
                );
            }
            if let Some(deg) = get(FieldKind::Angle) {
                add(
                    sketch,
                    ConstraintKind::AngleToAxis {
                        line: line.id,
                        axis: AxisDirection::Horizontal,
                        angle_rad: normalize_deg(deg).to_radians(),
                    },
                );
            }
        }
        ToolState::RectFrom { .. } | ToolState::RectCenterAt { .. } if changed => {
            // The last four elements are the edges bottom → right → top →
            // left; the diagonal corners are bottom.start and top.start.
            let corners = match (
                last_of(sketch, 4, |g| match g {
                    GeometryElement::Line(l) => Some(l),
                    _ => None,
                }),
                last_of(sketch, 2, |g| match g {
                    GeometryElement::Line(l) => Some(l),
                    _ => None,
                }),
            ) {
                (Some(bottom), Some(top)) => Some((bottom.start, top.start)),
                _ => None,
            };
            let Some((pa, pc)) = corners else { return 0 };
            if let Some(w) = get(FieldKind::Width) {
                add(
                    sketch,
                    ConstraintKind::DistanceX {
                        a: pa,
                        b: Some(pc),
                        value: w.abs(),
                    },
                );
            }
            if let Some(h) = get(FieldKind::Height) {
                add(
                    sketch,
                    ConstraintKind::DistanceY {
                        a: pa,
                        b: Some(pc),
                        value: h.abs(),
                    },
                );
            }
        }
        ToolState::CircleFrom { .. } | ToolState::Circle3Two { .. } if changed => {
            let circle = last_of(sketch, 1, |g| match g {
                GeometryElement::Circle(c) => Some(c),
                _ => None,
            })
            .map(|c| c.id);
            if let (Some(circle), Some(d)) = (circle, get(FieldKind::Diameter)) {
                add(
                    sketch,
                    ConstraintKind::Diameter {
                        circle,
                        diameter: d.abs(),
                    },
                );
            }
        }
        ToolState::ArcCenter { .. } => {
            // The arc only materializes at the end click; carry the radius.
            if matches!(state_after, ToolState::ArcStart { .. }) {
                if let Some(r) = get(FieldKind::Radius) {
                    capture.pending = Some(PendingDim::ArcRadius(r.abs()));
                }
            }
        }
        ToolState::ArcStart { .. } if changed => {
            if let Some(PendingDim::ArcRadius(r)) = capture.pending.take() {
                let arc = last_of(sketch, 1, |g| match g {
                    GeometryElement::Arc(a) => Some(a),
                    _ => None,
                })
                .map(|a| a.id);
                if let Some(circle) = arc {
                    add(sketch, ConstraintKind::Radius { circle, radius: r });
                }
            }
        }
        ToolState::PolygonCenter { center } if changed => {
            // A construction circumcircle carries the driving radius (the
            // polygon tool itself has no circle element).
            if let Some(r) = get(FieldKind::Radius) {
                let center_id = match center {
                    SnapTarget::Existing(id) => *id,
                    SnapTarget::New(p) => {
                        let id = sketch.add_geometry(GeometryElement::Point(Point::new(*p)));
                        sketch.set_construction(id, true);
                        id
                    }
                };
                let circle =
                    sketch.add_geometry(GeometryElement::Circle(Circle::new(center_id, r.abs())));
                sketch.set_construction(circle, true);
                add(
                    sketch,
                    ConstraintKind::Radius {
                        circle,
                        radius: r.abs(),
                    },
                );
            }
        }
        ToolState::SlotFrom { .. } if changed => {
            // The two cap arcs (last two elements) are centered on the slot
            // centerline's endpoints.
            let centers = match (
                last_of(sketch, 2, |g| match g {
                    GeometryElement::Arc(a) => Some(a),
                    _ => None,
                }),
                last_of(sketch, 1, |g| match g {
                    GeometryElement::Arc(a) => Some(a),
                    _ => None,
                }),
            ) {
                (Some(cap_b), Some(cap_a)) => Some((cap_a.center, cap_b.center)),
                _ => None,
            };
            if let (Some((a, b)), Some(l)) = (centers, get(FieldKind::Length)) {
                add(
                    sketch,
                    ConstraintKind::Distance {
                        point1: a,
                        point2: b,
                        distance: l.abs(),
                    },
                );
            }
        }
        ToolState::ArcSlotCenter { .. } => {
            if matches!(state_after, ToolState::ArcSlotStart { .. }) {
                if let Some(l) = get(FieldKind::Length) {
                    capture.pending = Some(PendingDim::ArcSlotLength(l.abs()));
                }
            }
        }
        ToolState::ArcSlotStart { .. } if changed => {
            if let Some(PendingDim::ArcSlotLength(l)) = capture.pending.take() {
                // Outer rail (4th from last) is centered on the slot center;
                // the start cap (last) is centered on the centerline start.
                let pair = match (
                    last_of(sketch, 4, |g| match g {
                        GeometryElement::Arc(a) => Some(a),
                        _ => None,
                    }),
                    last_of(sketch, 1, |g| match g {
                        GeometryElement::Arc(a) => Some(a),
                        _ => None,
                    }),
                ) {
                    (Some(outer), Some(cap_a)) => Some((outer.center, cap_a.center)),
                    _ => None,
                };
                if let Some((c, s)) = pair {
                    add(
                        sketch,
                        ConstraintKind::Distance {
                            point1: c,
                            point2: s,
                            distance: l,
                        },
                    );
                }
            }
        }
        _ => {}
    }
    added
}

/// Readout labels stacked next to the cursor: one row per field, grey live
/// values, highlighted typed buffers, arrow on the focused row.
pub fn readout_labels(
    capture: &DimCapture,
    state: &ToolState,
    sketch: &Sketch,
    cursor: Vec2D,
    cursor_px: [f32; 2],
) -> Vec<ScreenSpaceLabel> {
    let fields = fields_for(state);
    let synced = capture.fields == fields;
    let mut out = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        let buffer = if synced {
            capture.buffers[i].as_str()
        } else {
            ""
        };
        let focused = synced && capture.focus == i;
        let marker = if focused { "▸ " } else { "" };
        let unit = if field.is_angle() { "°" } else { " mm" };
        let (text, color, background) = if buffer.is_empty() {
            let value = implied_value(state, sketch, cursor, *field);
            let shown = match value {
                Some(v) if field.is_angle() => format!("{v:.1}"),
                Some(v) => format!("{v:.2}"),
                None => "—".to_string(),
            };
            let color = if focused { COLOR_TYPED } else { COLOR_READOUT };
            (
                format!("{marker}{} {shown}{unit}", field.short()),
                color,
                focused,
            )
        } else {
            (
                format!("{marker}{} {buffer}{unit}", field.short()),
                COLOR_TYPED,
                true,
            )
        };
        out.push(ScreenSpaceLabel {
            pos: [
                cursor_px[0] + READOUT_OFFSET_PX[0],
                cursor_px[1] + READOUT_OFFSET_PX[1] + i as f32 * READOUT_ROW_PX,
            ],
            text,
            color,
            size: READOUT_SIZE,
            background,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Line, Sketch};

    fn sketch_with_point(pos: Vec2D) -> (Sketch, SnapTarget) {
        let sketch = Sketch::new("t");
        (sketch, SnapTarget::New(pos))
    }

    #[test]
    fn capture_typing_focus_and_clear() {
        let mut cap = DimCapture::default();
        let state = ToolState::LineFrom {
            from: SnapTarget::New(Vec2D::new(0.0, 0.0)),
            chain: false,
        };
        cap.sync(&state);
        assert!(cap.is_active());
        assert!(cap.handle_key(KeyCode::Key2));
        assert!(cap.handle_key(KeyCode::Key5));
        assert!(cap.handle_key(KeyCode::Period));
        assert!(cap.handle_key(KeyCode::Key5));
        assert_eq!(cap.typed(), vec![(FieldKind::Length, 25.5)]);

        // Tab moves the focus to the angle field.
        assert!(cap.handle_key(KeyCode::Tab));
        assert!(cap.handle_key(KeyCode::Key4));
        assert!(cap.handle_key(KeyCode::Key5));
        assert_eq!(
            cap.typed(),
            vec![(FieldKind::Length, 25.5), (FieldKind::Angle, 45.0)]
        );

        // Minus toggles the sign; backspace edits; clear drops everything.
        assert!(cap.handle_key(KeyCode::Minus));
        assert_eq!(cap.typed()[1].1, -45.0);
        assert!(cap.handle_key(KeyCode::Backspace));
        assert_eq!(cap.typed()[1].1, -4.0);
        cap.clear_buffers();
        assert!(!cap.has_typed_input());

        // Idle drops the fields.
        cap.sync(&ToolState::Idle);
        assert!(!cap.is_active());
        assert!(!cap.handle_key(KeyCode::Key1));
    }

    #[test]
    fn line_override_length_angle_and_polar() {
        let (sketch, from) = sketch_with_point(Vec2D::new(0.0, 0.0));
        let state = ToolState::LineFrom { from, chain: false };
        // Length only: direction from the cursor.
        let p = override_cursor(
            &state,
            &sketch,
            Vec2D::new(3.0, 4.0),
            &[(FieldKind::Length, 10.0)],
        );
        assert!((p.x - 6.0).abs() < 1e-4 && (p.y - 8.0).abs() < 1e-4);
        // Angle only: length from the cursor distance.
        let p = override_cursor(
            &state,
            &sketch,
            Vec2D::new(5.0, 0.0),
            &[(FieldKind::Angle, 90.0)],
        );
        assert!(p.x.abs() < 1e-4 && (p.y - 5.0).abs() < 1e-4);
        // Both: exact polar.
        let p = override_cursor(
            &state,
            &sketch,
            Vec2D::new(-1.0, -1.0),
            &[(FieldKind::Length, 2.0), (FieldKind::Angle, 180.0)],
        );
        assert!((p.x + 2.0).abs() < 1e-4 && p.y.abs() < 1e-4);
    }

    #[test]
    fn rect_override_keeps_cursor_quadrant() {
        let (sketch, corner) = sketch_with_point(Vec2D::new(1.0, 1.0));
        let state = ToolState::RectFrom { corner };
        let p = override_cursor(
            &state,
            &sketch,
            Vec2D::new(-3.0, 5.0),
            &[(FieldKind::Width, 4.0), (FieldKind::Height, 2.0)],
        );
        assert!((p.x + 3.0).abs() < 1e-4, "width toward -x: {}", p.x);
        assert!((p.y - 3.0).abs() < 1e-4, "height toward +y: {}", p.y);
    }

    #[test]
    fn implied_values_track_cursor() {
        let (sketch, from) = sketch_with_point(Vec2D::new(0.0, 0.0));
        let state = ToolState::LineFrom { from, chain: false };
        let cursor = Vec2D::new(3.0, 4.0);
        let len = implied_value(&state, &sketch, cursor, FieldKind::Length).unwrap();
        assert!((len - 5.0).abs() < 1e-4);
        let ang = implied_value(&state, &sketch, cursor, FieldKind::Angle).unwrap();
        assert!((ang - 53.1301).abs() < 1e-3);
    }

    #[test]
    fn typed_line_commit_adds_distance_and_angle_constraints() {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(25.0, 0.0))));
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let mut cap = DimCapture::default();
        let before = ToolState::LineFrom {
            from: SnapTarget::Existing(a),
            chain: false,
        };
        let after = ToolState::LineFrom {
            from: SnapTarget::Existing(b),
            chain: true,
        };
        let added = apply_typed_constraints(
            &mut cap,
            &mut sketch,
            &before,
            &after,
            true,
            &[(FieldKind::Length, 25.0), (FieldKind::Angle, 360.0)],
        );
        assert_eq!(added, 2);
        assert!(sketch.constraints.iter().any(|c| matches!(
            c.kind,
            ConstraintKind::Distance { distance, .. } if (distance - 25.0).abs() < 1e-5
        )));
        // 360° normalizes to 0°.
        assert!(sketch.constraints.iter().any(|c| matches!(
            c.kind,
            ConstraintKind::AngleToAxis { angle_rad, axis: AxisDirection::Horizontal, .. }
                if angle_rad.abs() < 1e-5
        )));
    }

    #[test]
    fn normalize_deg_wraps_into_half_open_range() {
        assert!((normalize_deg(270.0) + 90.0).abs() < 1e-5);
        assert!((normalize_deg(-270.0) - 90.0).abs() < 1e-5);
        assert!((normalize_deg(180.0) - 180.0).abs() < 1e-5);
        assert!((normalize_deg(-180.0) - 180.0).abs() < 1e-5);
    }
}
