//! The Hole task panel: the thread it is sized from (standard, size,
//! class, hand), the drill and its point, the cut around its mouth.

use bench_api::Tone;
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use super::{Formulas, choice_shown, deg_drag, mm_drag, sketch_combo, start_rows};
use crate::feature::{DesignFeature, DrillPoint, HoleCut, HoleFit, NutSide, NutTrap, ThreadSpec};
use crate::form::Form;
use crate::hole_tables::{NutStandard, ScrewSeat, ThreadStandard, user_cut_profiles};

/// The drill points the panel offers by name, included angle in degrees.
const POINT_ANGLES: [f32; 2] = [118.0, 135.0];

/// A mono line under the fields: what the numbers make.
fn summary_line(f: &mut Form, text: &str) {
    f.caption(text, Tone::Neutral, false, true);
}

/// The Hole editor; `true` when it changed the feature.
pub(super) fn hole_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
) -> bool {
    // What the hole drills as it stands, before this frame's edits.
    let drilled = crate::build::hole_diameter(feature);
    let DesignFeature::Hole {
        cosmetic_thread,
        refine: _,
        boolean_tolerance: _,
        sketch,
        diameter,
        depth,
        through_all,
        cut,
        thread,
        threaded,
        modeled_thread,
        thread_depth,
        fit,
        clearance,
        thread_length,
        drill_point,
        point_in_depth,
        taper_deg,
        nut_trap,
        reversed,
        start,
    } = feature
    else {
        return false;
    };
    let mut changed = false;
    if let Some(new) = sketch_combo(f, ctx, body, feature_id, Some(*sketch), "Positions:") {
        *sketch = new;
        changed = true;
    }

    let standard = thread.as_ref().map(|t| t.standard);
    let mut options = vec!["Custom".to_string()];
    options.extend(ThreadStandard::ALL.iter().map(|s| s.label().to_string()));
    let at = match standard {
        None => Some(0),
        Some(s) => ThreadStandard::ALL
            .iter()
            .position(|c| *c == s)
            .map(|i| i + 1),
    };
    let shown = standard.map_or("Custom", ThreadStandard::label).to_string();
    match choice_shown(f, "Standard", shown, options, at) {
        Some(0) => {
            *thread = None;
            changed = true;
        }
        Some(i) => {
            let candidate = ThreadStandard::ALL[i - 1];
            // The size nearest what the hole drills now.
            let nearest = candidate
                .sizes()
                .into_iter()
                .min_by(|a, b| {
                    let off = |d: f64| (d - f64::from(drilled)).abs();
                    off(a.major).total_cmp(&off(b.major))
                })
                .map_or("", |s| s.name);
            let left_handed = thread.as_ref().is_some_and(|t| t.left_handed);
            let mut spec = ThreadSpec::new(candidate, nearest);
            spec.left_handed = left_handed;
            *thread = Some(spec);
            changed = true;
        }
        None => {}
    }

    if let Some(spec) = thread {
        let sizes = spec.standard.sizes();
        let names = sizes.iter().map(|s| s.name.to_string()).collect();
        let at = sizes.iter().position(|s| spec.size == s.name);
        if let Some(i) = choice_shown(f, "Size", spec.size.clone(), names, at) {
            spec.size = sizes[i].name.to_string();
            changed = true;
        }
        let classes = spec.standard.classes();
        if !classes.is_empty() {
            let current = spec.class().to_string();
            let names = classes.iter().map(|c| c.to_string()).collect();
            let at = classes.iter().position(|c| current == *c);
            if let Some(i) = choice_shown(f, "Class", current, names, at) {
                spec.class = classes[i].to_string();
                changed = true;
            }
        }
        changed |= f.check(
            &mut spec.left_handed,
            "Left-hand thread",
            "The thread turns the other way",
        );
        let size = spec.resolve();
        changed |= f.check(
            threaded,
            "Threaded (tap drill)",
            "Use the tap-drill diameter for later thread cutting",
        );
        if *threaded {
            if !*modeled_thread {
                changed |= f.check(
                    cosmetic_thread,
                    "Cosmetic thread",
                    "Drill the tap size and show the thread down the bore: tap it after printing",
                );
            }
            if f.check(
                modeled_thread,
                "Modeled thread",
                "Cut the thread itself into the wall, to print it",
            ) {
                if *modeled_thread && *thread_depth <= 0.0 {
                    *thread_depth = if *through_all { 10.0 } else { *depth };
                }
                changed = true;
            }
            if *modeled_thread {
                changed |= super::pick(
                    f,
                    "Thread length",
                    &crate::feature::ThreadLength::ALL,
                    thread_length,
                    |candidate| candidate.label(),
                );
                if *thread_length == crate::feature::ThreadLength::Given {
                    changed |= mm_drag(f, fx, thread_depth, "Thread depth:");
                }
            }
        } else if size.as_ref().is_ok() {
            let shown = match clearance {
                Some(_) => "Custom",
                None if size.as_ref().is_ok_and(|s| s.clearance.is_some()) => fit.label(),
                None => "Major diameter",
            };
            let fits: &[HoleFit] = if size.as_ref().is_ok_and(|s| s.clearance.is_some()) {
                &HoleFit::ALL
            } else {
                &[]
            };
            let mut options: Vec<String> = fits.iter().map(|c| c.label().to_string()).collect();
            options.push("Custom".to_string());
            let at = match clearance {
                Some(_) => Some(fits.len()),
                None => fits.iter().position(|c| c == fit),
            };
            match choice_shown(f, "Fit", shown.to_string(), options, at) {
                Some(i) if i < fits.len() => {
                    *fit = fits[i];
                    *clearance = None;
                    changed = true;
                }
                Some(_) => {
                    *clearance = Some(drilled);
                    changed = true;
                }
                None => {}
            }
            if let Some(own) = clearance {
                changed |= mm_drag(f, fx, own, "Clearance:");
            }
        }
        let summary = match &size {
            Ok(size) => {
                let mut text = format!(
                    "{} · pitch {:.3} mm · drill Ø {drilled:.2} mm",
                    spec.designation(),
                    size.pitch
                );
                if *threaded && spec.standard.is_tapered() {
                    text.push_str(" at the face, 1:16 taper");
                }
                text
            }
            Err(e) => e.clone(),
        };
        summary_line(f, &summary);
    } else {
        changed |= mm_drag(f, fx, diameter, "Diameter:");
    }

    changed |= f.check(through_all, "Through all", "");
    if !*through_all {
        changed |= mm_drag(f, fx, depth, "Depth:");
        let point_label = match drill_point {
            DrillPoint::Flat => "Flat".to_string(),
            DrillPoint::Angled { angle_deg } => format!("{angle_deg}°"),
        };
        let points: Vec<DrillPoint> = std::iter::once(DrillPoint::Flat)
            .chain(POINT_ANGLES.map(|angle_deg| DrillPoint::Angled { angle_deg }))
            .collect();
        let names = points
            .iter()
            .map(|p| match p {
                DrillPoint::Flat => "Flat".to_string(),
                DrillPoint::Angled { angle_deg } => format!("{angle_deg}°"),
            })
            .collect();
        let at = points.iter().position(|p| p == drill_point);
        if let Some(i) = choice_shown(f, "Drill point", point_label, names, at) {
            *drill_point = points[i];
            changed = true;
        }
        if let DrillPoint::Angled { angle_deg } = drill_point {
            changed |= deg_drag(f, fx, angle_deg, "Point angle:", 10.0..=170.0);
            changed |= f.check(
                point_in_depth,
                "Point within the depth",
                "The depth runs to the tip of the point, not to the wall's end",
            );
        }
    }
    let standard_taper = *threaded && thread.as_ref().is_some_and(|t| t.standard.is_tapered());
    if !standard_taper {
        changed |= deg_drag(f, fx, taper_deg, "Taper:", -44.0..=44.0);
    }

    let metric = thread.as_ref().is_some_and(|t| t.standard.is_metric());
    let profiles = user_cut_profiles();
    let current = profiles
        .iter()
        .find(|p| p.cut == *cut)
        .map_or_else(|| cut.label().to_string(), |p| p.name.clone());
    {
        // Sized from the diameter the hole is drilled at, a standard
        // size's as much as a custom one.
        let mut options = vec![
            HoleCut::None,
            HoleCut::Counterbore {
                diameter: drilled * 2.0,
                depth: 2.0,
            },
            HoleCut::Spotface {
                diameter: drilled * 2.0,
                depth: 0.5,
            },
            HoleCut::Countersink {
                diameter: drilled * 2.0,
                angle_deg: 90.0,
            },
            HoleCut::Counterdrill {
                diameter: drilled * 1.5,
                depth: 2.0,
                angle_deg: 90.0,
            },
        ];
        if metric {
            options.extend(ScrewSeat::ALL.map(|seat| HoleCut::Seat { seat }));
        }
        let builtin = options.len();
        let at = options
            .iter()
            .position(|candidate| match (&*cut, candidate) {
                (HoleCut::Seat { seat: a }, HoleCut::Seat { seat: b }) => a == b,
                (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
            })
            .or_else(|| {
                profiles
                    .iter()
                    .position(|p| p.cut == *cut)
                    .map(|i| builtin + i)
            });
        let mut names: Vec<String> = options.iter().map(|c| c.label().to_string()).collect();
        names.extend(profiles.iter().map(|p| p.name.clone()));
        let picked = f.select(
            "Hole cut",
            bench_api::Select {
                label: "Hole cut".into(),
                selected: at.unwrap_or(names.len()),
                options: names,
                width: Some(super::CHOICE_WIDTH),
                text: Some(current),
                separated: if profiles.is_empty() {
                    Vec::new()
                } else {
                    vec![builtin]
                },
                ..Default::default()
            },
        );
        match picked {
            Some(i) if i < builtin => {
                *cut = options[i];
                changed = true;
            }
            Some(i) => {
                *cut = profiles[i - builtin].cut;
                changed = true;
            }
            None => {}
        }
    }
    match cut {
        HoleCut::None => {}
        HoleCut::Counterbore { diameter, depth } => {
            changed |= mm_drag(f, fx, diameter, "Bore Ø:");
            changed |= mm_drag(f, fx, depth, "Bore depth:");
        }
        HoleCut::Spotface { diameter, depth } => {
            changed |= mm_drag(f, fx, diameter, "Spot Ø:");
            changed |= mm_drag(f, fx, depth, "Spot depth:");
        }
        HoleCut::Countersink {
            diameter,
            angle_deg,
        } => {
            changed |= mm_drag(f, fx, diameter, "Sink Ø:");
            changed |= deg_drag(f, fx, angle_deg, "Sink angle:", 10.0..=170.0);
        }
        HoleCut::Counterdrill {
            diameter,
            depth,
            angle_deg,
        } => {
            changed |= mm_drag(f, fx, diameter, "Counterdrill Ø:");
            changed |= mm_drag(f, fx, depth, "Counterdrill depth:");
            changed |= deg_drag(f, fx, angle_deg, "Counterdrill angle:", 10.0..=170.0);
        }
        HoleCut::Seat { seat } => {
            let made = thread
                .as_ref()
                .filter(|t| t.standard.is_metric())
                .and_then(|t| t.resolve().ok())
                .and_then(|size| seat.cut(size.major));
            let text = match made {
                Some(HoleCut::Counterbore { diameter, depth }) => {
                    format!("Counterbore Ø {diameter:.1} mm, {depth:.1} mm deep")
                }
                Some(HoleCut::Countersink {
                    diameter,
                    angle_deg,
                }) => format!("Countersink Ø {diameter:.1} mm at {angle_deg}°"),
                _ => "No seat for this size".to_string(),
            };
            summary_line(f, &text);
        }
    }
    let nominal = thread
        .as_ref()
        .filter(|t| t.standard.is_metric())
        .and_then(|t| t.resolve().ok())
        .map(|size| size.major);
    changed |= nut_trap_editor(f, fx, nut_trap, nominal, *through_all);
    changed |= f.check(reversed, "Reversed", "");
    changed |= start_rows(
        f,
        ctx,
        body,
        fx,
        &mut start.at,
        &mut start.face,
        &mut start.offset,
    );
    changed
}

/// The nut trap's rows: on or off, the nut it is sized from (or a size
/// of its own), clearance, depth, side and turn, and what it cuts.
fn nut_trap_editor(
    f: &mut Form,
    fx: &mut Formulas,
    nut_trap: &mut Option<NutTrap>,
    nominal: Option<f64>,
    through_all: bool,
) -> bool {
    let mut changed = false;
    let mut on = nut_trap.is_some();
    if f.check(
        &mut on,
        "Nut trap",
        "A hexagonal pocket at one end that holds a nut captive",
    ) {
        *nut_trap = on.then(NutTrap::default);
        changed = true;
    }
    let Some(trap) = nut_trap else {
        return changed;
    };
    let shown = match trap.across_flats {
        Some(_) => "Own size",
        None => trap.standard.label(),
    };
    let mut options: Vec<String> = NutStandard::ALL
        .iter()
        .map(|s| s.label().to_string())
        .collect();
    options.push("Own size".to_string());
    let at = match trap.across_flats {
        Some(_) => Some(NutStandard::ALL.len()),
        None => NutStandard::ALL.iter().position(|s| *s == trap.standard),
    };
    match choice_shown(f, "Nut", shown.to_string(), options, at) {
        Some(i) if i < NutStandard::ALL.len() => {
            trap.standard = NutStandard::ALL[i];
            trap.across_flats = None;
            changed = true;
        }
        Some(_) => {
            let made = crate::build::nut_pocket(trap, nominal).ok();
            trap.across_flats = Some(made.map_or(8.0, |p| p.across_flats as f32));
            trap.depth = Some(made.map_or(3.0, |p| p.depth as f32));
            changed = true;
        }
        None => {}
    }
    if let Some(across_flats) = &mut trap.across_flats {
        changed |= mm_drag(f, fx, across_flats, "Across flats:");
    } else {
        changed |= mm_drag(f, fx, &mut trap.clearance, "Nut clearance:");
    }
    let mut own_depth = trap.depth.is_some();
    if f.check(
        &mut own_depth,
        "Own depth",
        "Deeper or shallower than the nut and its clearance",
    ) {
        trap.depth = if own_depth {
            Some(crate::build::nut_pocket(trap, nominal).map_or(3.0, |p| p.depth as f32))
        } else {
            None
        };
        changed = true;
    }
    if let Some(depth) = &mut trap.depth {
        changed |= mm_drag(f, fx, depth, "Nut depth:");
    }
    changed |= super::pick(
        f,
        "Nut side",
        &[NutSide::Top, NutSide::Bottom],
        &mut trap.side,
        |side| side_label(*side),
    );
    changed |= deg_drag(f, fx, &mut trap.turn_deg, "Nut turn:", -180.0..=180.0);
    let text = match crate::build::nut_pocket(trap, nominal) {
        Ok(_) if trap.side == NutSide::Bottom && through_all => {
            "At the bottom: give the hole a depth".to_string()
        }
        Ok(pocket) => format!(
            "Hex {:.2} mm across flats, {:.2} mm deep",
            pocket.across_flats, pocket.depth
        ),
        Err(why) => why,
    };
    summary_line(f, &text);
    changed
}

fn side_label(side: NutSide) -> &'static str {
    match side {
        NutSide::Top => "Top (at the mouth)",
        NutSide::Bottom => "Bottom (where it ends)",
    }
}
