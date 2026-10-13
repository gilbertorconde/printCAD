//! A document saved while sketches kept single precision
//! (`tests/data/single_precision_sketch.prtcad`: a rectangle with a hole
//! on a plane off the origin, dimensioned and padded) loads with the
//! numbers it was saved with and builds through the real benches.

use core_document::{Document, DocumentService, WorkbenchFeature};
use kernel_api::TessellationSettings;
use kernel_ogeom::OgeomKernel;
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{ConstraintKind, GeometryElement};

const FIXTURE: &[u8] = include_bytes!("../data/single_precision_sketch.prtcad");

fn registry() -> DocumentService {
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
        .unwrap();
    registry
        .register_workbench(Box::new(wb_design::DesignWorkbench::default()))
        .unwrap();
    registry
}

fn outline(doc: &Document) -> SketchFeature {
    let (_, node) = doc
        .feature_tree()
        .all_nodes()
        .find(|(_, n)| n.name == "Outline")
        .expect("the sketch");
    SketchFeature::from_json(&node.data).unwrap()
}

/// Every stored number is the single-precision value it was saved as,
/// to the bit.
#[test]
fn a_single_precision_sketch_loads_with_its_numbers() {
    let doc = Document::load_from_bytes(FIXTURE.to_vec()).unwrap();
    let feature = outline(&doc);
    let single = |v: f32| f64::from(v);
    assert_eq!(
        feature.plane.origin,
        [12.345678f32, -3.3, 7.1].map(single),
        "the plane stays where it was"
    );
    assert_eq!(feature.plane.normal, [0.0, 0.0, 1.0]);
    let mut points: Vec<(f64, f64)> = feature
        .sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some((p.position.x, p.position.y)),
            _ => None,
        })
        .collect();
    points.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut want: Vec<(f64, f64)> = [
        (0.0f32, 0.0f32),
        (33.3333, 0.0),
        (33.3333, 7.77777),
        (0.0, 7.77777),
        (10.1, 3.3),
    ]
    .into_iter()
    .map(|(x, y)| (single(x), single(y)))
    .collect();
    want.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(points, want);
    let radius = feature.sketch.geometry.iter().find_map(|g| match g {
        GeometryElement::Circle(c) => Some(c.radius),
        _ => None,
    });
    assert_eq!(radius, Some(single(1.99997)));
    let dimensions: Vec<f64> = feature
        .sketch
        .constraints
        .iter()
        .filter_map(|c| match c.kind {
            ConstraintKind::Length { length, .. } => Some(length),
            ConstraintKind::Radius { radius, .. } => Some(radius),
            _ => None,
        })
        .collect();
    assert_eq!(dimensions, [single(33.3333), single(1.99997)]);
}

/// Its sketch solves and its pad builds the solid it always made.
#[test]
fn a_single_precision_document_rebuilds() {
    let registry = registry();
    let mut doc = Document::load_from_bytes(FIXTURE.to_vec()).unwrap();
    registry.evaluate(&mut doc);
    let solved = SketchFeature::from_json(
        doc.feature_values(
            *doc.feature_tree()
                .all_nodes()
                .find(|(_, n)| n.name == "Outline")
                .unwrap()
                .0,
        )
        .unwrap(),
    )
    .unwrap();
    let before = outline(&doc);
    for (a, b) in solved.sketch.geometry.iter().zip(&before.sketch.geometry) {
        if let (GeometryElement::Point(a), GeometryElement::Point(b)) = (a, b) {
            assert!(
                (a.position - b.position).to_glam().length() < 1e-5,
                "solving moves {:?} to {:?}",
                b.position,
                a.position
            );
        }
    }
    // As the app does with a document it opens.
    registry.invalidate_all(&mut doc);
    let jobs = registry.rebuild_jobs(&mut doc);
    assert_eq!(jobs.len(), 1, "the plate builds");
    let plan = jobs[0].plan.as_ref().expect("the history translates");
    let mut kernel = OgeomKernel::new();
    let result = kernel
        .execute_solid_chain(&plan.ops, &TessellationSettings::default())
        .expect("the plate builds");
    let volume = kernel
        .physical_properties(&result.brep_blob)
        .unwrap()
        .volume_mm3
        .expect("a closed solid");
    let r = f64::from(1.99997f32);
    let expected =
        (f64::from(33.3333f32) * f64::from(7.77777f32) - std::f64::consts::PI * r * r) * 5.5;
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "volume {volume}, want {expected}"
    );
    let (lo, hi) = result.bounds_mm.expect("bounds");
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    assert!(
        near(lo[0], 12.345678) && near(lo[1], -3.3) && near(lo[2], 7.1),
        "{lo:?}"
    );
    assert!(
        near(hi[0], 12.345678 + 33.3333) && near(hi[1], -3.3 + 7.77777) && near(hi[2], 12.6),
        "{hi:?}"
    );
}
