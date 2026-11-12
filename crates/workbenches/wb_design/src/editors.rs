//! Per-feature parameter editors for the task panel, declared on a
//! [`Form`].
//!
//! Every option maps 1:1 to a feature field. Editors return `true` when
//! the feature payload changed; the task applies edits live. Rows are a
//! fixed-width label column beside a control, the way the design lays out
//! its parameter cards.

use bench_api::{Bind, ButtonStyle, Dim, Item, Quantity, Select, Tone};
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use crate::feature::{DesignFeature, EdgePick, ExtrudeMode, FacePick, MirrorPlane};
use crate::form::Form;

mod bodies;
mod borrow;
mod dressup;
mod extrude;
mod hole;
mod patterns;
mod primitive;
mod project;
mod sweeps;
mod transforms;

/// The label column's width, left-aligned labels before every control.
pub(crate) const LABEL_COLUMN: f32 = 96.0;

/// The width of a choice field, as the system's dropdown is at rest.
pub(crate) const CHOICE_WIDTH: f32 = 160.0;

/// An empty label cell, keeping a row's control in line with the
/// fields above it.
pub(crate) const LABEL_BLANK: &str = " ";

/// A label as the column shows it.
fn clean(label: &str) -> &str {
    label.trim_end_matches(':')
}

/// A row holding only `label` in the label column.
fn label_row(f: &mut Form, label: &str) {
    f.flow(clean(label), false, |_| {});
}

/// A choice of `options` under `label`, `selected` lit; the option
/// picked other than it.
fn choice(f: &mut Form, label: &str, options: Vec<String>, selected: usize) -> Option<usize> {
    f.select(
        clean(label),
        Select {
            label: clean(label).into(),
            options,
            selected,
            width: Some(CHOICE_WIDTH),
            ..Default::default()
        },
    )
}

/// One of `all` under `label`, each named by `name`; whether `current`
/// changed.
fn pick<T: Copy + PartialEq>(
    f: &mut Form,
    label: &str,
    all: &[T],
    current: &mut T,
    name: impl Fn(&T) -> &'static str,
) -> bool {
    let options = all.iter().map(|c| name(c).to_string()).collect();
    let at = all.iter().position(|c| c == current).unwrap_or(all.len());
    match choice(f, label, options, at) {
        Some(i) => {
            *current = all[i];
            true
        }
        None => false,
    }
}

/// A choice whose closed field shows `text` rather than an option: a
/// count, or a prompt for a list that only adds (`selected` none).
fn choice_shown(
    f: &mut Form,
    label: &str,
    text: String,
    options: Vec<String>,
    selected: Option<usize>,
) -> Option<usize> {
    f.select(
        clean(label),
        Select {
            label: clean(label).into(),
            selected: selected.unwrap_or(options.len()),
            options,
            width: Some(CHOICE_WIDTH),
            text: Some(text),
            ..Default::default()
        },
    )
}

/// A palette colour as the blocks take it, sRGB.
fn rgb(color: egui::Color32) -> [u8; 3] {
    [color.r(), color.g(), color.b()]
}

/// What a picked point reads as: `(x, y, z)` to a tenth.
fn coords(p: [f32; 3]) -> String {
    format!("({:.1}, {:.1}, {:.1})", p[0], p[1], p[2])
}

/// A mono value in the small size, in the primary text colour or dim.
fn mono_value(f: &mut Form, text: &str, strong: bool) {
    if strong {
        f.reading(text);
    } else {
        f.caption(text, Tone::Muted, true, true);
    }
}

/// A hint under a field, small and dim.
fn taper_note(f: &mut Form, text: &str) {
    f.caption(text, Tone::Muted, true, false);
}

/// A button that takes what is picked in the view, outlined in the
/// accent; whether it was pressed.
fn use_button(f: &mut Form, label: &str, enabled: bool, hint: &str) -> bool {
    f.button(label, ButtonStyle::Outline, enabled, hint)
}

fn small_button(f: &mut Form, label: &str, hint: &str) -> bool {
    f.button(label, ButtonStyle::Small, true, hint)
}

/// What a feature's fields know of formulas: the feature's parameters,
/// which the fields that show them bind to, and the formulas an editor
/// takes away itself this pass, for the task to apply (`(key, formula)`,
/// `None` taking one away). Formulas typed into a field reach the task
/// from the panel.
pub(crate) struct Formulas<'a> {
    document: &'a core_document::Document,
    feature: FeatureId,
    params: Vec<core_document::Parameter>,
    pub edits: Vec<(String, Option<String>)>,
    /// Put before each name looked up: a multi-transform step's fields
    /// are `step2_length` and the like.
    prefix: Option<String>,
}

impl<'a> Formulas<'a> {
    pub(crate) fn of(document: &'a core_document::Document, feature: FeatureId) -> Self {
        let params = document
            .get_feature_meta(feature)
            .map(|node| {
                if node.workbench_id.as_str() == "core.datum" {
                    crate::params::datum_parameters()
                } else {
                    crate::params::feature_parameters(node)
                }
            })
            .unwrap_or_default();
        Self {
            document,
            feature,
            params,
            edits: Vec::new(),
            prefix: None,
        }
    }

    /// The parameter formulas call `name`, when the feature has it.
    fn named(&self, name: &str) -> Option<core_document::Parameter> {
        let node = self.document.get_feature_meta(self.feature)?;
        self.params
            .iter()
            .find(|p| p.name.as_deref() == Some(name) && node.data.pointer(&p.pointer).is_some())
            .cloned()
    }

    /// The field `label` as a formula field, when the feature has the
    /// parameter formulas call `name` (after the step's prefix); an empty
    /// `name` never binds. `Some(changed)` when it did, the value set by
    /// hand coming back.
    fn show(&mut self, f: &mut Form, field: (&str, &str), value: f64) -> Option<(bool, f64)> {
        self.show_within(f, field, value, Bounds::default())
    }

    /// As [`Self::show`], the field held within `bounds`.
    fn show_within(
        &mut self,
        f: &mut Form,
        (label, name): (&str, &str),
        value: f64,
        bounds: Bounds,
    ) -> Option<(bool, f64)> {
        if name.is_empty() {
            return None;
        }
        debug_assert!(
            crate::params::every_name().iter().any(|n| n == name),
            "{label} binds {name}, which no feature lists"
        );
        let name = match &self.prefix {
            Some(prefix) => format!("{prefix}{name}"),
            None => name.to_string(),
        };
        let p = self.named(&name)?;
        Some(self.show_param(f, label, p, value, bounds))
    }

    /// As [`Self::show`], for the parameter formulas call `name` whatever
    /// the prefix (a primitive's `radius`, a step's `step2_length`).
    fn show_named(
        &mut self,
        f: &mut Form,
        label: &str,
        name: &str,
        value: f64,
    ) -> Option<(bool, f64)> {
        self.show_named_within(f, label, name, value, Bounds::default())
    }

    /// As [`Self::show_named`], the field held within `bounds`.
    fn show_named_within(
        &mut self,
        f: &mut Form,
        label: &str,
        name: &str,
        value: f64,
        bounds: Bounds,
    ) -> Option<(bool, f64)> {
        let p = self.named(name)?;
        Some(self.show_param(f, label, p, value, bounds))
    }

    fn show_param(
        &mut self,
        f: &mut Form,
        label: &str,
        p: core_document::Parameter,
        value: f64,
        bounds: Bounds,
    ) -> (bool, f64) {
        let dim = match p.dim {
            core_document::expr::Dim::LENGTH => Dim::Length,
            core_document::expr::Dim::ANGLE => Dim::Angle,
            _ => Dim::Number,
        };
        let q = Quantity {
            label: clean(label).into(),
            value,
            dim,
            bind: Some(Bind {
                feature: self.feature.0.to_string(),
                key: p.key,
            }),
            decimals: if p.integer { 0 } else { 2 },
            speed: Some(if p.integer { 0.05 } else { 0.5 }),
            min: bounds.min,
            max: bounds.max,
            drag_max: bounds.drag_max,
            ..Default::default()
        };
        match f.quantity(q) {
            Some(v) => (true, v),
            None => (false, value),
        }
    }
}

/// What a formula field holds its number to: `min` and `max` whatever
/// sets it, `drag_max` only what a drag sets.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Bounds {
    min: Option<f64>,
    max: Option<f64>,
    drag_max: Option<f64>,
}

impl Bounds {
    pub(super) fn within(min: f64, max: f64) -> Self {
        Self {
            min: Some(min),
            max: Some(max),
            drag_max: None,
        }
    }
}

/// A number field's kind: its unit and how far it may go.
#[derive(Clone, Copy)]
enum Kind {
    /// A length above zero.
    Mm,
    /// A length either way.
    Offset,
    Degrees,
    Plain,
}

/// A number under `label` as `kind` reads it, dragged `speed` a point.
fn quantity(label: &str, value: f64, kind: Kind, speed: f64) -> Quantity {
    let (dim, min, max) = match kind {
        Kind::Mm => (Dim::Length, Some(0.001), Some(1.0e6)),
        Kind::Offset => (Dim::Length, Some(-1.0e6), Some(1.0e6)),
        Kind::Degrees => (Dim::Angle, None, None),
        Kind::Plain => (Dim::Number, None, None),
    };
    Quantity {
        label: clean(label).into(),
        value,
        dim,
        min,
        max,
        speed: Some(speed),
        ..Default::default()
    }
}

/// A number field of `kind`; the value set.
fn number(f: &mut Form, label: &str, value: f64, kind: Kind, speed: f64) -> Option<f64> {
    f.quantity(quantity(label, value, kind, speed))
}

/// The system's bare number drag under `label`, `suffix` after it.
fn bare(f: &mut Form, label: &str, value: f64, speed: f64, suffix: &str) -> Option<f64> {
    f.quantity(Quantity {
        label: label.into(),
        value,
        speed: Some(speed),
        unit: (!suffix.is_empty()).then(|| suffix.into()),
        bare: true,
        ..Default::default()
    })
}

/// A number formulas call `name`, as a formula field when the feature has
/// that parameter, else as `fallback` draws it.
fn named_f64(
    f: &mut Form,
    fx: &mut Formulas,
    value: &mut f64,
    (label, name, bounds): (&str, &str, Bounds),
    fallback: impl FnOnce(&mut Form, &mut f64) -> bool,
) -> bool {
    if let Some((changed, v)) = fx.show_named_within(f, label, name, *value, bounds) {
        *value = v;
        return changed;
    }
    fallback(f, value)
}

/// A field `fx` binds when it can, else `kind`'s; `Some(value)` when set.
fn bound_or(
    f: &mut Form,
    fx: &mut Formulas,
    (label, name): (&str, &str),
    value: f64,
    (kind, speed): (Kind, f64),
) -> Option<f64> {
    match fx.show(f, (label, name), value) {
        Some((true, v)) => Some(v),
        Some((false, _)) => None,
        None => number(f, label, value, kind, speed),
    }
}

fn set_f32(slot: &mut f32, value: Option<f64>) -> bool {
    match value {
        Some(v) => {
            *slot = v as f32;
            true
        }
        None => false,
    }
}

fn mm_drag(f: &mut Form, fx: &mut Formulas, value: &mut f32, field: (&str, &str)) -> bool {
    let set = bound_or(f, fx, field, f64::from(*value), (Kind::Mm, 0.5));
    set_f32(value, set)
}

/// A length above zero whose drag stops at `most`, the largest size the
/// feature builds at when that is known; typing may go past it.
fn mm_drag_upto(
    f: &mut Form,
    fx: &mut Formulas,
    value: &mut f32,
    field: (&str, &str),
    most: Option<f32>,
) -> bool {
    let most = most.map(f64::from);
    let bounds = Bounds {
        drag_max: most,
        ..Bounds::default()
    };
    let set = match fx.show_within(f, field, f64::from(*value), bounds) {
        Some((true, v)) => Some(v),
        Some((false, _)) => None,
        None => {
            let mut q = quantity(field.0, f64::from(*value), Kind::Mm, 0.5);
            q.drag_max = most;
            f.quantity(q)
        }
    };
    set_f32(value, set)
}

/// A length that may be zero or negative: a shift either way.
fn offset_drag(f: &mut Form, fx: &mut Formulas, value: &mut f32, field: (&str, &str)) -> bool {
    let set = bound_or(f, fx, field, f64::from(*value), (Kind::Offset, 0.5));
    set_f32(value, set)
}

/// A length that may be negative, its formula under `name` when it has one.
fn signed_mm(f: &mut Form, fx: &mut Formulas, value: &mut f32, label: &str, name: &str) -> bool {
    let set = match fx.show_named(f, label, name, f64::from(*value)) {
        Some((true, v)) => Some(v),
        Some((false, _)) => None,
        None => number(f, label, f64::from(*value), Kind::Offset, 0.1),
    };
    set_f32(value, set)
}

fn deg_drag(
    f: &mut Form,
    fx: &mut Formulas,
    value: &mut f32,
    (label, name): (&str, &str),
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    let bounds = Bounds::within(f64::from(*range.start()), f64::from(*range.end()));
    let set = match fx.show_within(f, (label, name), f64::from(*value), bounds) {
        Some((true, v)) => Some(v),
        Some((false, _)) => None,
        None => {
            let mut q = quantity(label, f64::from(*value), Kind::Degrees, 1.0);
            (q.min, q.max) = (
                Some(f64::from(*range.start())),
                Some(f64::from(*range.end())),
            );
            f.quantity(q)
        }
    };
    set_f32(
        value,
        set.map(|v| v.clamp(f64::from(*range.start()), f64::from(*range.end()))),
    )
}

fn count_drag(f: &mut Form, fx: &mut Formulas, value: &mut u32, field: (&str, &str)) -> bool {
    let label = field.0;
    let set = match fx.show_within(f, field, f64::from(*value), Bounds::within(1.0, 1000.0)) {
        Some((true, v)) => Some(v),
        Some((false, _)) => None,
        None => {
            let mut q = quantity(label, f64::from(*value), Kind::Plain, 0.1);
            (q.min, q.max, q.decimals) = (Some(1.0), Some(1000.0), 0);
            f.quantity(q)
        }
    };
    match set {
        Some(v) => {
            *value = v.round().clamp(1.0, 1000.0) as u32;
            true
        }
        None => false,
    }
}

/// A sketch of `body` picked under `label`, `current` shown.
fn sketch_combo(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    this: FeatureId,
    current: Option<FeatureId>,
    label: &str,
) -> Option<FeatureId> {
    let sketches = crate::build::sketch_choices(ctx.document, body, this, current);
    let at = current.and_then(|id| sketches.iter().position(|(sid, _)| *sid == id));
    let shown = at.map_or_else(|| "(pick)".to_string(), |i| sketches[i].1.clone());
    let names = sketches.iter().map(|(_, n)| n.clone()).collect();
    choice_shown(f, label, shown, names, at).map(|i| sketches[i].0)
}

/// The end conditions that name what they stop on, for `body`: each face
/// it borrows, its own planes, and its datum planes (a coordinate system
/// by each of its planes).
fn end_targets(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Vec<(ExtrudeMode, String)> {
    use crate::feature::PlaneTarget;
    use core_document::{BasePlane, DatumShape};
    let mut out: Vec<(ExtrudeMode, String)> = crate::borrow::faces_of_body(ctx.document, body)
        .into_iter()
        .map(|(r, name)| (ExtrudeMode::UpToBorrowed(r), format!("Up to {name}")))
        .collect();
    for plane in BasePlane::ALL {
        out.push((
            ExtrudeMode::UpToPlane(PlaneTarget::Base(plane)),
            format!("Up to the {} plane", plane.label()),
        ));
    }
    for (datum, name, made) in core_document::datums_of_body(ctx.document, body) {
        match made.shape {
            DatumShape::Plane { .. } => out.push((
                ExtrudeMode::UpToPlane(PlaneTarget::Datum { datum, plane: None }),
                format!("Up to {name}"),
            )),
            DatumShape::CoordinateSystem { .. } => {
                for plane in BasePlane::ALL {
                    let key = match plane {
                        BasePlane::XY => "XY",
                        BasePlane::XZ => "XZ",
                        BasePlane::YZ => "YZ",
                    };
                    out.push((
                        ExtrudeMode::UpToPlane(PlaneTarget::Datum {
                            datum,
                            plane: Some(plane),
                        }),
                        format!("Up to {name} {key}"),
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

/// "Use selected face" picker row. Shows the current pick and captures the
/// viewport's selected face on click.
fn face_pick_row(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    pick: &mut Option<FacePick>,
    label: &str,
) -> bool {
    let mut changed = false;
    // The button goes under the point when the row is narrow.
    f.flow(clean(label), true, |f| {
        match pick {
            Some(p) => mono_value(f, &coords(p.point), true),
            None => mono_value(f, "(none)", false),
        }
        let has_selection = ctx.selected_face.is_some();
        if use_button(
            f,
            "Use selected face",
            has_selection,
            "Click a face in the viewport first, then press this",
        ) && let Some(face) = picked_face(ctx)
        {
            *pick = Some(FacePick::of(face));
            changed = true;
        }
    });
    changed
}

fn face_list_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    faces: &mut Vec<FacePick>,
    label: &str,
) -> bool {
    let mut changed = false;
    label_row(f, label);
    let mut remove = None;
    for (i, face) in faces.iter().enumerate() {
        if picked_row(f, "tree-plane", "Face", i, face.point) {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        faces.remove(i);
        changed = true;
    }
    // Every face picked (Ctrl adds more) that the list does not hold yet.
    let fresh: Vec<FacePick> = picked_faces(ctx)
        .into_iter()
        .map(FacePick::of)
        .filter(|pick| !faces.iter().any(|f| f.same_place(pick)))
        .collect();
    let label = match fresh.len() {
        0 | 1 => "Add selected face".to_string(),
        n => format!("Add the {n} selected faces"),
    };
    if use_button(
        f,
        &label,
        !fresh.is_empty(),
        "Click faces in the viewport first (Ctrl adds more), then press this",
    ) {
        faces.extend(fresh);
        changed = true;
    }
    changed
}

/// One picked face or edge as a list row: its kind and number, where it
/// was picked, and a button taking it out. Whether that was pressed.
fn picked_row(f: &mut Form, icon: &str, kind: &str, index: usize, point: [f32; 3]) -> bool {
    f.removable(
        Item {
            icon: Some(icon.into()),
            // The colour of the controls at rest, as a picked row's is.
            color: Some(rgb(ui_kit::tokens::ICON)),
            label: format!("{kind} {}", index + 1),
            secondary: Some(format!(
                "at {:.1}, {:.1}, {:.1}",
                point[0], point[1], point[2]
            )),
            ..Default::default()
        },
        "Take it out",
    )
}

/// A picked edge, shown by where it was picked, and a button that takes
/// the edge picked in the viewport.
fn edge_pick_row(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    pick: &mut Option<EdgePick>,
    label: &str,
) -> bool {
    let mut changed = false;
    f.flow(clean(label), true, |f| {
        match pick {
            Some(p) => mono_value(f, &coords(p.point), true),
            None => mono_value(f, "(none)", false),
        }
        let has_selection = !ctx.selected_edges.is_empty();
        if use_button(
            f,
            "Use selected edge",
            has_selection,
            "Click a straight edge in the viewport first, then press this",
        ) && let Some(edge) = picked_edges(ctx).first()
        {
            *pick = Some(EdgePick::of(edge));
            changed = true;
        }
    });
    changed
}

/// The regions of the profile sketch a feature takes, a check each, shown
/// when the sketch's curves close more than one. With none chosen the
/// checks show the regions the whole profile takes; choosing those again
/// goes back to taking the whole profile.
fn regions_rows(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    sketch: FeatureId,
    chosen: &mut Vec<String>,
) -> bool {
    let regions = crate::build::sketch_regions(ctx.document, sketch);
    if regions.len() < 2 && chosen.is_empty() {
        return false;
    }
    let mut changed = false;
    let taken: Vec<bool> = regions
        .iter()
        .map(|r| {
            if chosen.is_empty() {
                r.default
            } else {
                chosen.contains(&r.name)
            }
        })
        .collect();
    label_row(f, "Regions:");
    for (index, region) in regions.iter().enumerate() {
        let mut on = taken[index];
        let label = format!(
            "Region {} · {:.1} mm² at ({:.1}, {:.1})",
            index + 1,
            region.area,
            region.inside[0],
            region.inside[1]
        );
        if !f.check(&mut on, &label, "") {
            continue;
        }
        let mut now = taken.clone();
        now[index] = on;
        // Nothing taken is no profile: the last region stays.
        if !now.contains(&true) {
            continue;
        }
        let defaults: Vec<bool> = regions.iter().map(|r| r.default).collect();
        *chosen = if now == defaults {
            Vec::new()
        } else {
            regions
                .iter()
                .zip(&now)
                .filter(|(_, on)| **on)
                .map(|(r, _)| r.name.clone())
                .collect()
        };
        changed = true;
    }
    let gone = chosen
        .iter()
        .filter(|name| !regions.iter().any(|r| &r.name == *name))
        .count();
    if gone > 0 {
        taper_note(
            f,
            &format!("{gone} chosen region(s) are no longer in the sketch."),
        );
        if small_button(f, "Take the whole profile", "") {
            chosen.clear();
            changed = true;
        }
    }
    changed
}

/// A plain number, a formula field where the feature has its parameter.
fn number_drag(f: &mut Form, fx: &mut Formulas, value: &mut f32, field: (&str, &str)) -> bool {
    let set = bound_or(f, fx, field, f64::from(*value), (Kind::Plain, 0.05));
    set_f32(value, set)
}

/// The planes of `body`'s datums and sketches, and the planes through
/// each sketch's axes, named.
fn plane_references(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Vec<(MirrorPlane, String)> {
    use crate::feature::SketchAxis;
    use core_document::{BasePlane, DatumShape};
    let mut out = Vec::new();
    for (datum, name, made) in core_document::datums_of_body(ctx.document, body) {
        match made.shape {
            DatumShape::Plane { .. } => out.push((MirrorPlane::Datum { datum, plane: None }, name)),
            DatumShape::CoordinateSystem { .. } => {
                for (plane, key) in [
                    (BasePlane::XY, "XY"),
                    (BasePlane::XZ, "XZ"),
                    (BasePlane::YZ, "YZ"),
                ] {
                    out.push((
                        MirrorPlane::Datum {
                            datum,
                            plane: Some(plane),
                        },
                        format!("{name} {key}"),
                    ));
                }
            }
            _ => {}
        }
    }
    let mut sketches: Vec<(u64, FeatureId, String)> = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "wb.sketch" && n.body == Some(body))
        .map(|(id, n)| (n.seq, *id, n.name.clone()))
        .collect();
    sketches.sort_by_key(|(seq, ..)| *seq);
    for (_, sketch, name) in sketches {
        for (axis, what) in [
            (None, "plane"),
            (Some(SketchAxis::Horizontal), "H axis"),
            (Some(SketchAxis::Vertical), "V axis"),
        ] {
            out.push((
                MirrorPlane::Sketch { sketch, axis },
                format!("{name} {what}"),
            ));
        }
    }
    out
}

/// A plane picked from the base planes, a picked face and `references`.
fn plane_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    plane: &mut MirrorPlane,
    references: &[(MirrorPlane, String)],
) -> bool {
    let mut changed = false;
    let shown = references
        .iter()
        .find(|(c, _)| c == plane)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| plane.label().to_string());
    let base = MirrorPlane::BASE.len();
    let mut options: Vec<String> = MirrorPlane::BASE
        .iter()
        .map(|p| p.label().to_string())
        .collect();
    options.push("Picked face".to_string());
    options.extend(references.iter().map(|(_, n)| n.clone()));
    let at = if matches!(plane, MirrorPlane::Face(_)) {
        Some(base)
    } else {
        MirrorPlane::BASE
            .iter()
            .position(|p| p == plane)
            .or_else(|| {
                references
                    .iter()
                    .position(|(c, _)| c == plane)
                    .map(|i| base + 1 + i)
            })
    };
    match choice_shown(f, "Plane", shown, options, at) {
        Some(i) if i < base => {
            *plane = MirrorPlane::BASE[i];
            changed = true;
        }
        Some(i) if i == base => {
            if let Some(face) = picked_face(ctx) {
                *plane = MirrorPlane::Face(FacePick::of(face));
                changed = true;
            }
        }
        Some(i) => {
            *plane = references[i - base - 1].0;
            changed = true;
        }
        None => {}
    }
    if let MirrorPlane::Face(pick) = plane {
        let mut opt = Some(*pick);
        if face_pick_row(f, ctx, &mut opt, "Face:")
            && let Some(new_pick) = opt
        {
            *pick = new_pick;
            changed = true;
        }
    }
    changed
}

/// Settings editor for a datum feature. Returns true when the payload
/// changed.
pub fn datum_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    feature_id: FeatureId,
    datum: &mut core_document::DatumFeature,
) -> bool {
    use core_document::DatumShape;
    let mut changed = false;

    match &mut datum.shape {
        DatumShape::Plane { size } => changed |= mm_drag(f, fx, size, ("Display size:", "")),
        DatumShape::Line { length } => changed |= mm_drag(f, fx, length, ("Display length:", "")),
        DatumShape::CoordinateSystem { size } => {
            changed |= mm_drag(f, fx, size, ("Display size:", ""))
        }
        DatumShape::Point => {}
    }

    changed |= crate::datum_panel::attachment_editor(f, ctx, feature_id, datum);

    // One row each: side by side they are wider than the panel.
    let [x, y, n] = &mut datum.offset.translation;
    changed |= offset_drag(f, fx, x, ("Offset X:", "offset_x"));
    changed |= offset_drag(f, fx, y, ("Offset Y:", "offset_y"));
    changed |= offset_drag(f, fx, n, ("Normal offset:", "offset_z"));
    changed |= deg_drag(
        f,
        fx,
        &mut datum.offset.rotation_deg,
        ("Rotation:", "rotation"),
        -180.0..=180.0,
    );
    let [about_x, about_y] = &mut datum.offset.tilt;
    changed |= deg_drag(f, fx, about_x, ("Tilt about X:", "tilt_x"), -180.0..=180.0);
    changed |= deg_drag(f, fx, about_y, ("Tilt about Y:", "tilt_y"), -180.0..=180.0);
    changed |= f.check(&mut datum.offset.flip, "Flip side", "");
    changed
}

/// The full settings editor for one feature. Returns true when the payload
/// changed and needs a rebuild.
pub fn feature_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
) -> bool {
    let mut changed = false;
    // No feature before this one: nothing yet for a mode that needs
    // material to cut through or stop at.
    let first_feature = crate::build::design_features_of_body(ctx.document, body)
        .iter()
        .take_while(|(id, _)| *id != feature_id)
        .next()
        .is_none();
    match feature {
        DesignFeature::Base {} => {
            changed |= bodies::body_editor(f, ctx, fx, body, feature_id, feature)
        }
        DesignFeature::Pad { .. } | DesignFeature::Pocket { .. } => {
            changed |= extrude::extrude_editor(f, ctx, fx, body, feature, first_feature);
        }
        DesignFeature::Revolution { .. }
        | DesignFeature::Groove { .. }
        | DesignFeature::Loft { .. }
        | DesignFeature::Pipe { .. }
        | DesignFeature::Helix { .. } => {
            changed |= sweeps::sweep_editor(f, ctx, fx, body, feature_id, feature, first_feature);
        }
        DesignFeature::Primitive { .. } => {
            changed |= primitive::primitive_feature_editor(f, ctx, fx, feature_id, feature);
        }
        DesignFeature::Hole { .. } => {
            changed |= hole::hole_editor(f, ctx, fx, body, feature_id, feature);
        }
        DesignFeature::Fillet { .. }
        | DesignFeature::Chamfer { .. }
        | DesignFeature::Draft { .. }
        | DesignFeature::OffsetFaces { .. }
        | DesignFeature::OffsetSolid { .. }
        | DesignFeature::Scale { .. }
        | DesignFeature::MoveFaces { .. } => {
            changed |= dressup::dressup_editor(f, ctx, fx, body, feature_id, feature);
        }
        DesignFeature::Project { .. } | DesignFeature::Text { .. } => {
            changed |= project::project_editor(f, ctx, fx, feature);
        }
        DesignFeature::DeleteFaces { .. } | DesignFeature::Thickness { .. } => {
            changed |= dressup::dressup_editor(f, ctx, fx, body, feature_id, feature);
        }
        DesignFeature::Mirrored { .. }
        | DesignFeature::LinearPattern { .. }
        | DesignFeature::PolarPattern { .. }
        | DesignFeature::CircularPattern { .. }
        | DesignFeature::PointPattern { .. }
        | DesignFeature::PathPattern { .. }
        | DesignFeature::MultiTransform { .. } => {
            changed |= transforms::transform_editor(f, ctx, fx, body, feature_id, feature);
        }
        DesignFeature::Clone { .. }
        | DesignFeature::Split { .. }
        | DesignFeature::Piece { .. }
        | DesignFeature::Borrow { .. }
        | DesignFeature::BodyBoolean { .. } => {
            changed |= bodies::body_editor(f, ctx, fx, body, feature_id, feature);
        }
    }
    // A cutting feature can keep what it shares with the body instead.
    if let Some(keep_inside) = feature.keep_inside_mut() {
        changed |= f.check(
            keep_inside,
            "Keep what it shares",
            "Keep only the material the feature and the body share, cutting the rest away",
        );
    }
    // Every feature that fuses or cuts can merge the coplanar faces it
    // leaves; new ones take the preference, this switch changes one.
    if feature.can_refine() {
        let mut refine = feature.refine();
        if f.check(&mut refine, "Refine result", "") {
            feature.set_refine(refine);
            changed = true;
        }
        changed |= advanced_editor(f, fx, feature);
    }
    changed
}

/// The Advanced group of a feature that fuses or cuts: the tolerance its
/// boolean runs at.
fn advanced_editor(f: &mut Form, fx: &mut Formulas, feature: &mut DesignFeature) -> bool {
    let summary = match feature.boolean_tolerance() {
        Some(t) => format!("tolerance {t} mm"),
        None => "automatic tolerance".to_string(),
    };
    let open = feature.boolean_tolerance().is_some();
    f.section("Advanced", &summary, open, |f| {
        let changed = tolerance_editor(f, fx, feature);
        taper_note(
            f,
            "Faces this close to touching are joined by the fuse or cut, closing \
             a gap or a sliver between them. Empty leaves it automatic.",
        );
        changed
    })
}

/// The boolean tolerance: a length while one is stated, with a button
/// putting it back to automatic; else an empty field, a length typed
/// into it stating one.
fn tolerance_editor(f: &mut Form, fx: &mut Formulas, feature: &mut DesignFeature) -> bool {
    const LABEL: &str = "Tolerance";
    const NAME: &str = "boolean_tolerance";
    if let Some(stated) = feature.boolean_tolerance() {
        let mut changed = false;
        let set = match fx.show_named(f, LABEL, NAME, f64::from(stated)) {
            Some((true, v)) => Some(v),
            Some((false, _)) => None,
            None => {
                let mut q = quantity(LABEL, f64::from(stated), Kind::Mm, 0.001);
                q.decimals = 3;
                f.quantity(q)
            }
        };
        if let Some(value) = set {
            feature.set_boolean_tolerance(Some(value as f32));
            changed = true;
        }
        let automatic = f.flow(LABEL_BLANK, false, |f| small_button(f, "Automatic", ""));
        if automatic {
            if let Some(key) = fx.named(NAME).map(|p| p.key) {
                fx.edits.push((key, None));
            }
            feature.set_boolean_tolerance(None);
            changed = true;
        }
        return changed;
    }
    let typed = f.entry(
        LABEL,
        bench_api::Entry {
            label: LABEL.into(),
            placeholder: "Automatic".into(),
            width: Some(120.0),
            ..Default::default()
        },
    );
    let stated = match typed {
        Some(bench_api::PanelEvent::Text { value, .. }) => parse_tolerance(value),
        _ => None,
    };
    if stated.is_some() {
        feature.set_boolean_tolerance(stated);
        return true;
    }
    false
}

/// A tolerance as typed: a positive length in mm, the unit optional.
fn parse_tolerance(text: &str) -> Option<f32> {
    text.trim()
        .trim_end_matches("mm")
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|t| t.is_finite() && *t > 0.0)
}

/// The body of the feature the task panel edits.
fn edited_body(ctx: &WorkbenchRuntimeContext) -> Option<core_document::BodyId> {
    ctx.active_document_object
        .and_then(|id| ctx.document.get_feature_meta(id))
        .and_then(|node| node.body)
}

/// The picked face in the edited feature's body frame, where the feature
/// keeps its references.
fn picked_face(ctx: &WorkbenchRuntimeContext) -> Option<core_document::FaceRef> {
    match edited_body(ctx) {
        Some(body) => ctx.selected_face_in(body),
        None => ctx.selected_face,
    }
}

/// Every picked face in the edited feature's body frame, in the order
/// picked.
fn picked_faces(ctx: &WorkbenchRuntimeContext) -> Vec<core_document::FaceRef> {
    match edited_body(ctx) {
        Some(body) => ctx.selected_faces_in(body),
        None => ctx.picked_faces(),
    }
}

/// A point picked in the viewport, in the edited feature's body frame: a
/// circular edge's centre, else where an edge was picked, else a face.
fn picked_point(ctx: &WorkbenchRuntimeContext) -> Option<[f32; 3]> {
    let edges = picked_edges(ctx);
    edges
        .first()
        .map(|e| e.circle.map_or(e.point, |c| c.center))
        .or_else(|| picked_face(ctx).map(|f| f.point))
}

/// The picked edges in the edited feature's body frame.
fn picked_edges(ctx: &WorkbenchRuntimeContext) -> Vec<core_document::EdgeRef> {
    match edited_body(ctx) {
        Some(body) => ctx.selected_edges_in(body),
        None => ctx.selected_edges.clone(),
    }
}

#[cfg(test)]
mod panel_width {
    use super::*;
    use core_document::{Document, WorkbenchFeature};
    use serde_json::json;

    const WIDTH: f32 = 300.0;

    /// How wide `declare`'s widgets lay out in a column `WIDTH` wide.
    fn used_width(
        doc: &mut Document,
        feature: FeatureId,
        declare: &dyn Fn(&mut Form, &WorkbenchRuntimeContext, &mut Formulas),
    ) -> f32 {
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        let wctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        let shown: &WorkbenchRuntimeContext = &wctx;
        let mut f = Form::declare();
        let mut fx = Formulas::of(shown.document, feature);
        declare(&mut f, shown, &mut fx);
        let widgets = f.finish();
        let widgets = [bench_api::Fields {
            children: widgets,
            label_width: Some(LABEL_COLUMN),
        }
        .into()];
        let mut width = 0.0;
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut column = ui.new_child(egui::UiBuilder::new().max_rect(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(WIDTH, 3000.0)),
                ));
                bench_panel::show(
                    &mut column,
                    egui::Id::new("width"),
                    &widgets,
                    Some(shown.document),
                );
                width = column.min_rect().width();
            });
            output.textures_delta.clear();
        }
        width
    }

    #[test]
    fn every_editor_fits_a_narrow_task_panel() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let sketch = doc
            .add_feature_in_body(
                wb_sketch::SketchFeature::from_sketch(wb_sketch::sketch::Sketch::new("s")),
                "Sketch".into(),
                Some(body),
            )
            .unwrap();
        let s = sketch.0.to_string();
        let features = [
            json!({"Pad": {"sketch": s, "length": 10.0, "reversed": false}}),
            json!({"Pocket": {"sketch": s, "depth": 5.0, "reversed": false}}),
            json!({"Pad": {"sketch": s, "length": 10.0, "reversed": false, "mode": "UpToShape",
                "mode2": "UpToFace", "direction": {"Custom": [0.0, 1.0, 1.0]},
                "up_to_shape": [{"point": [1.0, 2.0, 3.0], "normal": [0.0, 0.0, 1.0]}]}}),
            json!({"Pocket": {"sketch": null, "depth": 5.0, "reversed": false, "mode": "TwoLengths",
                "mode2": "UpToShape",
                "profile_face": {"point": [100.0, 200.0, 300.0], "normal": [0.0, 0.0, 1.0]},
                "direction": {"Edge": {"point": [100.0, 200.0, 300.0], "direction": [0.0, 0.0, 1.0]}}}}),
            json!({"Revolution": {"sketch": s, "angle_deg": 360.0, "mode": "UpToFace",
                "axis": {"Edge": {"point": [100.0, 200.0, 300.0], "direction": [0.0, 1.0, 0.0]}}}}),
            json!({"Hole": {"sketch": s, "diameter": 5.0, "depth": 8.0, "through_all": false,
                "cut": {"Counterbore": {"diameter": 9.0, "depth": 2.0}}}}),
            json!({"Chamfer": {"size": 1.0}}),
            json!({"LinearPattern": {"originals": [], "axis": "X", "length": 20.0, "occurrences": 3}}),
            json!({"PolarPattern": {"originals": [], "axis": "Z", "angle_deg": 360.0, "occurrences": 6}}),
            json!({"MultiTransform": {"originals": [], "steps": [
                {"Linear": {"axis": "X", "length": 20.0, "occurrences": 3}}]}}),
            json!({"Revolution": {"sketch": s, "angle_deg": 360.0}}),
            json!({"Groove": {"sketch": s, "angle_deg": 360.0}}),
            json!({"Helix": {"sketch": s, "axis": "SketchY", "mode": "PitchHeight", "pitch": 2.0,
                "height": 10.0, "turns": 5.0, "left_handed": false, "cone_angle_deg": 0.0,
                "reversed": false, "subtractive": false}}),
            json!({"Loft": {"sections": [s], "ruled": false, "closed": false, "subtractive": false}}),
            json!({"Pipe": {"profile": s, "spine": s, "frenet": false, "subtractive": false}}),
            json!({"Pipe": {"profile": s, "spine": s, "subtractive": false,
                "orientation": {"Binormal": {"x": 0.0, "y": 0.0, "z": 1.0}},
                "corner": "Round", "sections": [s]}}),
            json!({"Pipe": {"profile": s, "spine": s, "subtractive": false,
                "orientation": {"Auxiliary": {"path": s}}}}),
            json!({"Helix": {"sketch": s, "axis": "SketchY", "mode": "HeightTurnsGrowth",
                "pitch": 2.0, "height": 0.0, "turns": 3.0, "growth": 2.0, "left_handed": false,
                "cone_angle_deg": 0.0, "reversed": false, "subtractive": true,
                "keep_inside": true}}),
            json!({"Fillet": {"radius": 1.0}}),
            json!({"Chamfer": {"size": 1.0, "mode": "DistanceAngle"}}),
            json!({"Draft": {"angle_deg": 3.0, "neutral": {"point": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0]}, "faces": []}}),
            json!({"Thickness": {"value": 1.0, "faces": []}}),
            json!({"Mirrored": {"originals": [], "plane": "XY"}}),
            json!({"MultiTransform": {"originals": [], "steps": [
                {"Scale": {"factor": 2.0, "center": [0.0, 0.0, 0.0], "occurrences": 2}},
                {"Mirror": {"plane": "XZ"}}]}}),
            json!({"Borrow": {"source": {"Sketch": s}}}),
            json!({"Borrow": {"source": {"Solid": {"body": body.0.to_string(),
                "faces": [{"point": [100.0, 200.0, 300.0], "normal": [0.0, 0.0, 1.0]}],
                "edges": [{"point": [100.0, 200.0, 300.0], "direction": [0.0, 0.0, 1.0]}]}}}}),
            json!({"Primitive": {
                "kind": {"Cylinder": {"radius": 5.0, "height": 10.0, "angle_deg": 360.0}},
                "placement": {"origin": [0.0, 0.0, 0.0], "x_axis": [1.0, 0.0, 0.0], "z_axis": [0.0, 0.0, 1.0]},
                "subtractive": false}}),
        ];
        for data in features {
            let feature = DesignFeature::from_json(&data).unwrap();
            let id = doc
                .add_feature_in_body(feature.clone(), "Feature".into(), Some(body))
                .unwrap();
            let width = used_width(&mut doc, id, &|form, ctx, fx| {
                let mut f = feature.clone();
                feature_editor(form, ctx, fx, body, id, &mut f);
            });
            assert!(width <= WIDTH + 0.5, "{data} lays out {width} px wide");
        }
        let datum = core_document::DatumFeature {
            shape: core_document::DatumShape::Plane { size: 30.0 },
            attachment: core_document::DatumAttachment::BasePlane(core_document::BasePlane::XY),
            offset: Default::default(),
        };
        let id = doc
            .add_feature_in_body(datum, "Plane".into(), Some(body))
            .unwrap();
        let width = used_width(&mut doc, id, &|form, ctx, fx| {
            let mut d = datum;
            datum_editor(form, ctx, fx, id, &mut d);
        });
        assert!(
            width <= WIDTH + 0.5,
            "the datum editor lays out {width} px wide"
        );
    }
}
