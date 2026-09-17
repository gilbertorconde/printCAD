//! Names and icons shared by the viewport glyphs and the task panel: which
//! icon a constraint draws with, how an element is called in a list, and
//! how a sketch plane is labelled.

use uuid::Uuid;

use crate::sketch::{ConstraintKind, GeometryElement, Sketch, SketchPlane};

/// The design set's icon for a constraint kind.
pub fn constraint_icon(kind: &ConstraintKind) -> &'static str {
    match kind {
        ConstraintKind::FixedPoint { .. } => "constraint-lock",
        ConstraintKind::Coincident { .. } => "constraint-coincident",
        ConstraintKind::Parallel { .. } => "constraint-parallel",
        ConstraintKind::Perpendicular { .. } => "constraint-perpendicular",
        ConstraintKind::EqualLength { .. } | ConstraintKind::EqualRadius { .. } => {
            "constraint-equal"
        }
        ConstraintKind::Length { .. } | ConstraintKind::Distance { .. } => "constraint-distance",
        ConstraintKind::Radius { .. } => "constraint-radius",
        ConstraintKind::Diameter { .. } => "constraint-diameter",
        ConstraintKind::PointOnLine { .. }
        | ConstraintKind::PointOnCircle { .. }
        | ConstraintKind::PointOnEllipse { .. }
        | ConstraintKind::Midpoint { .. } => "constraint-point-on-object",
        ConstraintKind::Horizontal { .. } => "constraint-horizontal",
        ConstraintKind::Vertical { .. } => "constraint-vertical",
        ConstraintKind::Block { .. } => "constraint-block",
        ConstraintKind::DistanceX { .. } => "constraint-distance-x",
        ConstraintKind::DistanceY { .. } => "constraint-distance-y",
        ConstraintKind::Angle { .. } | ConstraintKind::AngleToAxis { .. } => "constraint-angle",
        ConstraintKind::Tangent { .. } => "constraint-tangent",
        ConstraintKind::Symmetric { .. } | ConstraintKind::SymmetricAboutPoint { .. } => {
            "constraint-symmetric"
        }
    }
}

/// The design set's icon for a geometry element.
pub fn element_icon(element: &GeometryElement) -> &'static str {
    match element {
        GeometryElement::Point(_) => "point",
        GeometryElement::Line(_) => "line",
        GeometryElement::Arc(_) => "arc",
        GeometryElement::Circle(_) => "circle",
        GeometryElement::Ellipse(_) => "ellipse",
        GeometryElement::BSpline(_) => "bspline",
    }
}

/// The element's kind as a word.
pub fn element_kind(element: &GeometryElement) -> &'static str {
    match element {
        GeometryElement::Point(_) => "Point",
        GeometryElement::Line(_) => "Line",
        GeometryElement::Arc(_) => "Arc",
        GeometryElement::Circle(_) => "Circle",
        GeometryElement::Ellipse(_) => "Ellipse",
        GeometryElement::BSpline(_) => "B-spline",
    }
}

/// "Line 3": the element's kind and its 1-based ordinal among elements of
/// that kind, in sketch order.
pub fn element_name(sketch: &Sketch, id: Uuid) -> String {
    if let Some(reference) = crate::sketch::Reference::of(id) {
        return reference.label().to_string();
    }
    let mut ordinal = 0;
    for g in &sketch.geometry {
        let same_kind = sketch
            .get_geometry(id)
            .map(|target| std::mem::discriminant(target) == std::mem::discriminant(g))
            .unwrap_or(false);
        if same_kind {
            ordinal += 1;
        }
        if g.id() == id {
            return format!("{} {ordinal}", element_kind(g));
        }
    }
    format!("Element {}", &id.to_string()[..8])
}

/// A short label for the plane a sketch sits on.
pub fn plane_label(plane: &SketchPlane) -> &'static str {
    let n = plane.normal;
    let along = |axis: usize| n[axis].abs() > 0.999;
    if along(2) {
        "XY plane"
    } else if along(1) {
        "XZ plane"
    } else if along(0) {
        "YZ plane"
    } else {
        "Face plane"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Line, Point, Vec2D};

    #[test]
    fn element_names_count_per_kind_in_sketch_order() {
        let mut sketch = Sketch::new("t");
        let a = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        let b = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(1.0, 0.0))));
        let l = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let c = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(2.0, 0.0))));
        assert_eq!(element_name(&sketch, a), "Point 1");
        assert_eq!(element_name(&sketch, l), "Line 1");
        assert_eq!(element_name(&sketch, c), "Point 3");
    }

    #[test]
    fn plane_labels_follow_the_normal() {
        assert_eq!(plane_label(&SketchPlane::xy()), "XY plane");
        assert_eq!(plane_label(&SketchPlane::xz()), "XZ plane");
        assert_eq!(plane_label(&SketchPlane::yz()), "YZ plane");
    }
}
