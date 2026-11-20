//! The Thread task panel: its face, its standard size or own pitch and
//! depth, how far it runs and how it ends.

use bench_api::Tone;
use core_document::{BodyId, WorkbenchRuntimeContext};

use super::{Formulas, choice_shown, face_pick_row, mm_drag, taper_note};
use crate::feature::{DesignFeature, ThreadSpec};
use crate::form::Form;
use crate::hole_tables::ThreadStandard;

/// The standards a thread on a face offers: every parallel one, a taper
/// thread being a Hole's.
fn standards() -> Vec<ThreadStandard> {
    ThreadStandard::ALL
        .into_iter()
        .filter(|s| !s.is_tapered())
        .collect()
}

/// The Thread editor; `true` when it changed the feature.
pub(super) fn thread_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature: &mut DesignFeature,
) -> bool {
    let seat = crate::build::thread_seat(ctx.document, body, feature);
    let left = crate::build::thread_left_handed(feature);
    let form = crate::build::thread_form(feature);
    let designation = crate::build::thread_designation(feature);
    let DesignFeature::Thread {
        face,
        thread,
        pitch,
        depth,
        length,
        full_length,
        from_far_end,
        left_handed,
        chamfer,
        run_out,
        cosmetic,
        ..
    } = feature
    else {
        return false;
    };
    let mut changed = false;
    let mut picked = Some(*face);
    if face_pick_row(f, ctx, &mut picked, "Face:")
        && let Some(new) = picked
    {
        *face = new;
        changed = true;
    }
    if let Some(seat) = &seat {
        taper_note(
            f,
            &format!(
                "{} Ø {:.2} mm, {:.2} mm long",
                if seat.external { "Shaft" } else { "Bore" },
                seat.radius * 2.0,
                seat.face_length
            ),
        );
    }

    let offered = standards();
    let mut options = vec!["Custom".to_string()];
    options.extend(offered.iter().map(|s| s.label().to_string()));
    let standard = thread.as_ref().map(|t| t.standard);
    let at = match standard {
        None => Some(0),
        Some(s) => offered.iter().position(|c| *c == s).map(|i| i + 1),
    };
    let shown = standard.map_or("Custom", ThreadStandard::label).to_string();
    match choice_shown(f, "Standard", shown, options, at) {
        Some(0) => {
            *thread = None;
            changed = true;
        }
        Some(i) => {
            let candidate = offered[i - 1];
            let size = seat.as_ref().map_or_else(
                || candidate.sizes().first().map_or("", |s| s.name).to_string(),
                |s| crate::build::nearest_thread_size(candidate, s.radius * 2.0, s.external),
            );
            *thread = Some(ThreadSpec::new(candidate, &size));
            changed = true;
        }
        None => {}
    }
    match thread {
        Some(spec) => {
            let sizes = spec.standard.sizes();
            let names = sizes.iter().map(|s| s.name.to_string()).collect();
            let at = sizes.iter().position(|s| spec.size == s.name);
            if let Some(i) = choice_shown(f, "Size", spec.size.clone(), names, at) {
                spec.size = sizes[i].name.to_string();
                changed = true;
            }
        }
        None => {
            changed |= mm_drag(f, fx, pitch, ("Pitch:", "pitch"));
            changed |= mm_drag(f, fx, depth, ("Depth:", "depth"));
        }
    }
    let mut lh = left;
    if f.check(
        &mut lh,
        "Left-hand thread",
        "The thread turns the other way",
    ) {
        *left_handed = lh;
        if let Some(spec) = thread {
            spec.left_handed = false;
        }
        changed = true;
    }

    changed |= f.check(
        full_length,
        "Whole face",
        "The thread runs the face's whole length",
    );
    if !*full_length {
        changed |= mm_drag(f, fx, length, ("Length:", "length"));
    }
    changed |= f.check(
        from_far_end,
        "From the other end",
        "Start at the end of the face farther from where it was picked",
    );
    if !*cosmetic {
        changed |= f.check(
            chamfer,
            "Chamfer the start",
            "A 45° chamfer at the starting end, down to the thread's root, so it starts \
             cleanly and prints without a ragged first turn",
        );
        changed |= f.check(
            run_out,
            "Run out at the end",
            "The groove grows shallower over its last two pitches instead of ending square",
        );
    }
    changed |= f.check(
        cosmetic,
        "Cosmetic thread",
        "Show the thread on the face and call it out, without cutting it",
    );
    match (&form, designation) {
        (Ok((pitch, depth, _)), Some(name)) => f.caption(
            &format!("{name} · pitch {pitch:.3} mm · depth {depth:.3} mm"),
            Tone::Neutral,
            false,
            true,
        ),
        (Err(why), _) => f.caption(why, Tone::Bad, false, false),
        _ => {}
    }
    changed
}
