//! Ribs grown from an open sketch chain until they meet the solid.

use core_document::Document;
use kernel_api::{RibForm, SolidOp};

use super::sweeps::sketch_spine;
use crate::feature::DesignFeature;

/// The op a rib grows: its sketch's chain, which must be open.
pub(super) fn rib_op(document: &Document, feature: &DesignFeature) -> Result<SolidOp, String> {
    let DesignFeature::Rib {
        sketch,
        thickness,
        side,
        along_normal,
        ..
    } = feature
    else {
        return Err("not a rib feature".into());
    };
    if *thickness <= 0.0 {
        return Err("the rib's thickness must be more than zero".into());
    }
    let profile = sketch_spine(document, *sketch, None, &[]).map_err(|e| {
        e.replace("the spine", "the rib's sketch")
            .replace("spine", "rib line")
    })?;
    let open = profile.wires.first().is_some_and(|w| {
        let ends = |s: &kernel_api::ProfileSegment| match *s {
            kernel_api::ProfileSegment::Line { start, end }
            | kernel_api::ProfileSegment::Arc { start, end, .. } => Some((start, end)),
            _ => None,
        };
        match (
            w.segments.first().and_then(ends),
            w.segments.last().and_then(ends),
        ) {
            (Some((start, _)), Some((_, end))) => {
                (start[0] - end[0]).hypot(start[1] - end[1]) > 1e-9
            }
            _ => false,
        }
    });
    if !open {
        return Err("a rib grows from an open line or chain; its sketch closes on itself".into());
    }
    Ok(SolidOp::Rib {
        profile,
        form: RibForm {
            thickness: f64::from(*thickness),
            side: *side,
            along_normal: *along_normal,
        },
    })
}
