//! A context, joints, geometry and the bench's tools for the crate's
//! unit tests.

use super::*;
use core_document::{Document, ImportedGeometry, TriMesh};

/// A context over `doc` with an 800 × 600 view.
pub(crate) fn context(doc: &mut Document) -> WorkbenchRuntimeContext<'_> {
    WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600))
}

/// A joint of `kind` holding `moving`, on the body it is added to, to
/// `fixed` on `other`.
pub(crate) fn joint(kind: JointKind, moving: Anchor, other: BodyId, fixed: Anchor) -> JointFeature {
    JointFeature {
        kind,
        moving,
        other_body: other,
        fixed,
        names: [0; 2],
        ends: [0.0; 2],
        shape: Vec::new(),
        second: None,
    }
}

/// An undriven hinge `offset` along its axis, at no turn.
pub(crate) fn hinge(offset: f32) -> JointKind {
    JointKind::Hinge {
        offset,
        zero: glam::DQuat::IDENTITY.to_array(),
        drive: Drive::default(),
    }
}

/// Geometry of `mesh` alone, within `bounds_mm`.
pub(crate) fn imported(mesh: TriMesh, bounds_mm: Option<([f32; 3], [f32; 3])>) -> ImportedGeometry {
    ImportedGeometry {
        mesh: std::sync::Arc::new(mesh),
        source_asset: None,
        revision: 0,
        bounds_mm,
        brep_blob_path: None,
        mesh_path: None,
        face_colors_path: None,
        health: None,
    }
}

/// The bench's `tool` taken up, as a click on its button does.
pub(crate) fn activate(
    wb: &mut AssemblyWorkbench,
    ctx: &mut WorkbenchRuntimeContext<'_>,
    tool: &str,
) {
    wb.on_input(&WorkbenchInputEvent::ToolActivated, Some(tool), ctx);
}
