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
        ConstraintKind::Perpendicular { .. } | ConstraintKind::PerpendicularCurves { .. } => {
            "constraint-perpendicular"
        }
        ConstraintKind::EqualLength { .. }
        | ConstraintKind::EqualRadius { .. }
        | ConstraintKind::EqualEllipse { .. } => "constraint-equal",
        ConstraintKind::Length { .. }
        | ConstraintKind::Distance { .. }
        | ConstraintKind::Gap { .. }
        | ConstraintKind::Offset { .. }
        | ConstraintKind::Pitch { .. } => "constraint-distance",
        ConstraintKind::PolarPitch { .. } => "constraint-angle",
        ConstraintKind::ArcLength { .. } => "constraint-arc-length",
        ConstraintKind::ArcAngle { .. } | ConstraintKind::AngleThreePoints { .. } => {
            "constraint-angle"
        }
        ConstraintKind::Refraction { .. } => "constraint-refraction",
        ConstraintKind::Radius { .. } | ConstraintKind::EllipseRadius { .. } => "constraint-radius",
        ConstraintKind::CurveLength { .. } => "constraint-arc-length",
        ConstraintKind::Diameter { .. } => "constraint-diameter",
        ConstraintKind::PointOnLine { .. }
        | ConstraintKind::PointOnCircle { .. }
        | ConstraintKind::PointOnEllipse { .. }
        | ConstraintKind::PointOnCurve { .. }
        | ConstraintKind::Midpoint { .. } => "constraint-point-on-object",
        ConstraintKind::Horizontal { .. } | ConstraintKind::HorizontalPoints { .. } => {
            "constraint-horizontal"
        }
        ConstraintKind::Vertical { .. } | ConstraintKind::VerticalPoints { .. } => {
            "constraint-vertical"
        }
        ConstraintKind::Block { .. } => "constraint-block",
        ConstraintKind::DistanceX { .. } => "constraint-distance-x",
        ConstraintKind::DistanceY { .. } => "constraint-distance-y",
        ConstraintKind::Angle { .. }
        | ConstraintKind::AngleToAxis { .. }
        | ConstraintKind::AngleAtPoint { .. } => "constraint-angle",
        ConstraintKind::Tangent { .. } | ConstraintKind::TangentCurves { .. } => {
            "constraint-tangent"
        }
        ConstraintKind::Symmetric { .. } | ConstraintKind::SymmetricAboutPoint { .. } => {
            "constraint-symmetric"
        }
        ConstraintKind::InternalAlignment { .. } => "show-hide-internal-geometry",
        ConstraintKind::Group { .. } => "tree-group",
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
        GeometryElement::Conic(c) => match c.kind {
            crate::sketch::ConicKind::Parabola => "arc-of-parabola",
            crate::sketch::ConicKind::Hyperbola => "arc-of-hyperbola",
        },
    }
}

/// The icon of an element's row in the sketch's element list: projected
/// (external) geometry and its points draw the external geometry icon,
/// their kind said beside it in words; the rest draw their kind's.
pub fn element_row_icon(element: &GeometryElement, external: bool) -> &'static str {
    if external {
        "external-geometry"
    } else {
        element_icon(element)
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
        GeometryElement::Conic(c) => match c.kind {
            crate::sketch::ConicKind::Parabola => "Arc of parabola",
            crate::sketch::ConicKind::Hyperbola => "Arc of hyperbola",
        },
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

/// What a sketch stands on, for the view's footer: the face, datum or
/// attachment it was placed on, else its base plane and how far off the
/// origin it sits.
pub fn placement_label(feature: &crate::feature::SketchFeature) -> String {
    if feature.face.is_some() {
        return "On a face".to_string();
    }
    if feature.support.is_some() {
        return "On a datum".to_string();
    }
    if feature.attached.is_some() {
        return "Attached".to_string();
    }
    let plane = &feature.plane;
    let label = plane_label(plane);
    let offset: f64 = (0..3).map(|i| plane.origin[i] * plane.normal[i]).sum();
    if label == "Face plane" || offset.abs() < 1e-3 {
        label.to_string()
    } else {
        format!("{label}, offset {offset:.2} mm")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Line, Point, Vec2D};

    #[test]
    fn a_sketch_on_a_face_is_not_called_an_origin_plane() {
        use crate::feature::{FaceSupport, SketchFeature};
        let top = SketchPlane::from_face([5.0, 5.0, 40.0], [0.0, 0.0, 1.0]);
        let mut on_face = SketchFeature::new(Sketch::new("s"), top);
        assert_eq!(placement_label(&on_face), "XY plane, offset 40.00 mm");
        let face = core_document::FaceRef {
            point: [5.0, 5.0, 40.0],
            normal: [0.0, 0.0, 1.0],
            surface: None,
            name: 0,
        };
        on_face.face = Some(FaceSupport::on(&face, top));
        assert_eq!(placement_label(&on_face), "On a face");
        let origin = SketchFeature::new(Sketch::new("s"), SketchPlane::xy());
        assert_eq!(placement_label(&origin), "XY plane");
    }

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
    fn projected_rows_draw_the_external_geometry_icon() {
        let line = GeometryElement::Line(Line::new(Uuid::new_v4(), Uuid::new_v4()));
        assert_eq!(element_row_icon(&line, false), "line");
        let icon = element_row_icon(&line, true);
        assert_eq!(icon, "external-geometry");
        assert!(ui_kit::icon::exists(icon), "{icon}");
    }

    #[test]
    fn plane_labels_follow_the_normal() {
        assert_eq!(plane_label(&SketchPlane::xy()), "XY plane");
        assert_eq!(plane_label(&SketchPlane::xz()), "XZ plane");
        assert_eq!(plane_label(&SketchPlane::yz()), "YZ plane");
    }
}
