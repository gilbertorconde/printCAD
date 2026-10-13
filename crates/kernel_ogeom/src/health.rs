//! The kernel's checker, repair and measures, in the application's terms.
//!
//! Every imported body is checked as it is read (a few milliseconds a
//! body), so the tree can say which ones the kernel calls broken; a repair
//! runs only when asked for, on the body's snapshot, and hands back the
//! mended shape with the checker's verdict on it.

use kernel_api::{
    KernelError, KernelResult, PhysicalProperties, RepairResult, ShapeHealth, TessellationSettings,
};
use ogeom::algo::{Diagnosis, Severity, check, surface_properties, volume_properties};
use ogeom::mesh::Deflection;
use ogeom::topo::{Filter, Model, NodeData, Shape, ShapeType, explore};

use crate::tess;

/// The checker's findings on `shape`, broken ones first. A shape the
/// checker cannot walk is reported as one broken finding saying so.
pub fn diagnose(model: &Model, shape: &Shape) -> ShapeHealth {
    match check(model, shape, tess::tolerances()) {
        Ok(diagnosis) => health_of(&diagnosis, false),
        Err(e) => ShapeHealth {
            broken: 1,
            suspect: 0,
            findings: vec![format!("the checker could not walk the shape: {e}")],
            repaired: false,
            faceted: false,
        },
    }
}

fn health_of(diagnosis: &Diagnosis, repaired: bool) -> ShapeHealth {
    let broken = diagnosis.of(Severity::Broken);
    let suspect = diagnosis.of(Severity::Suspect);
    let findings = broken
        .iter()
        .chain(suspect.iter())
        .take(ShapeHealth::KEPT_FINDINGS)
        .map(|p| p.to_string())
        .collect();
    ShapeHealth {
        broken: broken.len(),
        suspect: suspect.len(),
        findings,
        repaired,
        faceted: false,
    }
}

/// Run the kernel's repair on a snapshot and mesh the result.
///
/// The colours carry over face for face when the repair kept the face
/// count, which sewing loose faces together or collapsing tiny ones
/// changes; otherwise the mended shape draws in the default material.
pub fn repair_blob(
    brep_blob: &[u8],
    face_colors: &[[f32; 3]],
    detail: &TessellationSettings,
) -> KernelResult<RepairResult> {
    crate::progress::context("Repairing shape");
    let (mut model, root) = tess::read_blob(brep_blob)?;
    let tol = tess::tolerances();
    let fixed = ogeom::heal::fix_shape(&mut model, &root, tol)
        .map_err(|e| KernelError::Other(anyhow::anyhow!("repair failed: {e}")))?;
    let mut shape = fixed.shape;
    let report = fixed.report;

    let mut mended = Vec::new();
    let mut note = |n: usize, what: &str| {
        if n > 0 {
            mended.push(format!("{n} {what}"));
        }
    };
    note(report.wires_reordered, "wire(s) put in order");
    note(report.edges_collapsed, "edge(s) collapsed");
    note(report.edges_trimmed, "edge(s) given a trim");
    if let Some((sewn, _)) = report.sewn {
        note(sewn, "edge pair(s) sewn");
    }
    note(report.tolerances_reduced, "tolerance(s) tightened");
    note(
        report.tolerances_widened,
        "tolerance(s) widened to hold what they bound",
    );
    note(report.faces_turned, "face(s) turned the right way out");

    // Then what the checker does not call broken but gets in the way: a
    // swept line or circle that is a plane, drum, cone, ball or torus is
    // named as one (a bore then brings its axis, and measures exactly),
    // faces too small to matter collapse, and specks of solid go. Sizes
    // are relative to the part; a pass the kernel refuses is left out.
    let diagonal = tess::robust_bounds(&model, &shape)
        .map(|(lo, hi)| (hi - lo).magnitude())
        .unwrap_or(0.0);
    let mut changed = false;
    let swept = swept_faces(&model, &shape);
    if swept > 0
        && let Ok(named) = ogeom::heal::swept_to_elementary(&mut model, &shape, tol)
    {
        note(
            swept.saturating_sub(swept_faces(&model, &named.shape)),
            "swept face(s) named as the plane, drum, cone, ball or torus they are",
        );
        shape = named.shape;
        changed = true;
    }
    if diagonal > 0.0 {
        if let Ok(small) =
            ogeom::heal::fix_small_faces(&mut model, &shape, (diagonal * 1e-4).max(1e-3), tol)
            && small.spots + small.strips > 0
        {
            note(small.spots, "tiny face(s) collapsed to a point");
            note(small.strips, "sliver face(s) collapsed to an edge");
            shape = small.built.shape;
            changed = true;
        }
        if let Ok((kept, dropped)) =
            ogeom::heal::remove_small_solids(&mut model, &shape, (diagonal * 1e-3).powi(3), tol)
            && dropped > 0
        {
            note(dropped, "speck solid(s) dropped");
            shape = kept.shape;
            changed = true;
        }
    }

    let faces = explore(&model, &shape, Filter::OfType(ShapeType::Face))
        .map_err(|e| KernelError::Other(anyhow::anyhow!("exploring faces failed: {e}")))?;
    let face_colors = if faces.len() == face_colors.len() {
        face_colors.to_vec()
    } else {
        Vec::new()
    };

    crate::progress::context("Meshing the repaired shape");
    let mesh = tess::mesh_shape_with(&model, &shape, &face_colors, detail, tess::Faces::Wide)
        .map_err(|e| KernelError::Other(anyhow::anyhow!("tessellation failed: {e}")))?;
    let bounds_mm = tess::robust_bounds(&model, &shape).map(|(lo, hi)| {
        (
            [lo.x as f32, lo.y as f32, lo.z as f32],
            [hi.x as f32, hi.y as f32, hi.z as f32],
        )
    });
    Ok(RepairResult {
        brep_blob: tess::write_blob(&model, &shape)?,
        face_colors,
        mesh,
        bounds_mm,
        health: if changed {
            let after = check(&model, &shape, tol).map_err(|e| {
                KernelError::Other(anyhow::anyhow!("checking the repair failed: {e}"))
            })?;
            health_of(&after, true)
        } else {
            health_of(&report.after, true)
        },
        mended,
    })
}

/// How many faces of `shape` lie on a surface of extrusion or revolution.
fn swept_faces(model: &Model, shape: &Shape) -> usize {
    use ogeom::geom::SurfaceGeometry;
    explore(model, shape, Filter::OfType(ShapeType::Face))
        .unwrap_or_else(|e| {
            tracing::warn!(target: "printcad.kernel", "the faces do not read: {e}");
            Vec::new()
        })
        .iter()
        .filter(|face| {
            let Some(node) = model.node(face) else {
                return false;
            };
            let NodeData::Face(data) = node.data() else {
                return false;
            };
            matches!(
                model.geometry().surface(data.surface),
                Some(SurfaceGeometry::Extrusion(_) | SurfaceGeometry::Revolution(_))
            )
        })
        .count()
}

/// Volume, area, centre of mass and inertia of a snapshot. A shape that encloses no
/// volume still has an area and a centre; one the kernel cannot measure at
/// all is an error.
pub fn measure_blob(brep_blob: &[u8]) -> KernelResult<PhysicalProperties> {
    crate::progress::context("Measuring");
    let (model, root) = tess::read_blob(brep_blob)?;
    let tol = tess::tolerances();
    // Where a face has no closed form, the kernel integrates over a
    // tessellation at the deflection given and says so. A finer one than the
    // default costs seconds on a real part for a fraction of a percent, so
    // the default stands and the result carries whether it is exact.
    let deflection = Deflection::default();
    let area = surface_properties(&model, &root, deflection, tol)
        .map_err(|e| KernelError::Other(anyhow::anyhow!("measuring the area failed: {e}")))?;
    // Volume is the solids': surfaces beside them, or alone, enclose none,
    // even where their faces happen to close a space.
    let solids = ogeom::topo::explore_unique(&model, &root, ShapeType::Solid).unwrap_or_default();
    let measured: Vec<_> = solids
        .iter()
        .filter_map(|solid| volume_properties(&model, solid, deflection, tol).ok())
        .collect();
    let volume = (!measured.is_empty()).then(|| {
        let mass: f64 = measured.iter().map(|v| v.mass).sum();
        let rough = measured.iter().any(|v| v.deflection > 0.0);
        (mass, rough)
    });
    let combined = volume
        .filter(|(mass, _)| *mass > 0.0)
        .map(|(mass, _)| combined_mass(&measured, mass));
    let centre = match &combined {
        Some(all) => [all.centre.x, all.centre.y, all.centre.z],
        None => [area.centre.x, area.centre.y, area.centre.z],
    };
    let inertia = combined.and_then(|all| {
        let axes = all.principal_axes(tol).ok()?;
        let unit = |d: ogeom::math::Direction| {
            let v = d.vector();
            [v.x, v.y, v.z]
        };
        let [a, b, _] = axes.map(|(_, axis)| axis.vector());
        // Right-handed: the third axis is the first two crossed.
        let c = a.cross(b);
        Some(kernel_api::Inertia {
            about_centre: all.inertia.rows,
            principal: [
                (axes[0].0, unit(axes[0].1)),
                (axes[1].0, unit(axes[1].1)),
                (axes[2].0, [c.x, c.y, c.z]),
            ],
        })
    });
    let approximate = area.deflection > 0.0 || volume.is_some_and(|(_, rough)| rough);
    Ok(PhysicalProperties {
        volume_mm3: volume.map(|(mass, _)| mass),
        area_mm2: area.mass,
        centre_mm: centre,
        approximate,
        inertia,
    })
}

/// Several solids' volumes as one: their total `mass`, its centre, and the
/// inertia about that centre (each solid's moved there by the parallel
/// axis theorem).
fn combined_mass(
    measured: &[ogeom::algo::MassProperties],
    mass: f64,
) -> ogeom::algo::MassProperties {
    let mut moment = ogeom::math::Vector::new(0.0, 0.0, 0.0);
    for v in measured {
        moment += ogeom::math::Vector::new(v.centre.x, v.centre.y, v.centre.z) * v.mass;
    }
    let c = moment * (1.0 / mass);
    let centre = ogeom::math::Point::new(c.x, c.y, c.z);
    let mut rows = [[0.0; 3]; 3];
    for v in measured {
        let about = v.inertia_about(centre);
        for (row, add) in rows.iter_mut().zip(about.rows) {
            for (cell, a) in row.iter_mut().zip(add) {
                *cell += a;
            }
        }
    }
    ogeom::algo::MassProperties {
        mass,
        centre,
        inertia: ogeom::math::Matrix3 { rows },
        deflection: measured.iter().map(|v| v.deflection).fold(0.0, f64::max),
    }
}
