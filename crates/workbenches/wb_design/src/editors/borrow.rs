//! The borrowed geometry task panel: what the body borrows (another body's
//! sketch, or faces and edges of another body's solid), and whether it
//! follows its source or keeps a frozen copy.

use bench_api::NoteKind;
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use super::{
    Kind, LABEL_BLANK, choice_shown, coords, mono_value, number, set_f32, small_button, use_button,
};
use crate::borrow;
use crate::feature::{BorrowOptions, BorrowSource, EdgePick, FacePick, FrozenBorrow};
use crate::form::Form;

/// A body's name, for the panel.
fn body_name(ctx: &WorkbenchRuntimeContext, body: BodyId) -> String {
    ctx.document
        .bodies()
        .iter()
        .find(|b| b.id == body)
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "a missing body".into())
}

/// Every sketch of another body than `body`, named with its body.
fn other_sketches(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Vec<(FeatureId, String)> {
    let mut sketches: Vec<(u64, FeatureId, String)> = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "wb.sketch" && n.body != Some(body))
        .map(|(id, n)| {
            let home = n
                .body
                .map(|b| body_name(ctx, b))
                .unwrap_or_else(|| "Document".into());
            (n.seq, *id, format!("{home} › {}", n.name))
        })
        .collect();
    sketches.sort_by_key(|(seq, id, _)| (*seq, *id));
    sketches.into_iter().map(|(_, id, n)| (id, n)).collect()
}

/// What the source is called in the panel.
fn source_label(ctx: &WorkbenchRuntimeContext, source: &BorrowSource) -> String {
    match source {
        BorrowSource::Sketch(sketch) => ctx
            .document
            .get_feature_meta(*sketch)
            .map(|n| {
                let home = n
                    .body
                    .map(|b| body_name(ctx, b))
                    .unwrap_or_else(|| "Document".into());
                format!("{home} › {}", n.name)
            })
            .unwrap_or_else(|| "a missing sketch".into()),
        BorrowSource::Solid { body, .. } => format!("Faces and edges of {}", body_name(ctx, *body)),
    }
}

/// Where the borrowed geometry is moved and turned to from where the
/// bodies put it.
fn offset_editor(f: &mut Form, options: &mut BorrowOptions) -> bool {
    let moved = !options.offset.eq(&Default::default());
    let summary = if moved {
        "moved"
    } else {
        "where the bodies put it"
    };
    f.section("Offset", summary, moved, |f| {
        let mut changed = false;
        let o = &mut options.offset;
        let [x, y, z] = &mut o.translation;
        for (label, value) in [("Along X", x), ("Along Y", y), ("Along Z", z)] {
            changed |= set_f32(
                value,
                number(f, label, f64::from(*value), Kind::Offset, 0.1),
            );
        }
        let [tilt_x, tilt_y] = &mut o.tilt;
        for (label, value) in [
            ("Turn about Z", &mut o.rotation_deg),
            ("Tilt about X", tilt_x),
            ("Tilt about Y", tilt_y),
        ] {
            changed |= set_f32(
                value,
                number(f, label, f64::from(*value), Kind::Degrees, 0.5),
            );
        }
        changed |= f.check(&mut o.flip, "Flip", "");
        changed
    })
}

/// The borrow editor; `true` when it changed the feature.
pub(super) fn borrow_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    feature_id: FeatureId,
    source: &mut BorrowSource,
    frozen: &mut Option<FrozenBorrow>,
    options: &mut BorrowOptions,
) -> bool {
    let mut changed = false;
    let sketches = other_sketches(ctx, body);
    let solids: Vec<BodyId> = ctx
        .document
        .bodies()
        .iter()
        .filter(|b| b.id != body && ctx.document.imported_brep_blob(b.id).is_some())
        .map(|b| b.id)
        .collect();
    let mut options_shown: Vec<String> = sketches.iter().map(|(_, n)| n.clone()).collect();
    options_shown.extend(
        solids
            .iter()
            .map(|other| format!("Faces and edges of {}", body_name(ctx, *other))),
    );
    let at = sketches
        .iter()
        .position(|(id, _)| *source == BorrowSource::Sketch(*id))
        .or_else(|| {
            solids
                .iter()
                .position(
                    |other| matches!(source, BorrowSource::Solid { body, .. } if body == other),
                )
                .map(|i| sketches.len() + i)
        });
    match choice_shown(f, "Source", source_label(ctx, source), options_shown, at) {
        Some(i) if i < sketches.len() => {
            *source = BorrowSource::Sketch(sketches[i].0);
            changed = true;
        }
        Some(i) => {
            *source = BorrowSource::Solid {
                body: solids[i - sketches.len()],
                faces: Vec::new(),
                edges: Vec::new(),
            };
            changed = true;
        }
        None => {}
    }
    if let BorrowSource::Solid {
        body: from,
        faces,
        edges,
    } = source
    {
        changed |= picks_editor(f, ctx, body, from, faces, edges);
        changed |= f.check(
            &mut options.fill,
            "Fill closed edges",
            "Edges that close into a loop in one plane lend the face they bound, as a profile",
        );
        changed |= f.check(
            &mut options.whole,
            "Whole solid as reference",
            "Draw all of the other body's edges here, to measure and stop on",
        );
    } else if options.fill || options.whole {
        options.fill = false;
        options.whole = false;
        changed = true;
    }
    changed |= offset_editor(f, options);

    let mut is_frozen = frozen.is_some();
    let toggled = f.check(
        &mut is_frozen,
        "Frozen",
        "Keep the geometry as it is now; unticked, it follows its source",
    );
    // The button is listed whenever it shows, an edit or not.
    let again =
        frozen.is_some() && small_button(f, "Take it again", "Freeze the source as it is now");
    let refreeze = frozen.is_some() && (changed || again);
    if toggled && !is_frozen {
        *frozen = None;
        changed = true;
    } else if (toggled && is_frozen) || refreeze {
        match borrow::freeze(ctx.document, ctx.kernel, body, source, options) {
            Ok(snapshot) => {
                *frozen = Some(snapshot);
                f.freeze_errors.remove(&feature_id);
            }
            Err(e) => {
                *frozen = None;
                f.freeze_errors.insert(feature_id, e);
            }
        }
        changed = true;
    }
    let error = f.freeze_errors.get(&feature_id).cloned();
    match (error, frozen.is_some()) {
        (Some(error), false) => {
            f.note(NoteKind::Error, Some("Could not freeze"), &error);
        }
        (_, true) => {
            f.note(
                NoteKind::Info,
                None,
                "Frozen: the source may change or move; this body keeps what it took.",
            );
        }
        (None, false) => {
            f.note(
                NoteKind::Info,
                None,
                "Follows its source: what stands on it rebuilds as the source changes or moves.",
            );
        }
    }
    changed
}

/// The faces and edges borrowed from `from`, with rows to take the ones
/// picked in the viewport. A pick on yet another body starts the lists
/// again from that body.
fn picks_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    using: BodyId,
    from: &mut BodyId,
    faces: &mut Vec<FacePick>,
    edges: &mut Vec<EdgePick>,
) -> bool {
    let mut changed = false;
    let mut remove_face = None;
    for (i, face) in faces.iter().enumerate() {
        f.flow(if i == 0 { "Faces" } else { LABEL_BLANK }, false, |f| {
            mono_value(f, &coords(face.point), true);
            if small_button(f, "×", "") {
                remove_face = Some(i);
            }
        });
    }
    if let Some(i) = remove_face {
        faces.remove(i);
        changed = true;
    }
    let mut remove_edge = None;
    for (i, edge) in edges.iter().enumerate() {
        f.flow(if i == 0 { "Edges" } else { LABEL_BLANK }, false, |f| {
            mono_value(f, &coords(edge.point), true);
            if small_button(f, "×", "") {
                remove_edge = Some(i);
            }
        });
    }
    if let Some(i) = remove_edge {
        edges.remove(i);
        changed = true;
    }

    // The face picked now belongs to the body the host says was clicked.
    let face_body = ctx
        .selected_body_id
        .map(BodyId)
        .filter(|b| *b != using && ctx.selected_face.is_some());
    let edge_body = ctx
        .selected_edges
        .first()
        .map(|e| BodyId(e.body))
        .filter(|b| *b != using);
    f.flow("", true, |f| {
        if use_button(
            f,
            "Add selected face",
            face_body.is_some(),
            "Click a face of another body in the viewport first, then press this",
        ) && let Some(picked) = face_body
            && let Some(face) = ctx.selected_face_in(picked)
        {
            restart_from(from, picked, faces, edges);
            faces.push(FacePick {
                name: 0,
                point: face.point,
                normal: face.normal,
            });
            changed = true;
        }
        if use_button(
            f,
            "Add selected edges",
            edge_body.is_some(),
            "Click edges of another body in the viewport first, then press this",
        ) && let Some(picked) = edge_body
        {
            restart_from(from, picked, faces, edges);
            for edge in ctx
                .selected_edges_in(picked)
                .iter()
                .filter(|e| BodyId(e.body) == picked)
            {
                edges.push(EdgePick {
                    faces: [0, 0],
                    point: edge.point,
                    direction: edge.direction,
                });
            }
            changed = true;
        }
    });
    changed
}

/// Borrowing from `picked` instead of `from`: the lists start again.
fn restart_from(
    from: &mut BodyId,
    picked: BodyId,
    faces: &mut Vec<FacePick>,
    edges: &mut Vec<EdgePick>,
) {
    if picked != *from {
        *from = picked;
        faces.clear();
        edges.clear();
    }
}
