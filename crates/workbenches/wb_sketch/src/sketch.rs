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

impl Default for SketchPlane {
    fn default() -> Self {
        // Default to XY plane at origin
        Self {
            origin: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
        }
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
