//! Design's Text: text on faces of the solid. The feature keeps what the
//! text says and how it stands; a hidden sketch of its own, standing off
//! the face it was put on, holds the text block those fields make
//! ([`sync`]), and the feature carries the sketch onto the faces as
//! Project does. The sketch can be opened to lay the text out in a frame
//! or along curves; the task's fields write over what they name.

use core_document::{
    BodyId, Document, FaceRef, FeatureId, WorkbenchFeature, WorkbenchRuntimeContext,
};
use glam::DVec3;
use kernel_api::FaceSurface;
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{GeometryElement, Sketch, SketchPlane, Vec2D};
use wb_sketch::text::{self, Align, TextSpec, VAlign};

use crate::DesignFeature;

/// How far off the face its sketch stands, mm: beside the face, so the
/// projection lands on it from outside.
const STANDOFF: f64 = 2.0;

/// What the text says when the tool puts it down.
const FIRST_TEXT: &str = "Text";

/// The face to write on, in `body`'s frame, with the surface it lies on:
/// the face picked in the view, its surface read from the body's mesh when
/// the pick did not carry one (a face given to a command).
fn picked_face(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Option<FaceRef> {
    let mut face = ctx.selected_face_in(body)?;
    if face.surface.is_none() {
        face.surface = surface_at(ctx.document, body, face.point);
    }
    Some(face)
}

/// The surface of `body`'s face nearest `point`, in the body's frame.
fn surface_at(document: &Document, body: BodyId, point: [f32; 3]) -> Option<FaceSurface> {
    let (mesh, _) = document.local_geometry(body)?;
    let p = glam::Vec3::from_array(point);
    let nearest = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .map(|(t, tri)| {
            let c = tri
                .iter()
                .map(|i| glam::Vec3::from_array(mesh.positions[*i as usize]))
                .sum::<glam::Vec3>()
                / 3.0;
            (t, c.distance_squared(p))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))?
        .0;
    let face = *mesh.faces.get(nearest)? as usize;
    mesh.face_surfaces.get(face).cloned()
}

/// The plane text on `face` is written in: off the face along its normal,
/// facing out, reading across it with its up along a cylinder's or cone's
/// axis, else as near the body's +Z (or +Y on a face square to it) as the
/// face allows. Whether the face is curved, and a cylinder's radius.
pub(crate) fn plane_for(face: &FaceRef) -> (SketchPlane, bool, f32) {
    let n = DVec3::from_array(face.normal.map(f64::from)).normalize_or(DVec3::Z);
    let point = DVec3::from_array(face.point.map(f64::from));
    let (up, curved, radius) = match &face.surface {
        Some(FaceSurface::Cylinder { axis, radius, .. }) => {
            (DVec3::from_array(axis.map(f64::from)), true, *radius)
        }
        Some(FaceSurface::Cone { axis, .. }) => (DVec3::from_array(axis.map(f64::from)), true, 0.0),
        Some(FaceSurface::Plane { .. }) | None => (DVec3::Z, false, 0.0),
        Some(_) => (DVec3::Z, true, 0.0),
    };
    let mut up = up - n * up.dot(n);
    if up.length() < 1e-6 {
        up = DVec3::Y - n * n.y;
    }
    let mut up = up.normalize_or(DVec3::Y);
    // Upright as the body stands, where the axis allows either way.
    if up.z < -1e-9 || (up.z.abs() <= 1e-9 && up.y < 0.0) {
        up = -up;
    }
    let x = up.cross(n).normalize_or(DVec3::X);
    let origin = point + n * STANDOFF;
    (
        SketchPlane::from_frame(origin.to_array(), n.to_array(), x.to_array()),
        curved,
        radius,
    )
}

/// Make the hidden sketch the Text feature put on the face picked in the
/// view writes in, with a text block in its middle. The sketch, whether
/// the face is curved (the text wraps onto it) and a cylinder's radius.
pub(crate) fn make_sketch(
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
) -> Result<(FeatureId, bool, f32), String> {
    let face = picked_face(ctx, body).ok_or("Click the face to write on in the viewport first")?;
    let (plane, curved, radius) = plane_for(&face);
    let name = crate::DesignWorkbench::next_feature_name(ctx, "TextSketch");
    let mut sketch = Sketch::new(name.clone());
    sketch.plane = plane;
    let spec = TextSpec {
        text: FIRST_TEXT.to_string(),
        size: 5.0,
        align: Align::Middle,
        valign: VAlign::Middle,
        ..TextSpec::default()
    };
    text::add(&mut sketch, Vec2D::new(0.0, 0.0), &spec)?;
    let id = ctx
        .document
        .add_feature_in_body(SketchFeature::new(sketch, plane), name, Some(body))
        .map_err(|e| e.to_string())?;
    ctx.document.set_feature_visible(id, false);
    Ok((id, curved, radius))
}

/// Bring the hidden sketch of Text feature `id` in step with its fields:
/// what it says, its font, size and spacing (a whole turn's spacing worked
/// out from its letters), its turn, and where its middle stands.
pub(crate) fn sync(document: &mut Document, id: FeatureId) -> Result<(), String> {
    let Some(DesignFeature::Text {
        sketch,
        text: says,
        font,
        size,
        spacing,
        angle,
        across,
        up,
        along,
        full_turn,
        radius,
        wrap,
        ..
    }) = document
        .get_feature_data(id)
        .and_then(|data| DesignFeature::from_json(data).ok())
    else {
        return Ok(());
    };
    let mut feature = document
        .get_feature_data(sketch)
        .and_then(|data| SketchFeature::from_json(data).ok())
        .ok_or("the text's sketch is gone")?;
    let Some(block) = feature.sketch.texts.first() else {
        return Err("the text's sketch holds no text".to_string());
    };
    let block_id = block.id;
    let anchor = block.anchor;
    let mut spec = TextSpec::of(block);
    spec.text = says.clone();
    spec.font = font.clone();
    if !text::is_bundled(&spec.font) && block.font != spec.font {
        spec.font_asset = None;
    }
    spec.size = f64::from(size).max(0.01);
    spec.spacing = f64::from(spacing);
    spec.angle = f64::from(angle) + if along && wrap { 90.0 } else { 0.0 };
    if spec.layout.is_point() {
        spec.align = Align::Middle;
        spec.valign = VAlign::Middle;
    }
    text::load_font(document, &mut spec)?;
    spec.values = text::values_of(document, &spec.text)?;
    if let Some(GeometryElement::Point(p)) = feature.sketch.get_geometry_mut(anchor) {
        p.position = Vec2D::new(f64::from(across), f64::from(up));
    }
    text::change(&mut feature.sketch, block_id, &spec)?;
    // Round the whole turn: what the letters leave of the circumference,
    // shared between them, the gap across the seam as wide as the rest.
    if full_turn && wrap && !along && radius > 0.0 {
        let block = &feature.sketch.texts[0];
        let shown = block.shown();
        let scale = spec.size / block.glyphs.em.max(1e-9);
        let advances: Vec<f64> = shown
            .chars()
            .filter_map(|c| block.glyphs.get(c))
            .map(|g| g.advance * scale)
            .collect();
        if !advances.is_empty() {
            let turn = std::f64::consts::TAU * f64::from(radius);
            spec.spacing = (turn - advances.iter().sum::<f64>()) / advances.len() as f64;
            text::change(&mut feature.sketch, block_id, &spec)?;
        }
    }
    document
        .update_feature_data(sketch, feature.to_json())
        .map_err(|e| e.to_string())?;
    text::keep_formulas(document, sketch, block_id, &says)?;
    document.mark_feature_dirty(sketch);
    document.mark_feature_dirty(id);
    Ok(())
}
