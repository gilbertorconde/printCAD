//! The Project and Text task panels: a sketch carried onto picked faces,
//! and text written into a sketch of its own.

use core_document::WorkbenchRuntimeContext;

use crate::feature::DesignFeature;
use crate::form::Form;

use super::{
    CHOICE_WIDTH, Formulas, Kind, choice, face_list_editor, mm_drag, number, pick, set_f32,
    taper_note,
};

/// The Project and Text editors; `true` when it changed the feature. Any
/// other kind is left alone.
pub(super) fn project_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    feature: &mut DesignFeature,
) -> bool {
    let mut changed = false;
    match feature {
        DesignFeature::Project {
            sketch: _,
            faces,
            mode,
            height,
            wrap,
        } => {
            changed |= face_list_editor(f, ctx, faces, "Faces to project onto:");
            changed |= pick(
                f,
                "Mode",
                &kernel_api::ProjectMode::ALL,
                mode,
                |candidate| candidate.label(),
            );
            if *mode != kernel_api::ProjectMode::Split {
                changed |= mm_drag(f, fx, height, ("Height:", "height"));
            }
            changed |= f.check(wrap, "Wrap onto the curved surface", "");
            taper_note(
                f,
                if *wrap {
                    "The sketch is wrapped onto the faces' surface from where a straight \
                     projection would put its middle: unrolled onto a cylinder or cone with \
                     its lengths kept, laid onto a sphere, torus or free-form face with the \
                     lengths out from its middle kept."
                } else {
                    "The sketch's curves are carried along its normal onto the faces, landing \
                     on the side nearest the sketch."
                },
            );
        }
        DesignFeature::Text {
            sketch: _,
            faces,
            mode,
            height,
            wrap,
            text,
            font,
            size,
            spacing,
            angle,
            across,
            up,
            along,
            full_turn,
            radius,
        } => {
            if let Some(bench_api::PanelEvent::Text { value, .. }) = f.entry(
                "Text",
                bench_api::Entry {
                    label: "Text".into(),
                    value: text.clone(),
                    multiline: true,
                    width: Some(CHOICE_WIDTH),
                    ..Default::default()
                },
            ) && value != text
                && !value.trim().is_empty()
            {
                *text = value.clone();
                changed = true;
            }
            let fonts = wb_sketch::text::FONTS;
            let mut options: Vec<String> = fonts.iter().map(|(n, _)| n.to_string()).collect();
            let at = fonts.iter().position(|(n, _)| *n == font.as_str());
            if at.is_none() {
                options.push(font.clone());
            }
            if let Some(i) = choice(f, "Font", options, at.unwrap_or(fonts.len()))
                && let Some((name, _)) = fonts.get(i)
                && *name != font.as_str()
            {
                *font = name.to_string();
                changed = true;
            }
            for (value, label, kind) in [
                (&mut *size, "Size", Kind::Mm),
                (&mut *spacing, "Spacing", Kind::Offset),
                (&mut *across, "Across", Kind::Offset),
                (&mut *up, "Up", Kind::Offset),
                (&mut *angle, "Angle", Kind::Degrees),
            ] {
                let set = number(f, label, f64::from(*value), kind, 0.1);
                changed |= set_f32(value, set);
            }
            let modes = [
                (
                    kernel_api::ProjectMode::Emboss,
                    "pad",
                    "Raised out of the face",
                ),
                (
                    kernel_api::ProjectMode::Deboss,
                    "pocket",
                    "Sunk into the face",
                ),
                (
                    kernel_api::ProjectMode::Split,
                    "split",
                    "Imprinted: the face split along the letters",
                ),
            ];
            let shown: Vec<(&str, &str, bool)> = modes
                .iter()
                .map(|(m, icon, hint)| (*icon, *hint, m == mode))
                .collect();
            if let Some(n) = f.toggles("Mode", &shown) {
                changed |= *mode != modes[n].0;
                *mode = modes[n].0;
            }
            if *mode != kernel_api::ProjectMode::Split {
                changed |= mm_drag(f, fx, height, ("Height:", "height"));
            }
            changed |= face_list_editor(f, ctx, faces, "Faces to write on:");
            // How it lies on the face: flat, or wrapped round the axis,
            // along it, or all the way round; the lit one clicked again lays
            // it flat.
            let mut ways = vec![
                (
                    "text-wrap-around",
                    "Wrapped onto the curved surface, round its axis",
                    *wrap && !*along && !*full_turn,
                ),
                (
                    "text-wrap-along",
                    "Wrapped onto the curved surface, along its axis",
                    *wrap && *along,
                ),
            ];
            if *radius > 0.0 {
                ways.push((
                    "text-wrap-turn",
                    "Spread evenly round the whole cylinder",
                    *wrap && !*along && *full_turn,
                ));
            }
            if let Some(n) = f.toggles("Wrap", &ways) {
                let now = match (n, ways[n].2) {
                    (_, true) => (false, false, false),
                    (0, false) => (true, false, false),
                    (1, false) => (true, true, false),
                    _ => (true, false, true),
                };
                changed |= (*wrap, *along, *full_turn) != now;
                (*wrap, *along, *full_turn) = now;
            }
            taper_note(
                f,
                "The text is written in a sketch of its own, hidden under this feature: open \
                 it to lay the text out in a frame or along curves, or to set its size by a \
                 formula. A {formula} in the text shows its value.",
            );
        }
        _ => {}
    }
    changed
}
