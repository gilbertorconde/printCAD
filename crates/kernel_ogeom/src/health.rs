//! The kernel's checker and repair, in the application's terms.
//!
//! Every imported body is checked as it is read (a few milliseconds a
//! body), so the tree can say which ones the kernel calls broken; a repair
//! runs only when asked for, on the body's snapshot, and hands back the
//! mended shape with the checker's verdict on it.

use kernel_api::{KernelError, KernelResult, RepairResult, ShapeHealth, TessellationSettings};
use ogeom::algo::{Diagnosis, Severity, check};
use ogeom::topo::{Filter, Model, Shape, ShapeType, explore};

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
    }
}

/// Run the kernel's repair on a snapshot and mesh the result.
///
/// The colours carry over face for face when the repair kept the face
/// count, which it does unless it sewed loose faces together; otherwise
/// the mended shape draws in the default material.
pub fn repair_blob(
    brep_blob: &[u8],
    face_colors: &[[f32; 3]],
    detail: &TessellationSettings,
) -> KernelResult<RepairResult> {
    crate::progress::context("Repairing shape");
    let (mut model, root) = tess::read_blob(brep_blob)?;
    let fixed = ogeom::heal::fix_shape(&mut model, &root, tess::tolerances())
        .map_err(|e| KernelError::Other(anyhow::anyhow!("repair failed: {e}")))?;
    let shape = fixed.shape;
    let report = fixed.report;

    let faces = explore(&model, &shape, Filter::OfType(ShapeType::Face))
        .map_err(|e| KernelError::Other(anyhow::anyhow!("exploring faces failed: {e}")))?;
    let face_colors = if faces.len() == face_colors.len() {
        face_colors.to_vec()
    } else {
        Vec::new()
    };

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
        health: health_of(&report.after, true),
        mended,
    })
}
