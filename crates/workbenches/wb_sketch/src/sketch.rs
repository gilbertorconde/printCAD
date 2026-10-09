//! Sketch data model: 2D geometry primitives and constraints.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 2D vector (serializable version of Vec2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec2D {
    pub x: f32,
    pub y: f32,
}

impl Vec2D {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn to_glam(self) -> glam::Vec2 {
        glam::Vec2::new(self.x, self.y)
    }

    pub fn from_glam(v: glam::Vec2) -> Self {
        Self { x: v.x, y: v.y }
    }
}

impl std::ops::Add for Vec2D {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }
}

impl std::ops::Sub for Vec2D {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }
}

/// A 2D sketch containing geometry and constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sketch {
    /// Unique identifier for this sketch.
    pub id: Uuid,
    /// Name of the sketch (user-facing).
    pub name: String,
    /// The plane the sketch lies on, in its body's own frame.
    pub plane: SketchPlane,
    /// Geometry elements in the sketch.
    pub geometry: Vec<GeometryElement>,
    /// Constraints applied to the geometry.
    pub constraints: Vec<Constraint>,
    /// Whether the sketch is fully constrained.
    pub is_fully_constrained: bool,
    /// The last solve left constraints unmet (they conflict, or the
    /// solver gave up): the geometry is not what the constraints say, and
    /// nothing is built from it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unsolved: bool,
    /// Ids of geometry flagged as construction:
    /// guides that snap, hit-test and constrain like normal geometry but are
    /// excluded from profile extraction. Defaults to empty so sketches saved
    /// without it keep loading.
    #[serde(default)]
    pub construction: std::collections::HashSet<Uuid>,
    /// Geometry projected from a solid's edges, or cut from its faces by the
    /// sketch plane, by element id, each with the edge or face it came
    /// from. It is fixed (the solver never moves it), left out of profiles,
    /// and brought up to the solid again when the sketch is edited.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub external: std::collections::HashMap<Uuid, ExternalSource>,
    /// How the solver works on this sketch.
    #[serde(default, skip_serializing_if = "SolverSettings::is_default")]
    pub solver: SolverSettings,
    /// Text laid out as outlines, each block standing on a point.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<TextBlock>,
    /// Pictures on the plane to draw over, shown while the sketch is
    /// edited; no part of the profile.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ReferenceImage>,
}

/// A picture laid on the sketch plane, its pixels a document asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceImage {
    pub id: Uuid,
    /// The document asset holding the file's bytes.
    pub asset: Uuid,
    /// Where its middle is, in sketch coordinates.
    pub center: Vec2D,
    /// How wide it lies, in sketch units (mm); its height keeps its
    /// proportions.
    pub width: f32,
    /// Its turn counter-clockwise, in degrees.
    #[serde(default)]
    pub angle_deg: f32,
    /// How much of it shows over the scene, 0..=1.
    #[serde(default = "half")]
    pub opacity: f32,
}

fn half() -> f32 {
    0.5
}

/// A string in a font, laid out as closed outlines standing on `anchor`
/// (the start of its first line on the baseline). The outlines' elements
/// are the block's own: made from the rest, carried along with the point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBlock {
    pub id: Uuid,
    pub text: String,
    /// A font the application carries, by name, or a font file's path.
    pub font: String,
    /// The font's em, in millimetres.
    pub size: f32,
    /// Millimetres added between letters.
    #[serde(default)]
    pub spacing: f32,
    /// Degrees it turns counter-clockwise about its point.
    #[serde(default)]
    pub angle: f32,
    pub anchor: Uuid,
    /// The outlines' curves and points.
    #[serde(default)]
    pub elements: Vec<Uuid>,
    /// Where the point stood when the outlines were last placed.
    pub placed: Vec2D,
}

/// How far the solver goes on a sketch: how many steps it takes at most,
/// and how small what is left must be to count as solved (against the
/// sketch's size).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SolverSettings {
    pub max_iterations: u32,
    pub tolerance: f64,
}

impl Default for SolverSettings {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            tolerance: 1e-9,
        }
    }
}

impl SolverSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Where an external element came from, in its body's own frame: a solid
/// edge it was projected from (a point on it and its direction there), or
/// a face the sketch plane cut it from (a point on it and its normal
/// there).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ExternalSource {
    pub body: Uuid,
    pub point: [f32; 3],
    pub direction: [f32; 3],
    /// A face cut by the sketch plane, not an edge projected onto it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub section: bool,
    /// It counts in the sketch's profiles, as drawn geometry does, rather
    /// than only guiding it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub defining: bool,
    /// Another sketch's element or a datum it comes from; then `body`,
    /// `point` and `direction` say nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<ExternalReference>,
}

/// What external geometry refers to when it is not a solid's edge or face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExternalReference {
    /// An element of another sketch.
    SketchElement { sketch: Uuid, element: Uuid },
    /// A datum line, point, plane or coordinate system.
    Datum { datum: Uuid },
}

impl ExternalSource {
    /// External geometry from another sketch's element or a datum.
    pub fn of_reference(reference: ExternalReference) -> Self {
        Self {
            body: Uuid::nil(),
            point: [0.0; 3],
            direction: [0.0; 3],
            section: false,
            defining: false,
            reference: Some(reference),
        }
    }
}

impl Sketch {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            plane: SketchPlane::default(),
            geometry: Vec::new(),
            constraints: Vec::new(),
            is_fully_constrained: false,
            unsolved: false,
            construction: std::collections::HashSet::new(),
            external: std::collections::HashMap::new(),
            solver: SolverSettings::default(),
            texts: Vec::new(),
            images: Vec::new(),
        }
    }

    /// Whether `id` is external geometry, or a point of some.
    pub fn is_external(&self, id: Uuid) -> bool {
        self.external_ids().contains(&id)
    }

    /// Every external element and every point one is drawn through: what
    /// the solver holds still and a drag leaves alone.
    pub fn external_ids(&self) -> std::collections::HashSet<Uuid> {
        let mut ids = std::collections::HashSet::new();
        for id in self.external.keys() {
            ids.insert(*id);
            if let Some(element) = self.get_geometry(*id) {
                ids.extend(Self::curve_point_ids(element));
            }
        }
        ids
    }

    /// Whether `id` is flagged as construction geometry.
    pub fn is_construction(&self, id: Uuid) -> bool {
        self.construction.contains(&id)
    }

    /// Set or clear the construction flag on `id`.
    pub fn set_construction(&mut self, id: Uuid, construction: bool) {
        if construction {
            self.construction.insert(id);
        } else {
            self.construction.remove(&id);
        }
    }

    /// Add a geometry element to the sketch.
    pub fn add_geometry(&mut self, element: GeometryElement) -> Uuid {
        let id = element.id();
        self.geometry.push(element);
        id
    }

    /// Add a constraint (driving, active, unnamed) and return its id.
    pub fn add_constraint(&mut self, kind: ConstraintKind) -> Uuid {
        let constraint = Constraint::new(kind);
        let id = constraint.id;
        self.constraints.push(constraint);
        id
    }

    /// Get a geometry element by ID.
    pub fn get_geometry(&self, id: Uuid) -> Option<&GeometryElement> {
        self.geometry.iter().find(|g| g.id() == id)
    }

    /// Get a mutable reference to a geometry element by ID.
    pub fn get_geometry_mut(&mut self, id: Uuid) -> Option<&mut GeometryElement> {
        self.geometry.iter_mut().find(|g| g.id() == id)
    }

    /// Position of the point element `id`, or of the origin reference.
    pub fn point_position(&self, id: Uuid) -> Option<Vec2D> {
        if id == ORIGIN_ID {
            return Some(Vec2D::new(0.0, 0.0));
        }
        self.stored_point_position(id)
    }

    fn stored_point_position(&self, id: Uuid) -> Option<Vec2D> {
        match self.get_geometry(id)? {
            GeometryElement::Point(p) => Some(p.position),
            _ => None,
        }
    }

    /// Every point id a curve references (empty for points).
    pub fn curve_point_ids(element: &GeometryElement) -> Vec<Uuid> {
        match element {
            GeometryElement::Point(_) => Vec::new(),
            GeometryElement::Line(l) => vec![l.start, l.end],
            GeometryElement::Arc(a) => vec![a.center, a.start, a.end],
            GeometryElement::Circle(c) => vec![c.center],
            GeometryElement::Ellipse(e) => match e.arc {
                Some(arc) => vec![e.center, arc.start, arc.end],
                None => vec![e.center],
            },
            GeometryElement::BSpline(b) => b.point_ids(),
            GeometryElement::Conic(c) => vec![c.center, c.start, c.end],
        }
    }

    /// Remove elements by id with cascade semantics:
    /// - removing a point also removes every curve that references it;
    /// - removing a curve takes the points it defined, unless a remaining
    ///   curve shares them (a standalone point is never touched: nothing
    ///   referenced it in the first place);
    /// - constraints referencing any removed element are dropped.
    ///
    /// Returns the ids of every element actually removed.
    pub fn remove_geometry_cascade(&mut self, ids: &[Uuid]) -> Vec<Uuid> {
        use std::collections::HashSet;
        let mut doomed: HashSet<Uuid> = ids.iter().copied().collect();
        // Text goes whole: its point or any piece of its outlines takes
        // the rest of it.
        let texts: Vec<Uuid> = self
            .texts
            .iter()
            .filter(|b| doomed.contains(&b.anchor) || b.elements.iter().any(|e| doomed.contains(e)))
            .map(|b| b.id)
            .collect();
        for block in self.texts.iter().filter(|b| texts.contains(&b.id)) {
            doomed.insert(block.anchor);
            doomed.extend(block.elements.iter().copied());
        }
        self.texts.retain(|b| !texts.contains(&b.id));

        // Curves that reference a doomed point are doomed too.
        for geom in &self.geometry {
            if doomed.contains(&geom.id()) {
                continue;
            }
            if Self::curve_point_ids(geom)
                .iter()
                .any(|pid| doomed.contains(pid))
            {
                doomed.insert(geom.id());
            }
        }
        // A curve's internal geometry goes with it.
        for c in &self.constraints {
            if let ConstraintKind::InternalAlignment { element, curve, .. } = c.kind
                && doomed.contains(&curve)
            {
                doomed.insert(element);
            }
        }

        // Points that only the doomed curves held on to go with them;
        // points a surviving curve still references stay.
        let mut released: HashSet<Uuid> = HashSet::new();
        let mut still_referenced: HashSet<Uuid> = HashSet::new();
        for geom in &self.geometry {
            let points = Self::curve_point_ids(geom);
            if doomed.contains(&geom.id()) {
                released.extend(points);
            } else {
                still_referenced.extend(points);
            }
        }
        for point in released {
            if !still_referenced.contains(&point) {
                doomed.insert(point);
            }
        }

        let removed: Vec<Uuid> = self
            .geometry
            .iter()
            .map(|g| g.id())
            .filter(|id| doomed.contains(id))
            .collect();
        self.geometry.retain(|g| !doomed.contains(&g.id()));
        self.constraints.retain(|c| {
            !constraint_refs(&c.kind)
                .iter()
                .any(|id| doomed.contains(id))
        });
        self.construction.retain(|id| !doomed.contains(id));
        self.external.retain(|id, _| !doomed.contains(id));
        removed
    }
}

/// The reference geometry every sketch carries: the origin and the two
/// axes through it. They hold no entry in `geometry` (nothing can move,
/// delete or extrude them), but they answer to fixed ids so constraints can
/// pin real geometry against them.
pub const ORIGIN_ID: Uuid = Uuid::from_u128(0x5c_e701_0000_0000_0000_0000_0000_0001);
pub const X_AXIS_ID: Uuid = Uuid::from_u128(0x5c_e701_0000_0000_0000_0000_0000_0002);
pub const Y_AXIS_ID: Uuid = Uuid::from_u128(0x5c_e701_0000_0000_0000_0000_0000_0003);

/// Which piece of reference geometry an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference {
    Origin,
    XAxis,
    YAxis,
}

impl Reference {
    pub fn of(id: Uuid) -> Option<Self> {
        match id {
            ORIGIN_ID => Some(Reference::Origin),
            X_AXIS_ID => Some(Reference::XAxis),
            Y_AXIS_ID => Some(Reference::YAxis),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Reference::Origin => "Origin",
            Reference::XAxis => "X axis",
            Reference::YAxis => "Y axis",
        }
    }

    /// Whether the reference behaves as a point (the origin) or a line.
    pub fn is_point(self) -> bool {
        matches!(self, Reference::Origin)
    }

    /// Direction of an axis in sketch coordinates; `None` for the origin.
    pub fn direction(self) -> Option<Vec2D> {
        match self {
            Reference::Origin => None,
            Reference::XAxis => Some(Vec2D::new(1.0, 0.0)),
            Reference::YAxis => Some(Vec2D::new(0.0, 1.0)),
        }
    }
}

/// Every geometry id a constraint references.
pub fn constraint_refs(kind: &ConstraintKind) -> Vec<Uuid> {
    match kind {
        ConstraintKind::FixedPoint { point, .. } => vec![*point],
        ConstraintKind::Coincident { point1, point2 } => vec![*point1, *point2],
        ConstraintKind::Parallel { line1, line2 }
        | ConstraintKind::Perpendicular { line1, line2 }
        | ConstraintKind::EqualLength { line1, line2 } => vec![*line1, *line2],
        ConstraintKind::Length { line, .. } => vec![*line],
        ConstraintKind::EqualRadius { circle1, circle2 } => vec![*circle1, *circle2],
        ConstraintKind::EqualEllipse { ellipse1, ellipse2 } => vec![*ellipse1, *ellipse2],
        ConstraintKind::Radius { circle, .. } | ConstraintKind::Diameter { circle, .. } => {
            vec![*circle]
        }
        ConstraintKind::PointOnLine { point, line } => vec![*point, *line],
        ConstraintKind::PointOnCircle { point, circle } => vec![*point, *circle],
        ConstraintKind::PointOnEllipse { point, ellipse } => vec![*point, *ellipse],
        ConstraintKind::PointOnCurve { point, curve } => vec![*point, *curve],
        ConstraintKind::EllipseRadius { ellipse, .. } => vec![*ellipse],
        ConstraintKind::CurveLength { curve, .. } => vec![*curve],
        ConstraintKind::Offset { pairs, .. } => pairs.iter().flatten().copied().collect(),
        ConstraintKind::Pitch { points, .. } => points.clone(),
        ConstraintKind::PolarPitch { center, points, .. } => std::iter::once(*center)
            .chain(points.iter().copied())
            .collect(),
        ConstraintKind::TangentCurves { curve1, curve2 }
        | ConstraintKind::PerpendicularCurves { curve1, curve2 } => vec![*curve1, *curve2],
        ConstraintKind::Horizontal { element }
        | ConstraintKind::Vertical { element }
        | ConstraintKind::Block { element } => vec![*element],
        ConstraintKind::HorizontalPoints { point1, point2 }
        | ConstraintKind::VerticalPoints { point1, point2 } => vec![*point1, *point2],
        ConstraintKind::Distance { point1, point2, .. } => vec![*point1, *point2],
        ConstraintKind::DistanceX { a, b, .. } | ConstraintKind::DistanceY { a, b, .. } => {
            let mut refs = vec![*a];
            refs.extend(b.iter().copied());
            refs
        }
        ConstraintKind::Angle { line1, line2, .. } => vec![*line1, *line2],
        ConstraintKind::AngleToAxis { line, .. } => vec![*line],
        ConstraintKind::Tangent {
            line_or_circle1,
            item2,
        } => vec![*line_or_circle1, *item2],
        ConstraintKind::Symmetric {
            point1,
            point2,
            line,
        } => vec![*point1, *point2, *line],
        ConstraintKind::SymmetricAboutPoint {
            point1,
            point2,
            center,
        } => vec![*point1, *point2, *center],
        ConstraintKind::Midpoint { point, line } => vec![*point, *line],
        ConstraintKind::ArcLength { arc, .. } | ConstraintKind::ArcAngle { arc, .. } => {
            vec![*arc]
        }
        ConstraintKind::AngleThreePoints {
            point1,
            vertex,
            point2,
            ..
        } => vec![*point1, *vertex, *point2],
        ConstraintKind::Gap { item1, item2, .. } => vec![*item1, *item2],
        ConstraintKind::AngleAtPoint {
            curve1,
            curve2,
            point,
            ..
        } => vec![*curve1, *curve2, *point],
        ConstraintKind::Refraction {
            ray1,
            ray2,
            interface,
            point,
            ..
        } => vec![*ray1, *ray2, *interface, *point],
        ConstraintKind::InternalAlignment { element, curve, .. } => vec![*element, *curve],
    }
}

/// Short human label for a constraint kind (left-panel list).
pub fn constraint_label(kind: &ConstraintKind) -> String {
    match kind {
        ConstraintKind::FixedPoint { .. } => "Fixed point".to_string(),
        ConstraintKind::Coincident { .. } => "Coincident".to_string(),
        ConstraintKind::Parallel { .. } => "Parallel".to_string(),
        ConstraintKind::Perpendicular { .. } => "Perpendicular".to_string(),
        ConstraintKind::EqualLength { .. } => "Equal length".to_string(),
        ConstraintKind::Length { .. } => "Length".to_string(),
        ConstraintKind::EqualRadius { .. } => "Equal radius".to_string(),
        ConstraintKind::EqualEllipse { .. } => "Equal ellipse".to_string(),
        ConstraintKind::Radius { .. } => "Radius".to_string(),
        ConstraintKind::Diameter { .. } => "Diameter".to_string(),
        ConstraintKind::PointOnLine { .. } => "Point on line".to_string(),
        ConstraintKind::PointOnCircle { .. } => "Point on circle".to_string(),
        ConstraintKind::PointOnEllipse { .. } => "Point on ellipse".to_string(),
        ConstraintKind::PointOnCurve { .. } => "Point on curve".to_string(),
        ConstraintKind::EllipseRadius { major: true, .. } => "Major radius".to_string(),
        ConstraintKind::EllipseRadius { major: false, .. } => "Minor radius".to_string(),
        ConstraintKind::CurveLength { .. } => "Curve length".to_string(),
        ConstraintKind::Offset { .. } => "Offset".to_string(),
        ConstraintKind::Pitch { across: false, .. } => "Pitch".to_string(),
        ConstraintKind::Pitch { across: true, .. } => "Row pitch".to_string(),
        ConstraintKind::PolarPitch { .. } => "Angular pitch".to_string(),
        ConstraintKind::TangentCurves { .. } => "Tangent".to_string(),
        ConstraintKind::PerpendicularCurves { .. } => "Perpendicular".to_string(),
        ConstraintKind::Horizontal { .. } | ConstraintKind::HorizontalPoints { .. } => {
            "Horizontal".to_string()
        }
        ConstraintKind::Vertical { .. } | ConstraintKind::VerticalPoints { .. } => {
            "Vertical".to_string()
        }
        ConstraintKind::Block { .. } => "Block".to_string(),
        ConstraintKind::Distance { .. } => "Distance".to_string(),
        ConstraintKind::DistanceX { .. } => "Distance X".to_string(),
        ConstraintKind::DistanceY { .. } => "Distance Y".to_string(),
        ConstraintKind::Angle { .. } => "Angle".to_string(),
        ConstraintKind::AngleToAxis {
            axis: AxisDirection::Horizontal,
            ..
        } => "Angle to X axis".to_string(),
        ConstraintKind::AngleToAxis {
            axis: AxisDirection::Vertical,
            ..
        } => "Angle to Y axis".to_string(),
        ConstraintKind::Tangent { .. } => "Tangent".to_string(),
        ConstraintKind::Symmetric { .. } => "Symmetric".to_string(),
        ConstraintKind::SymmetricAboutPoint { .. } => "Symmetric (point)".to_string(),
        ConstraintKind::Midpoint { .. } => "Midpoint".to_string(),
        ConstraintKind::ArcLength { .. } => "Arc length".to_string(),
        ConstraintKind::Gap { .. } => "Gap".to_string(),
        ConstraintKind::AngleAtPoint { .. } => "Angle at point".to_string(),
        ConstraintKind::ArcAngle { .. } => "Arc angle".to_string(),
        ConstraintKind::AngleThreePoints { .. } => "Angle by three points".to_string(),
        ConstraintKind::Refraction { .. } => "Refraction".to_string(),
        ConstraintKind::InternalAlignment { role, .. } => {
            format!("Internal alignment ({})", role.label())
        }
    }
}

/// Current geometric value of a dimensional constraint, measured from the
/// sketch (reference dimensions display this instead of driving anything).
/// `None` for non-dimensional kinds or unresolvable references.
pub fn measured_value(sketch: &Sketch, kind: &ConstraintKind) -> Option<f32> {
    let line_dir = |id: Uuid| -> Option<glam::Vec2> {
        match sketch.get_geometry(id)? {
            GeometryElement::Line(l) => {
                Some((sketch.point_position(l.end)? - sketch.point_position(l.start)?).to_glam())
            }
            _ => None,
        }
    };
    let radius_of = |id: Uuid| -> Option<f32> {
        match sketch.get_geometry(id)? {
            GeometryElement::Circle(c) => Some(c.radius),
            GeometryElement::Arc(a) => Some(a.radius),
            _ => None,
        }
    };
    match *kind {
        ConstraintKind::Length { line, .. } => Some(line_dir(line)?.length()),
        ConstraintKind::Radius { circle, .. } => radius_of(circle),
        ConstraintKind::Diameter { circle, .. } => Some(2.0 * radius_of(circle)?),
        ConstraintKind::Distance { point1, point2, .. } => Some(
            (sketch.point_position(point2)? - sketch.point_position(point1)?)
                .to_glam()
                .length(),
        ),
        ConstraintKind::DistanceX { a, b, .. } => {
            let xa = sketch.point_position(a)?.x;
            Some(match b {
                Some(b) => (sketch.point_position(b)?.x - xa).abs(),
                None => xa.abs(),
            })
        }
        ConstraintKind::DistanceY { a, b, .. } => {
            let ya = sketch.point_position(a)?.y;
            Some(match b {
                Some(b) => (sketch.point_position(b)?.y - ya).abs(),
                None => ya.abs(),
            })
        }
        ConstraintKind::Angle { line1, line2, .. } => {
            let d1 = line_dir(line1)?;
            let d2 = line_dir(line2)?;
            Some(d1.angle_to(d2).to_degrees())
        }
        ConstraintKind::AngleToAxis { line, axis, .. } => {
            let d = line_dir(line)?;
            let base = match axis {
                AxisDirection::Horizontal => 0.0,
                AxisDirection::Vertical => std::f32::consts::FRAC_PI_2,
            };
            Some((d.y.atan2(d.x) - base).to_degrees())
        }
        ConstraintKind::ArcLength { arc, .. } => crate::measure::arc_length(sketch, arc),
        ConstraintKind::Gap { item1, item2, .. } => {
            crate::measure::gap(sketch, item1, item2).map(|g| g.distance)
        }
        ConstraintKind::ArcAngle { arc, .. } => {
            crate::measure::arc_sweep(sketch, arc).map(f32::to_degrees)
        }
        ConstraintKind::EllipseRadius { ellipse, major, .. } => {
            match sketch.get_geometry(ellipse) {
                Some(GeometryElement::Ellipse(e)) => {
                    let a = e.major.to_glam().length();
                    Some(if major { a } else { a * e.ratio })
                }
                _ => None,
            }
        }
        ConstraintKind::CurveLength { curve, .. } => crate::measure::curve_length(sketch, curve),
        ConstraintKind::Offset { ref pairs, .. } => {
            let [a, b] = *pairs.first()?;
            crate::measure::gap(sketch, a, b).map(|g| g.distance)
        }
        ConstraintKind::Pitch { .. } => {
            let (a, b) = pitch_ends(kind)?;
            Some(
                (sketch.point_position(b)? - sketch.point_position(a)?)
                    .to_glam()
                    .length(),
            )
        }
        ConstraintKind::PolarPitch {
            center, ref points, ..
        } => crate::measure::angle_three_points(sketch, *points.first()?, center, *points.get(1)?)
            .map(f32::to_degrees),
        ConstraintKind::AngleThreePoints {
            point1,
            vertex,
            point2,
            ..
        } => {
            crate::measure::angle_three_points(sketch, point1, vertex, point2).map(f32::to_degrees)
        }
        ConstraintKind::AngleAtPoint {
            curve1,
            curve2,
            point,
            ..
        } => crate::measure::angle_at_point(sketch, curve1, curve2, point).map(f32::to_degrees),
        ConstraintKind::Refraction {
            ray1,
            ray2,
            interface,
            point,
            ..
        } => crate::measure::refraction_ratio(sketch, ray1, ray2, interface, point),
        _ => None,
    }
}

/// Editable value of a dimensional kind in display units (degrees for
/// angles, mm otherwise); `None` for non-dimensional kinds.
pub fn dimension_value(kind: &ConstraintKind) -> Option<f32> {
    match *kind {
        ConstraintKind::Length { length, .. } => Some(length),
        ConstraintKind::Radius { radius, .. } => Some(radius),
        ConstraintKind::Diameter { diameter, .. } => Some(diameter),
        ConstraintKind::Distance { distance, .. } => Some(distance),
        ConstraintKind::DistanceX { value, .. } | ConstraintKind::DistanceY { value, .. } => {
            Some(value)
        }
        ConstraintKind::Angle { angle_rad, .. }
        | ConstraintKind::AngleToAxis { angle_rad, .. }
        | ConstraintKind::AngleAtPoint { angle_rad, .. }
        | ConstraintKind::ArcAngle { angle_rad, .. }
        | ConstraintKind::AngleThreePoints { angle_rad, .. } => Some(angle_rad.to_degrees()),
        ConstraintKind::ArcLength { length, .. } | ConstraintKind::CurveLength { length, .. } => {
            Some(length)
        }
        ConstraintKind::EllipseRadius { radius, .. } => Some(radius),
        ConstraintKind::Gap { distance, .. }
        | ConstraintKind::Offset { distance, .. }
        | ConstraintKind::Pitch { distance, .. } => Some(distance),
        ConstraintKind::PolarPitch { angle_rad, .. } => Some(angle_rad.to_degrees()),
        ConstraintKind::Refraction { ratio, .. } => Some(ratio),
        _ => None,
    }
}

/// The kind with its dimensional value replaced (display units, see
/// `dimension_value`). Non-dimensional kinds are returned unchanged.
pub fn with_dimension_value(kind: &ConstraintKind, v: f32) -> ConstraintKind {
    let mut kind = kind.clone();
    match &mut kind {
        ConstraintKind::Length { length, .. } => *length = v,
        ConstraintKind::Radius { radius, .. } => *radius = v,
        ConstraintKind::Diameter { diameter, .. } => *diameter = v,
        ConstraintKind::Distance { distance, .. } => *distance = v,
        ConstraintKind::DistanceX { value, .. } | ConstraintKind::DistanceY { value, .. } => {
            *value = v
        }
        ConstraintKind::Angle { angle_rad, .. }
        | ConstraintKind::AngleToAxis { angle_rad, .. }
        | ConstraintKind::AngleAtPoint { angle_rad, .. }
        | ConstraintKind::ArcAngle { angle_rad, .. }
        | ConstraintKind::AngleThreePoints { angle_rad, .. } => *angle_rad = v.to_radians(),
        ConstraintKind::ArcLength { length, .. } | ConstraintKind::CurveLength { length, .. } => {
            *length = v
        }
        ConstraintKind::EllipseRadius { radius, .. } => *radius = v,
        ConstraintKind::Gap { distance, .. }
        | ConstraintKind::Offset { distance, .. }
        | ConstraintKind::Pitch { distance, .. } => *distance = v,
        ConstraintKind::PolarPitch { angle_rad, .. } => *angle_rad = v.to_radians(),
        ConstraintKind::Refraction { ratio, .. } => *ratio = v,
        _ => {}
    }
    kind
}

/// Whether a dimensional kind is angular (displayed in degrees).
pub fn is_angular(kind: &ConstraintKind) -> bool {
    dimension_unit(kind) == DimensionUnit::Angle
}

/// What a dimension's value measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimensionUnit {
    /// A length, in mm.
    Length,
    /// An angle, shown in degrees.
    Angle,
    /// A plain number (a refraction's ratio of indices).
    Ratio,
}

/// The unit of a dimensional kind's value (`Length` for the rest).
pub fn dimension_unit(kind: &ConstraintKind) -> DimensionUnit {
    match kind {
        ConstraintKind::Angle { .. }
        | ConstraintKind::AngleToAxis { .. }
        | ConstraintKind::AngleAtPoint { .. }
        | ConstraintKind::ArcAngle { .. }
        | ConstraintKind::AngleThreePoints { .. }
        | ConstraintKind::PolarPitch { .. } => DimensionUnit::Angle,
        ConstraintKind::Refraction { .. } => DimensionUnit::Ratio,
        _ => DimensionUnit::Length,
    }
}

/// The two points a pitch measures: the first member's and the next one
/// along a row, or in the next row.
pub fn pitch_ends(kind: &ConstraintKind) -> Option<(Uuid, Uuid)> {
    let ConstraintKind::Pitch {
        points,
        columns,
        across,
        ..
    } = kind
    else {
        return None;
    };
    let step = if *across {
        (*columns).max(1) as usize
    } else {
        1
    };
    Some((*points.first()?, *points.get(step)?))
}

/// Reference plane for a sketch (2D coordinate system in 3D space).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SketchPlane {
    /// Origin point in world space.
    pub origin: [f32; 3],
    /// Normal vector (defines the plane orientation).
    pub normal: [f32; 3],
    /// X-axis direction in the plane (orthogonal to normal).
    pub x_axis: [f32; 3],
    /// Y-axis direction in the plane (orthogonal to normal and x_axis).
    pub y_axis: [f32; 3],
}

impl SketchPlane {
    /// Top plane: sketch on XY, normal +Z.
    pub fn xy() -> Self {
        Self {
            origin: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
        }
    }

    /// Front plane: sketch on XZ, normal -Y (x right, z up, right-handed:
    /// x_axis × y_axis = normal).
    pub fn xz() -> Self {
        Self {
            origin: [0.0, 0.0, 0.0],
            normal: [0.0, -1.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 0.0, 1.0],
        }
    }

    /// Side plane: sketch on YZ, normal +X.
    pub fn yz() -> Self {
        Self {
            origin: [0.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [0.0, 0.0, 1.0],
        }
    }

    /// The origin plane `plane`, as its button in the plane picker gives.
    pub fn of_base(plane: core_document::BasePlane) -> Self {
        match plane {
            core_document::BasePlane::XY => Self::xy(),
            core_document::BasePlane::XZ => Self::xz(),
            core_document::BasePlane::YZ => Self::yz(),
        }
    }
}

impl SketchPlane {
    /// Build a plane from a surface point + outward normal (e.g. a picked
    /// solid face). Axes are derived deterministically: x along the most
    /// stable world axis projected into the plane, y = normal × x.
    pub fn from_face(point: [f32; 3], normal: [f32; 3]) -> Self {
        let n = glam::Vec3::from_array(normal).normalize();
        // Reference direction least aligned with the normal.
        let reference = if n.z.abs() < 0.9 {
            glam::Vec3::Z
        } else {
            glam::Vec3::Y
        };
        let x_axis = reference.cross(n).normalize();
        let y_axis = n.cross(x_axis).normalize();
        // The document origin dropped onto the plane, not the spot that was
        // clicked: every sketch on a given plane then shares one frame, and
        // geometry built around the origin keeps its place on the far face.
        let picked = glam::Vec3::from_array(point);
        let origin = n * picked.dot(n);
        Self {
            origin: origin.to_array(),
            normal: n.to_array(),
            x_axis: x_axis.to_array(),
            y_axis: y_axis.to_array(),
        }
    }

    /// Build a plane from a fully resolved frame (e.g. a datum plane's
    /// placement, which carries its own in-plane x-axis).
    pub fn from_frame(origin: [f32; 3], normal: [f32; 3], x_axis: [f32; 3]) -> Self {
        let n = glam::Vec3::from_array(normal).normalize();
        let x = glam::Vec3::from_array(x_axis).normalize();
        let y_axis = n.cross(x).normalize();
        Self {
            origin,
            normal: n.to_array(),
            x_axis: x.to_array(),
            y_axis: y_axis.to_array(),
        }
    }
}

impl Default for SketchPlane {
    fn default() -> Self {
        Self::xy()
    }
}

/// A geometry element in a sketch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeometryElement {
    Point(Point),
    Line(Line),
    Arc(Arc),
    Circle(Circle),
    Ellipse(Ellipse),
    BSpline(BSpline),
    Conic(Conic),
}

impl GeometryElement {
    pub fn id(&self) -> Uuid {
        match self {
            GeometryElement::Point(p) => p.id,
            GeometryElement::Line(l) => l.id,
            GeometryElement::Arc(a) => a.id,
            GeometryElement::Circle(c) => c.id,
            GeometryElement::Ellipse(e) => e.id,
            GeometryElement::BSpline(b) => b.id,
            GeometryElement::Conic(c) => c.id,
        }
    }
}

/// A point in 2D sketch space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    pub id: Uuid,
    /// Position in sketch coordinates (2D).
    pub position: Vec2D,
}

impl Point {
    pub fn new(position: Vec2D) -> Self {
        Self {
            id: Uuid::new_v4(),
            position,
        }
    }
}

/// A line segment between two points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Line {
    pub id: Uuid,
    /// Start point ID.
    pub start: Uuid,
    /// End point ID.
    pub end: Uuid,
}

impl Line {
    pub fn new(start: Uuid, end: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            start,
            end,
        }
    }
}

/// A circular arc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arc {
    pub id: Uuid,
    /// Center point ID.
    pub center: Uuid,
    /// Start point ID.
    pub start: Uuid,
    /// End point ID.
    pub end: Uuid,
    /// Radius (can be computed from center to start, but stored for constraints).
    pub radius: f32,
}

impl Arc {
    pub fn new(center: Uuid, start: Uuid, end: Uuid, radius: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            center,
            start,
            end,
            radius,
        }
    }
}

/// A circle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Circle {
    pub id: Uuid,
    /// Center point ID.
    pub center: Uuid,
    /// Radius.
    pub radius: f32,
}

impl Circle {
    pub fn new(center: Uuid, radius: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            center,
            radius,
        }
    }
}

/// A full ellipse. `major` is the vector from the center to one major-axis
/// vertex; the minor radius is `|major| * ratio` with `ratio` in (0, 1].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ellipse {
    pub id: Uuid,
    /// Center point ID.
    pub center: Uuid,
    /// Center → major vertex vector (defines size and rotation).
    pub major: Vec2D,
    /// Minor radius as a fraction of the major radius.
    pub ratio: f32,
    /// When set, only the arc between these two points is drawn:
    /// counter-clockwise in the ellipse's own frame, `start` to `end`.
    /// Both points sit on the ellipse, held there by point-on-ellipse
    /// constraints, so the arc is wherever they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arc: Option<EllipseArcEnds>,
}

/// The endpoints of an arc of an ellipse.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EllipseArcEnds {
    pub start: Uuid,
    pub end: Uuid,
}

impl Ellipse {
    pub fn new(center: Uuid, major: Vec2D, ratio: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            center,
            major,
            ratio,
            arc: None,
        }
    }

    /// An arc of an ellipse, from `start` to `end` counter-clockwise.
    pub fn new_arc(center: Uuid, major: Vec2D, ratio: f32, start: Uuid, end: Uuid) -> Self {
        Self {
            arc: Some(EllipseArcEnds { start, end }),
            ..Self::new(center, major, ratio)
        }
    }

    /// The ellipse's parameter at a point: the angle, in the ellipse's own
    /// frame, of where the point would sit on the circle the ellipse is
    /// squashed from. A point on the ellipse is `center + major·cos t +
    /// minor·sin t` at its parameter `t`.
    pub fn param_at(&self, center: Vec2D, point: Vec2D) -> f32 {
        let major = self.major.to_glam();
        let a = major.length();
        if a <= 1e-12 {
            return 0.0;
        }
        let u = major / a;
        let d = (point - center).to_glam();
        let b = (a * self.ratio).max(1e-12);
        (d.dot(u.perp()) / b).atan2(d.dot(u) / a)
    }

    /// The span of parameters the curve covers, `(t0, t1)` with `t1 > t0`:
    /// the whole turn for an ellipse, from start to end for an arc.
    pub fn param_span(&self, sketch: &Sketch) -> Option<(f32, f32)> {
        let Some(arc) = self.arc else {
            return Some((0.0, std::f32::consts::TAU));
        };
        let center = sketch.point_position(self.center)?;
        let t0 = self.param_at(center, sketch.point_position(arc.start)?);
        let mut t1 = self.param_at(center, sketch.point_position(arc.end)?);
        while t1 <= t0 + 1e-6 {
            t1 += std::f32::consts::TAU;
        }
        Some((t0, t1))
    }

    /// The curve sampled at `segments` intervals: the whole ellipse, or its
    /// arc end to end.
    pub fn points(&self, sketch: &Sketch, segments: usize) -> Option<Vec<Vec2D>> {
        let center = sketch.point_position(self.center)?;
        let (t0, t1) = self.param_span(sketch)?;
        Some(crate::geom2d::ellipse_arc_points(
            center, self.major, self.ratio, t0, t1, segments,
        ))
    }
}

/// A B-spline over point-element control points. An open spline runs
/// first → last control point; a periodic one closes smoothly on itself.
/// A spline drawn through points keeps them, and the solver holds it
/// through each at the parameter it was drawn at, so moving one reshapes
/// the curve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BSpline {
    pub id: Uuid,
    /// Control point IDs, in order.
    pub control_points: Vec<Uuid>,
    /// Whether the spline closes on itself.
    #[serde(default)]
    pub periodic: bool,
    /// The polynomial degree, at most one less than the control points;
    /// cubic for splines stored without one.
    #[serde(default = "cubic", skip_serializing_if = "is_cubic")]
    pub degree: u32,
    /// An open spline's clamped knot vector, `control points + degree + 1`
    /// of them; empty for evenly spaced knots. A periodic spline's knots
    /// are always evenly spaced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knots: Vec<f64>,
    /// The points it passes through, in order, when it was drawn through
    /// points: point elements of the sketch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fit_points: Vec<Uuid>,
    /// The parameter each fit point sits at.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fit_params: Vec<f64>,
    /// A rational spline's weight of each control point, one each; empty
    /// when every weight is one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weights: Vec<f64>,
}

fn cubic() -> u32 {
    3
}

fn is_cubic(degree: &u32) -> bool {
    *degree == 3
}

impl BSpline {
    /// A cubic over evenly spaced knots.
    pub fn new(control_points: Vec<Uuid>, periodic: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            control_points,
            periodic,
            degree: 3,
            knots: Vec::new(),
            fit_points: Vec::new(),
            fit_params: Vec::new(),
            weights: Vec::new(),
        }
    }

    /// This spline with `other`'s degree, knots and fit parameters: a copy
    /// of `other` over points of its own.
    pub fn with_shape_of(self, other: &BSpline) -> Self {
        Self {
            degree: other.degree,
            knots: other.knots.clone(),
            fit_params: other.fit_params.clone(),
            weights: other.weights.clone(),
            ..self
        }
    }

    /// Every point it is drawn through: its control points, and the points
    /// it passes through when it was drawn through points.
    pub fn point_ids(&self) -> Vec<Uuid> {
        let mut ids = self.control_points.clone();
        for id in &self.fit_points {
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
        ids
    }
}

/// Which curve a conic arc is an arc of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConicKind {
    Parabola,
    Hyperbola,
}

/// An arc of a parabola, or of one branch of a hyperbola, between two
/// points held on it. In the curve's own frame, `axis` along x and its
/// left-hand perpendicular along y, the parabola is `(t²/4f, t)` from its
/// vertex, `f` the focal distance, and the hyperbola's branch
/// `(a·cosh t, b·sinh t)` from its centre. The shape (`axis`, `minor`)
/// holds still in the solver; the centre and the end points move.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conic {
    pub id: Uuid,
    pub kind: ConicKind,
    /// The parabola's vertex; the hyperbola's centre.
    pub center: Uuid,
    /// Along the axis, the way the curve opens: for a parabola from the
    /// vertex to the focus (its length the focal distance), for a
    /// hyperbola from the centre to the vertex of the branch (its length
    /// the semi-major axis).
    pub axis: Vec2D,
    /// The hyperbola's semi-minor axis; a parabola has none.
    #[serde(default)]
    pub minor: f32,
    /// Where the arc starts and ends: points on the curve.
    pub start: Uuid,
    pub end: Uuid,
}

impl Conic {
    pub fn new(
        kind: ConicKind,
        center: Uuid,
        axis: Vec2D,
        minor: f32,
        start: Uuid,
        end: Uuid,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            center,
            axis,
            minor,
            start,
            end,
        }
    }
}

/// A sketch axis direction for constraints against the coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisDirection {
    Horizontal,
    Vertical,
}

/// A constraint record: the geometric relation plus solver metadata.
#[derive(Debug, Clone, Serialize)]
pub struct Constraint {
    pub id: Uuid,
    pub kind: ConstraintKind,
    /// Dimensional constraints only: `false` makes it a *reference*
    /// dimension: measured and displayed, never enforced.
    pub driving: bool,
    /// `false` keeps the constraint but excludes it from the solve.
    pub active: bool,
    /// Optional user-facing name (falls back to the kind label).
    pub name: Option<String>,
    /// Viewport-glyph offset from the anchor, in sketch units (dimensional
    /// constraints; set by dragging the label).
    #[serde(default)]
    pub label_offset: Option<Vec2D>,
    /// Its glyph is on the parked layer, drawn only while that layer is
    /// shown; it solves all the same.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub parked: bool,
}

impl Constraint {
    pub fn new(kind: ConstraintKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            driving: true,
            active: true,
            name: None,
            label_offset: None,
            parked: false,
        }
    }

    /// Whether this constraint contributes residuals to the solver
    /// (active, and driving whenever the kind is dimensional).
    pub fn is_solved(&self) -> bool {
        self.active && (self.driving || !self.kind.is_dimensional())
    }
}

impl From<ConstraintKind> for Constraint {
    fn from(kind: ConstraintKind) -> Self {
        Self::new(kind)
    }
}

// Back-compat: sketches saved before the constraint record existed store a
// bare `ConstraintKind`. Accept both shapes; legacy records get defaults
// (driving, active, unnamed, fresh id).
impl<'de> Deserialize<'de> for Constraint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        fn default_true() -> bool {
            true
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Full {
                #[serde(default = "Uuid::new_v4")]
                id: Uuid,
                kind: ConstraintKind,
                #[serde(default = "default_true")]
                driving: bool,
                #[serde(default = "default_true")]
                active: bool,
                #[serde(default)]
                name: Option<String>,
                #[serde(default)]
                label_offset: Option<Vec2D>,
                #[serde(default)]
                parked: bool,
            },
            Legacy(ConstraintKind),
        }

        Ok(match Repr::deserialize(deserializer)? {
            Repr::Full {
                id,
                kind,
                driving,
                active,
                name,
                label_offset,
                parked,
            } => Constraint {
                id,
                kind,
                driving,
                active,
                name,
                label_offset,
                parked,
            },
            Repr::Legacy(kind) => Constraint::new(kind),
        })
    }
}

/// A geometric relation applied to sketch geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConstraintKind {
    /// Point is fixed at a specific position.
    FixedPoint { point: Uuid, position: Vec2D },
    /// Two points are coincident.
    Coincident { point1: Uuid, point2: Uuid },
    /// Two lines are parallel.
    Parallel { line1: Uuid, line2: Uuid },
    /// Two lines are perpendicular.
    Perpendicular { line1: Uuid, line2: Uuid },
    /// Two lines are equal in length.
    EqualLength { line1: Uuid, line2: Uuid },
    /// Line has a specific length.
    Length { line: Uuid, length: f32 },
    /// Two circles/arcs have equal radius.
    EqualRadius { circle1: Uuid, circle2: Uuid },
    /// Circle/arc has a specific radius.
    Radius { circle: Uuid, radius: f32 },
    /// Circle/arc has a specific diameter.
    Diameter { circle: Uuid, diameter: f32 },
    /// Point lies on a line.
    PointOnLine { point: Uuid, line: Uuid },
    /// Point lies on a circle/arc.
    PointOnCircle { point: Uuid, circle: Uuid },
    /// Point lies on an ellipse.
    PointOnEllipse { point: Uuid, ellipse: Uuid },
    /// A line is horizontal.
    Horizontal { element: Uuid },
    /// A line is vertical.
    Vertical { element: Uuid },
    /// Two points level with each other: the same y.
    HorizontalPoints { point1: Uuid, point2: Uuid },
    /// Two points one above the other: the same x.
    VerticalPoints { point1: Uuid, point2: Uuid },
    /// Freeze every point (and the radius) of one element where it is now.
    Block { element: Uuid },
    /// Distance between two points.
    Distance {
        point1: Uuid,
        point2: Uuid,
        distance: f32,
    },
    /// Horizontal distance between two points (`b = None` measures `a`
    /// from the sketch origin): either way round when positive, `b` to the
    /// left of `a` when negative.
    DistanceX {
        a: Uuid,
        b: Option<Uuid>,
        value: f32,
    },
    /// Vertical distance between two points (`b = None` measures `a`
    /// from the sketch origin): either way round when positive, `b` below
    /// `a` when negative.
    DistanceY {
        a: Uuid,
        b: Option<Uuid>,
        value: f32,
    },
    /// Angle between two lines.
    Angle {
        line1: Uuid,
        line2: Uuid,
        angle_rad: f32,
    },
    /// Angle between a line and a sketch axis.
    AngleToAxis {
        line: Uuid,
        axis: AxisDirection,
        angle_rad: f32,
    },
    /// Tangency: a line tangent to a circle/arc, or two circles/arcs
    /// tangent to each other (externally or internally, whichever is
    /// closer to the current configuration when the solve starts).
    Tangent { line_or_circle1: Uuid, item2: Uuid },
    /// Two points mirror-symmetric about a line.
    Symmetric {
        point1: Uuid,
        point2: Uuid,
        line: Uuid,
    },
    /// Two points symmetric about a center point.
    SymmetricAboutPoint {
        point1: Uuid,
        point2: Uuid,
        center: Uuid,
    },
    /// Point sits at the midpoint of a line's endpoints.
    Midpoint { point: Uuid, line: Uuid },
    /// Length along an arc, from its start to its end.
    ArcLength { arc: Uuid, length: f32 },
    /// The shortest distance between two items that are not both points:
    /// a point, line, circle or arc (an arc as its whole circle) against a
    /// circle or arc, a point against a line, or two lines held parallel
    /// (the second's midpoint from the first's line: a wall's thickness).
    /// Two circles one inside the other measure the gap between them
    /// inside.
    Gap {
        item1: Uuid,
        item2: Uuid,
        distance: f32,
    },
    /// The angle two curves make where they meet: from the first curve's
    /// tangent at `point` to the second's (a line's direction runs start to
    /// end, a circle's or arc's counter-clockwise). `point` is expected to
    /// lie on both; the tool that makes it adds what keeps it there.
    AngleAtPoint {
        curve1: Uuid,
        curve2: Uuid,
        point: Uuid,
        angle_rad: f32,
    },
    /// An ellipse's semi-major (`major`) or semi-minor radius.
    EllipseRadius {
        ellipse: Uuid,
        major: bool,
        radius: f32,
    },
    /// The length along a spline, or along an arc of a parabola or
    /// hyperbola between its ends.
    CurveLength { curve: Uuid, length: f32 },
    /// A point on any curve: a spline, a parabola or hyperbola, or any of
    /// the others.
    PointOnCurve { point: Uuid, curve: Uuid },
    /// Two curves of any kind touching: where they meet, their directions
    /// agree.
    TangentCurves { curve1: Uuid, curve2: Uuid },
    /// Two curves of any kind crossing square: where they meet, their
    /// directions stand at a right angle.
    PerpendicularCurves { curve1: Uuid, curve2: Uuid },
    /// Two ellipses (or arcs of them) the same size: equal major and minor
    /// radii.
    EqualEllipse { ellipse1: Uuid, ellipse2: Uuid },
    /// Offset copies held `distance` from their originals, as one
    /// dimension: each pair is an original and its copy, two lines kept
    /// parallel that far apart, or two circles or arcs about one centre
    /// that much apart in radius.
    Offset {
        pairs: Vec<[Uuid; 2]>,
        distance: f32,
    },
    /// The members of an array stepped evenly: `points` holds one point of
    /// each, row by row, `columns` to a row, the original first. Along a
    /// row each steps from the one before as the second does from the
    /// first, `distance` apart along `direction` (any way, without one);
    /// `across`, it is the rows that step so from one another, the first
    /// column's points `distance` apart.
    Pitch {
        points: Vec<Uuid>,
        columns: u32,
        distance: f32,
        #[serde(default)]
        across: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        direction: Option<Vec2D>,
    },
    /// Rotated copies stepped evenly about `center`: each of `points` (one
    /// point of each copy, the original first) as far from it as the first,
    /// turned `angle_rad` counter-clockwise from the one before.
    PolarPitch {
        center: Uuid,
        points: Vec<Uuid>,
        angle_rad: f32,
    },
    /// An arc's opening: the angle it sweeps counter-clockwise from its
    /// start to its end.
    ArcAngle { arc: Uuid, angle_rad: f32 },
    /// The angle at `vertex` counter-clockwise from the arm to `point1` to
    /// the arm to `point2`.
    AngleThreePoints {
        point1: Uuid,
        vertex: Uuid,
        point2: Uuid,
        angle_rad: f32,
    },
    /// Refraction at an interface: `ray1` arrives at `point` on the
    /// `interface` curve and `ray2` leaves it, at angles to the interface's
    /// normal whose sines are in `ratio` (sin in / sin out, the index of
    /// refraction of the second medium over the first's).
    Refraction {
        ray1: Uuid,
        ray2: Uuid,
        interface: Uuid,
        point: Uuid,
        ratio: f32,
    },
    /// `element` is a piece of `curve`'s internal geometry, held where
    /// `role` puts it: an axis or a focus of an ellipse or conic, a side of
    /// a spline's control polygon.
    InternalAlignment {
        element: Uuid,
        curve: Uuid,
        role: InternalRole,
    },
}

/// What a piece of internal geometry stands for on its curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InternalRole {
    /// A line along the axis: an ellipse's from one vertex of the major
    /// axis to the other; a conic's from its centre along its axis, to the
    /// focus of a parabola, to the vertex of a hyperbola's branch.
    MajorAxis,
    /// A line across: an ellipse's minor axis, vertex to vertex; a
    /// hyperbola's, through its centre, twice its semi-minor axis long.
    MinorAxis,
    /// A point at a focus: an ellipse's on the major vertex's side, a
    /// parabola's, a hyperbola's branch's.
    Focus1,
    /// A point at an ellipse's other focus.
    Focus2,
    /// A line along the control polygon, from control point `n` to the
    /// next (to the first, round a periodic spline's end).
    ControlEdge(u32),
}

impl InternalRole {
    pub fn label(self) -> String {
        match self {
            InternalRole::MajorAxis => "major axis".to_string(),
            InternalRole::MinorAxis => "minor axis".to_string(),
            InternalRole::Focus1 => "focus".to_string(),
            InternalRole::Focus2 => "second focus".to_string(),
            InternalRole::ControlEdge(n) => format!("control polygon {}", n + 1),
        }
    }

    /// Whether the role is a line (else a point).
    pub fn is_line(self) -> bool {
        !matches!(self, InternalRole::Focus1 | InternalRole::Focus2)
    }
}

impl ConstraintKind {
    /// Dimensional constraints carry a numeric value that can be edited or
    /// demoted to a reference (driven) dimension.
    pub fn is_dimensional(&self) -> bool {
        matches!(
            self,
            ConstraintKind::Length { .. }
                | ConstraintKind::Radius { .. }
                | ConstraintKind::Diameter { .. }
                | ConstraintKind::Distance { .. }
                | ConstraintKind::DistanceX { .. }
                | ConstraintKind::DistanceY { .. }
                | ConstraintKind::Angle { .. }
                | ConstraintKind::AngleToAxis { .. }
                | ConstraintKind::ArcLength { .. }
                | ConstraintKind::CurveLength { .. }
                | ConstraintKind::EllipseRadius { .. }
                | ConstraintKind::Gap { .. }
                | ConstraintKind::Offset { .. }
                | ConstraintKind::Pitch { .. }
                | ConstraintKind::PolarPitch { .. }
                | ConstraintKind::AngleAtPoint { .. }
                | ConstraintKind::ArcAngle { .. }
                | ConstraintKind::AngleThreePoints { .. }
                | ConstraintKind::Refraction { .. }
        )
    }
}

#[cfg(test)]
mod construction_tests {
    use super::*;

    #[test]
    fn construction_flag_round_trips() {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 0.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));

        assert!(!sketch.is_construction(line));
        sketch.set_construction(line, true);
        assert!(sketch.is_construction(line));
        assert!(!sketch.is_construction(a), "flag applies per element");

        // Survives serde round-trip.
        let json = serde_json::to_value(&sketch).unwrap();
        let restored: Sketch = serde_json::from_value(json).unwrap();
        assert!(restored.is_construction(line));

        sketch.set_construction(line, false);
        assert!(!sketch.is_construction(line));
    }

    #[test]
    fn old_json_without_construction_field_deserializes() {
        // A sketch serialized before the `construction` field existed.
        let json = serde_json::json!({
            "id": Uuid::new_v4(),
            "name": "legacy",
            "plane": SketchPlane::xy(),
            "geometry": [],
            "constraints": [],
            "is_fully_constrained": false,
        });
        let sketch: Sketch = serde_json::from_value(json).expect("legacy sketch loads");
        assert!(sketch.construction.is_empty());
    }

    #[test]
    fn ellipse_and_bspline_round_trip_and_cascade() {
        let mut sketch = Sketch::new("t");
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let e = sketch.add_geometry(GeometryElement::Ellipse(Ellipse::new(
            c,
            Vec2D::new(4.0, 0.0),
            0.5,
        )));
        let p1 = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 5.0))));
        let p2 = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 8.0))));
        let p3 = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(9.0, 5.0))));
        let b = sketch.add_geometry(GeometryElement::BSpline(BSpline::new(
            vec![p1, p2, p3],
            false,
        )));

        // Serde round-trip preserves both element kinds.
        let json = serde_json::to_value(&sketch).unwrap();
        let restored: Sketch = serde_json::from_value(json).unwrap();
        assert!(matches!(
            restored.get_geometry(e),
            Some(GeometryElement::Ellipse(el)) if (el.ratio - 0.5).abs() < 1e-6
        ));
        assert!(matches!(
            restored.get_geometry(b),
            Some(GeometryElement::BSpline(bs)) if bs.control_points.len() == 3 && !bs.periodic
        ));

        // Removing a referenced point cascades to the curve.
        sketch.remove_geometry_cascade(&[c]);
        assert!(sketch.get_geometry(e).is_none(), "ellipse follows center");
        sketch.remove_geometry_cascade(&[p2]);
        assert!(sketch.get_geometry(b).is_none(), "spline follows its cp");
        assert!(
            sketch.get_geometry(p1).is_none(),
            "the spline's other control points go with it"
        );
    }

    #[test]
    fn cascade_delete_cleans_construction_set() {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(5.0, 0.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.set_construction(line, true);
        sketch.remove_geometry_cascade(&[a]);
        assert!(sketch.construction.is_empty());
    }
}

#[cfg(test)]
mod constraint_record_tests {
    use super::*;

    #[test]
    fn legacy_bare_enum_constraint_deserializes_with_defaults() {
        // Exactly what pre-record sketches stored: the bare kind enum.
        let line = Uuid::new_v4();
        let json = serde_json::json!({ "Length": { "line": line, "length": 12.5 } });
        let c: Constraint = serde_json::from_value(json).expect("legacy constraint loads");
        assert!(matches!(
            c.kind,
            ConstraintKind::Length { line: l, length } if l == line && (length - 12.5).abs() < 1e-6
        ));
        assert!(c.driving && c.active && c.name.is_none());
    }

    #[test]
    fn full_record_without_label_offset_deserializes() {
        // A record saved before `label_offset` existed.
        let json = serde_json::json!({
            "id": Uuid::new_v4(),
            "kind": { "Horizontal": { "element": Uuid::new_v4() } },
            "driving": true,
            "active": true,
            "name": null,
        });
        let c: Constraint = serde_json::from_value(json).expect("pre-offset record loads");
        assert!(c.label_offset.is_none());
    }

    #[test]
    fn label_offset_round_trips() {
        let mut c = Constraint::new(ConstraintKind::Horizontal {
            element: Uuid::new_v4(),
        });
        c.label_offset = Some(Vec2D::new(-3.5, 2.0));
        let json = serde_json::to_value(&c).unwrap();
        let back: Constraint = serde_json::from_value(json).unwrap();
        let off = back.label_offset.unwrap();
        assert!((off.x + 3.5).abs() < 1e-6 && (off.y - 2.0).abs() < 1e-6);
    }

    #[test]
    fn a_parked_constraint_round_trips_and_an_unparked_one_says_nothing() {
        let mut c = Constraint::new(ConstraintKind::Horizontal {
            element: Uuid::new_v4(),
        });
        let json = serde_json::to_value(&c).unwrap();
        assert!(json.get("parked").is_none());
        c.parked = true;
        let back: Constraint = serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
        assert!(back.parked);
        assert!(back.is_solved(), "a parked constraint still solves");
    }

    #[test]
    fn full_constraint_record_round_trips() {
        let mut c = Constraint::new(ConstraintKind::Horizontal {
            element: Uuid::new_v4(),
        });
        c.driving = false;
        c.active = false;
        c.name = Some("width".to_string());
        let json = serde_json::to_value(&c).unwrap();
        let back: Constraint = serde_json::from_value(json).unwrap();
        assert_eq!(back.id, c.id);
        assert!(!back.driving && !back.active);
        assert_eq!(back.name.as_deref(), Some("width"));
    }

    #[test]
    fn curve_dimensions_round_trip_with_their_values() {
        let id = Uuid::new_v4;
        for kind in [
            ConstraintKind::ArcLength {
                arc: id(),
                length: 7.5,
            },
            ConstraintKind::Gap {
                item1: id(),
                item2: id(),
                distance: 2.0,
            },
            ConstraintKind::AngleAtPoint {
                curve1: id(),
                curve2: id(),
                point: id(),
                angle_rad: 0.5,
            },
            ConstraintKind::Refraction {
                ray1: id(),
                ray2: id(),
                interface: id(),
                point: id(),
                ratio: 1.5,
            },
        ] {
            assert!(kind.is_dimensional());
            let back: Constraint = serde_json::from_value(
                serde_json::to_value(Constraint::new(kind.clone())).unwrap(),
            )
            .unwrap();
            assert_eq!(dimension_value(&back.kind), dimension_value(&kind));
            assert_eq!(constraint_refs(&back.kind), constraint_refs(&kind));
        }
    }

    #[test]
    fn legacy_sketch_with_bare_constraints_deserializes() {
        let p = Uuid::new_v4();
        let json = serde_json::json!({
            "id": Uuid::new_v4(),
            "name": "legacy",
            "plane": SketchPlane::xy(),
            "geometry": [],
            "constraints": [
                { "FixedPoint": { "point": p, "position": { "x": 1.0, "y": 2.0 } } },
                { "Coincident": { "point1": p, "point2": p } },
            ],
            "is_fully_constrained": false,
        });
        let sketch: Sketch = serde_json::from_value(json).expect("legacy sketch loads");
        assert_eq!(sketch.constraints.len(), 2);
        assert!(sketch.constraints.iter().all(|c| c.driving && c.active));
    }

    #[test]
    fn measured_values_track_geometry() {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(1.0, 2.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(4.0, 6.0))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(a, 3.0)));

        let near = |v: Option<f32>, want: f32| (v.unwrap() - want).abs() < 1e-4;
        assert!(near(
            measured_value(&sketch, &ConstraintKind::Length { line, length: 0.0 }),
            5.0
        ));
        assert!(near(
            measured_value(
                &sketch,
                &ConstraintKind::Diameter {
                    circle,
                    diameter: 0.0
                }
            ),
            6.0
        ));
        assert!(near(
            measured_value(
                &sketch,
                &ConstraintKind::DistanceX {
                    a,
                    b: Some(b),
                    value: 0.0
                }
            ),
            3.0
        ));
        assert!(near(
            measured_value(
                &sketch,
                &ConstraintKind::DistanceY {
                    a,
                    b: None,
                    value: 0.0
                }
            ),
            2.0
        ));
        // Non-dimensional kinds have no measured value.
        assert!(measured_value(&sketch, &ConstraintKind::Horizontal { element: line }).is_none());
    }
}

#[cfg(test)]
mod plane_tests {
    use super::*;

    fn assert_orthonormal(plane: &SketchPlane) {
        let n = glam::Vec3::from_array(plane.normal);
        let x = glam::Vec3::from_array(plane.x_axis);
        let y = glam::Vec3::from_array(plane.y_axis);
        assert!((n.length() - 1.0).abs() < 1e-5);
        assert!((x.length() - 1.0).abs() < 1e-5);
        assert!((y.length() - 1.0).abs() < 1e-5);
        assert!(x.dot(n).abs() < 1e-5);
        assert!(y.dot(n).abs() < 1e-5);
        assert!(x.dot(y).abs() < 1e-5);
        // Right-handed: x × y = n.
        assert!((x.cross(y) - n).length() < 1e-5);
    }

    #[test]
    fn presets_are_orthonormal_and_right_handed() {
        for plane in [SketchPlane::xy(), SketchPlane::xz(), SketchPlane::yz()] {
            assert_orthonormal(&plane);
        }
    }

    #[test]
    fn face_plane_top_face_matches_world_axes() {
        // Top face of a padded box: normal +Z at height 8. Wherever the face
        // was clicked, the sketch starts at the document origin's projection.
        let plane = SketchPlane::from_face([5.0, 3.0, 8.0], [0.0, 0.0, 1.0]);
        assert_orthonormal(&plane);
        assert_eq!(plane.origin, [0.0, 0.0, 8.0]);
        let n = glam::Vec3::from_array(plane.normal);
        assert!((n - glam::Vec3::Z).length() < 1e-5);
    }

    #[test]
    fn facing_faces_of_one_solid_share_a_centre() {
        // A pad around the origin: sketches on its two ends land on the same
        // axis, so a circle drawn at 0,0 on one is concentric with the other.
        let front = SketchPlane::from_face([4.0, -2.0, 10.0], [0.0, 0.0, 1.0]);
        let back = SketchPlane::from_face([-7.0, 5.0, 0.0], [0.0, 0.0, -1.0]);
        assert_eq!(front.origin, [0.0, 0.0, 10.0]);
        assert_eq!(back.origin, [0.0, 0.0, 0.0]);
        assert_orthonormal(&front);
        assert_orthonormal(&back);
    }

    #[test]
    fn a_face_away_from_the_origin_keeps_its_own_plane() {
        // Normal +X at x = 500: the origin drops onto the plane, not to zero.
        let plane = SketchPlane::from_face([500.0, 120.0, 30.0], [1.0, 0.0, 0.0]);
        assert_eq!(plane.origin, [500.0, 0.0, 0.0]);
    }

    #[test]
    fn face_plane_side_and_arbitrary_normals() {
        // Front face (normal -Y): the plane must be usable for sketching.
        let plane = SketchPlane::from_face([0.0, 0.0, 0.0], [0.0, -1.0, 0.0]);
        assert_orthonormal(&plane);

        // Slanted face: still orthonormal, normalized input not required.
        let plane = SketchPlane::from_face([1.0, 2.0, 3.0], [1.0, 1.0, 1.0]);
        assert_orthonormal(&plane);
        let n = glam::Vec3::from_array(plane.normal);
        assert!((n - glam::Vec3::ONE.normalize()).length() < 1e-5);
    }
}
