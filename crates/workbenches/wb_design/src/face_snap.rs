//! A pad's or pocket's length dragged by its handle snaps to a flat face
//! square to its way when the handle's dot comes within a few pixels of
//! that face's plane: the face is lit while it holds, and Alt drags past
//! it. The feature's own end is never a face to snap to: a face standing
//! where the drag has already set the length, at its start or behind it,
//! is left out.

use core_document::{BodyId, OverlayMesh, WorkbenchRuntimeContext};
use glam::Vec3;

use crate::handles::{Handle, Shape};

/// How near, in physical pixels, the dot must come to a face's plane.
const REACH_PX: f32 = 6.0;
/// How square to the way a face's normal must be (the cosine).
const SQUARE: f32 = 0.9999;
/// Lengths closer than this (mm) count as one.
const SAME: f32 = 1e-3;

/// A face a length snapped to: its body, its index in the body's mesh,
/// and the length that puts the dot on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Snap {
    pub body: BodyId,
    pub face: u32,
    pub value: f32,
}

/// Whether `handle` sets a pad's or pocket's length, the numbers that snap.
pub(crate) fn snaps(handle: &Handle) -> bool {
    ["/length", "/length2", "/depth", "/depth2"]
        .iter()
        .any(|end| handle.pointer.ends_with(end))
        && matches!(handle.pointer.split('/').nth(1), Some("Pad" | "Pocket"))
}

/// The face `value` snaps to, for `handle`'s line: a flat face of a
/// visible body, square to the line, whose plane the dot at `value` comes
/// within reach of on screen; the nearest such. `set` holds the lengths the
/// drag has set, faces at which are the feature's own.
pub(crate) fn nearest(
    ctx: &WorkbenchRuntimeContext,
    handle: &Handle,
    value: f32,
    set: &[f32],
) -> Option<Snap> {
    let Shape::Line { origin, dir, scale } = handle.shape else {
        return None;
    };
    let at = |v: f32| ctx.world_to_viewport((origin + dir * v * scale).to_array());
    let here = at(value)?;
    let mut best: Option<(f32, Snap)> = None;
    for body in ctx.document.bodies() {
        if !ctx.document.imported_body_effective_visible(body.id) {
            continue;
        }
        let Some(geometry) = ctx.document.imported_geometry(body.id) else {
            continue;
        };
        for (face, surface) in geometry.mesh.face_surfaces.iter().enumerate() {
            let kernel_api::FaceSurface::Plane { origin: on, normal } = surface else {
                continue;
            };
            let normal = Vec3::from_array(*normal);
            if normal.dot(dir).abs() < SQUARE {
                continue;
            }
            let v = (Vec3::from_array(*on) - origin).dot(dir) / scale;
            // Only ahead of the start, and never the feature's own end.
            if v < SAME || set.iter().any(|s| (s - v).abs() < SAME) {
                continue;
            }
            let Some(there) = at(v) else { continue };
            let px = (there.0 - here.0).hypot(there.1 - here.1);
            if px <= REACH_PX && best.is_none_or(|(d, _)| px < d) {
                best = Some((
                    px,
                    Snap {
                        body: body.id,
                        face: face as u32,
                        value: v,
                    },
                ));
            }
        }
    }
    best.map(|(_, snap)| snap)
}

/// The snapped face, lit: its triangles drawn see-through over it.
pub(crate) fn lit(ctx: &WorkbenchRuntimeContext, snap: &Snap) -> Option<OverlayMesh> {
    let mesh = &ctx.document.imported_geometry(snap.body)?.mesh;
    let mut lit = kernel_api::TriMesh::default();
    for (t, face) in mesh.indices.chunks(3).zip(&mesh.faces) {
        if *face != snap.face {
            continue;
        }
        for i in t {
            lit.indices.push(lit.positions.len() as u32);
            lit.positions.push(*mesh.positions.get(*i as usize)?);
            lit.normals
                .push(*mesh.normals.get(*i as usize).unwrap_or(&[0.0, 0.0, 1.0]));
        }
    }
    let color = ctx.sketch_palette.preselect;
    Some(OverlayMesh {
        mesh: std::sync::Arc::new(lit),
        revision: None,
        color,
        wireframe: false,
        opacity: 0.45,
        on_top: false,
    })
}
