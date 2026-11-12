//! The Primitive task panel: the solid's kind and sizes, and where it
//! stands (attached, or placed by numbers).

use bench_api::Quantity;
use core_document::{FeatureId, WorkbenchRuntimeContext};

use crate::feature::DesignFeature;
use crate::form::Form;

use super::{
    Bounds, Formulas, bare, choice_shown, clean, deg_drag, named_f64, offset_drag, taper_note,
};

/// The Primitive editor; `true` when it changed the feature. Any other kind
/// is left alone.
#[allow(clippy::single_match)]
pub(super) fn primitive_feature_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
) -> bool {
    let mut changed = false;
    match feature {
        DesignFeature::Primitive {
            keep_inside: _,
            refine: _,
            boolean_tolerance: _,
            kind,
            placement,
            subtractive,
            attached,
        } => {
            changed |= primitive_editor(f, fx, kind);
            let mut attach = attached.is_some();
            if f.check(
                &mut attach,
                "Attached",
                "Placed by a mode on what is picked, as a datum is, and following it",
            ) {
                *attached = attach.then(|| {
                    Box::new(crate::feature::Attached {
                        attachment: core_document::DatumAttachment::BasePlane(
                            core_document::BasePlane::XY,
                        ),
                        offset: core_document::AttachmentOffset::default(),
                    })
                });
                changed = true;
            }
            match attached {
                Some(a) => {
                    let mut datum = a.datum();
                    if crate::datum_panel::attachment_editor(f, ctx, feature_id, &mut datum) {
                        **a = crate::feature::Attached::from_datum(&datum);
                        changed = true;
                    }
                    let [x, y, n] = &mut a.offset.translation;
                    changed |= offset_drag(f, fx, x, ("Offset X:", "offset_x"));
                    changed |= offset_drag(f, fx, y, ("Offset Y:", "offset_y"));
                    changed |= offset_drag(f, fx, n, ("Normal offset:", "offset_z"));
                    changed |= deg_drag(
                        f,
                        fx,
                        &mut a.offset.rotation_deg,
                        ("Rotation:", "rotation"),
                        -180.0..=180.0,
                    );
                    changed |= f.check(&mut a.offset.flip, "Flip side", "");
                }
                None => changed |= placement_editor(f, fx, placement),
            }
            changed |= f.check(subtractive, "Subtractive", "");
        }
        _ => {}
    }
    changed
}

fn primitive_editor(f: &mut Form, fx: &mut Formulas, kind: &mut kernel_api::PrimitiveKind) -> bool {
    use kernel_api::PrimitiveKind as P;
    let mut changed = false;
    let variants: [(&str, P); 9] = [
        (
            "Box",
            P::Box {
                length: 10.0,
                width: 10.0,
                height: 10.0,
            },
        ),
        (
            "Cylinder",
            P::Cylinder {
                radius: 5.0,
                height: 10.0,
                angle_deg: 360.0,
            },
        ),
        (
            "Sphere",
            P::Sphere {
                radius: 5.0,
                angle1_deg: -90.0,
                angle2_deg: 90.0,
                angle3_deg: 360.0,
            },
        ),
        (
            "Cone",
            P::Cone {
                radius1: 5.0,
                radius2: 2.0,
                height: 10.0,
                angle_deg: 360.0,
            },
        ),
        (
            "Torus",
            P::Torus {
                radius1: 10.0,
                radius2: 2.0,
                angle1_deg: -180.0,
                angle2_deg: 180.0,
                angle3_deg: 360.0,
            },
        ),
        (
            "Ellipsoid",
            P::Ellipsoid {
                radius1: 8.0,
                radius2: 5.0,
                radius3: 3.0,
                angle1_deg: -90.0,
                angle2_deg: 90.0,
                angle3_deg: 360.0,
            },
        ),
        (
            "Prism",
            P::Prism {
                sides: 6,
                circumradius: 5.0,
                height: 10.0,
                skew_x_deg: 0.0,
                skew_y_deg: 0.0,
            },
        ),
        (
            "Wedge",
            P::Wedge {
                xmin: 0.0,
                xmax: 10.0,
                ymin: 0.0,
                ymax: 10.0,
                zmin: 0.0,
                zmax: 10.0,
                x2min: 2.0,
                x2max: 8.0,
                z2min: 2.0,
                z2max: 8.0,
            },
        ),
        (
            "Tube",
            P::Tube {
                outer: 5.0,
                inner: 3.0,
                height: 10.0,
            },
        ),
    ];
    let at = variants
        .iter()
        .position(|(_, v)| std::mem::discriminant(kind) == std::mem::discriminant(v));
    let current_label = at.map_or("?", |i| variants[i].0);
    let names = variants.iter().map(|(l, _)| l.to_string()).collect();
    if let Some(i) = choice_shown(f, "Shape", current_label.into(), names, at) {
        *kind = variants[i].1;
        changed = true;
    }

    // Each number is the parameter formulas call by its field's name (an
    // angle's without `_deg`), a formula field where the feature has it.
    let name_of = |label: &str| -> String {
        label
            .trim_end_matches(':')
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_lowercase()
    };
    let mut num = |f: &mut Form, value: &mut f64, label: &str, angular: bool, lo: f64, hi: f64| {
        let bounds = Bounds::within(lo, hi);
        let changed = named_f64(
            f,
            fx,
            value,
            (label, &name_of(label), bounds),
            |f, value| {
                let set = f.quantity(Quantity {
                    label: clean(label).into(),
                    value: *value,
                    speed: Some(if angular { 1.0 } else { 0.5 }),
                    min: Some(lo),
                    max: Some(hi),
                    unit: Some(if angular { "°" } else { " mm" }.into()),
                    bare: true,
                    ..Default::default()
                });
                match set {
                    Some(v) => {
                        *value = v;
                        true
                    }
                    None => false,
                }
            },
        );
        *value = value.clamp(lo, hi);
        changed
    };

    match kind {
        P::Box {
            length,
            width,
            height,
        } => {
            changed |= num(f, length, "Length:", false, 0.01, 1.0e6);
            changed |= num(f, width, "Width:", false, 0.01, 1.0e6);
            changed |= num(f, height, "Height:", false, 0.01, 1.0e6);
        }
        P::Tube {
            outer,
            inner,
            height,
        } => {
            changed |= num(f, outer, "Outer:", false, 0.01, 1.0e6);
            changed |= num(f, inner, "Inner:", false, 0.01, 1.0e6);
            changed |= num(f, height, "Height:", false, 0.01, 1.0e6);
            let note = if *inner >= *outer {
                "The inner radius must be smaller than the outer."
            } else {
                "Outer and inner are the wall's radii."
            };
            taper_note(f, note);
        }
        P::Cylinder {
            radius,
            height,
            angle_deg,
        } => {
            changed |= num(f, radius, "Radius:", false, 0.01, 1.0e6);
            changed |= num(f, height, "Height:", false, 0.01, 1.0e6);
            changed |= num(f, angle_deg, "Angle:", true, 1.0, 360.0);
        }
        P::Sphere {
            radius,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => {
            changed |= num(f, radius, "Radius:", false, 0.01, 1.0e6);
            changed |= num(f, angle1_deg, "Angle 1:", true, -90.0, 90.0);
            changed |= num(f, angle2_deg, "Angle 2:", true, -90.0, 90.0);
            changed |= num(f, angle3_deg, "Angle 3:", true, 1.0, 360.0);
        }
        P::Cone {
            radius1,
            radius2,
            height,
            angle_deg,
        } => {
            changed |= num(f, radius1, "Radius 1:", false, 0.0, 1.0e6);
            changed |= num(f, radius2, "Radius 2:", false, 0.0, 1.0e6);
            changed |= num(f, height, "Height:", false, 0.01, 1.0e6);
            changed |= num(f, angle_deg, "Angle:", true, 1.0, 360.0);
        }
        P::Torus {
            radius1,
            radius2,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => {
            changed |= num(f, radius1, "Radius 1:", false, 0.01, 1.0e6);
            changed |= num(f, radius2, "Radius 2:", false, 0.01, 1.0e6);
            changed |= num(f, angle1_deg, "Angle 1:", true, -180.0, 180.0);
            changed |= num(f, angle2_deg, "Angle 2:", true, -180.0, 180.0);
            changed |= num(f, angle3_deg, "Angle 3:", true, 1.0, 360.0);
        }
        P::Ellipsoid {
            radius1,
            radius2,
            radius3,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => {
            changed |= num(f, radius1, "Radius 1:", false, 0.01, 1.0e6);
            changed |= num(f, radius2, "Radius 2:", false, 0.01, 1.0e6);
            changed |= num(f, radius3, "Radius 3:", false, 0.01, 1.0e6);
            changed |= num(f, angle1_deg, "Angle 1:", true, -90.0, 90.0);
            changed |= num(f, angle2_deg, "Angle 2:", true, -90.0, 90.0);
            changed |= num(f, angle3_deg, "Angle 3:", true, 1.0, 360.0);
        }
        P::Prism {
            sides,
            circumradius,
            height,
            skew_x_deg,
            skew_y_deg,
        } => {
            changed |= num(f, skew_x_deg, "Skew X:", true, -85.0, 85.0);
            changed |= num(f, skew_y_deg, "Skew Y:", true, -85.0, 85.0);
            let set = f.quantity(Quantity {
                label: "Sides".into(),
                value: f64::from(*sides),
                speed: Some(0.1),
                min: Some(3.0),
                max: Some(64.0),
                decimals: 0,
                bare: true,
                ..Default::default()
            });
            if let Some(v) = set {
                *sides = v.round() as u32;
                changed = true;
            }
            changed |= num(f, circumradius, "Circumradius:", false, 0.01, 1.0e6);
            changed |= num(f, height, "Height:", false, 0.01, 1.0e6);
        }
        P::Wedge {
            xmin,
            xmax,
            ymin,
            ymax,
            zmin,
            zmax,
            x2min,
            x2max,
            z2min,
            z2max,
        } => {
            for (value, label) in [
                (xmin, "X min:"),
                (xmax, "X max:"),
                (ymin, "Y min:"),
                (ymax, "Y max:"),
                (zmin, "Z min:"),
                (zmax, "Z max:"),
                (x2min, "X2 min:"),
                (x2max, "X2 max:"),
                (z2min, "Z2 min:"),
                (z2max, "Z2 max:"),
            ] {
                changed |= num(f, value, label, false, -1.0e6, 1.0e6);
            }
        }
    }
    changed
}

fn placement_editor(
    f: &mut Form,
    fx: &mut Formulas,
    placement: &mut kernel_api::Placement,
) -> bool {
    let mut changed = false;
    for (value, (label, name)) in placement.origin.iter_mut().zip([
        ("Position X", "x"),
        ("Position Y", "y"),
        ("Position Z", "z"),
    ]) {
        changed |= named_f64(
            f,
            fx,
            value,
            (label, name, Bounds::default()),
            |f, value| match bare(f, label, *value, 0.5, " mm") {
                Some(v) => {
                    *value = v;
                    true
                }
                None => false,
            },
        );
    }
    changed
}
