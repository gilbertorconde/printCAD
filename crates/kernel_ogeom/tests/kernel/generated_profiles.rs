//! Generated profiles through the full stack: a gear and a sprocket made
//! from their numbers and padded, a shaft revolved, each measured against
//! what its numbers say it is; and a gear whose teeth a variable sets.

use core_document::{BodyId, Document, FeatureId, WorkbenchFeature};
use kernel_api::TessellationSettings;
use kernel_ogeom::OgeomKernel;
use wb_design::DesignFeature;
use wb_sketch::SketchFeature;
use wb_sketch::generator::{
    GearSpec, Generator, ShaftSection, ShaftSpec, SprocketSpec, new_sketch,
};
use wb_sketch::sketch::SketchPlane;

fn pad(sketch: FeatureId, length: f32) -> DesignFeature {
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
        mode: wb_design::ExtrudeMode::Dimension,
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

fn revolve(sketch: FeatureId) -> DesignFeature {
    DesignFeature::Revolution {
        regions: Vec::new(),
        refine: false,
        boolean_tolerance: None,
        sketch,
        angle_deg: 360.0,
        axis: wb_design::RevolveAxis::SketchY,
        project_axis: false,
        reversed: false,
        midplane: false,
        second_angle_deg: None,
        start_angle_deg: 0.0,
        mode: Default::default(),
        up_to_face: None,
    }
}

/// A document with `generator`'s sketch on `plane` in a body, and the
/// feature `make` builds from it.
fn scene(
    generator: Generator,
    plane: SketchPlane,
    make: impl FnOnce(FeatureId) -> DesignFeature,
) -> (Document, BodyId, FeatureId) {
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Body".into()));
    let sketch = new_sketch(generator, plane, "Profile").expect("a profile");
    let sketch_id = doc
        .add_feature_in_body(sketch, "Profile".into(), Some(body))
        .unwrap();
    doc.add_feature_in_body(make(sketch_id), "Feature".into(), Some(body))
        .unwrap();
    (doc, body, sketch_id)
}

/// The solid the body builds, its volume and its mesh.
fn build(doc: &Document, body: BodyId) -> (f64, kernel_api::TriMesh) {
    let ops = wb_design::body_build_ops(doc, body).unwrap().ops;
    let mut kernel = OgeomKernel::new();
    let result = kernel
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .expect("the profile builds");
    let volume = kernel
        .physical_properties(&result.brep_blob)
        .unwrap()
        .volume_mm3
        .expect("a closed solid");
    (volume, result.mesh)
}

/// How many separate runs of mesh vertices lie on the circle of `radius`
/// about the Z axis: one per tooth for a tip circle.
fn runs_on_circle(mesh: &kernel_api::TriMesh, radius: f32) -> usize {
    let mut angles: Vec<f32> = mesh
        .positions
        .iter()
        .filter(|p| (p[0].hypot(p[1]) - radius).abs() < 1e-3)
        .map(|p| p[1].atan2(p[0]))
        .collect();
    angles.sort_by(f32::total_cmp);
    let Some(first) = angles.first().copied() else {
        return 0;
    };
    let gaps = angles.windows(2).filter(|w| w[1] - w[0] > 0.05).count();
    let wraps = angles.last().unwrap() - first < std::f32::consts::TAU - 0.05;
    gaps + usize::from(wraps)
}

fn max_radius(mesh: &kernel_api::TriMesh) -> f32 {
    mesh.positions
        .iter()
        .map(|p| p[0].hypot(p[1]))
        .fold(0.0, f32::max)
}

fn outline_area(generator: &Generator) -> f64 {
    generator.outline().unwrap().area()
}

#[test]
fn a_padded_gear_has_its_teeth_its_tip_and_its_area_times_its_thickness() {
    let spec = GearSpec {
        module: 2.0,
        teeth: 17,
        bore: 6.0,
        ..GearSpec::default()
    };
    let g = spec.geometry().unwrap();
    let generator = Generator::Gear(spec);
    let area = outline_area(&generator);
    let (doc, body, _) = scene(generator, SketchPlane::xy(), |s| pad(s, 8.0));
    let (volume, mesh) = build(&doc, body);
    let expected = area * 8.0;
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "volume {volume} vs {expected}"
    );
    let tip = max_radius(&mesh);
    assert!((f64::from(tip) - g.tip_radius).abs() < 1e-3, "tip {tip}");
    assert_eq!(runs_on_circle(&mesh, tip), 17, "one tip land a tooth");
}

#[test]
fn a_padded_ring_gear_is_its_rim_less_its_teeth_spaces() {
    let spec = GearSpec {
        module: 1.5,
        teeth: 36,
        internal: true,
        ..GearSpec::default()
    };
    let (tip, _) = spec.tip_and_root_diameters().unwrap();
    let generator = Generator::Gear(spec);
    let area = outline_area(&generator);
    let (doc, body, _) = scene(generator, SketchPlane::xy(), |s| pad(s, 6.0));
    let (volume, mesh) = build(&doc, body);
    let expected = area * 6.0;
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "volume {volume} vs {expected}"
    );
    let inner = mesh
        .positions
        .iter()
        .map(|p| p[0].hypot(p[1]))
        .fold(f32::MAX, f32::min);
    assert!((f64::from(inner) - tip / 2.0).abs() < 1e-3, "tip {inner}");
    assert_eq!(runs_on_circle(&mesh, inner), 36, "one tip land a tooth");
}

#[test]
fn a_padded_sprocket_seats_its_rollers_and_measures_its_area() {
    let spec = SprocketSpec {
        teeth: 21,
        ..SprocketSpec::default()
    };
    let g = spec.geometry().unwrap();
    let generator = Generator::Sprocket(spec);
    let area = outline_area(&generator);
    let (doc, body, _) = scene(generator, SketchPlane::xy(), |s| pad(s, 5.0));
    let (volume, mesh) = build(&doc, body);
    let expected = area * 5.0;
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "volume {volume} vs {expected}"
    );
    let tip = max_radius(&mesh);
    assert!(f64::from(tip) <= g.tip_diameter / 2.0 + 1e-3, "tip {tip}");
    assert_eq!(runs_on_circle(&mesh, tip), 21);
    let root = mesh
        .positions
        .iter()
        .map(|p| p[0].hypot(p[1]))
        .filter(|r| *r > 8.0)
        .fold(f32::MAX, f32::min);
    assert!(
        (f64::from(root) - g.root_diameter / 2.0).abs() < 2e-2,
        "root {root}"
    );
}

#[test]
fn a_revolved_shaft_is_its_sections_cylinders() {
    let section = |length, diameter| ShaftSection {
        length,
        diameter,
        ..ShaftSection::default()
    };
    let plain = ShaftSpec {
        loads: Default::default(),
        sections: vec![section(10.0, 8.0), section(25.0, 14.0), section(15.0, 10.0)],
        start_chamfer: 0.0,
    };
    let expected = plain.plain_volume();
    let (doc, body, _) = scene(Generator::Shaft(plain), SketchPlane::xz(), revolve);
    let (volume, mesh) = build(&doc, body);
    assert!(
        (volume - expected).abs() < 1e-4 * expected,
        "volume {volume} vs {expected}"
    );
    // On XZ, the shaft stands along world Z.
    let (lo, hi) = mesh.bounds().unwrap();
    assert!((hi[2] - lo[2] - 50.0).abs() < 1e-3);
    assert!((hi[0] - 7.0).abs() < 1e-2 && (lo[0] + 7.0).abs() < 1e-2);
}

#[test]
fn a_shaft_with_chamfers_and_fillets_revolves_to_its_pappus_volume() {
    let spec = ShaftSpec::default();
    let generator = Generator::Shaft(spec);
    let lp = generator.outline().unwrap().loops.remove(0);
    let points = lp.polyline(256);
    // Pappus: 2π times the half section's first moment about the axis.
    let n = points.len();
    let moment: f64 = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            (a[0] + b[0]) * (a[0] * b[1] - b[0] * a[1])
        })
        .sum::<f64>()
        / 6.0;
    let expected = 2.0 * std::f64::consts::PI * moment.abs();
    let (doc, body, _) = scene(generator, SketchPlane::xz(), revolve);
    let (volume, _) = build(&doc, body);
    assert!(
        (volume - expected).abs() < 1e-4 * expected,
        "volume {volume} vs {expected}"
    );
}

#[test]
fn a_variable_sets_the_teeth_and_the_padded_gear_follows() {
    use core_document::{DocumentService, Variable, VariableSet};
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
        .unwrap();
    registry
        .register_workbench(Box::new(wb_design::DesignWorkbench::default()))
        .unwrap();
    let generator = Generator::Gear(GearSpec {
        module: 1.0,
        teeth: 20,
        ..GearSpec::default()
    });
    let (mut doc, body, sketch) = scene(generator, SketchPlane::xy(), |s| pad(s, 4.0));
    let sizes = doc
        .add_feature(
            VariableSet {
                variables: vec![Variable {
                    name: "z".into(),
                    formula: "30".into(),
                    comment: String::new(),
                }],
            },
            "Sizes".into(),
        )
        .unwrap();
    doc.set_feature_formula(sketch, "/generator/Gear/teeth", Some("Sizes.z".into()))
        .unwrap();

    let tip = |doc: &mut Document| {
        let job = registry
            .rebuild_jobs(doc)
            .into_iter()
            .find(|j| j.body == body)
            .expect("a rebuild");
        let result = OgeomKernel::new()
            .execute_solid_chain(&job.plan.unwrap().ops, &TessellationSettings::default())
            .unwrap();
        let tip = max_radius(&result.mesh);
        (tip, runs_on_circle(&result.mesh, tip))
    };
    let (radius, teeth) = tip(&mut doc);
    assert_eq!(teeth, 30);
    assert!((radius - 16.0).abs() < 1e-3, "m (z + 2) / 2: {radius}");

    let mut set = VariableSet::from_json(doc.get_feature_data(sizes).unwrap()).unwrap();
    set.variables[0].formula = "24".into();
    doc.update_feature_data(sizes, set.to_json()).unwrap();
    let (radius, teeth) = tip(&mut doc);
    assert_eq!(teeth, 24);
    assert!((radius - 13.0).abs() < 1e-3, "{radius}");
    // The stored sketch keeps its own numbers; the formula's are derived.
    let stored = SketchFeature::from_json(doc.get_feature_data(sketch).unwrap()).unwrap();
    let Some(Generator::Gear(spec)) = stored.generator else {
        panic!("still a gear")
    };
    assert_eq!(spec.teeth, 20);
}
