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
    /// Reference plane (normal vector and origin) - for now just a placeholder.
    pub plane: SketchPlane,
    /// Geometry elements in the sketch.
    pub geometry: Vec<GeometryElement>,
    /// Constraints applied to the geometry.
    pub constraints: Vec<Constraint>,
    /// Whether the sketch is fully constrained.
    pub is_fully_constrained: bool,
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
        }
    }

    /// Add a geometry element to the sketch.
    pub fn add_geometry(&mut self, element: GeometryElement) -> Uuid {
        let id = element.id();
        self.geometry.push(element);
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

    /// Position of a point element, if `id` refers to one.
    pub fn point_position(&self, id: Uuid) -> Option<Vec2D> {
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
        }
    }

    /// Remove elements by id with FreeCAD-style cascade semantics:
    /// - removing a point also removes every curve that references it;
    /// - removing a curve leaves its points in place (they may be shared);
    /// - constraints referencing any removed element are dropped.
    ///
    /// Returns the ids of every element actually removed.
    pub fn remove_geometry_cascade(&mut self, ids: &[Uuid]) -> Vec<Uuid> {
        use std::collections::HashSet;
        let mut doomed: HashSet<Uuid> = ids.iter().copied().collect();

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

        let removed: Vec<Uuid> = self
            .geometry
            .iter()
            .map(|g| g.id())
            .filter(|id| doomed.contains(id))
            .collect();
        self.geometry.retain(|g| !doomed.contains(&g.id()));
        self.constraints
            .retain(|c| !constraint_refs(c).iter().any(|id| doomed.contains(id)));
        removed
    }

    /// Ids of every point that no remaining curve references. Used to offer
    /// cleanup of construction leftovers; NOT auto-removed on delete because
    /// standalone points are legitimate sketch geometry.
    pub fn orphan_point_ids(&self) -> Vec<Uuid> {
        use std::collections::HashSet;
        let mut referenced: HashSet<Uuid> = HashSet::new();
        for geom in &self.geometry {
            referenced.extend(Self::curve_point_ids(geom));
        }
        self.geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Point(p) if !referenced.contains(&p.id) => Some(p.id),
                _ => None,
            })
            .collect()
    }
}

/// Every geometry id a constraint references.
pub fn constraint_refs(constraint: &Constraint) -> Vec<Uuid> {
    match constraint {
        Constraint::FixedPoint { point, .. } => vec![*point],
        Constraint::Coincident { point1, point2 } => vec![*point1, *point2],
        Constraint::Parallel { line1, line2 }
        | Constraint::Perpendicular { line1, line2 }
        | Constraint::EqualLength { line1, line2 } => vec![*line1, *line2],
        Constraint::Length { line, .. } => vec![*line],
        Constraint::EqualRadius { circle1, circle2 } => vec![*circle1, *circle2],
        Constraint::Radius { circle, .. } => vec![*circle],
        Constraint::PointOnLine { point, line } => vec![*point, *line],
        Constraint::PointOnCircle { point, circle } => vec![*point, *circle],
        Constraint::Horizontal { element } | Constraint::Vertical { element } => vec![*element],
        Constraint::Distance { point1, point2, .. } => vec![*point1, *point2],
        Constraint::Angle { line1, line2, .. } => vec![*line1, *line2],
    }
}

/// Short human label for a constraint (left-panel list).
pub fn constraint_label(constraint: &Constraint) -> String {
    match constraint {
        Constraint::FixedPoint { .. } => "Fixed point".to_string(),
        Constraint::Coincident { .. } => "Coincident".to_string(),
        Constraint::Parallel { .. } => "Parallel".to_string(),
        Constraint::Perpendicular { .. } => "Perpendicular".to_string(),
        Constraint::EqualLength { .. } => "Equal length".to_string(),
        Constraint::Length { length, .. } => format!("Length {length:.2}"),
        Constraint::EqualRadius { .. } => "Equal radius".to_string(),
        Constraint::Radius { radius, .. } => format!("Radius {radius:.2}"),
        Constraint::PointOnLine { .. } => "Point on line".to_string(),
        Constraint::PointOnCircle { .. } => "Point on circle".to_string(),
        Constraint::Horizontal { .. } => "Horizontal".to_string(),
        Constraint::Vertical { .. } => "Vertical".to_string(),
        Constraint::Distance { distance, .. } => format!("Distance {distance:.2}"),
        Constraint::Angle { angle_rad, .. } => {
            format!("Angle {:.1}°", angle_rad.to_degrees())
        }
    }
}

/// Reference plane for a sketch (2D coordinate system in 3D space).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
        Self {
            origin: point,
            normal: n.to_array(),
            x_axis: x_axis.to_array(),
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
}

impl GeometryElement {
    pub fn id(&self) -> Uuid {
        match self {
            GeometryElement::Point(p) => p.id,
            GeometryElement::Line(l) => l.id,
            GeometryElement::Arc(a) => a.id,
            GeometryElement::Circle(c) => c.id,
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

/// A constraint applied to sketch geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Constraint {
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
    /// Point lies on a line.
    PointOnLine { point: Uuid, line: Uuid },
    /// Point lies on a circle/arc.
    PointOnCircle { point: Uuid, circle: Uuid },
    /// Horizontal constraint (line is horizontal, or two points have same Y).
    Horizontal { element: Uuid },
    /// Vertical constraint (line is vertical, or two points have same X).
    Vertical { element: Uuid },
    /// Distance between two points.
    Distance {
        point1: Uuid,
        point2: Uuid,
        distance: f32,
    },
    /// Angle between two lines.
    Angle {
        line1: Uuid,
        line2: Uuid,
        angle_rad: f32,
    },
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
        // Top face of a padded box: normal +Z at height 8.
        let plane = SketchPlane::from_face([5.0, 3.0, 8.0], [0.0, 0.0, 1.0]);
        assert_orthonormal(&plane);
        assert_eq!(plane.origin, [5.0, 3.0, 8.0]);
        let n = glam::Vec3::from_array(plane.normal);
        assert!((n - glam::Vec3::Z).length() < 1e-5);
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
