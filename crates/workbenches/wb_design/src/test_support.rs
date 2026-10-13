//! What the crate's tests build their documents from.

use core_document::{FeatureId, ImportedGeometry};
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{GeometryElement, Line, Point, Sketch, Vec2D};

use crate::feature::{DesignFeature, DrillPoint, HoleCut, ThreadLength, ThreadSpec};

/// A `width` × `height` rectangle on the XY plane, its corner at the origin.
pub(crate) fn rect_sketch(width: f64, height: f64) -> SketchFeature {
    let mut sketch = Sketch::new("s");
    let corners = [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]
        .map(|(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))));
    for i in 0..4 {
        sketch.add_geometry(GeometryElement::Line(Line::new(
            corners[i],
            corners[(i + 1) % 4],
        )));
    }
    let plane = sketch.plane;
    SketchFeature::new(sketch, plane)
}

/// A pad `length` long from `sketch`, every other field at its default.
pub(crate) fn pad(sketch: FeatureId, length: f32) -> DesignFeature {
    DesignFeature::Pad {
        regions: Vec::new(),
        profile_borrowed: None,
        extras: Default::default(),
        refine: false,
        boolean_tolerance: None,
        sketch: Some(sketch),
        length,
        reversed: false,
        symmetric: false,
        mode: Default::default(),
        length2: 0.0,
        taper_deg: 0.0,
        up_to_face: None,
        up_to_offset: 0.0,
        profile_face: None,
        direction: Default::default(),
        up_to_shape: Vec::new(),
        mode2: None,
        up_to_face2: None,
        up_to_offset2: 0.0,
        up_to_shape2: Vec::new(),
    }
}

/// A plain blind hole 3 mm across and 6 deep at `sketch`'s centres, as
/// `edit` changes it.
pub(crate) fn hole(sketch: FeatureId, edit: impl FnOnce(&mut HoleFields)) -> DesignFeature {
    let mut hole = DesignFeature::Hole {
        cosmetic_thread: false,
        start: Default::default(),
        clearance: None,
        thread_length: Default::default(),
        refine: false,
        boolean_tolerance: None,
        sketch,
        diameter: 3.0,
        depth: 6.0,
        through_all: false,
        cut: HoleCut::None,
        thread: None,
        threaded: false,
        modeled_thread: false,
        thread_depth: 0.0,
        fit: Default::default(),
        drill_point: DrillPoint::Flat,
        point_in_depth: false,
        taper_deg: 0.0,
        nut_trap: None,
        reversed: false,
    };
    set_hole(&mut hole, edit);
    hole
}

/// The fields of a hole the tests set.
pub(crate) struct HoleFields<'a> {
    pub diameter: &'a mut f32,
    pub depth: &'a mut f32,
    pub through_all: &'a mut bool,
    pub cut: &'a mut HoleCut,
    pub thread: &'a mut Option<ThreadSpec>,
    pub threaded: &'a mut bool,
    pub modeled_thread: &'a mut bool,
    pub thread_depth: &'a mut f32,
    pub thread_length: &'a mut ThreadLength,
    pub cosmetic_thread: &'a mut bool,
    pub clearance: &'a mut Option<f32>,
    pub drill_point: &'a mut DrillPoint,
    pub point_in_depth: &'a mut bool,
    pub taper_deg: &'a mut f32,
}

/// Change a hole's fields through `edit`.
pub(crate) fn set_hole(feature: &mut DesignFeature, edit: impl FnOnce(&mut HoleFields)) {
    let DesignFeature::Hole {
        diameter,
        depth,
        through_all,
        cut,
        thread,
        threaded,
        modeled_thread,
        thread_depth,
        thread_length,
        cosmetic_thread,
        clearance,
        drill_point,
        point_in_depth,
        taper_deg,
        ..
    } = feature
    else {
        panic!("not a hole: {feature:?}");
    };
    edit(&mut HoleFields {
        diameter,
        depth,
        through_all,
        cut,
        thread,
        threaded,
        modeled_thread,
        thread_depth,
        thread_length,
        cosmetic_thread,
        clearance,
        drill_point,
        point_in_depth,
        taper_deg,
    });
}

/// A body's geometry with nothing drawn: an import's when `source_asset`
/// names the file it came from, a build's otherwise.
pub(crate) fn bare_geometry(source_asset: Option<uuid::Uuid>) -> ImportedGeometry {
    ImportedGeometry {
        mesh: std::sync::Arc::new(kernel_api::TriMesh::default()),
        source_asset,
        revision: 0,
        bounds_mm: None,
        brep_blob_path: None,
        mesh_path: None,
        face_colors_path: None,
        health: None,
    }
}

/// The feature a command answered with.
pub(crate) fn made_id(value: &serde_json::Value) -> FeatureId {
    FeatureId(uuid::Uuid::parse_str(value.as_str().expect("a feature id")).unwrap())
}
