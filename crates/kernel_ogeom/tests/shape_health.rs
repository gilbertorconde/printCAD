//! Every imported body carries the kernel checker's verdict, and a body's
//! snapshot can be run through the kernel's repair.

use kernel_api::{ImportedModel, Kernel, TessellationSettings};
use kernel_ogeom::OgeomKernel;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn import(name: &str) -> (OgeomKernel, ImportedModel) {
    let mut kernel = OgeomKernel::new();
    let model = kernel
        .import_step(&fixture(name), &TessellationSettings::default())
        .expect("fixture imports");
    (kernel, model)
}

#[test]
fn a_clean_file_checks_clean() {
    let (_, model) = import("box_native.step");
    assert!(!model.bodies.is_empty());
    for body in &model.bodies {
        let health = body.health.as_ref().expect("every body is checked");
        assert_eq!(health.broken, 0, "{}", health.describe());
        assert!(!health.repaired);
    }
}

#[test]
fn a_repair_hands_back_a_mesh_and_the_checker_s_verdict() {
    let (mut kernel, model) = import("drive_frame_upper.step");
    for body in &model.bodies {
        let result = kernel
            .repair_brep(
                &body.brep_blob,
                &body.face_colors,
                &TessellationSettings::default(),
            )
            .expect("the repair runs");
        assert!(result.health.repaired);
        assert!(!result.mesh.indices.is_empty(), "the mended shape draws");
        assert!(!result.brep_blob.is_empty());
        assert_eq!(
            result.face_colors.len(),
            body.face_colors.len(),
            "no faces were sewn, so every face keeps its colour"
        );
        let before = body.health.as_ref().expect("checked on import");
        assert!(
            result.health.broken <= before.broken,
            "a repair never adds defects"
        );
    }
}

#[test]
fn a_well_formed_export_checks_clean() {
    let (_, model) = import("drive_frame_upper.step");
    for body in &model.bodies {
        let health = body.health.as_ref().expect("every body is checked");
        assert_eq!(health.broken, 0, "{}", health.describe());
    }
}

#[test]
fn a_repair_clears_tolerance_containment_findings() {
    let (mut kernel, model) = import("drive_frame_upper.step");
    for body in &model.bodies {
        let result = kernel
            .repair_brep(
                &body.brep_blob,
                &body.face_colors,
                &TessellationSettings::default(),
            )
            .expect("the repair runs");
        assert_eq!(result.health.broken, 0, "{}", result.health.describe());
    }
}

/// A box is a measure its own bounds can check.
#[test]
fn a_box_measures_its_volume_area_and_centre() {
    let (mut kernel, model) = import("box_native.step");
    let body = model.bodies.first().expect("one body");
    let props = kernel
        .physical_properties(&body.brep_blob)
        .expect("a closed box measures");
    let (lo, hi) = body.bounds_mm.expect("bounds");
    let size: Vec<f64> = (0..3).map(|i| f64::from(hi[i] - lo[i])).collect();
    let volume = size[0] * size[1] * size[2];
    let area = 2.0 * (size[0] * size[1] + size[1] * size[2] + size[0] * size[2]);
    assert!(!props.approximate, "flat faces have a closed form");
    let got = props.volume_mm3.expect("a closed box has a volume");
    assert!(
        (got - volume).abs() < 1e-3 * volume,
        "volume {got} vs {volume}"
    );
    assert!(
        (props.area_mm2 - area).abs() < 1e-3 * area,
        "area {} vs {area}",
        props.area_mm2
    );
    for i in 0..3 {
        let mid = f64::from(lo[i] + hi[i]) / 2.0;
        assert!((props.centre_mm[i] - mid).abs() < 1e-3, "centre axis {i}");
    }
}

/// An IGES file imports through the same path as STEP: the bundled box,
/// written out as IGES by the kernel, comes back as one body of the same
/// size, checked like any other.
#[test]
fn an_iges_file_imports_like_a_step_file() {
    let step_text = std::fs::read_to_string(fixture("box_native.step")).expect("fixture");
    let tol = ogeom::core::Tolerances::millimetres();
    let read = ogeom::io::step::read_step(&step_text, tol).expect("fixture reads");
    let iges_text = ogeom::io::write_iges(&read.document, tol).expect("the box writes as IGES");
    let path = std::env::temp_dir().join(format!("printcad_box_{}.igs", std::process::id()));
    std::fs::write(&path, iges_text).expect("staged");

    let mut kernel = OgeomKernel::new();
    let from_iges = kernel
        .import_step(&path, &TessellationSettings::default())
        .expect("the IGES file imports");
    let _ = std::fs::remove_file(&path);
    let (_, from_step) = import("box_native.step");

    assert_eq!(from_iges.bodies.len(), from_step.bodies.len());
    let (a, b) = (&from_iges.bodies[0], &from_step.bodies[0]);
    let (ia, ib) = (a.bounds_mm.expect("bounds"), b.bounds_mm.expect("bounds"));
    for axis in 0..3 {
        assert!((ia.0[axis] - ib.0[axis]).abs() < 1e-3, "min {axis}");
        assert!((ia.1[axis] - ib.1[axis]).abs() < 1e-3, "max {axis}");
    }
    assert!(!a.mesh.indices.is_empty(), "the IGES body draws");
    assert!(a.health.is_some(), "the IGES body is checked");
}

/// The fixture written as IGES by the kernel, staged to a temp file of its
/// own: tests run side by side, and each removes its file when done.
fn fixture_as_iges(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static STAGED: AtomicUsize = AtomicUsize::new(0);
    let n = STAGED.fetch_add(1, Ordering::Relaxed);
    let tol = ogeom::core::Tolerances::millimetres();
    let text = std::fs::read_to_string(fixture(name)).expect("fixture");
    let read = ogeom::io::step::read_step(&text, tol).expect("fixture reads");
    let iges = ogeom::io::write_iges(&read.document, tol).expect("fixture writes as IGES");
    let path = std::env::temp_dir().join(format!(
        "printcad_{}_{}_{n}.igs",
        name.replace('.', "_"),
        std::process::id()
    ));
    std::fs::write(&path, iges).expect("staged");
    path
}

#[test]
fn a_real_part_written_as_iges_keeps_the_record_width() {
    let path = fixture_as_iges("drive_frame_upper.step");
    let text = std::fs::read_to_string(&path).expect("staged file");
    let _ = std::fs::remove_file(&path);
    let long = text.lines().filter(|l| l.len() != 80).count();
    assert_eq!(long, 0, "{long} records are not 80 columns");
}

#[test]
fn a_real_part_imports_back_from_iges() {
    let path = fixture_as_iges("drive_frame_upper.step");
    let mut kernel = OgeomKernel::new();
    let back = kernel.import_step(&path, &TessellationSettings::default());
    let _ = std::fs::remove_file(&path);
    let (_, from_step) = import("drive_frame_upper.step");
    let back = back.expect("the IGES file imports");
    assert_eq!(back.bodies.len(), from_step.bodies.len());
}

/// Every face of a body meshes on itself: the carriage is a few
/// centimetres across, and so is its mesh.
#[test]
fn a_body_meshes_within_its_own_extent() {
    let (_, model) = import("monolith_carriage.step");
    let body = model.bodies.first().expect("one body");
    let (lo, hi) = body.mesh.bounds().expect("the body draws");
    for axis in 0..3 {
        let size = hi[axis] - lo[axis];
        assert!(size < 150.0, "the mesh spans {size} mm along axis {axis}");
    }
}

/// Repair names a swept face for what it is: a cylinder written as a
/// revolved line comes back a drum, so a bore brings its axis and the
/// solid measures exactly.
#[test]
fn repair_names_a_swept_drum_as_a_cylinder() {
    use ogeom::core::{OgeomResult, Tolerances};
    use ogeom::geom::{Curve, LineCurve, RevolutionSurface, Surface as _, SurfaceGeometry};
    use ogeom::math::{Axis, Frame};
    use ogeom::topo::Model;
    const T: Tolerances = Tolerances::millimetres();

    // A drum restated as the revolution of its ruling.
    let as_revolution = |s: &SurfaceGeometry| -> OgeomResult<Option<(SurfaceGeometry, bool)>> {
        let SurfaceGeometry::Cylinder(c) = s else {
            return Ok(None);
        };
        let (lo, hi) = c.domain().1;
        let f = c.cylinder().frame();
        let ruling = Axis {
            location: f.origin() + f.x().vector() * c.cylinder().radius(),
            direction: f.z(),
        };
        let axis = Axis {
            location: f.origin(),
            direction: f.z(),
        };
        let revolved: SurfaceGeometry = RevolutionSurface::new(
            LineCurve::over(ruling, lo, hi)?.into(),
            axis,
            core::f64::consts::TAU,
        )?
        .into();
        Ok(Some((revolved, false)))
    };
    let keep = |_: &Curve, _: (f64, f64)| -> OgeomResult<Option<(Curve, (f64, f64))>> { Ok(None) };
    let mut model = Model::new();
    let drum = ogeom::algo::make_cylinder(&mut model, Frame::WORLD, 2.0, 5.0, T)
        .unwrap()
        .shape;
    let swept = ogeom::algo::restate_geometry(&mut model, &drum, &as_revolution, &keep, T)
        .unwrap()
        .shape;
    let blob = ogeom::io::native::write(
        &model,
        std::slice::from_ref(&swept),
        ogeom::io::native::WriteOptions {
            triangulations: false,
        },
    )
    .unwrap()
    .into_bytes();

    let mut kernel = OgeomKernel::new();
    let before = kernel.physical_properties(&blob).unwrap();
    let repaired = kernel
        .repair_brep(&blob, &[], &TessellationSettings::default())
        .expect("the drum repairs");
    assert!(
        repaired.mended.iter().any(|m| m.contains("swept face")),
        "{:?}",
        repaired.mended
    );
    assert!(
        repaired
            .mesh
            .face_surfaces
            .iter()
            .any(|s| matches!(s, kernel_api::FaceSurface::Cylinder { .. })),
        "{:?}",
        repaired.mesh.face_surfaces
    );
    let after = kernel.physical_properties(&repaired.brep_blob).unwrap();
    let want = core::f64::consts::PI * 4.0 * 5.0;
    assert!(!after.approximate);
    assert!(
        (after.volume_mm3.unwrap() - want).abs() < 1e-9,
        "{after:?} (was {before:?})"
    );
}
