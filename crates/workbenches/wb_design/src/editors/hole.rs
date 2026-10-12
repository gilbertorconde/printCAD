//! The Hole task panel: the thread it is sized from (standard, size,
//! class, hand), the drill and its point, the cut around its mouth.

use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};
use egui::Ui;
use ui_kit::tokens::*;
use ui_kit::widgets::{check_row, mono_label};

use super::{Formulas, deg_drag, label_cell, mm_drag, sketch_combo};
use crate::feature::{DesignFeature, DrillPoint, HoleCut, HoleFit, NutSide, NutTrap, ThreadSpec};
use crate::hole_tables::{NutStandard, ScrewSeat, ThreadStandard, user_cut_profiles};

/// The drill points the panel offers by name, included angle in degrees.
const POINT_ANGLES: [f32; 2] = [118.0, 135.0];

/// A combo row: `label` beside a dropdown showing `current`, whose
/// entries `add` draws.
fn combo_row(
    ui: &mut Ui,
    label: &str,
    id_salt: impl egui::AsIdSalt,
    current: impl Into<egui::WidgetText>,
    add: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        label_cell(ui, label);
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(current)
            .show_ui(ui, add);
    });
}

/// The Hole editor; `true` when it changed the feature.
pub(super) fn hole_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
) -> bool {
    // What the hole drills as it stands, before this frame's edits.
    let drilled = crate::build::hole_diameter(feature);
    let DesignFeature::Hole {
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
    } = feature
    else {
        return false;
    };
    let mut changed = false;
    if let Some(new) = sketch_combo(
        ui,
        ctx,
        body,
        ("hole_sketch", feature_id),
        Some(*sketch),
        "Positions:",
    ) {
        *sketch = new;
        changed = true;
    }

    let standard = thread.as_ref().map(|t| t.standard);
    combo_row(
        ui,
        "Standard",
        ("hole_standard", feature_id),
        standard.map_or("Custom", ThreadStandard::label),
        |ui| {
            if ui.selectable_label(standard.is_none(), "Custom").clicked() && standard.is_some() {
                *thread = None;
                changed = true;
            }
            for candidate in ThreadStandard::ALL {
                if ui
                    .selectable_label(standard == Some(candidate), candidate.label())
                    .clicked()
                    && standard != Some(candidate)
                {
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
            }
        },
    );

    if let Some(spec) = thread {
        let sizes = spec.standard.sizes();
        combo_row(
            ui,
            "Size",
            ("hole_size", feature_id),
            spec.size.clone(),
            |ui| {
                for size in &sizes {
                    if ui
                        .selectable_label(spec.size == size.name, size.name)
                        .clicked()
                        && spec.size != size.name
                    {
                        spec.size = size.name.to_string();
                        changed = true;
                    }
                }
            },
        );
        let classes = spec.standard.classes();
        if !classes.is_empty() {
            let current = spec.class().to_string();
            combo_row(
                ui,
                "Class",
                ("hole_class", feature_id),
                current.clone(),
                |ui| {
                    for class in classes {
                        if ui.selectable_label(current == *class, *class).clicked()
                            && current != *class
                        {
                            spec.class = class.to_string();
                            changed = true;
                        }
                    }
                },
            );
        }
        changed |= check_row(ui, &mut spec.left_handed, "Left-hand thread")
            .on_hover_text("The thread turns the other way")
            .changed();
        let size = spec.resolve();
        changed |= check_row(ui, threaded, "Threaded (tap drill)")
            .on_hover_text("Use the tap-drill diameter for later thread cutting")
            .changed();
        if *threaded {
            if check_row(ui, modeled_thread, "Modeled thread")
                .on_hover_text("Cut the thread itself into the wall, to print it")
                .changed()
            {
                if *modeled_thread && *thread_depth <= 0.0 {
                    *thread_depth = if *through_all { 10.0 } else { *depth };
                }
                changed = true;
            }
            if *modeled_thread {
                combo_row(
                    ui,
                    "Thread length",
                    ("hole_thread_length", feature_id),
                    thread_length.label(),
                    |ui| {
                        for candidate in crate::feature::ThreadLength::ALL {
                            if ui
                                .selectable_label(*thread_length == candidate, candidate.label())
                                .clicked()
                                && *thread_length != candidate
                            {
                                *thread_length = candidate;
                                changed = true;
                            }
                        }
                    },
                );
                if *thread_length == crate::feature::ThreadLength::Given {
                    changed |= mm_drag(ui, fx, thread_depth, "Thread depth:");
                }
            }
        } else if size.as_ref().is_ok() {
            let shown = match clearance {
                Some(_) => "Custom",
                None if size.as_ref().is_ok_and(|s| s.clearance.is_some()) => fit.label(),
                None => "Major diameter",
            };
            combo_row(ui, "Fit", ("hole_fit", feature_id), shown, |ui| {
                if size.as_ref().is_ok_and(|s| s.clearance.is_some()) {
                    for candidate in HoleFit::ALL {
                        let current = clearance.is_none() && *fit == candidate;
                        if ui.selectable_label(current, candidate.label()).clicked() && !current {
                            *fit = candidate;
                            *clearance = None;
                            changed = true;
                        }
                    }
                }
                if ui.selectable_label(clearance.is_some(), "Custom").clicked()
                    && clearance.is_none()
                {
                    *clearance = Some(drilled);
                    changed = true;
                }
            });
            if let Some(own) = clearance {
                changed |= mm_drag(ui, fx, own, "Clearance:");
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
        mono_label(ui, summary, FONT_SM, TEXT2);
    } else {
        changed |= mm_drag(ui, fx, diameter, "Diameter:");
    }

    changed |= check_row(ui, through_all, "Through all").changed();
    if !*through_all {
        changed |= mm_drag(ui, fx, depth, "Depth:");
        let point_label = match drill_point {
            DrillPoint::Flat => "Flat".to_string(),
            DrillPoint::Angled { angle_deg } => format!("{angle_deg}°"),
        };
        combo_row(
            ui,
            "Drill point",
            ("hole_point", feature_id),
            point_label,
            |ui| {
                if ui
                    .selectable_label(*drill_point == DrillPoint::Flat, "Flat")
                    .clicked()
                    && *drill_point != DrillPoint::Flat
                {
                    *drill_point = DrillPoint::Flat;
                    changed = true;
                }
                for angle_deg in POINT_ANGLES {
                    let point = DrillPoint::Angled { angle_deg };
                    if ui
                        .selectable_label(*drill_point == point, format!("{angle_deg}°"))
                        .clicked()
                        && *drill_point != point
                    {
                        *drill_point = point;
                        changed = true;
                    }
                }
            },
        );
        if let DrillPoint::Angled { angle_deg } = drill_point {
            changed |= deg_drag(ui, fx, angle_deg, "Point angle:", 10.0..=170.0);
            changed |= check_row(ui, point_in_depth, "Point within the depth")
                .on_hover_text("The depth runs to the tip of the point, not to the wall's end")
                .changed();
        }
    }
    let standard_taper = *threaded && thread.as_ref().is_some_and(|t| t.standard.is_tapered());
    if !standard_taper {
        changed |= deg_drag(ui, fx, taper_deg, "Taper:", -44.0..=44.0);
    }

    let metric = thread.as_ref().is_some_and(|t| t.standard.is_metric());
    let profiles = user_cut_profiles();
    let current = profiles
        .iter()
        .find(|p| p.cut == *cut)
        .map_or_else(|| cut.label().to_string(), |p| p.name.clone());
    combo_row(ui, "Hole cut", ("hole_cut", feature_id), current, |ui| {
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
        for candidate in options {
            let is_current = match (&*cut, &candidate) {
                (HoleCut::Seat { seat: a }, HoleCut::Seat { seat: b }) => a == b,
                (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
            };
            if ui.selectable_label(is_current, candidate.label()).clicked() && !is_current {
                *cut = candidate;
                changed = true;
            }
        }
        if !profiles.is_empty() {
            ui.separator();
            for profile in profiles {
                let is_current = profile.cut == *cut;
                if ui.selectable_label(is_current, &profile.name).clicked() && !is_current {
                    *cut = profile.cut;
                    changed = true;
                }
            }
        }
    });
    match cut {
        HoleCut::None => {}
        HoleCut::Counterbore { diameter, depth } => {
            changed |= mm_drag(ui, fx, diameter, "Bore Ø:");
            changed |= mm_drag(ui, fx, depth, "Bore depth:");
        }
        HoleCut::Spotface { diameter, depth } => {
            changed |= mm_drag(ui, fx, diameter, "Spot Ø:");
            changed |= mm_drag(ui, fx, depth, "Spot depth:");
        }
        HoleCut::Countersink {
            diameter,
            angle_deg,
        } => {
            changed |= mm_drag(ui, fx, diameter, "Sink Ø:");
            changed |= deg_drag(ui, fx, angle_deg, "Sink angle:", 10.0..=170.0);
        }
        HoleCut::Counterdrill {
            diameter,
            depth,
            angle_deg,
        } => {
            changed |= mm_drag(ui, fx, diameter, "Counterdrill Ø:");
            changed |= mm_drag(ui, fx, depth, "Counterdrill depth:");
            changed |= deg_drag(ui, fx, angle_deg, "Counterdrill angle:", 10.0..=170.0);
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
            mono_label(ui, text, FONT_SM, TEXT2);
        }
    }
    let nominal = thread
        .as_ref()
        .filter(|t| t.standard.is_metric())
        .and_then(|t| t.resolve().ok())
        .map(|size| size.major);
    changed |= nut_trap_editor(ui, fx, feature_id, nut_trap, nominal, *through_all);
    changed |= check_row(ui, reversed, "Reversed").changed();
    changed
}

/// The nut trap's rows: on or off, the nut it is sized from (or a size
/// of its own), clearance, depth, side and turn, and what it cuts.
fn nut_trap_editor(
    ui: &mut Ui,
    fx: &mut Formulas,
    feature_id: FeatureId,
    nut_trap: &mut Option<NutTrap>,
    nominal: Option<f64>,
    through_all: bool,
) -> bool {
    let mut changed = false;
    let mut on = nut_trap.is_some();
    if check_row(ui, &mut on, "Nut trap")
        .on_hover_text("A hexagonal pocket at one end that holds a nut captive")
        .changed()
    {
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
    combo_row(ui, "Nut", ("hole_nut", feature_id), shown, |ui| {
        for standard in NutStandard::ALL {
            let current = trap.across_flats.is_none() && trap.standard == standard;
            if ui.selectable_label(current, standard.label()).clicked() && !current {
                trap.standard = standard;
                trap.across_flats = None;
                changed = true;
            }
        }
        if ui
            .selectable_label(trap.across_flats.is_some(), "Own size")
            .clicked()
            && trap.across_flats.is_none()
        {
            let made = crate::build::nut_pocket(trap, nominal).ok();
            trap.across_flats = Some(made.map_or(8.0, |p| p.across_flats as f32));
            trap.depth = Some(made.map_or(3.0, |p| p.depth as f32));
            changed = true;
        }
    });
    if let Some(across_flats) = &mut trap.across_flats {
        changed |= mm_drag(ui, fx, across_flats, "Across flats:");
    } else {
        changed |= mm_drag(ui, fx, &mut trap.clearance, "Nut clearance:");
    }
    let mut own_depth = trap.depth.is_some();
    if check_row(ui, &mut own_depth, "Own depth")
        .on_hover_text("Deeper or shallower than the nut and its clearance")
        .changed()
    {
        trap.depth = if own_depth {
            Some(crate::build::nut_pocket(trap, nominal).map_or(3.0, |p| p.depth as f32))
        } else {
            None
        };
        changed = true;
    }
    if let Some(depth) = &mut trap.depth {
        changed |= mm_drag(ui, fx, depth, "Nut depth:");
    }
    combo_row(
        ui,
        "Nut side",
        ("hole_nut_side", feature_id),
        side_label(trap.side),
        |ui| {
            for side in [NutSide::Top, NutSide::Bottom] {
                if ui
                    .selectable_label(trap.side == side, side_label(side))
                    .clicked()
                    && trap.side != side
                {
                    trap.side = side;
                    changed = true;
                }
            }
        },
    );
    changed |= deg_drag(ui, fx, &mut trap.turn_deg, "Nut turn:", -180.0..=180.0);
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
    mono_label(ui, text, FONT_SM, TEXT2);
    changed
}

fn side_label(side: NutSide) -> &'static str {
    match side {
        NutSide::Top => "Top (at the mouth)",
        NutSide::Bottom => "Bottom (where it ends)",
    }
}
