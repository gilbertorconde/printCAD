//! The Rib task panel: its sketch, its thickness and which way it grows.

use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use super::{Formulas, mm_drag, pick, sketch_combo, taper_note};
use crate::feature::DesignFeature;
use crate::form::Form;

/// The Rib editor; `true` when it changed the feature.
pub(super) fn rib_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
) -> bool {
    let DesignFeature::Rib {
        sketch,
        thickness,
        side,
        along_normal,
        ..
    } = feature
    else {
        return false;
    };
    let mut changed = false;
    if let Some(new) = sketch_combo(f, ctx, body, feature_id, Some(*sketch), "Line:") {
        *sketch = new;
        changed = true;
    }
    changed |= mm_drag(f, fx, thickness, ("Thickness:", "thickness"));
    changed |= pick(
        f,
        "Thickness lies",
        &[
            kernel_api::RibSide::Both,
            kernel_api::RibSide::Front,
            kernel_api::RibSide::Back,
        ],
        side,
        |s| match s {
            kernel_api::RibSide::Both => "Both sides",
            kernel_api::RibSide::Front => "One side",
            kernel_api::RibSide::Back => "Other side",
        },
    );
    changed |= f.check(
        along_normal,
        "Along the normal",
        "The web runs from the line along the sketch plane's normal, the thickness across \
         the line, rather than across the plane",
    );
    taper_note(
        f,
        if *along_normal {
            "The line thickened in its plane runs along the normal, either way, until it \
             meets the solid."
        } else {
            "The line's ends run on until they meet the solid, and the web fills between \
             the line and the solid on the side where the solid closes it in."
        },
    );
    changed
}
