//! On-view parameters: live dimension readouts near the cursor and
//! type-to-constrain entry while a drawing tool is mid-flight.
//!
//! What a field holds is typed text: a number, or a formula over the
//! document's variables, worked out when it is used. Characters arrive as
//! `WorkbenchInputEvent::Text` (a key's or a paste's); the keys that edit
//! (Tab, Backspace) as key presses.

use core_document::expr::Dim;
use core_document::{Document, KeyCode};

use crate::geom2d;
use crate::sketch::{AxisDirection, ConstraintKind, GeometryElement, Sketch, Vec2D};
use crate::snap::{SnapTarget, arc_angles};
use crate::tools::{PolySegment, ToolParams, ToolState};

/// Where the readout card's top-left corner sits from the cursor, in
/// pixels: right of it, its top a little above.
pub const CARD_OFFSET_PX: [f32; 2] = [22.0, -12.0];

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
    /// How far an arc turns from its start, counter-clockwise.
    Sweep,
    /// Where a shape's first point goes.
    X,
    Y,
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
            FieldKind::Sweep => "Δ",
            FieldKind::X => "X",
            FieldKind::Y => "Y",
        }
    }

    pub fn is_angle(self) -> bool {
        matches!(self, FieldKind::Angle | FieldKind::Sweep)
    }

    /// What a formula in this field must come to.
    fn dim(self) -> Dim {
        if self.is_angle() {
            Dim::ANGLE
        } else {
            Dim::LENGTH
        }
    }
}

/// Whether `tool`, waiting for its first click, takes that point typed.
fn takes_first_point(tool: Option<&str>) -> bool {
    tool.is_some_and(|t| crate::is_draw_tool(t) && t != "sketch.text")
}

/// The dimension fields a tool state exposes; `tool` is the tool in hand,
/// which says what an idle state takes.
pub fn fields_for(state: &ToolState, tool: Option<&str>) -> &'static [FieldKind] {
    use FieldKind::*;
    match state {
        ToolState::Idle if takes_first_point(tool) => &[X, Y],
        ToolState::PolylineFrom {
            segment,
            heading: Some(_),
            ..
        } if *segment != PolySegment::Line => &[Radius, Sweep],
        ToolState::LineFrom { .. }
        | ToolState::PolylineFrom { .. }
        | ToolState::Rect3A { .. }
        | ToolState::RectCenter3At { .. }
        | ToolState::Circle3One { .. }
        | ToolState::Arc3Start { .. }
        | ToolState::Ellipse3A { .. }
        | ToolState::ConicAt { .. } => &[Length, Angle],
        ToolState::RectFrom { .. } | ToolState::RectCenterAt { .. } => &[Width, Height],
        ToolState::Rect3B { .. } | ToolState::RectCenter3Corner { .. } => &[Width],
        ToolState::CircleFrom { .. } | ToolState::Circle3Two { .. } => &[Diameter],
        ToolState::ArcCenter { .. } | ToolState::Arc3End { .. } => &[Radius],
        ToolState::ArcStart { .. } | ToolState::EllipseArcStart { .. } => &[Sweep],
        ToolState::PolygonCenter { .. } => &[Radius, Angle],
        ToolState::SlotFrom { .. } => &[Length, Angle, Width],
        ToolState::ArcSlotCenter { .. } => &[Length],
        ToolState::ArcSlotStart { .. } => &[Sweep, Width],
        ToolState::EllipseCenter { .. } | ToolState::EllipseArcCenter { .. } => &[MajorRadius],
        ToolState::EllipseMajor { .. }
        | ToolState::EllipseArcMajor { .. }
        | ToolState::Ellipse3B { .. } => &[MinorRadius],
        _ => &[],
    }
}

/// Whether values typed in `state` only place the click, never becoming
/// constraints.
pub fn only_places(state: &ToolState) -> bool {
    matches!(
        state,
        ToolState::Idle
            | ToolState::RectCenter3At { .. }
            | ToolState::Circle3One { .. }
            | ToolState::Arc3Start { .. }
            | ToolState::EllipseCenter { .. }
            | ToolState::EllipseMajor { .. }
            | ToolState::Ellipse3A { .. }
            | ToolState::Ellipse3B { .. }
            | ToolState::EllipseArcCenter { .. }
            | ToolState::EllipseArcMajor { .. }
            | ToolState::EllipseArcStart { .. }
            | ToolState::ConicAt { .. }
    )
}

/// A typed value whose geometry (and constraint target) only materializes
/// on a later click of the same tool.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PendingDim {
    /// `constrain` remembers whether the value came in with Enter.
    ArcRadius {
        r: f64,
        constrain: bool,
    },
    ArcSlotLength {
        l: f64,
        constrain: bool,
    },
    /// A three-corner rectangle's first edge.
    Rect3Edge {
        length: Option<f64>,
        angle: Option<f64>,
        constrain: bool,
    },
}

/// Per-field typed buffers + focus for the active tool's dimension fields.
#[derive(Debug, Default)]
pub struct DimCapture {
    fields: Vec<FieldKind>,
    buffers: Vec<String>,
    focus: usize,
    /// The tool state has no shape begun.
    idle: bool,
    pending: Option<PendingDim>,
    /// The way from the state's anchor the cursor last pointed, and that
    /// anchor: once a value is typed it turns only gradually, so the shape
    /// does not flip over as the cursor crosses the anchor.
    heading: Option<(Vec2D, glam::DVec2)>,
}

impl DimCapture {
    /// Align the capture with the tool state: a different field set drops
    /// the typed buffers; returning to Idle drops any deferred value too.
    pub fn sync(&mut self, state: &ToolState, tool: Option<&str>) {
        let fields = fields_for(state, tool);
        if self.fields != fields {
            self.fields = fields.to_vec();
            self.buffers = vec![String::new(); fields.len()];
            self.focus = 0;
        }
        self.idle = state.is_idle();
        if self.idle {
            self.pending = None;
        }
    }

    pub fn is_active(&self) -> bool {
        !self.fields.is_empty()
    }

    pub fn has_typed_input(&self) -> bool {
        self.buffers.iter().any(|b| !b.is_empty())
    }

    /// The focused field holds text: every key that types goes to it.
    pub fn takes_text(&self) -> bool {
        self.buffers.get(self.focus).is_some_and(|b| !b.is_empty())
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

    /// Route a key press into the focused buffer; `true` when consumed.
    /// The characters themselves arrive as text ([`Self::handle_text`]):
    /// the keys that type them are only kept from acting elsewhere.
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        if self.fields.is_empty() {
            return false;
        }
        if key == KeyCode::Tab {
            self.cycle_focus();
            return true;
        }
        let typing = self.takes_text();
        let buffer = &mut self.buffers[self.focus];
        match key {
            // An empty field keeps the key while a shape is begun: it never
            // falls through to deleting the selection.
            KeyCode::Backspace => buffer.pop().is_some() || !self.idle,
            KeyCode::Minus
            | KeyCode::Period
            | KeyCode::Comma
            | KeyCode::Key0
            | KeyCode::Key1
            | KeyCode::Key2
            | KeyCode::Key3
            | KeyCode::Key4
            | KeyCode::Key5
            | KeyCode::Key6
            | KeyCode::Key7
            | KeyCode::Key8
            | KeyCode::Key9 => true,
            KeyCode::Space | KeyCode::Equals | KeyCode::Slash => typing,
            key => typing && is_letter(key),
        }
    }

    /// Text typed or pasted into the focused field; `true` when taken. A
    /// minus on a plain number turns its sign, and a comma there is the
    /// decimal point; in a formula both are what they are.
    pub fn handle_text(&mut self, text: &str) -> bool {
        let Some(buffer) = self.buffers.get_mut(self.focus) else {
            return false;
        };
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => type_char(buffer, c),
            _ => paste(buffer, text),
        }
    }

    /// The value of every non-empty buffer that works out (display units:
    /// degrees for angles, millimetres otherwise).
    pub fn typed(&self, document: &Document) -> Vec<(FieldKind, f64)> {
        self.fields
            .iter()
            .zip(&self.buffers)
            .filter(|(_, b)| !b.trim().is_empty())
            .filter_map(|(f, b)| evaluate(b, *f, document).ok().map(|v| (*f, v)))
            .collect()
    }

    /// Why the first typed buffer that does not work out does not.
    pub fn error(&self, document: &Document) -> Option<String> {
        self.fields
            .iter()
            .zip(&self.buffers)
            .filter(|(_, b)| !b.trim().is_empty())
            .find_map(|(f, b)| {
                evaluate(b, *f, document)
                    .err()
                    .map(|why| format!("{}: {why}", b.trim()))
            })
    }

    /// Follow the cursor's way from the state's anchor. Before anything is
    /// typed it follows freely; after, a move that would turn it more than
    /// a right angle at once (the cursor crossing the anchor) is not taken.
    pub fn track(&mut self, state: &ToolState, sketch: &Sketch, cursor: Vec2D) {
        let Some(anchor) = anchor(state, sketch) else {
            self.heading = None;
            return;
        };
        let d = (cursor - anchor).to_glam();
        if d.length() < 1e-6 {
            return;
        }
        let dir = d.normalize();
        let held = match self.heading {
            Some((at, h)) if at == anchor => Some(h),
            _ => None,
        };
        let heading = match held {
            Some(h) if self.has_typed_input() && dir.dot(h) <= 0.0 => h,
            _ => dir,
        };
        self.heading = Some((anchor, heading));
    }

    /// The cursor as the typed values read it: on the far side of the
    /// anchor from the kept heading, it stands at the same distance along
    /// that heading instead.
    pub fn steer(&self, state: &ToolState, sketch: &Sketch, cursor: Vec2D) -> Vec2D {
        if !self.has_typed_input() {
            return cursor;
        }
        let (Some(anchor), Some((at, h))) = (anchor(state, sketch), self.heading) else {
            return cursor;
        };
        let d = (cursor - anchor).to_glam();
        if at != anchor || d.dot(h) > 0.0 {
            return cursor;
        }
        Vec2D::from_glam(anchor.to_glam() + h * d.length().max(1e-3))
    }
}

fn is_letter(key: KeyCode) -> bool {
    use KeyCode::*;
    matches!(
        key,
        A | B
            | C
            | D
            | E
            | F
            | G
            | H
            | I
            | J
            | K
            | L
            | M
            | N
            | O
            | P
            | Q
            | R
            | S
            | T
            | U
            | V
            | W
            | X
            | Y
            | Z
    )
}

/// Whether `buffer` is a number being typed, rather than a formula.
fn plain(buffer: &str) -> bool {
    buffer
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
}

fn type_char(buffer: &mut String, c: char) -> bool {
    match c {
        '-' if plain(buffer) => match buffer.strip_prefix('-') {
            Some(rest) => *buffer = rest.to_string(),
            None => buffer.insert(0, '-'),
        },
        '.' | ',' if plain(buffer) => {
            if !buffer.contains('.') {
                buffer.push('.');
            }
        }
        c if c.is_control() => return false,
        c if c.is_whitespace() => {
            if !buffer.is_empty() {
                buffer.push(' ');
            }
        }
        c => buffer.push(c),
    }
    true
}

/// A pasted value: one line, put in as it is, a number written with a
/// decimal comma read with a point.
fn paste(buffer: &mut String, text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || text.contains(['\n', '\r']) || text.chars().count() > 120 {
        return false;
    }
    let pointed = text.replace(',', ".");
    if !text.contains('.') && pointed.parse::<f64>().is_ok() {
        buffer.push_str(&pointed);
    } else {
        buffer.push_str(text);
    }
    true
}

/// What `text` in a `kind` field comes to: a number as written, a formula
/// worked out against the document's variables.
fn evaluate(text: &str, kind: FieldKind, document: &Document) -> Result<f64, String> {
    let text = text.trim();
    if let Ok(v) = text.parse::<f64>() {
        return Ok(v);
    }
    document
        .evaluate_formula(text, Some(kind.dim()))
        .map(|q| q.value)
}

fn typed_value(typed: &[(FieldKind, f64)], kind: FieldKind) -> Option<f64> {
    typed.iter().find(|(f, _)| *f == kind).map(|(_, v)| *v)
}

/// The point a state measures lengths and directions from, where the
/// cursor's way from it is what the typed values steer.
fn anchor(state: &ToolState, sketch: &Sketch) -> Option<Vec2D> {
    let pos = |t: &SnapTarget| t.position(sketch);
    match state {
        ToolState::LineFrom { from, .. }
        | ToolState::PolylineFrom {
            from,
            segment: PolySegment::Line,
            ..
        }
        | ToolState::SlotFrom { from } => pos(from),
        ToolState::Rect3A { a } => pos(a),
        ToolState::Arc3Start { start } => pos(start),
        ToolState::Circle3One { a } | ToolState::Ellipse3A { a } => Some(*a),
        ToolState::RectCenter3At { center }
        | ToolState::ConicAt { center, .. }
        | ToolState::CircleFrom { center }
        | ToolState::ArcCenter { center }
        | ToolState::PolygonCenter { center }
        | ToolState::EllipseCenter { center }
        | ToolState::EllipseArcCenter { center }
        | ToolState::ArcSlotCenter { center } => pos(center),
        _ => None,
    }
}

/// Counter-clockwise turn from `from` to `to`, in degrees (0, 360].
fn sweep_deg(from: glam::DVec2, to: glam::DVec2) -> f64 {
    arc_angles(from, to).1.to_degrees()
}

/// A polyline arc's way out of `from` toward `cursor`, and whether it
/// turns left (counter-clockwise).
fn polyline_arc(
    segment: PolySegment,
    heading: Vec2D,
    from: Vec2D,
    cursor: Vec2D,
) -> Option<(glam::DVec2, bool)> {
    let t = segment
        .arc_heading(heading, cursor - from)?
        .to_glam()
        .normalize_or_zero();
    (t != glam::DVec2::ZERO).then(|| (t, t.perp_dot((cursor - from).to_glam()) >= 0.0))
}

/// A point `along` the ellipse centred at `c` with semi-axis `a` along
/// unit `u` and `b` across it, on the ray from `c` through `toward`.
fn on_ellipse_ray(c: Vec2D, u: glam::DVec2, a: f64, b: f64, toward: Vec2D) -> Option<Vec2D> {
    let d = (toward - c).to_glam();
    let (x, y) = (d.dot(u), d.dot(u.perp()));
    let s = ((x / a).powi(2) + (y / b).powi(2)).sqrt();
    (s > 1e-9 && a > 1e-6 && b > 1e-6).then(|| Vec2D::from_glam(c.to_glam() + d / s))
}

/// The other semi-axis of the ellipse centred at `c` with semi-axis `a`
/// along unit `u` that passes through `p`.
fn semi_axis_through(c: Vec2D, u: glam::DVec2, a: f64, p: Vec2D) -> Option<f64> {
    let d = (p - c).to_glam();
    let (x, y) = (d.dot(u), d.dot(u.perp()));
    let k = 1.0 - (x / a).powi(2);
    (a > 1e-6 && k > 1e-9).then(|| y.abs() / k.sqrt())
}

/// Live value of `field` implied by the cursor position (display units:
/// degrees for angles, sketch units otherwise).
pub fn implied_value(
    state: &ToolState,
    sketch: &Sketch,
    cursor: Vec2D,
    params: &ToolParams,
    field: FieldKind,
) -> Option<f64> {
    let pos = |t: &SnapTarget| t.position(sketch);
    let from_anchor = || anchor(state, sketch).map(|a| (cursor - a).to_glam());
    match (state, field) {
        (ToolState::Idle, FieldKind::X) => Some(cursor.x),
        (ToolState::Idle, FieldKind::Y) => Some(cursor.y),
        (
            ToolState::PolylineFrom {
                from,
                heading: Some(heading),
                segment,
                ..
            },
            FieldKind::Radius | FieldKind::Sweep,
        ) => {
            let start = pos(from)?;
            let (t, _) = polyline_arc(*segment, *heading, start, cursor)?;
            let (center, radius, ccw) = geom2d::tangent_arc(start, Vec2D::from_glam(t), cursor)?;
            let (s, e) = ((start - center).to_glam(), (cursor - center).to_glam());
            Some(match (field, ccw) {
                (FieldKind::Radius, _) => radius,
                (_, true) => sweep_deg(s, e),
                (_, false) => sweep_deg(e, s),
            })
        }
        (ToolState::ArcCenter { .. }, FieldKind::Radius)
        | (ToolState::PolygonCenter { .. }, FieldKind::Radius)
        | (ToolState::EllipseCenter { .. }, FieldKind::MajorRadius)
        | (ToolState::EllipseArcCenter { .. }, FieldKind::MajorRadius)
        | (ToolState::ArcSlotCenter { .. }, FieldKind::Length)
        | (_, FieldKind::Length) => Some(from_anchor()?.length()),
        (ToolState::CircleFrom { .. }, FieldKind::Diameter) => Some(2.0 * from_anchor()?.length()),
        (_, FieldKind::Angle) => {
            let d = from_anchor()?;
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
        (ToolState::Rect3B { a, b }, FieldKind::Width) => {
            let (a, b) = (pos(a)?, pos(b)?);
            let edge = (b - a).to_glam().normalize_or_zero();
            Some(edge.perp_dot((cursor - a).to_glam()).abs())
        }
        (ToolState::RectCenter3Corner { center, corner }, FieldKind::Width) => {
            let [p1, p2, _, _] =
                crate::tools::rect_center3_corners(pos(center)?, pos(corner)?, cursor)?;
            Some((p2 - p1).to_glam().length())
        }
        (ToolState::SlotFrom { .. } | ToolState::ArcSlotStart { .. }, FieldKind::Width) => {
            Some(params.slot_width)
        }
        (ToolState::Circle3Two { a, b }, FieldKind::Diameter) => {
            let c = geom2d::circumcenter(a.to_glam(), b.to_glam(), cursor.to_glam())?;
            Some(2.0 * (a.to_glam() - c).length())
        }
        (ToolState::Arc3End { start, end }, FieldKind::Radius) => {
            let (s, e) = (pos(start)?.to_glam(), pos(end)?.to_glam());
            let c = geom2d::circumcenter(s, e, cursor.to_glam())?;
            Some((s - c).length())
        }
        (ToolState::ArcStart { center, start }, FieldKind::Sweep)
        | (ToolState::ArcSlotStart { center, start }, FieldKind::Sweep) => {
            let c = pos(center)?;
            Some(sweep_deg(
                (pos(start)? - c).to_glam(),
                (cursor - c).to_glam(),
            ))
        }
        (ToolState::EllipseArcStart { center, start, .. }, FieldKind::Sweep) => {
            let c = pos(center)?;
            Some(sweep_deg((*start - c).to_glam(), (cursor - c).to_glam()))
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
        (ToolState::EllipseArcMajor { center, major_pos }, FieldKind::MinorRadius) => {
            let c = pos(center)?;
            let major = (*major_pos - c).to_glam();
            semi_axis_through(c, major.normalize_or_zero(), major.length(), cursor)
        }
        (ToolState::Ellipse3B { a, b }, FieldKind::MinorRadius) => {
            let c = Vec2D::from_glam((a.to_glam() + b.to_glam()) * 0.5);
            let major = (*b - c).to_glam();
            semi_axis_through(c, major.normalize_or_zero(), major.length(), cursor)
        }
        _ => None,
    }
}

/// Anchor + radial direction from the cursor, at typed distance `r`.
fn radial(anchor: Vec2D, cursor: Vec2D, r: f64) -> Vec2D {
    let d = (cursor - anchor).to_glam();
    let dir = if d.length() > 1e-6 {
        d.normalize()
    } else {
        glam::DVec2::X
    };
    Vec2D::from_glam(anchor.to_glam() + dir * r)
}

/// From `anchor` at the typed `length` and `angle` (degrees), each taken
/// from the cursor where it was not typed.
fn polar(anchor: Vec2D, cursor: Vec2D, length: Option<f64>, angle: Option<f64>) -> Vec2D {
    let d = (cursor - anchor).to_glam();
    let dir = match angle {
        Some(deg) => glam::DVec2::from_angle(deg.to_radians()),
        None if d.length() > 1e-6 => d.normalize(),
        None => glam::DVec2::X,
    };
    let length = length.map(f64::abs).unwrap_or_else(|| d.length());
    Vec2D::from_glam(anchor.to_glam() + dir * length)
}

/// `p` turned about `c` by `deg` degrees counter-clockwise.
fn turned(c: Vec2D, p: Vec2D, deg: f64) -> Vec2D {
    let r = glam::DVec2::from_angle(deg.to_radians()).rotate((p - c).to_glam());
    Vec2D::from_glam(c.to_glam() + r)
}

fn sign_or(v: f64, fallback: f64) -> f64 {
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
    typed: &[(FieldKind, f64)],
) -> Vec2D {
    let get = |k: FieldKind| typed_value(typed, k);
    let pos = |t: &SnapTarget| t.position(sketch);
    match state {
        ToolState::Idle => Vec2D::new(
            get(FieldKind::X).unwrap_or(cursor.x),
            get(FieldKind::Y).unwrap_or(cursor.y),
        ),
        ToolState::PolylineFrom {
            from,
            heading: Some(heading),
            segment,
            ..
        } if *segment != PolySegment::Line => {
            let Some(start) = pos(from) else {
                return cursor;
            };
            let Some((t, left)) = polyline_arc(*segment, *heading, start, cursor) else {
                return cursor;
            };
            let radius = match get(FieldKind::Radius) {
                Some(r) => r.abs(),
                None => match geom2d::tangent_arc(start, Vec2D::from_glam(t), cursor) {
                    Some((_, r, _)) => r,
                    None => return cursor,
                },
            };
            if radius < 1e-6 {
                return cursor;
            }
            let side = if left { 1.0 } else { -1.0 };
            let center = Vec2D::from_glam(start.to_glam() + t.perp() * side * radius);
            match get(FieldKind::Sweep) {
                Some(deg) => turned(center, start, side * deg),
                None => radial(center, cursor, radius),
            }
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
        // The opposite edge at the typed distance, on the cursor's side.
        ToolState::Rect3B { a, b } => match (pos(a), pos(b), get(FieldKind::Width)) {
            (Some(a), Some(b), Some(w)) => {
                let n = (b - a).to_glam().normalize_or_zero().perp();
                let side = sign_or(n.dot((cursor - a).to_glam()), 1.0);
                Vec2D::from_glam(b.to_glam() + n * side * w.abs())
            }
            _ => cursor,
        },
        // The next corner the typed side away round the circle the
        // corners share, the way the cursor turns.
        ToolState::RectCenter3Corner { center, corner } => {
            match (pos(center), pos(corner), get(FieldKind::Width)) {
                (Some(c), Some(k), Some(w)) => {
                    let r = (k - c).to_glam().length();
                    if r < 1e-6 {
                        return cursor;
                    }
                    let half = (0.5 * w.abs() / r).min(1.0).asin();
                    let side = sign_or((k - c).to_glam().perp_dot((cursor - c).to_glam()), 1.0);
                    turned(c, k, side * 2.0 * half.to_degrees())
                }
                _ => cursor,
            }
        }
        ToolState::CircleFrom { center } => match (pos(center), get(FieldKind::Diameter)) {
            (Some(c), Some(d)) => radial(c, cursor, 0.5 * d.abs()),
            _ => cursor,
        },
        // Through the two rim points at the typed size: of the two such
        // circles, the one on the cursor's side of them, the click where
        // the cursor points on it.
        ToolState::Circle3Two { a, b } => match get(FieldKind::Diameter) {
            Some(d) => {
                let (a, b) = (a.to_glam(), b.to_glam());
                let half_chord = 0.5 * (b - a).length();
                let r = (0.5 * d.abs()).max(half_chord);
                let middle = (a + b) * 0.5;
                let normal = (b - a).perp().normalize_or_zero();
                let side = sign_or(normal.dot(cursor.to_glam() - middle), 1.0);
                let rise = (r * r - half_chord * half_chord).max(0.0).sqrt();
                let center = Vec2D::from_glam(middle + normal * side * rise);
                radial(center, cursor, r)
            }
            None => cursor,
        },
        // The arc at the typed radius through both ends, bulging toward the
        // cursor: the click at the middle of its rim.
        ToolState::Arc3End { start, end } => match (pos(start), pos(end), get(FieldKind::Radius)) {
            (Some(s), Some(e), Some(r)) => {
                let (s, e) = (s.to_glam(), e.to_glam());
                let half_chord = 0.5 * (e - s).length();
                let r = r.abs().max(half_chord);
                let middle = (s + e) * 0.5;
                let normal = (e - s).perp().normalize_or_zero();
                let side = sign_or(normal.dot(cursor.to_glam() - middle), 1.0);
                let rise = (r * r - half_chord * half_chord).max(0.0).sqrt();
                Vec2D::from_glam(middle + normal * side * (r - rise))
            }
            _ => cursor,
        },
        ToolState::ArcCenter { center } => match (pos(center), get(FieldKind::Radius)) {
            (Some(c), Some(r)) => radial(c, cursor, r.abs()),
            _ => cursor,
        },
        ToolState::ArcStart { center, start } | ToolState::ArcSlotStart { center, start } => {
            match (pos(center), pos(start), get(FieldKind::Sweep)) {
                (Some(c), Some(s), Some(deg)) => turned(c, s, deg),
                _ => cursor,
            }
        }
        ToolState::PolygonCenter { center } => match pos(center) {
            Some(c) => polar(c, cursor, get(FieldKind::Radius), get(FieldKind::Angle)),
            None => cursor,
        },
        ToolState::EllipseCenter { center } | ToolState::EllipseArcCenter { center } => {
            match (pos(center), get(FieldKind::MajorRadius)) {
                (Some(c), Some(r)) => radial(c, cursor, r.abs()),
                _ => cursor,
            }
        }
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
        // The arc's start where the cursor points, on the ellipse with the
        // typed minor radius.
        ToolState::EllipseArcMajor { center, major_pos } => {
            let (Some(c), Some(minor)) = (pos(center), get(FieldKind::MinorRadius)) else {
                return cursor;
            };
            let major = (*major_pos - c).to_glam();
            on_ellipse_ray(
                c,
                major.normalize_or_zero(),
                major.length(),
                minor.abs(),
                cursor,
            )
            .unwrap_or(cursor)
        }
        ToolState::EllipseArcStart { center, start, .. } => {
            match (pos(center), get(FieldKind::Sweep)) {
                (Some(c), Some(deg)) => turned(c, *start, deg),
                _ => cursor,
            }
        }
        // The rim point at the end of the minor axis, on the cursor's side.
        ToolState::Ellipse3B { a, b } => match get(FieldKind::MinorRadius) {
            Some(minor) => {
                let c = (a.to_glam() + b.to_glam()) * 0.5;
                let n = (b.to_glam() - c).normalize_or_zero().perp();
                let side = sign_or(n.dot(cursor.to_glam() - c), 1.0);
                Vec2D::from_glam(c + n * side * minor.abs())
            }
            None => cursor,
        },
        ToolState::ArcSlotCenter { center } => match (pos(center), get(FieldKind::Length)) {
            (Some(c), Some(l)) => radial(c, cursor, l.abs()),
            _ => cursor,
        },
        _ => match anchor(state, sketch) {
            Some(a) => polar(a, cursor, get(FieldKind::Length), get(FieldKind::Angle)),
            None => cursor,
        },
    }
}

/// The tool settings a click in `state` runs with: a typed slot width
/// stands in for the setting's.
pub fn typed_params(
    params: &ToolParams,
    state: &ToolState,
    typed: &[(FieldKind, f64)],
) -> ToolParams {
    match (state, typed_value(typed, FieldKind::Width)) {
        (ToolState::SlotFrom { .. } | ToolState::ArcSlotStart { .. }, Some(w)) => ToolParams {
            slot_width: w.abs(),
            ..*params
        },
        _ => *params,
    }
}

/// Wrap degrees into (-180, 180].
fn normalize_deg(deg: f64) -> f64 {
    let mut d = deg % 360.0;
    if d <= -180.0 {
        d += 360.0;
    }
    if d > 180.0 {
        d -= 360.0;
    }
    d
}

/// A typed sweep as an arc's angle in radians, (0, 2π]; `None` for none.
fn sweep_rad(deg: f64) -> Option<f64> {
    let d = deg.rem_euclid(360.0);
    let d = if d < 1e-4 && deg.abs() > 1e-4 {
        360.0
    } else {
        d
    };
    (d > 1e-4).then(|| d.to_radians())
}

fn last_of<'a, T>(
    sketch: &'a Sketch,
    back: usize,
    pick: impl Fn(&'a GeometryElement) -> Option<&'a T>,
) -> Option<&'a T> {
    let n = sketch.geometry.len();
    sketch.geometry.get(n.checked_sub(back)?).and_then(pick)
}

fn as_arc(g: &GeometryElement) -> Option<&crate::sketch::Arc> {
    match g {
        GeometryElement::Arc(a) => Some(a),
        _ => None,
    }
}

fn as_line(g: &GeometryElement) -> Option<&crate::sketch::Line> {
    match g {
        GeometryElement::Line(l) => Some(l),
        _ => None,
    }
}

/// The ids of the last `n` lines made, oldest first.
fn last_lines(sketch: &Sketch, n: usize) -> Option<Vec<uuid::Uuid>> {
    let mut lines: Vec<uuid::Uuid> = sketch
        .geometry
        .iter()
        .rev()
        .filter_map(as_line)
        .take(n)
        .map(|l| l.id)
        .collect();
    lines.reverse();
    (lines.len() == n).then_some(lines)
}

fn angle_to_x(line: uuid::Uuid, deg: f64) -> ConstraintKind {
    ConstraintKind::AngleToAxis {
        line,
        axis: AxisDirection::Horizontal,
        angle_rad: normalize_deg(deg).to_radians(),
    }
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
    typed: &[(FieldKind, f64)],
    constrain: bool,
) -> usize {
    let get = |k: FieldKind| typed_value(typed, k);
    let mut kinds: Vec<(ConstraintKind, bool)> = Vec::new();
    match state_before {
        ToolState::LineFrom { .. }
        | ToolState::PolylineFrom {
            segment: PolySegment::Line,
            ..
        }
        | ToolState::PolylineFrom { heading: None, .. }
            if changed =>
        {
            let Some(line) = last_of(sketch, 1, as_line).cloned() else {
                return 0;
            };
            if let Some(len) = get(FieldKind::Length) {
                kinds.push((
                    ConstraintKind::Distance {
                        point1: line.start,
                        point2: line.end,
                        distance: len.abs(),
                    },
                    constrain,
                ));
            }
            if let Some(deg) = get(FieldKind::Angle) {
                kinds.push((angle_to_x(line.id, deg), constrain));
            }
        }
        ToolState::PolylineFrom { .. } if changed => {
            let Some(arc) = last_of(sketch, 1, as_arc).map(|a| a.id) else {
                return 0;
            };
            if let Some(r) = get(FieldKind::Radius) {
                kinds.push((
                    ConstraintKind::Radius {
                        circle: arc,
                        radius: r.abs(),
                    },
                    constrain,
                ));
            }
            if let Some(angle_rad) = get(FieldKind::Sweep).and_then(sweep_rad) {
                kinds.push((ConstraintKind::ArcAngle { arc, angle_rad }, constrain));
            }
        }
        ToolState::RectFrom { .. } | ToolState::RectCenterAt { .. } if changed => {
            // The rectangle's four edges are the last lines made, bottom →
            // right → top → left. A plain one ends with them: the width sits
            // on the bottom edge and the height on the right one, so the two
            // dimensions draw along their own sides. A rounded one ends with
            // its corner arcs, and its edges stop short of the corners, so
            // the width and height run between opposite edges instead.
            let mut edges = sketch.geometry.iter().rev().filter_map(|g| match g {
                GeometryElement::Line(l) => Some(l.clone()),
                _ => None,
            });
            let (Some(left), Some(top), Some(right), Some(bottom)) =
                (edges.next(), edges.next(), edges.next(), edges.next())
            else {
                return 0;
            };
            let rounded = !matches!(sketch.geometry.last(), Some(GeometryElement::Line(_)));
            if let Some(w) = get(FieldKind::Width) {
                let kind = if rounded {
                    ConstraintKind::DistanceX {
                        a: left.start,
                        b: Some(right.start),
                        value: w.abs(),
                    }
                } else {
                    ConstraintKind::Length {
                        line: bottom.id,
                        length: w.abs(),
                    }
                };
                kinds.push((kind, constrain));
            }
            if let Some(h) = get(FieldKind::Height) {
                let kind = if rounded {
                    ConstraintKind::DistanceY {
                        a: bottom.start,
                        b: Some(top.start),
                        value: h.abs(),
                    }
                } else {
                    ConstraintKind::Length {
                        line: right.id,
                        length: h.abs(),
                    }
                };
                kinds.push((kind, constrain));
            }
        }
        ToolState::Rect3A { .. } => {
            let (length, angle) = (get(FieldKind::Length), get(FieldKind::Angle));
            if matches!(state_after, ToolState::Rect3B { .. })
                && (length.is_some() || angle.is_some())
            {
                capture.pending = Some(PendingDim::Rect3Edge {
                    length: length.map(f64::abs),
                    angle,
                    constrain,
                });
            }
        }
        // Its edges are the last four lines: the first two clicks' edge,
        // then the one across to the width.
        ToolState::Rect3B { .. } if changed => {
            let Some(edges) = last_lines(sketch, 4) else {
                return 0;
            };
            if let Some(PendingDim::Rect3Edge {
                length,
                angle,
                constrain: c,
            }) = capture.pending.take()
            {
                if let Some(length) = length {
                    kinds.push((
                        ConstraintKind::Length {
                            line: edges[0],
                            length,
                        },
                        c,
                    ));
                }
                if let Some(deg) = angle {
                    kinds.push((angle_to_x(edges[0], deg), c));
                }
            }
            if let Some(w) = get(FieldKind::Width) {
                kinds.push((
                    ConstraintKind::Length {
                        line: edges[1],
                        length: w.abs(),
                    },
                    constrain,
                ));
            }
        }
        ToolState::RectCenter3Corner { .. } if changed => {
            if let (Some(edges), Some(w)) = (last_lines(sketch, 4), get(FieldKind::Width)) {
                kinds.push((
                    ConstraintKind::Length {
                        line: edges[0],
                        length: w.abs(),
                    },
                    constrain,
                ));
            }
        }
        ToolState::CircleFrom { .. } | ToolState::Circle3Two { .. } if changed => {
            let circle = last_of(sketch, 1, |g| match g {
                GeometryElement::Circle(c) => Some(c),
                _ => None,
            })
            .map(|c| c.id);
            if let (Some(circle), Some(d)) = (circle, get(FieldKind::Diameter)) {
                kinds.push((
                    ConstraintKind::Diameter {
                        circle,
                        diameter: d.abs(),
                    },
                    constrain,
                ));
            }
        }
        ToolState::ArcCenter { .. } => {
            // The arc only materializes at the end click; carry the radius.
            if matches!(state_after, ToolState::ArcStart { .. })
                && let Some(r) = get(FieldKind::Radius)
            {
                capture.pending = Some(PendingDim::ArcRadius {
                    r: r.abs(),
                    constrain,
                });
            }
        }
        ToolState::ArcStart { .. } if changed => {
            let arc = last_of(sketch, 1, as_arc).map(|a| a.id);
            if let Some(PendingDim::ArcRadius { r, constrain: c }) = capture.pending.take()
                && let Some(circle) = arc
            {
                kinds.push((ConstraintKind::Radius { circle, radius: r }, c));
            }
            if let (Some(arc), Some(angle_rad)) = (arc, get(FieldKind::Sweep).and_then(sweep_rad)) {
                kinds.push((ConstraintKind::ArcAngle { arc, angle_rad }, constrain));
            }
        }
        ToolState::Arc3End { .. } if changed => {
            if let (Some(arc), Some(r)) = (last_of(sketch, 1, as_arc), get(FieldKind::Radius)) {
                kinds.push((
                    ConstraintKind::Radius {
                        circle: arc.id,
                        radius: r.abs(),
                    },
                    constrain,
                ));
            }
        }
        ToolState::PolygonCenter { .. } if changed => {
            // The polygon's own construction circle carries the radius.
            let circle = sketch.geometry.iter().rev().find_map(|g| match g {
                GeometryElement::Circle(c) => Some(c.id),
                _ => None,
            });
            if let (Some(r), Some(circle)) = (get(FieldKind::Radius), circle) {
                kinds.push((
                    ConstraintKind::Radius {
                        circle,
                        radius: r.abs(),
                    },
                    constrain,
                ));
            }
            // Its sides are the last lines made, the first from the vertex
            // at the typed turn to the next one counter-clockwise: a chord
            // running square to the middle of the two.
            if let Some(deg) = get(FieldKind::Angle) {
                let sides = sketch
                    .geometry
                    .iter()
                    .rev()
                    .take_while(|g| matches!(g, GeometryElement::Line(_)))
                    .count();
                if let Some(first) = Some(sides)
                    .filter(|n| *n >= 3)
                    .and_then(|n| last_of(sketch, n, as_line))
                {
                    let turn = deg + 90.0 + 180.0 / sides as f64;
                    kinds.push((angle_to_x(first.id, turn), constrain));
                }
            }
        }
        ToolState::SlotFrom { .. } if changed => {
            // Made last: the two rails, then the cap at the end and the one
            // at the start, each centred on its end of the centerline.
            let (Some(cap_b), Some(cap_a)) = (
                last_of(sketch, 2, as_arc).cloned(),
                last_of(sketch, 1, as_arc).cloned(),
            ) else {
                return 0;
            };
            if let Some(l) = get(FieldKind::Length) {
                kinds.push((
                    ConstraintKind::Distance {
                        point1: cap_a.center,
                        point2: cap_b.center,
                        distance: l.abs(),
                    },
                    constrain,
                ));
            }
            if let (Some(deg), Some(rail)) = (get(FieldKind::Angle), last_of(sketch, 4, as_line)) {
                kinds.push((angle_to_x(rail.id, deg), constrain));
            }
            if let Some(w) = get(FieldKind::Width) {
                kinds.push((
                    ConstraintKind::Radius {
                        circle: cap_a.id,
                        radius: 0.5 * w.abs(),
                    },
                    constrain,
                ));
            }
        }
        ToolState::ArcSlotCenter { .. } => {
            if matches!(state_after, ToolState::ArcSlotStart { .. })
                && let Some(l) = get(FieldKind::Length)
            {
                capture.pending = Some(PendingDim::ArcSlotLength {
                    l: l.abs(),
                    constrain,
                });
            }
        }
        ToolState::ArcSlotStart { .. } if changed => {
            // Made last: the outer rail, the inner one, the end cap, the
            // start cap; the outer rail is centred on the slot's centre and
            // the start cap on the centerline's start.
            let (Some(outer), Some(cap_a)) = (
                last_of(sketch, 4, as_arc).cloned(),
                last_of(sketch, 1, as_arc).cloned(),
            ) else {
                return 0;
            };
            if let Some(PendingDim::ArcSlotLength { l, constrain: c }) = capture.pending.take() {
                kinds.push((
                    ConstraintKind::Distance {
                        point1: outer.center,
                        point2: cap_a.center,
                        distance: l,
                    },
                    c,
                ));
            }
            if let Some(angle_rad) = get(FieldKind::Sweep).and_then(sweep_rad) {
                kinds.push((
                    ConstraintKind::ArcAngle {
                        arc: outer.id,
                        angle_rad,
                    },
                    constrain,
                ));
            }
            if let Some(w) = get(FieldKind::Width) {
                kinds.push((
                    ConstraintKind::Radius {
                        circle: cap_a.id,
                        radius: 0.5 * w.abs(),
                    },
                    constrain,
                ));
            }
        }
        _ => {}
    }
    // Without its flag a typed value only shaped the geometry.
    let mut added = 0;
    for (kind, keep) in kinds {
        if keep {
            sketch.add_constraint(kind);
            added += 1;
        }
    }
    added
}

/// The on-view parameter rows for the tool's current fields: the implied
/// value while the cursor drives it, the typed text once the user types (a
/// formula with what it comes to).
pub fn readout_rows(
    capture: &DimCapture,
    state: &ToolState,
    tool: Option<&str>,
    sketch: &Sketch,
    cursor: Vec2D,
    params: &ToolParams,
    document: &Document,
) -> Vec<core_document::OvpRow> {
    let fields = fields_for(state, tool);
    let synced = capture.fields == fields;
    let cursor = capture.steer(state, sketch, cursor);
    fields
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let buffer = if synced {
                capture.buffers[i].as_str()
            } else {
                ""
            };
            let focused = synced && capture.focus == i;
            let unit = if field.is_angle() { "°" } else { "mm" };
            let shown = |v: f64| {
                if field.is_angle() {
                    format!("{v:.1}")
                } else {
                    format!("{v:.2}")
                }
            };
            let value = if buffer.is_empty() {
                match implied_value(state, sketch, cursor, params, *field) {
                    Some(v) => shown(v),
                    None => "-".to_string(),
                }
            } else if plain(buffer) {
                buffer.to_string()
            } else {
                match evaluate(buffer, *field, document) {
                    Ok(v) => format!("{buffer} = {}", shown(v)),
                    Err(_) => format!("{buffer} = ?"),
                }
            };
            core_document::OvpRow {
                label: field.short(),
                value,
                unit,
                focused,
                locked: !buffer.is_empty(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Line, Point, Sketch};

    fn sketch_with_point(pos: Vec2D) -> (Sketch, SnapTarget) {
        let sketch = Sketch::new("t");
        (sketch, SnapTarget::New(pos))
    }

    fn type_in(cap: &mut DimCapture, text: &str) {
        for c in text.chars() {
            assert!(cap.handle_text(&c.to_string()), "took {c:?}");
        }
    }

    #[test]
    fn capture_typing_focus_and_clear() {
        let doc = Document::new("t");
        let mut cap = DimCapture::default();
        let state = ToolState::LineFrom {
            from: SnapTarget::New(Vec2D::new(0.0, 0.0)),
            chain: false,
        };
        cap.sync(&state, Some("sketch.line"));
        assert!(cap.is_active());
        assert!(cap.handle_key(KeyCode::Key2), "the digit's key is kept");
        type_in(&mut cap, "25,5");
        assert_eq!(cap.typed(&doc), vec![(FieldKind::Length, 25.5)]);

        // Tab moves the focus to the angle field.
        assert!(cap.handle_key(KeyCode::Tab));
        type_in(&mut cap, "45");
        assert_eq!(
            cap.typed(&doc),
            vec![(FieldKind::Length, 25.5), (FieldKind::Angle, 45.0)]
        );

        // Minus toggles the sign; backspace edits; clear drops everything.
        type_in(&mut cap, "-");
        assert_eq!(cap.typed(&doc)[1].1, -45.0);
        assert!(cap.handle_key(KeyCode::Backspace));
        assert_eq!(cap.typed(&doc)[1].1, -4.0);
        cap.clear_buffers();
        assert!(!cap.has_typed_input());

        // Idle with no tool drops the fields.
        cap.sync(&ToolState::Idle, None);
        assert!(!cap.is_active());
        assert!(!cap.handle_key(KeyCode::Key1));
    }

    #[test]
    fn a_formula_and_a_paste_are_entries_too() {
        let doc = Document::new("t");
        let mut cap = DimCapture::default();
        let state = ToolState::LineFrom {
            from: SnapTarget::New(Vec2D::new(0.0, 0.0)),
            chain: false,
        };
        cap.sync(&state, Some("sketch.line"));
        // A letter is no key while the field is empty, and is typed once
        // it holds something.
        assert!(!cap.handle_key(KeyCode::W));
        type_in(&mut cap, "2*3-1");
        assert!(cap.takes_text());
        assert!(cap.handle_key(KeyCode::W));
        assert_eq!(cap.typed(&doc), vec![(FieldKind::Length, 5.0)]);
        type_in(&mut cap, "*");
        assert!(
            cap.typed(&doc).is_empty(),
            "an unfinished formula is no value"
        );
        assert!(cap.error(&doc).is_some());

        cap.clear_buffers();
        assert!(cap.handle_text("12,5"), "a pasted decimal comma");
        assert_eq!(cap.typed(&doc), vec![(FieldKind::Length, 12.5)]);
        cap.clear_buffers();
        assert!(!cap.handle_text("1\n2"), "not two lines");
    }

    #[test]
    fn the_heading_does_not_flip_across_the_anchor_once_typed() {
        let (sketch, from) = sketch_with_point(Vec2D::new(0.0, 0.0));
        let state = ToolState::LineFrom { from, chain: false };
        let mut cap = DimCapture::default();
        cap.sync(&state, Some("sketch.line"));
        cap.track(&state, &sketch, Vec2D::new(5.0, 1.0));
        cap.handle_text("8");
        // Straight through the start to the other side.
        cap.track(&state, &sketch, Vec2D::new(-5.0, -1.0));
        let steered = cap.steer(&state, &sketch, Vec2D::new(-5.0, -1.0));
        let p = override_cursor(&state, &sketch, steered, &[(FieldKind::Length, 8.0)]);
        assert!(p.x > 7.0, "still to the right: {p:?}");
        // Round by the side, it turns.
        for (x, y) in [(3.0, 4.0), (0.0, 5.0), (-3.0, 4.0), (-5.0, 0.5)] {
            cap.track(&state, &sketch, Vec2D::new(x, y));
        }
        let steered = cap.steer(&state, &sketch, Vec2D::new(-5.0, 0.5));
        let p = override_cursor(&state, &sketch, steered, &[(FieldKind::Length, 8.0)]);
        assert!(p.x < -7.0, "turned round to the left: {p:?}");
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
    fn a_tangent_arc_takes_its_radius_and_sweep() {
        let (sketch, from) = sketch_with_point(Vec2D::new(0.0, 0.0));
        let state = ToolState::PolylineFrom {
            from,
            first: None,
            heading: Some(Vec2D::new(1.0, 0.0)),
            prev: None,
            segment: PolySegment::Tangent,
        };
        // Leaving along +x and turning left: a quarter turn of radius 5
        // ends at (5, 5).
        let p = override_cursor(
            &state,
            &sketch,
            Vec2D::new(3.0, 2.0),
            &[(FieldKind::Radius, 5.0), (FieldKind::Sweep, 90.0)],
        );
        assert!(
            (p.x - 5.0).abs() < 1e-4 && (p.y - 5.0).abs() < 1e-4,
            "{p:?}"
        );
        let sweep = implied_value(&state, &sketch, p, &ToolParams::default(), FieldKind::Sweep);
        assert!((sweep.unwrap() - 90.0).abs() < 1e-3);
    }

    #[test]
    fn implied_values_track_cursor() {
        let (sketch, from) = sketch_with_point(Vec2D::new(0.0, 0.0));
        let state = ToolState::LineFrom { from, chain: false };
        let cursor = Vec2D::new(3.0, 4.0);
        let params = ToolParams::default();
        let len = implied_value(&state, &sketch, cursor, &params, FieldKind::Length).unwrap();
        assert!((len - 5.0).abs() < 1e-4);
        let ang = implied_value(&state, &sketch, cursor, &params, FieldKind::Angle).unwrap();
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
            true,
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

    #[test]
    fn a_typed_sweep_is_an_arc_angle_in_its_range() {
        assert!((sweep_rad(90.0).unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((sweep_rad(-90.0).unwrap().to_degrees() - 270.0).abs() < 1e-3);
        assert!((sweep_rad(360.0).unwrap().to_degrees() - 360.0).abs() < 1e-3);
        assert!(sweep_rad(0.0).is_none());
    }
}
