//! Per-feature parameter editors for the task panel, declared on a
//! [`Form`].
//!
//! Every option maps 1:1 to a feature field. Editors return `true` when
//! the feature payload changed; the task applies edits live. Rows are a
//! fixed-width label column beside a control, the way the design lays out
//! its parameter cards.

use bench_api::{Bind, ButtonStyle, Dim, Item, Quantity, Select, Tone};
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use crate::build::design_features_of_body;
use crate::feature::{
    ChamferMode, DesignFeature, EdgePick, EdgeSel, ExtrudeDirection, ExtrudeMode, FacePick,
    HelixMode, MirrorPlane, PatternAxis, PipeCorner, PipeOrientation, RevolveAxis, RevolveMode,
    SketchAxis, TransformStep,
};
use crate::form::Form;

mod borrow;
mod hole;
mod patterns;

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

/// The occurrences a pattern leaves out, each brought back by its button,
/// all of them by Clear; the view's dots leave one out or bring it back.
fn skipped_editor(f: &mut Form, skipped: &mut Vec<u32>) -> bool {
    let mut changed = false;
    f.flow("Skipped", false, |f| {
        if skipped.is_empty() {
            f.caption("None", Tone::Neutral, false, false);
        } else if small_button(f, "Clear", "Bring every occurrence back") {
            skipped.clear();
            changed = true;
        }
    });
    if !skipped.is_empty() {
        let mut back = None;
        f.flow(LABEL_BLANK, true, |f| {
            for (at, index) in skipped.iter().enumerate() {
                let name = format!("{} ×", crate::instances::occurrence_name(*index));
                if small_button(f, &name, "Bring it back") {
                    back = Some(at);
                }
            }
        });
        if let Some(at) = back {
            skipped.remove(at);
            changed = true;
        }
    }
    f.caption(
        "Click an occurrence's dot in the view to leave it out or bring it back",
        Tone::Neutral,
        false,
        false,
    );
    changed
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
        self.show_upto(f, field, value, None)
    }

    /// As [`Self::show`], a drag stopping at `drag_max`.
    fn show_upto(
        &mut self,
        f: &mut Form,
        (label, name): (&str, &str),
        value: f64,
        drag_max: Option<f64>,
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
        Some(self.show_param(f, label, p, value, drag_max))
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
        let p = self.named(name)?;
        Some(self.show_param(f, label, p, value, None))
    }

    fn show_param(
        &mut self,
        f: &mut Form,
        label: &str,
        p: core_document::Parameter,
        value: f64,
        drag_max: Option<f64>,
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
            drag_max,
            ..Default::default()
        };
        match f.quantity(q) {
            Some(v) => (true, v),
            None => (false, value),
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
    (label, name): (&str, &str),
    fallback: impl FnOnce(&mut Form, &mut f64) -> bool,
) -> bool {
    if let Some((changed, v)) = fx.show_named(f, label, name, *value) {
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

/// A helix's turns, a formula field where it has the parameter.
fn turns_field(f: &mut Form, fx: &mut Formulas, turns: &mut f32) -> bool {
    if let Some((changed, v)) = fx.show(f, ("Turns:", "turns"), f64::from(*turns)) {
        *turns = (v as f32).clamp(0.1, 1000.0);
        return changed;
    }
    let mut q = quantity("Turns", f64::from(*turns), Kind::Plain, 0.1);
    (q.min, q.max) = (Some(0.1), Some(1000.0));
    q.bare = true;
    match f.quantity(q) {
        Some(v) => {
            *turns = v as f32;
            true
        }
        None => false,
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
    let set = match fx.show_upto(f, field, f64::from(*value), most) {
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
    let set = match fx.show(f, (label, name), f64::from(*value)) {
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
    let set = match fx.show(f, field, f64::from(*value)) {
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

fn extrude_mode_combo(
    f: &mut Form,
    mode: &mut ExtrudeMode,
    first_feature: bool,
    borrowed: &[(ExtrudeMode, String)],
) -> bool {
    // Material-relative modes need an earlier solid.
    let offered: Vec<(ExtrudeMode, String)> = mode_choices(&ExtrudeMode::ALL, borrowed)
        .into_iter()
        .filter(|(candidate, _)| !(first_feature && needs_own_material(*candidate, borrowed)))
        .collect();
    let at = offered.iter().position(|(c, _)| c == mode);
    let names = offered.iter().map(|(_, n)| n.clone()).collect();
    match choice_shown(f, "Type", mode_name(*mode, borrowed), names, at) {
        Some(i) => {
            *mode = offered[i].0;
            true
        }
        None => false,
    }
}

/// Whether `mode` ends only on the body's own material: up to shape stops
/// on borrowed faces too, when the body borrows any.
fn needs_own_material(mode: ExtrudeMode, targets: &[(ExtrudeMode, String)]) -> bool {
    let lends_faces = targets
        .iter()
        .any(|(t, _)| matches!(t, ExtrudeMode::UpToBorrowed(_)));
    mode.needs_material() && !(mode == ExtrudeMode::UpToShape && lends_faces)
}

/// The end conditions offered: `modes`, then the ones that name what they
/// stop on (a borrowed face, a plane).
fn mode_choices(
    modes: &[ExtrudeMode],
    targets: &[(ExtrudeMode, String)],
) -> Vec<(ExtrudeMode, String)> {
    modes
        .iter()
        .map(|m| (*m, m.label().to_string()))
        .chain(targets.iter().cloned())
        .collect()
}

/// What an end condition is called: one that names what it stops on by
/// it.
fn mode_name(mode: ExtrudeMode, targets: &[(ExtrudeMode, String)]) -> String {
    targets
        .iter()
        .find(|(c, _)| *c == mode)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| mode.label().to_string())
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

/// What a dress-up rounds or bevels: every edge, the edges of picked
/// faces, picked edges, or (a fillet's, when `between` is set) the round
/// between two picked faces.
fn edge_sel_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    edges: &mut EdgeSel,
    between: bool,
) -> bool {
    let mut changed = false;
    let shown = match edges {
        EdgeSel::All => "All edges".to_string(),
        EdgeSel::Faces(f) => format!("{} face(s)", f.len()),
        EdgeSel::Edges(e) => format!("{} edge(s)", e.len()),
        EdgeSel::BetweenFaces(f) => format!("Between {} of 2 faces", f.len()),
    };
    let mut options = vec![
        "All edges".to_string(),
        "Edges of picked faces".to_string(),
        "Picked edges".to_string(),
    ];
    let mut hints = vec![String::new(); 3];
    if between {
        options.push("Between two faces".to_string());
        hints.push(
            "The round between two faces that need share no edge, \
             such as a step across a groove"
                .to_string(),
        );
    }
    let at = match edges {
        EdgeSel::All => 0,
        EdgeSel::Faces(_) => 1,
        EdgeSel::Edges(_) => 2,
        EdgeSel::BetweenFaces(_) => 3,
    };
    let picked = f.select(
        "Edges",
        Select {
            label: "Edges".into(),
            options,
            selected: at,
            width: Some(CHOICE_WIDTH),
            text: Some(shown),
            hints,
            ..Default::default()
        },
    );
    match picked {
        Some(0) => {
            *edges = EdgeSel::All;
            changed = true;
        }
        Some(1) => {
            *edges = EdgeSel::Faces(faces_of_selection(edges));
            changed = true;
        }
        Some(2) => {
            *edges = EdgeSel::Edges(Vec::new());
            changed = true;
        }
        Some(3) => {
            let mut faces = faces_of_selection(edges);
            faces.truncate(2);
            *edges = EdgeSel::BetweenFaces(faces);
            changed = true;
        }
        _ => {}
    }
    match edges {
        EdgeSel::Faces(faces) => changed |= face_list_editor(f, ctx, faces, "Faces:"),
        EdgeSel::Edges(picks) => changed |= edge_list_editor(f, ctx, picks),
        EdgeSel::BetweenFaces(faces) => {
            changed |= face_list_editor(f, ctx, faces, "Faces:");
            if faces.len() > 2 {
                faces.drain(..faces.len() - 2);
            }
        }
        EdgeSel::All => {}
    }
    changed
}

/// The faces a selection picked, kept when it changes kind.
fn faces_of_selection(edges: &EdgeSel) -> Vec<crate::feature::FacePick> {
    match edges {
        EdgeSel::Faces(faces) | EdgeSel::BetweenFaces(faces) => faces.clone(),
        EdgeSel::All | EdgeSel::Edges(_) => Vec::new(),
    }
}

/// The switch that makes a dress-up take the edges tangent to its own.
fn tangent_row(f: &mut Form, follow_tangent: &mut bool) -> bool {
    f.check(
        follow_tangent,
        "Follow tangent edges",
        "Take every edge that runs on smoothly from a selected one, \
             such as the round of a filleted corner and the side past it",
    )
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

/// The picked edges, each removable, and a button that adds whatever is
/// picked in the viewport.
fn edge_list_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    picks: &mut Vec<EdgePick>,
) -> bool {
    let mut changed = false;
    label_row(f, "Edges:");
    let mut remove = None;
    for (i, pick) in picks.iter().enumerate() {
        if picked_row(f, "line", "Edge", i, pick.point) {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        picks.remove(i);
        changed = true;
    }
    let has_selection = !ctx.selected_edges.is_empty();
    if use_button(
        f,
        "Add selected edges",
        has_selection,
        "Click edges in the viewport first (Ctrl adds), then press this",
    ) {
        for edge in &picked_edges(ctx) {
            let pick = EdgePick::of(edge);
            if !picks.contains(&pick) {
                picks.push(pick);
                changed = true;
            }
        }
    }
    changed
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

/// The datums of `body` (a line along itself, a plane square to it) and the
/// lines of its sketches, as ways an extrusion may run.
fn direction_references(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
) -> Vec<(ExtrudeDirection, String)> {
    let mut out: Vec<(ExtrudeDirection, String)> =
        core_document::datums_of_body(ctx.document, body)
            .into_iter()
            .filter(|(_, _, d)| !matches!(d.shape, core_document::DatumShape::Point))
            .map(|(id, name, _)| (ExtrudeDirection::Datum(id), name))
            .collect();
    let mut sketches: Vec<(u64, FeatureId, String)> = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "wb.sketch" && n.body == Some(body))
        .map(|(id, n)| (n.seq, *id, n.name.clone()))
        .collect();
    sketches.sort_by_key(|(seq, ..)| *seq);
    for (_, sketch, name) in sketches {
        let Some(feature) = ctx.document.feature_values(sketch).and_then(|data| {
            <wb_sketch::SketchFeature as core_document::WorkbenchFeature>::from_json(data).ok()
        }) else {
            continue;
        };
        let mut n = 0;
        for element in &feature.sketch.geometry {
            if let wb_sketch::sketch::GeometryElement::Line(line) = element {
                n += 1;
                out.push((
                    ExtrudeDirection::SketchLine {
                        sketch,
                        element: line.id,
                    },
                    format!("{name} › line {n}"),
                ));
            }
        }
    }
    out
}

/// Which way a pad or pocket runs: the profile's normal, a vector typed
/// in, or a picked edge.
fn extrude_direction_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    direction: &mut ExtrudeDirection,
) -> bool {
    let mut changed = false;
    let borrowed = crate::borrow::edges_of_body(ctx.document, body);
    let shown = match direction {
        ExtrudeDirection::Borrowed(r) => borrowed
            .iter()
            .find(|(c, _)| c == r)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| direction.label().to_string()),
        ExtrudeDirection::Datum(_) | ExtrudeDirection::SketchLine { .. } => {
            direction_references(ctx, body)
                .into_iter()
                .find(|(c, _)| c == direction)
                .map(|(_, name)| name)
                .unwrap_or_else(|| direction.label().to_string())
        }
        _ => direction.label().to_string(),
    };
    let mut candidates = vec![
        (
            ExtrudeDirection::Normal,
            ExtrudeDirection::Normal.label().to_string(),
        ),
        (
            ExtrudeDirection::Custom([0.0, 0.0, 1.0]),
            "Custom vector".to_string(),
        ),
        (
            ExtrudeDirection::Edge(EdgePick {
                faces: [0, 0],
                point: [0.0; 3],
                direction: [0.0, 0.0, 1.0],
            }),
            "Picked edge".to_string(),
        ),
    ];
    candidates.extend(
        borrowed
            .iter()
            .map(|(r, name)| (ExtrudeDirection::Borrowed(*r), name.clone())),
    );
    candidates.extend(
        crate::feature::BaseAxis::ALL
            .into_iter()
            .map(|axis| (ExtrudeDirection::Axis(axis), axis.label().to_string())),
    );
    candidates.extend(direction_references(ctx, body));
    let at = candidates
        .iter()
        .position(|(candidate, _)| match (&*direction, candidate) {
            (ExtrudeDirection::Custom(_), ExtrudeDirection::Custom(_))
            | (ExtrudeDirection::Edge(_), ExtrudeDirection::Edge(_))
            | (ExtrudeDirection::Normal, ExtrudeDirection::Normal) => true,
            _ => direction == candidate,
        });
    let names = candidates.iter().map(|(_, n)| n.clone()).collect();
    if let Some(i) = choice_shown(f, "Direction", shown, names, at) {
        // A picked edge starts from the one picked now.
        *direction = match candidates[i].0 {
            ExtrudeDirection::Edge(_) => match picked_edges(ctx).first() {
                Some(edge) => ExtrudeDirection::Edge(EdgePick::of(edge)),
                None => candidates[i].0,
            },
            other => other,
        };
        changed = true;
    }
    match direction {
        ExtrudeDirection::Normal
        | ExtrudeDirection::Borrowed(_)
        | ExtrudeDirection::Datum(_)
        | ExtrudeDirection::SketchLine { .. }
        | ExtrudeDirection::Axis(_) => {}
        // Each component a number formulas can set: `Pad.direction_x`.
        ExtrudeDirection::Custom(v) => {
            for (c, (label, name)) in v.iter_mut().zip([
                ("Vector X:", "direction_x"),
                ("Vector Y:", "direction_y"),
                ("Vector Z:", "direction_z"),
            ]) {
                let mut value = f64::from(*c);
                let edited = named_f64(f, fx, &mut value, (label, name), |f, value| {
                    match bare(f, clean(label), *value, 0.05, "") {
                        Some(v) => {
                            *value = v;
                            true
                        }
                        None => false,
                    }
                });
                if edited {
                    *c = value as f32;
                    changed = true;
                }
            }
        }
        ExtrudeDirection::Edge(edge) => {
            let mut pick = Some(*edge);
            if edge_pick_row(f, ctx, &mut pick, "Edge:")
                && let Some(new) = pick
            {
                *edge = new;
                changed = true;
            }
        }
    }
    changed
}

/// Where a feature starts: the profile's own plane, a datum plane's,
/// another sketch's or a flat face of the solid, and the offset from there.
fn start_rows(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    fx: &mut Formulas,
    start_at: &mut Option<FeatureId>,
    start_face: &mut Option<FacePick>,
    start_offset: &mut f32,
) -> bool {
    let mut changed = false;
    let mut planes: Vec<(Option<FeatureId>, String)> = vec![(None, "Profile".to_string())];
    planes.extend(
        core_document::datums_of_body(ctx.document, body)
            .into_iter()
            .filter(|(_, _, d)| {
                !matches!(
                    d.shape,
                    core_document::DatumShape::Line { .. } | core_document::DatumShape::Point
                )
            })
            .map(|(id, name, _)| (Some(id), name)),
    );
    planes.extend(
        crate::build::sketches_of_body(ctx.document, body)
            .into_iter()
            .map(|(id, name)| (Some(id), name)),
    );
    let shown = if start_face.is_some() {
        "Face".to_string()
    } else {
        planes
            .iter()
            .find(|(id, _)| id == start_at)
            .map_or_else(|| "(gone)".to_string(), |(_, name)| name.clone())
    };
    // A flat face picked in the view.
    let flat = picked_face(ctx).filter(|f| {
        f.surface
            .is_none_or(|s| matches!(s, kernel_api::FaceSurface::Plane { .. }))
    });
    let at = if start_face.is_some() {
        planes.len()
    } else {
        planes
            .iter()
            .position(|(id, _)| id == start_at)
            .unwrap_or(planes.len() + 1)
    };
    let mut options: Vec<String> = planes.iter().map(|(_, n)| n.clone()).collect();
    options.push("Selected face".to_string());
    let mut hints = vec![String::new(); planes.len()];
    hints.push("Click a flat face in the viewport first".to_string());
    let picked = f.select(
        "Start at",
        Select {
            label: "Start at".into(),
            options,
            selected: at,
            width: Some(CHOICE_WIDTH),
            text: Some(shown),
            disabled: if flat.is_some() {
                Vec::new()
            } else {
                vec![planes.len()]
            },
            hints,
            ..Default::default()
        },
    );
    match picked {
        Some(i) if i < planes.len() => {
            *start_at = planes[i].0;
            *start_face = None;
            changed = true;
        }
        Some(_) => {
            if let Some(face) = &flat {
                *start_face = Some(FacePick::of(face));
                *start_at = None;
                changed = true;
            }
        }
        None => {}
    }
    if start_face.is_some() {
        changed |= face_pick_row(f, ctx, start_face, "Start face:");
    }
    changed |= offset_drag(f, fx, start_offset, ("Start offset:", "start_offset"));
    changed
}

/// A pad's or pocket's less used settings: where it starts, the second
/// side's own taper, and a slanted length measured along the normal.
fn extrude_extras_rows(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    fx: &mut Formulas,
    extras: &mut crate::feature::ExtrudeExtras,
    two_sided: bool,
    direction: ExtrudeDirection,
) -> bool {
    let mut changed = start_rows(
        f,
        ctx,
        body,
        fx,
        &mut extras.start_at,
        &mut extras.start_face,
        &mut extras.start_offset,
    );
    if two_sided {
        let mut own = extras.taper2_deg.is_some();
        if f.check(&mut own, "Second side's own taper", "") {
            extras.taper2_deg = own.then_some(0.0);
            changed = true;
        }
        if let Some(taper) = &mut extras.taper2_deg {
            changed |= deg_drag(f, fx, taper, ("Second taper:", ""), -85.0..=85.0);
        }
    }
    if direction != ExtrudeDirection::Normal {
        changed |= f.check(
            &mut extras.along_normal,
            "Length along the normal",
            "With a slanted direction, the length is measured square to the profile",
        );
    }
    changed
}

/// The rows of one side's end condition, beyond its mode: the length, the
/// face or the faces it stops on and their offset.
#[expect(clippy::too_many_arguments)]
fn extrude_side_rows(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    mode: ExtrudeMode,
    length: (&mut f32, (&str, &str)),
    face: &mut Option<FacePick>,
    offset: (&mut f32, (&str, &str)),
    shape: &mut Vec<FacePick>,
    (body, borrowed): (BodyId, &mut Vec<crate::feature::BorrowedRef>),
) -> bool {
    let mut changed = false;
    match mode {
        ExtrudeMode::Dimension | ExtrudeMode::TwoLengths => {
            changed |= mm_drag(f, fx, length.0, length.1);
        }
        ExtrudeMode::UpToFace => {
            changed |= face_pick_row(f, ctx, face, "Target face:");
            changed |= offset_drag(f, fx, offset.0, offset.1);
        }
        ExtrudeMode::UpToBorrowed(_) | ExtrudeMode::UpToPlane(_) => {
            changed |= offset_drag(f, fx, offset.0, offset.1);
        }
        ExtrudeMode::UpToShape => {
            changed |= face_list_editor(f, ctx, shape, "Stop faces:");
            changed |= borrowed_face_checks(f, ctx, body, borrowed);
            changed |= offset_drag(f, fx, offset.0, offset.1);
        }
        _ => {}
    }
    changed
}

/// A check row for each face `body` borrows: the ones ticked are stop
/// faces too.
fn borrowed_face_checks(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    borrowed: &mut Vec<crate::feature::BorrowedRef>,
) -> bool {
    let faces = crate::borrow::faces_of_body(ctx.document, body);
    if faces.is_empty() {
        return false;
    }
    let mut changed = false;
    label_row(f, "Borrowed stop faces:");
    for (r, name) in faces {
        let mut on = borrowed.contains(&r);
        if f.check(&mut on, &name, "") {
            if on {
                borrowed.push(r);
            } else {
                borrowed.retain(|b| *b != r);
            }
            changed = true;
        }
    }
    changed
}

/// The second side of a two-sided extrusion: none, or how it ends. Two
/// lengths always has one.
fn second_side_combo(
    f: &mut Form,
    first: ExtrudeMode,
    mode2: &mut Option<ExtrudeMode>,
    first_feature: bool,
    borrowed: &[(ExtrudeMode, String)],
) -> bool {
    let required = first == ExtrudeMode::TwoLengths;
    let shown = match *mode2 {
        None if required => ExtrudeMode::Dimension.label().to_string(),
        None => "None".to_string(),
        Some(mode) => mode_name(mode, borrowed),
    };
    let mut offered: Vec<(Option<ExtrudeMode>, String)> = Vec::new();
    if !required {
        offered.push((None, "None".to_string()));
    }
    for (candidate, name) in mode_choices(&ExtrudeMode::SECOND_SIDE, borrowed) {
        if first_feature && needs_own_material(candidate, borrowed) {
            continue;
        }
        offered.push((Some(candidate), name));
    }
    let at = offered.iter().position(|(c, _)| {
        *c == *mode2 || (required && mode2.is_none() && *c == Some(ExtrudeMode::Dimension))
    });
    let names = offered.iter().map(|(_, n)| n.clone()).collect();
    match choice_shown(f, "Second side", shown, names, at) {
        Some(i) => {
            *mode2 = offered[i].0;
            true
        }
        None => false,
    }
}

/// The lines of a sketch and the datum lines of a body, as axis choices.
fn axis_choices(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    sketch: FeatureId,
) -> Vec<(RevolveAxis, String)> {
    let mut choices = Vec::new();
    if let Some(sketch) = ctx.document.feature_values(sketch).and_then(|data| {
        <wb_sketch::SketchFeature as core_document::WorkbenchFeature>::from_json(data).ok()
    }) {
        let mut n = 0;
        for element in &sketch.sketch.geometry {
            if let wb_sketch::sketch::GeometryElement::Line(line) = element {
                n += 1;
                let kind = if sketch.sketch.is_construction(line.id) {
                    "construction line"
                } else {
                    "line"
                };
                choices.push((
                    RevolveAxis::SketchLine(line.id),
                    format!("Sketch {kind} {n}"),
                ));
            }
        }
    }
    for (id, name, datum) in core_document::datums_of_body(ctx.document, body) {
        if matches!(datum.shape, core_document::DatumShape::Line { .. }) {
            choices.push((RevolveAxis::Datum(id), name));
        }
    }
    for (r, name) in crate::borrow::edges_of_body(ctx.document, body) {
        choices.push((RevolveAxis::Borrowed(r), name));
    }
    choices
}

/// What a revolution or helix spins about: the sketch's axes, a custom
/// axis, a line of the sketch, a datum line or a picked edge.
fn revolve_axis_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    sketch: FeatureId,
    axis: &mut RevolveAxis,
    helix: bool,
) -> bool {
    let mut changed = false;
    let references = axis_choices(ctx, body, sketch);
    let shown = references
        .iter()
        .find(|(choice, _)| choice == axis)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| axis.label().to_string());
    let picked_edge = picked_edges(ctx).first().map(EdgePick::of);
    let mut candidates = vec![
        (
            RevolveAxis::SketchY,
            RevolveAxis::SketchY.label().to_string(),
        ),
        (
            RevolveAxis::SketchX,
            RevolveAxis::SketchX.label().to_string(),
        ),
        (
            RevolveAxis::Custom {
                origin: [0.0, 0.0],
                dir: [1.0, 1.0],
            },
            "Custom axis".to_string(),
        ),
    ];
    candidates.extend(
        crate::feature::BaseAxis::ALL
            .into_iter()
            .map(|a| (RevolveAxis::Base(a), a.label().to_string())),
    );
    if helix {
        candidates.push((
            RevolveAxis::SketchNormal,
            RevolveAxis::SketchNormal.label().to_string(),
        ));
    }
    candidates.extend(references.iter().cloned());
    candidates.push((
        RevolveAxis::Edge(picked_edge.unwrap_or(EdgePick {
            faces: [0, 0],
            point: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        })),
        "Picked edge".to_string(),
    ));
    let at = candidates
        .iter()
        .position(|(candidate, _)| match (&*axis, candidate) {
            (RevolveAxis::SketchLine(_), RevolveAxis::SketchLine(_))
            | (RevolveAxis::Datum(_), RevolveAxis::Datum(_))
            | (RevolveAxis::Borrowed(_), RevolveAxis::Borrowed(_))
            | (RevolveAxis::Base(_), RevolveAxis::Base(_)) => axis == candidate,
            _ => std::mem::discriminant(axis) == std::mem::discriminant(candidate),
        });
    let names = candidates.iter().map(|(_, n)| n.clone()).collect();
    if let Some(i) = choice_shown(f, "Axis", shown, names, at) {
        *axis = candidates[i].0;
        changed = true;
    }
    if let RevolveAxis::Edge(edge) = axis {
        let mut pick = Some(*edge);
        if edge_pick_row(f, ctx, &mut pick, "Edge:")
            && let Some(new) = pick
        {
            *edge = new;
            changed = true;
        }
    }
    if let RevolveAxis::Custom { origin, dir } = axis {
        f.flow("", false, |f| {
            f.flow("Origin", false, |f| {
                for v in origin.iter_mut() {
                    changed |= set_f32(v, bare(f, "", f64::from(*v), 0.5, ""));
                }
            });
            f.flow("Dir", false, |f| {
                for v in dir.iter_mut() {
                    changed |= set_f32(v, bare(f, "", f64::from(*v), 0.1, ""));
                }
            });
        });
    }
    changed
}

/// The axis taken as its shadow on the sketch plane, offered for the axes
/// from outside the sketch, which can stand tilted to it.
fn project_axis_row(f: &mut Form, axis: &RevolveAxis, project_axis: &mut bool) -> bool {
    matches!(
        axis,
        RevolveAxis::Edge(_)
            | RevolveAxis::Datum(_)
            | RevolveAxis::Borrowed(_)
            | RevolveAxis::Base(_)
    ) && f.check(
        project_axis,
        "Project onto the sketch",
        "An axis tilted to the sketch plane turns about its shadow on the plane",
    )
}

/// A pad's or a pocket's face profile, when it has one: where it is, and a
/// flat face picked in the view to take its place.
fn face_profile_row(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    profile_face: &mut Option<FacePick>,
) -> bool {
    let Some(face) = profile_face else {
        return false;
    };
    f.flow("Profile", true, |f| {
        mono_value(f, &format!("face at {}", coords(face.point)), true);
    });
    // Its own line: beside the face's position it runs past a narrow
    // panel.
    let picked = ctx.selected_face_in(body).filter(|f| {
        matches!(
            f.surface,
            None | Some(kernel_api::FaceSurface::Plane { .. })
        )
    });
    let clicked = use_button(
        f,
        "Use selected face",
        picked.is_some(),
        "Click another flat face of the solid first, then press this",
    );
    match picked {
        Some(pick) if clicked => {
            *face = FacePick::of(pick);
            true
        }
        _ => false,
    }
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

/// What a pipe sweeps and along what: its profile a sketch or a picked
/// face, its path a sketch (all its curves or those ticked), picked edges
/// of the solid or edges another body lends.
fn pipe_inputs_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    (body, feature_id): (BodyId, FeatureId),
    spine: &mut FeatureId,
    profile_face: &mut Option<FacePick>,
    (path_edges, path_borrowed, path_curves): (
        &mut Vec<EdgePick>,
        &mut Vec<crate::feature::BorrowedRef>,
        &mut Vec<uuid::Uuid>,
    ),
) -> bool {
    let mut changed = false;
    f.flow("Profile", true, |f| {
        let shown = match profile_face {
            Some(p) => format!("face at {}", coords(p.point)),
            None => "its sketch".to_string(),
        };
        mono_value(f, &shown, true);
        let picked = ctx.selected_face_in(body).map(FacePick::of);
        if use_button(
            f,
            "Use selected face",
            picked.is_some(),
            "Click a flat face of the solid first, then press this",
        ) && let Some(pick) = picked
        {
            *profile_face = Some(pick);
            changed = true;
        }
        if profile_face.is_some() && small_button(f, "Use the sketch", "") {
            *profile_face = None;
            changed = true;
        }
    });
    let along_edges = !path_edges.is_empty() || !path_borrowed.is_empty();
    if !along_edges && let Some(new) = sketch_combo(f, ctx, body, feature_id, Some(*spine), "Path:")
    {
        *spine = new;
        path_curves.clear();
        changed = true;
    }
    if !along_edges {
        changed |= path_curves_editor(f, ctx, *spine, path_curves);
    }
    f.flow("Path edges", true, |f| {
        let shown = if !path_edges.is_empty() {
            format!("{} of the solid", path_edges.len())
        } else if !path_borrowed.is_empty() {
            format!("{} borrowed", path_borrowed.len())
        } else {
            "none: the path sketch".to_string()
        };
        mono_value(f, &shown, true);
        let picked: Vec<EdgePick> = picked_edges(ctx).iter().map(EdgePick::of).collect();
        if use_button(
            f,
            "Use selected edges",
            !picked.is_empty(),
            "Click the edges of the solid in turn (Ctrl adds), then press this",
        ) {
            *path_edges = picked;
            path_borrowed.clear();
            changed = true;
        }
        if along_edges && small_button(f, "Use the sketch", "") {
            path_edges.clear();
            path_borrowed.clear();
            changed = true;
        }
    });
    for (r, name) in &crate::borrow::edges_of_body(ctx.document, body) {
        let mut on = path_borrowed.contains(r);
        if f.check(&mut on, &format!("Along {name}"), "") {
            if on {
                path_borrowed.push(*r);
                path_edges.clear();
            } else {
                path_borrowed.retain(|b| b != r);
            }
            changed = true;
        }
    }
    changed
}

/// The curves of the path sketch a pipe runs along: a row for each, every
/// one ticked while the pipe takes the whole sketch. The last one ticked
/// stays.
fn path_curves_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    spine: FeatureId,
    path_curves: &mut Vec<uuid::Uuid>,
) -> bool {
    use wb_sketch::sketch::GeometryElement;
    let Ok(drawn) = crate::build::load_sketch(ctx.document, spine) else {
        return false;
    };
    let sketch = &drawn.sketch;
    let curves: Vec<uuid::Uuid> = sketch
        .geometry
        .iter()
        .filter(|e| !matches!(e, GeometryElement::Point(_)) && !sketch.is_construction(e.id()))
        .map(|e| e.id())
        .collect();
    let mut changed = false;
    f.flow("Path curves", true, |f| {
        let shown = if path_curves.is_empty() {
            "the whole sketch".to_string()
        } else {
            format!("{} of {}", path_curves.len(), curves.len())
        };
        mono_value(f, &shown, true);
        if !path_curves.is_empty() && small_button(f, "Whole sketch", "") {
            path_curves.clear();
            changed = true;
        }
    });
    if curves.len() < 2 {
        return changed;
    }
    for id in &curves {
        let mut on = path_curves.is_empty() || path_curves.contains(id);
        let name = wb_sketch::style::element_name(sketch, *id);
        if f.check(&mut on, &name, "") {
            let taken: Vec<uuid::Uuid> = curves
                .iter()
                .copied()
                .filter(|c| {
                    if c == id {
                        on
                    } else {
                        path_curves.is_empty() || path_curves.contains(c)
                    }
                })
                .collect();
            if taken.is_empty() {
                continue;
            }
            *path_curves = if taken.len() == curves.len() {
                Vec::new()
            } else {
                taken
            };
            changed = true;
        }
    }
    changed
}

/// A pipe's orientation: which kind, and the path or direction it takes.
fn pipe_orientation_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    (body, feature_id): (BodyId, FeatureId),
    spine: FeatureId,
    orientation: &mut PipeOrientation,
) -> bool {
    let mut changed = false;
    // A binormal starts square to the path's plane, where it holds.
    let normal = crate::build::sketch_normal(ctx.document, spine).unwrap_or([0.0, 0.0, 1.0]);
    let kinds = [
        PipeOrientation::Standard,
        PipeOrientation::Frenet,
        PipeOrientation::Auxiliary { path: spine },
        PipeOrientation::Binormal {
            x: normal[0],
            y: normal[1],
            z: normal[2],
        },
        PipeOrientation::Fixed,
    ];
    let at = kinds
        .iter()
        .position(|k| std::mem::discriminant(orientation) == std::mem::discriminant(k));
    let hint = "Standard keeps the section from twisting; Frenet turns it with the \
                path's curvature; an auxiliary path turns it to face a second path; \
                a binormal holds one direction of it fixed; fixed keeps it as drawn";
    let picked = f.select_hinted(
        "Orientation",
        Select {
            label: "Orientation".into(),
            options: kinds.iter().map(|k| k.label().to_string()).collect(),
            selected: at.unwrap_or(kinds.len()),
            width: Some(CHOICE_WIDTH),
            text: Some(orientation.label().to_string()),
            ..Default::default()
        },
        hint,
    );
    if let Some(i) = picked {
        *orientation = kinds[i];
        changed = true;
    }
    match orientation {
        PipeOrientation::Auxiliary { path } => {
            if let Some(new) =
                sketch_combo(f, ctx, body, feature_id, Some(*path), "Auxiliary path:")
            {
                *path = new;
                changed = true;
            }
        }
        PipeOrientation::Binormal { x, y, z } => {
            for (value, label) in [
                (x, ("Binormal X:", "binormal_x")),
                (y, ("Binormal Y:", "binormal_y")),
                (z, ("Binormal Z:", "binormal_z")),
            ] {
                changed |= number_drag(f, fx, value, label);
            }
        }
        PipeOrientation::Standard | PipeOrientation::Frenet | PipeOrientation::Fixed => {}
    }
    changed
}

/// A plain number, a formula field where the feature has its parameter.
fn number_drag(f: &mut Form, fx: &mut Formulas, value: &mut f32, field: (&str, &str)) -> bool {
    let set = bound_or(f, fx, field, f64::from(*value), (Kind::Plain, 0.05));
    set_f32(value, set)
}

/// The datum lines of a body and the axes of the sketches before
/// `feature`, as pattern axis choices.
fn pattern_axis_choices(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    feature: FeatureId,
) -> Vec<(PatternAxis, String)> {
    let mut choices = Vec::new();
    for (id, name, datum) in core_document::datums_of_body(ctx.document, body) {
        if matches!(datum.shape, core_document::DatumShape::Line { .. }) {
            choices.push((PatternAxis::Datum(id), name));
        }
    }
    for (sketch, name) in crate::build::sketch_choices(ctx.document, body, feature, None) {
        for axis in SketchAxis::ALL {
            choices.push((
                PatternAxis::Sketch { sketch, axis },
                format!("{name} {}", axis.label()),
            ));
        }
    }
    choices
}

/// A picked edge as a pattern axis: a circular edge gives its axis
/// (centre and normal), a straight one itself.
fn edge_axis(edge: &core_document::EdgeRef) -> EdgePick {
    match edge.circle {
        Some(circle) => EdgePick {
            faces: edge.faces,
            point: circle.center,
            direction: circle.normal,
        },
        None => EdgePick::of(edge),
    }
}

/// What a pattern runs along or turns about: the body's axes, a custom
/// axis, a datum line, a sketch's axis or a picked edge.
fn pattern_axis_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    (body, feature): (BodyId, FeatureId),
    axis: &mut PatternAxis,
) -> bool {
    let mut changed = false;
    let references = pattern_axis_choices(ctx, body, feature);
    let shown = references
        .iter()
        .find(|(choice, _)| choice == axis)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| axis.label().to_string());
    let picked_edge = picked_edges(ctx).first().map(edge_axis);
    let mut candidates: Vec<(PatternAxis, String)> = [
        PatternAxis::X,
        PatternAxis::Y,
        PatternAxis::Z,
        PatternAxis::Custom {
            origin: [0.0; 3],
            dir: [0.0, 0.0, 1.0],
        },
    ]
    .into_iter()
    .map(|c| (c, c.label().to_string()))
    .collect();
    candidates.extend(references.iter().cloned());
    candidates.push((
        PatternAxis::Edge(picked_edge.unwrap_or(EdgePick {
            faces: [0, 0],
            point: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        })),
        "Picked edge".to_string(),
    ));
    let at = candidates
        .iter()
        .position(|(candidate, _)| match (&*axis, candidate) {
            (PatternAxis::Datum(_), PatternAxis::Datum(_))
            | (PatternAxis::Sketch { .. }, PatternAxis::Sketch { .. }) => axis == candidate,
            _ => std::mem::discriminant(axis) == std::mem::discriminant(candidate),
        });
    let names = candidates.iter().map(|(_, n)| n.clone()).collect();
    if let Some(i) = choice_shown(f, "Axis", shown, names, at) {
        *axis = candidates[i].0;
        changed = true;
    }
    if let PatternAxis::Edge(edge) = axis {
        f.flow("Edge", true, |f| {
            mono_value(f, &coords(edge.point), true);
            if use_button(
                f,
                "Use selected edge",
                !ctx.selected_edges.is_empty(),
                "Click a straight edge in the viewport, or a round one for its axis, \
                 then press this",
            ) && let Some(picked) = picked_edges(ctx).first()
            {
                *edge = edge_axis(picked);
                changed = true;
            }
        });
    }
    if let PatternAxis::Custom { origin, dir } = axis {
        f.flow("Origin", false, |f| {
            for v in origin.iter_mut() {
                changed |= set_f32(v, bare(f, "", f64::from(*v), 0.5, ""));
            }
        });
        f.flow("Dir", false, |f| {
            for v in dir.iter_mut() {
                changed |= set_f32(v, bare(f, "", f64::from(*v), 0.1, ""));
            }
        });
    }
    changed
}

/// How an uneven pattern's gaps show: the switch that turns them on, the
/// formula name and label of each (numbered from 1), and their unit.
struct Gaps {
    toggle: &'static str,
    name: &'static str,
    label: &'static str,
    degrees: bool,
}

/// An uneven pattern's gaps, one per step from an occurrence to the next.
/// Turned on, the list starts at the even gap and follows the occurrence
/// count; turned off, it empties and the pattern spaces evenly.
fn gaps_editor(
    f: &mut Form,
    fx: &mut Formulas,
    gaps: &mut Vec<f32>,
    (occurrences, even): (u32, f32),
    how: Gaps,
) -> bool {
    let mut changed = false;
    let mut uneven = !gaps.is_empty();
    if f.check(
        &mut uneven,
        how.toggle,
        "Set the gap from each occurrence to the next on its own",
    ) {
        gaps.clear();
        changed = true;
    }
    if !uneven {
        return changed;
    }
    let wanted = occurrences.saturating_sub(1).max(1) as usize;
    if gaps.len() != wanted {
        gaps.resize(wanted, even);
        changed = true;
    }
    for (i, gap) in gaps.iter_mut().enumerate() {
        let label = format!("{} {}:", how.label, i + 1);
        let name = format!("{}{}", how.name, i + 1);
        let mut value = f64::from(*gap);
        changed |= named_f64(f, fx, &mut value, (&label, &name), |f, v| {
            let (kind, speed) = if how.degrees {
                (Kind::Degrees, 1.0)
            } else {
                (Kind::Mm, 0.5)
            };
            match number(f, &label, *v, kind, speed) {
                Some(set) => {
                    *v = set;
                    true
                }
                None => false,
            }
        });
        *gap = value as f32;
    }
    changed
}

/// The datum planes (a coordinate system by each of its planes) and the
/// sketches (each by its plane and its two axes) of the edited feature's
/// body, as mirror planes.
/// The planes of the edited body's datums and sketches.
fn mirror_references(ctx: &WorkbenchRuntimeContext) -> Vec<(MirrorPlane, String)> {
    edited_body(ctx)
        .map(|body| plane_references(ctx, body))
        .unwrap_or_default()
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

fn mirror_plane_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    plane: &mut MirrorPlane,
) -> bool {
    let references = mirror_references(ctx);
    plane_editor(f, ctx, plane, &references)
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

/// A draft's neutral plane (a picked face, one of the body's planes or a
/// datum plane) and what it pulls along (the neutral plane's normal, a
/// picked edge or a datum line).
fn draft_references_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    neutral: &mut FacePick,
    neutral_plane: &mut Option<crate::feature::PlaneTarget>,
    pull: &mut Option<crate::feature::PullRef>,
) -> bool {
    use crate::feature::{PlaneTarget, PullRef};
    let mut changed = false;
    let planes: Vec<(PlaneTarget, String)> = end_targets(ctx, body)
        .into_iter()
        .filter_map(|(mode, name)| match mode {
            ExtrudeMode::UpToPlane(target) => {
                Some((target, name.trim_start_matches("Up to ").to_string()))
            }
            _ => None,
        })
        .collect();
    let shown = match neutral_plane {
        None => "Picked face".to_string(),
        Some(target) => planes
            .iter()
            .find(|(t, _)| t == target)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| "(gone)".into()),
    };
    let mut options = vec!["Picked face".to_string()];
    options.extend(planes.iter().map(|(_, n)| n.clone()));
    let at = match neutral_plane {
        None => Some(0),
        Some(target) => planes.iter().position(|(t, _)| t == target).map(|i| i + 1),
    };
    match choice_shown(f, "Neutral plane", shown, options, at) {
        Some(0) => {
            *neutral_plane = None;
            changed = true;
        }
        Some(i) => {
            *neutral_plane = Some(planes[i - 1].0);
            changed = true;
        }
        None => {}
    }
    if neutral_plane.is_none() {
        let mut neutral_opt = Some(*neutral);
        if face_pick_row(f, ctx, &mut neutral_opt, "Neutral face:")
            && let Some(pick) = neutral_opt
        {
            *neutral = pick;
            changed = true;
        }
    }
    let lines: Vec<(FeatureId, String)> = core_document::datums_of_body(ctx.document, body)
        .into_iter()
        .filter(|(_, _, d)| matches!(d.shape, core_document::DatumShape::Line { .. }))
        .map(|(id, name, _)| (id, name))
        .collect();
    let shown = match pull {
        None => "Neutral plane's normal".to_string(),
        Some(PullRef::Edge(_)) => "Picked edge".to_string(),
        Some(PullRef::Datum(id)) => lines
            .iter()
            .find(|(l, _)| l == id)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| "(gone)".into()),
    };
    let picked = picked_edges(ctx).first().map(EdgePick::of);
    let is_edge = matches!(pull, Some(PullRef::Edge(_)));
    let mut options = vec![
        "Neutral plane's normal".to_string(),
        "Picked edge".to_string(),
    ];
    options.extend(lines.iter().map(|(_, n)| n.clone()));
    let mut hints = vec![String::new(), "Click a straight edge first".to_string()];
    hints.resize(options.len(), String::new());
    let at = match pull {
        None => Some(0),
        Some(PullRef::Edge(_)) => Some(1),
        Some(PullRef::Datum(id)) => lines.iter().position(|(l, _)| l == id).map(|i| i + 2),
    };
    let chosen = f.select(
        "Pull",
        Select {
            label: "Pull".into(),
            selected: at.unwrap_or(options.len()),
            options,
            width: Some(CHOICE_WIDTH),
            text: Some(shown),
            disabled: if picked.is_some() || is_edge {
                Vec::new()
            } else {
                vec![1]
            },
            hints,
            ..Default::default()
        },
    );
    match chosen {
        Some(0) => {
            *pull = None;
            changed = true;
        }
        Some(1) => {
            if let Some(edge) = picked {
                *pull = Some(PullRef::Edge(edge));
                changed = true;
            }
        }
        Some(i) => {
            *pull = Some(PullRef::Datum(lines[i - 2].0));
            changed = true;
        }
        None => {}
    }
    changed
}

/// The body's Design features before this one, modifiers left out,
/// selectable as pattern originals.
fn originals_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    this_feature: FeatureId,
    originals: &mut Vec<FeatureId>,
) -> bool {
    let mut changed = false;
    label_row(f, "Originals (empty = whole body)");
    let features = design_features_of_body(ctx.document, body);
    for (id, feature) in &features {
        if *id == this_feature {
            break;
        }
        if feature.is_modifier() {
            continue;
        }
        let name = ctx
            .document
            .get_feature_meta(*id)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| feature.kind_label().to_string());
        let mut included = originals.contains(id);
        if f.check(&mut included, &name, "") {
            if included {
                originals.push(*id);
            } else {
                originals.retain(|o| o != id);
            }
            changed = true;
        }
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
        let changed = named_f64(f, fx, value, (label, &name_of(label)), |f, value| {
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
        });
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
        changed |= named_f64(f, fx, value, (label, name), |f, value| {
            match bare(f, label, *value, 0.5, " mm") {
                Some(v) => {
                    *value = v;
                    true
                }
                None => false,
            }
        });
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
            let from = ctx.document.base_geometry(body);
            let health = from.and_then(|g| g.health.as_ref());
            taper_note(
                f,
                "The shape this body starts from, as it was imported or converted. \
                 The features after it change it.",
            );
            if let Some(health) = health.filter(|h| h.is_broken()) {
                taper_note(
                    f,
                    &format!(
                        "The checker found {} problem(s) in it; Repair shape mends it.",
                        health.broken
                    ),
                );
            }
        }
        DesignFeature::Pad {
            refine: _,
            boolean_tolerance: _,
            regions,
            sketch,
            length,
            reversed,
            symmetric,
            mode,
            length2,
            taper_deg,
            up_to_face,
            up_to_offset,
            profile_face,
            profile_borrowed: _,
            direction,
            up_to_shape,
            mode2,
            up_to_face2,
            up_to_offset2,
            up_to_shape2,
            extras,
        } => {
            let borrowed = end_targets(ctx, body);
            changed |= face_profile_row(f, ctx, body, profile_face);
            if let (Some(sketch), None) = (*sketch, profile_face.as_ref()) {
                changed |= regions_rows(f, ctx, sketch, regions);
            }
            changed |= extrude_mode_combo(f, mode, first_feature, &borrowed);
            changed |= extrude_side_rows(
                f,
                ctx,
                fx,
                *mode,
                (length, ("Length:", "length")),
                up_to_face,
                (up_to_offset, ("Offset:", "offset")),
                up_to_shape,
                (body, &mut extras.up_to_shape_borrowed),
            );
            if *mode == ExtrudeMode::Dimension && mode2.is_none() {
                changed |= f.check(symmetric, "Symmetric to plane", "");
            }
            if !(*mode == ExtrudeMode::Dimension && *symmetric) {
                changed |= second_side_combo(f, *mode, mode2, first_feature, &borrowed);
                if let (_, Some(second)) = mode.sides(*mode2) {
                    changed |= extrude_side_rows(
                        f,
                        ctx,
                        fx,
                        second,
                        (length2, ("Second length:", "length2")),
                        up_to_face2,
                        (up_to_offset2, ("Second offset:", "offset2")),
                        up_to_shape2,
                        (body, &mut extras.up_to_shape_borrowed2),
                    );
                }
            }
            changed |= extrude_direction_editor(f, ctx, fx, body, direction);
            changed |= f.check(reversed, "Reversed", "");
            changed |= deg_drag(f, fx, taper_deg, ("Taper:", "taper"), -85.0..=85.0);
            taper_note(f, "Positive opens the pad out as it rises.");
            let two_sided = mode.sides(*mode2).1.is_some();
            changed |= extrude_extras_rows(f, ctx, body, fx, extras, two_sided, *direction);
        }
        DesignFeature::Pocket {
            keep_inside: _,
            refine: _,
            boolean_tolerance: _,
            regions,
            sketch,
            depth,
            reversed,
            symmetric,
            through_all,
            mode,
            depth2,
            taper_deg,
            up_to_face,
            up_to_offset,
            profile_face,
            profile_borrowed: _,
            direction,
            up_to_shape,
            mode2,
            up_to_face2,
            up_to_offset2,
            up_to_shape2,
            extras,
        } => {
            // The flag and the ThroughAll mode are one setting: a file that
            // has only the flag set opens in that mode, and the flag
            // follows the mode picked.
            if *through_all && *mode != ExtrudeMode::ThroughAll {
                *mode = ExtrudeMode::ThroughAll;
                changed = true;
            }
            changed |= face_profile_row(f, ctx, body, profile_face);
            if let (Some(sketch), None) = (*sketch, profile_face.as_ref()) {
                changed |= regions_rows(f, ctx, sketch, regions);
            }
            let borrowed = end_targets(ctx, body);
            changed |= extrude_mode_combo(f, mode, first_feature, &borrowed);
            *through_all = *mode == ExtrudeMode::ThroughAll;
            changed |= extrude_side_rows(
                f,
                ctx,
                fx,
                *mode,
                (depth, ("Depth:", "depth")),
                up_to_face,
                (up_to_offset, ("Offset:", "offset")),
                up_to_shape,
                (body, &mut extras.up_to_shape_borrowed),
            );
            if *mode == ExtrudeMode::Dimension && mode2.is_none() {
                changed |= f.check(symmetric, "Symmetric to plane", "");
            }
            if !(*mode == ExtrudeMode::Dimension && *symmetric) {
                changed |= second_side_combo(f, *mode, mode2, first_feature, &borrowed);
                if let (_, Some(second)) = mode.sides(*mode2) {
                    changed |= extrude_side_rows(
                        f,
                        ctx,
                        fx,
                        second,
                        (depth2, ("Second depth:", "depth2")),
                        up_to_face2,
                        (up_to_offset2, ("Second offset:", "offset2")),
                        up_to_shape2,
                        (body, &mut extras.up_to_shape_borrowed2),
                    );
                }
            }
            changed |= extrude_direction_editor(f, ctx, fx, body, direction);
            changed |= f.check(
                reversed,
                "Reversed",
                "Cut along the sketch normal instead of against it",
            );
            changed |= deg_drag(f, fx, taper_deg, ("Taper:", "taper"), -85.0..=85.0);
            taper_note(f, "Positive widens the pocket as it goes deeper.");
            let two_sided = mode.sides(*mode2).1.is_some();
            changed |= extrude_extras_rows(f, ctx, body, fx, extras, two_sided, *direction);
        }
        DesignFeature::Revolution {
            refine: _,
            boolean_tolerance: _,
            regions,
            sketch,
            angle_deg,
            axis,
            project_axis,
            reversed,
            midplane,
            second_angle_deg,
            start_angle_deg,
            mode,
            up_to_face,
        }
        | DesignFeature::Groove {
            keep_inside: _,
            refine: _,
            boolean_tolerance: _,
            regions,
            sketch,
            angle_deg,
            axis,
            project_axis,
            reversed,
            midplane,
            second_angle_deg,
            start_angle_deg,
            mode,
            up_to_face,
        } => {
            changed |= regions_rows(f, ctx, *sketch, regions);
            // Each face the body borrows is a target of its own.
            let targets: Vec<(RevolveMode, String)> =
                crate::borrow::faces_of_body(ctx.document, body)
                    .into_iter()
                    .map(|(r, name)| (RevolveMode::UpToBorrowed(r), format!("Up to {name}")))
                    .collect();
            let shown = targets
                .iter()
                .find(|(t, _)| t == mode)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| mode.label().to_string());
            let offered: Vec<(RevolveMode, String)> = RevolveMode::ALL
                .iter()
                .map(|m| (*m, m.label().to_string()))
                .chain(targets.iter().cloned())
                // Every mode but an angle or a borrowed face stops on the body's own
                // material.
                .filter(|(candidate, _)| {
                    !first_feature
                        || matches!(candidate, RevolveMode::Angle | RevolveMode::UpToBorrowed(_))
                })
                .collect();
            let at = offered.iter().position(|(c, _)| c == mode);
            let names = offered.iter().map(|(_, n)| n.clone()).collect();
            if let Some(i) = choice_shown(f, "Type", shown, names, at) {
                *mode = offered[i].0;
                changed = true;
            }
            match mode {
                RevolveMode::Angle => {
                    changed |= deg_drag(f, fx, angle_deg, ("Angle:", "angle"), 0.1..=360.0);
                }
                RevolveMode::UpToFace => {
                    changed |= face_pick_row(f, ctx, up_to_face, "Target face:");
                }
                RevolveMode::ToFirst | RevolveMode::ToLast | RevolveMode::UpToBorrowed(_) => {}
            }
            changed |= revolve_axis_editor(f, ctx, body, *sketch, axis, false);
            changed |= project_axis_row(f, axis, project_axis);
            if *mode == RevolveMode::Angle {
                changed |= f.check(midplane, "Midplane", "");
                let mut two_sided = second_angle_deg.is_some();
                if f.check(&mut two_sided, "Second angle", "") {
                    *second_angle_deg = two_sided.then_some(90.0);
                    changed = true;
                }
                if let Some(second) = second_angle_deg {
                    changed |= deg_drag(f, fx, second, ("Angle 2:", "angle2"), 0.1..=360.0);
                }
            }
            changed |= f.check(reversed, "Reversed", "");
            changed |= deg_drag(
                f,
                fx,
                start_angle_deg,
                ("Start angle:", "start_angle"),
                -360.0..=360.0,
            );
        }
        DesignFeature::Loft {
            keep_inside: _,
            refine: _,
            boolean_tolerance: _,
            sections,
            ruled,
            closed,
            subtractive,
        } => {
            use crate::feature::LoftSection;
            label_row(f, "Sections (in order)");
            let mut remove = None;
            for (i, section) in sections.iter().enumerate() {
                let name = match section {
                    LoftSection::Feature(id) => ctx
                        .document
                        .get_feature_meta(*id)
                        .map(|n| n.name.clone())
                        .unwrap_or_else(|| "(missing)".into()),
                    LoftSection::Face(pick) => format!(
                        "face at ({:.1}, {:.1}, {:.1})",
                        pick.point[0], pick.point[1], pick.point[2]
                    ),
                };
                f.flow("", false, |f| {
                    mono_value(f, &format!("{}. {name}", i + 1), true);
                    if small_button(f, "×", "") && sections.len() > 1 {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                sections.remove(i);
                changed = true;
            }
            if let Some(new) = sketch_combo(f, ctx, body, feature_id, None, "Add section:")
                && !sections.contains(&LoftSection::Feature(new))
            {
                sections.push(LoftSection::Feature(new));
                changed = true;
            }
            // A datum point closes the loft to it, at either end.
            let points: Vec<(FeatureId, String)> =
                core_document::datums_of_body(ctx.document, body)
                    .into_iter()
                    .filter(|(_, _, d)| matches!(d.shape, core_document::DatumShape::Point))
                    .map(|(id, name, _)| (id, name))
                    .collect();
            if !points.is_empty() {
                let names = points.iter().map(|(_, n)| n.clone()).collect();
                if let Some(i) = choice_shown(f, "Add point:", "A datum point…".into(), names, None)
                {
                    let section = LoftSection::Feature(points[i].0);
                    if !sections.contains(&section) {
                        sections.push(section);
                        changed = true;
                    }
                }
            }
            let picked = ctx.selected_face_in(body).map(FacePick::of);
            f.flow("Add face", false, |f| {
                if use_button(
                    f,
                    "Use selected face",
                    picked.is_some(),
                    "Click a flat face of the solid first, then press this",
                ) && let Some(pick) = picked
                {
                    sections.push(LoftSection::Face(pick));
                    changed = true;
                }
            });
            changed |= f.check(ruled, "Ruled (straight transitions)", "");
            changed |= f.check(closed, "Closed (loop back)", "");
            changed |= f.check(subtractive, "Subtractive", "");
        }
        DesignFeature::Pipe {
            keep_inside: _,
            refine: _,
            boolean_tolerance: _,
            profile,
            spine,
            orientation,
            corner,
            sections,
            subtractive,
            profile_face,
            path_edges,
            path_borrowed,
            path_curves,
        } => {
            changed |= pipe_inputs_editor(
                f,
                ctx,
                (body, feature_id),
                spine,
                profile_face,
                (path_edges, path_borrowed, path_curves),
            );
            changed |= pipe_orientation_editor(f, ctx, fx, (body, feature_id), *spine, orientation);
            changed |= pick(f, "Corners", &PipeCorner::ALL, corner, |candidate| {
                candidate.label()
            });
            label_row(f, "Sections along the path (in order)");
            let mut remove = None;
            for (i, section) in sections.iter().enumerate() {
                let name = ctx
                    .document
                    .get_feature_meta(*section)
                    .map(|n| n.name.clone())
                    .unwrap_or_else(|| "(missing)".into());
                f.flow("", false, |f| {
                    mono_value(f, &format!("{}. {name}", i + 1), true);
                    if small_button(f, "×", "") {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                sections.remove(i);
                changed = true;
            }
            if let Some(new) = sketch_combo(f, ctx, body, feature_id, None, "Add section:")
                && new != *profile
                && new != *spine
                && !sections.contains(&new)
            {
                sections.push(new);
                changed = true;
            }
            // A datum point last closes the pipe to it.
            let points: Vec<(FeatureId, String)> =
                core_document::datums_of_body(ctx.document, body)
                    .into_iter()
                    .filter(|(_, _, d)| matches!(d.shape, core_document::DatumShape::Point))
                    .map(|(id, name, _)| (id, name))
                    .collect();
            if !points.is_empty() {
                let names = points.iter().map(|(_, n)| n.clone()).collect();
                if let Some(i) =
                    choice_shown(f, "End at point:", "A datum point…".into(), names, None)
                    && !sections.contains(&points[i].0)
                {
                    sections.push(points[i].0);
                    changed = true;
                }
            }
            changed |= f.check(subtractive, "Subtractive", "");
        }
        DesignFeature::Helix {
            refine: _,
            boolean_tolerance: _,
            sketch,
            axis,
            project_axis,
            mode,
            pitch,
            height,
            turns,
            left_handed,
            cone_angle_deg,
            reversed,
            subtractive,
            growth,
            keep_inside,
        } => {
            changed |= revolve_axis_editor(f, ctx, body, *sketch, axis, true);
            changed |= project_axis_row(f, axis, project_axis);
            changed |= pick(f, "Mode", &HelixMode::ALL, mode, |candidate| {
                candidate.label()
            });
            match mode {
                HelixMode::PitchHeight => {
                    changed |= mm_drag(f, fx, pitch, ("Pitch:", "pitch"));
                    changed |= mm_drag(f, fx, height, ("Height:", "height"));
                }
                HelixMode::PitchTurns => {
                    changed |= mm_drag(f, fx, pitch, ("Pitch:", "pitch"));
                    changed |= turns_field(f, fx, turns);
                }
                HelixMode::HeightTurns => {
                    changed |= mm_drag(f, fx, height, ("Height:", "height"));
                    changed |= turns_field(f, fx, turns);
                }
                HelixMode::HeightTurnsGrowth => {
                    // A height of 0 is a flat spiral; below it is nothing.
                    if offset_drag(f, fx, height, ("Height:", "height")) {
                        *height = height.max(0.0);
                        changed = true;
                    }
                    changed |= turns_field(f, fx, turns);
                    changed |= offset_drag(f, fx, growth, ("Growth:", "growth"));
                }
            }
            if *mode != HelixMode::HeightTurnsGrowth {
                changed |= deg_drag(
                    f,
                    fx,
                    cone_angle_deg,
                    ("Cone angle:", "cone_angle"),
                    -85.0..=85.0,
                );
            }
            changed |= f.check(left_handed, "Left handed", "");
            changed |= f.check(reversed, "Reversed", "");
            changed |= f.check(subtractive, "Subtractive", "");
            if *subtractive {
                changed |= f.check(
                    keep_inside,
                    "Keep inside",
                    "Keep what the helix shares with the body instead of cutting it",
                );
            }
        }
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
        DesignFeature::Hole { .. } => {
            changed |= hole::hole_editor(f, ctx, fx, body, feature_id, feature);
        }
        DesignFeature::Fillet {
            radius,
            edges,
            follow_tangent,
        } => {
            let most = crate::size_limit::drag_limits(ctx.document, feature_id);
            changed |= mm_drag_upto(f, fx, radius, ("Radius:", "radius"), most.map(|m| m.first));
            changed |= edge_sel_editor(f, ctx, edges, true);
            if !matches!(edges, EdgeSel::BetweenFaces(_)) {
                changed |= tangent_row(f, follow_tangent);
            }
        }
        DesignFeature::Chamfer {
            size,
            mode,
            size2,
            angle_deg,
            flip,
            edges,
            follow_tangent,
        } => {
            changed |= pick(f, "Type", &ChamferMode::ALL, mode, |candidate| {
                candidate.label()
            });
            let most = crate::size_limit::drag_limits(ctx.document, feature_id);
            changed |= mm_drag_upto(f, fx, size, ("Size:", "size"), most.map(|m| m.first));
            match mode {
                ChamferMode::EqualDistance => {}
                ChamferMode::TwoDistances => {
                    let most = most.and_then(|m| m.second);
                    changed |= mm_drag_upto(f, fx, size2, ("Size 2:", "size2"), most);
                    changed |= f.check(flip, "Flip direction", "");
                }
                ChamferMode::DistanceAngle => {
                    changed |= deg_drag(f, fx, angle_deg, ("Angle:", "angle"), 1.0..=89.0);
                    changed |= f.check(flip, "Flip direction", "");
                }
            }
            changed |= edge_sel_editor(f, ctx, edges, false);
            changed |= tangent_row(f, follow_tangent);
        }
        DesignFeature::Draft {
            angle_deg,
            neutral,
            faces,
            reversed,
            neutral_plane,
            pull,
        } => {
            changed |= deg_drag(f, fx, angle_deg, ("Angle:", "angle"), 0.1..=45.0);
            changed |= draft_references_editor(f, ctx, body, neutral, neutral_plane, pull);
            changed |= face_list_editor(f, ctx, faces, "Faces to draft:");
            changed |= f.check(reversed, "Reversed pull", "");
        }
        DesignFeature::OffsetFaces { faces, distance } => {
            changed |= face_list_editor(f, ctx, faces, "Faces to offset:");
            changed |= signed_mm(f, fx, distance, "Distance", "distance");
            taper_note(
                f,
                "Along each face's outward normal; negative moves it into the material. A \
                 bore's face offset in widens it.",
            );
        }
        DesignFeature::OffsetSolid { distance } => {
            changed |= signed_mm(f, fx, distance, "Distance", "distance");
            taper_note(
                f,
                "Every face moves this far along its outward normal, the faces around it \
                 following: positive grows the solid, negative shrinks it. A box grows twice \
                 the distance each way; a hole in it narrows.",
            );
        }
        DesignFeature::Scale {
            factor,
            per_axis,
            factors,
            about,
        } => {
            changed |= scale_editor(f, ctx, fx, body, factor, per_axis, factors, about);
        }
        DesignFeature::MoveFaces {
            faces,
            translation,
            angle_deg,
            axis_point,
            axis_dir,
        } => {
            changed |= face_list_editor(f, ctx, faces, "Faces to move:");
            for (i, (label, name)) in [("Move X", "x"), ("Move Y", "y"), ("Move Z", "z")]
                .into_iter()
                .enumerate()
            {
                changed |= signed_mm(f, fx, &mut translation[i], label, name);
            }
            changed |= deg_drag(f, fx, angle_deg, ("Angle", "angle"), -360.0..=360.0);
            if angle_deg.abs() > 1e-6 {
                for (i, label) in ["Axis point X", "Axis point Y", "Axis point Z"]
                    .into_iter()
                    .enumerate()
                {
                    let set = number(f, label, f64::from(axis_point[i]), Kind::Offset, 0.1);
                    changed |= set_f32(&mut axis_point[i], set);
                }
                for (i, label) in ["Axis X", "Axis Y", "Axis Z"].into_iter().enumerate() {
                    let set = number(f, label, f64::from(axis_dir[i]), Kind::Plain, 0.05);
                    changed |= set_f32(&mut axis_dir[i], set);
                }
            }
        }
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
        DesignFeature::DeleteFaces { faces } => {
            changed |= face_list_editor(f, ctx, faces, "Faces to delete:");
            taper_note(
                f,
                "The faces around each opening grow to close it: a bore, a boss or a round \
                 taken away.",
            );
        }
        DesignFeature::Thickness {
            value,
            faces,
            inward,
            join,
            both_sides,
        } => {
            changed |= mm_drag(f, fx, value, ("Thickness:", "thickness"));
            changed |= face_list_editor(f, ctx, faces, "Faces to open:");
            changed |= f.check(
                both_sides,
                "Both sides",
                "Walls on both sides of the faces, the thickness each way",
            );
            if !*both_sides {
                changed |= f.check(inward, "Inward", "");
            }
            changed |= pick(
                f,
                "Join",
                &kernel_api::ThicknessJoin::ALL,
                join,
                |candidate| candidate.label(),
            );
        }
        DesignFeature::Mirrored {
            originals,
            plane,
            refine: _,
            boolean_tolerance: _,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            changed |= mirror_plane_editor(f, ctx, plane);
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::LinearPattern {
            refine: _,
            boolean_tolerance: _,
            originals,
            axis,
            length,
            occurrences,
            spacing_mode,
            reversed,
            spacings,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            changed |= pattern_axis_editor(f, ctx, (body, feature_id), axis);
            changed |= count_drag(f, fx, occurrences, ("Occurrences:", "occurrences"));
            changed |= f.check(
                spacing_mode,
                "Length is spacing",
                "Off: length is the overall span",
            );
            changed |= mm_drag(f, fx, length, ("Length:", "length"));
            let even = if *spacing_mode || *occurrences < 2 {
                *length
            } else {
                *length / (*occurrences - 1) as f32
            };
            changed |= gaps_editor(
                f,
                fx,
                spacings,
                (*occurrences, even),
                Gaps {
                    toggle: "Uneven spacing",
                    name: "spacing",
                    label: "Spacing",
                    degrees: false,
                },
            );
            changed |= f.check(reversed, "Reversed", "");
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::PolarPattern {
            refine: _,
            boolean_tolerance: _,
            originals,
            axis,
            angle_deg,
            occurrences,
            reversed,
            step_mode,
            angles,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            changed |= pattern_axis_editor(f, ctx, (body, feature_id), axis);
            changed |= count_drag(f, fx, occurrences, ("Occurrences:", "occurrences"));
            changed |= f.check(
                step_mode,
                "Angle is the step",
                "On: the angle between one occurrence and the next. \
                     Off: the angle all of them span",
            );
            changed |= deg_drag(f, fx, angle_deg, ("Angle:", "angle"), 1.0..=360.0);
            let even = if *step_mode || *occurrences < 2 {
                *angle_deg
            } else if (*angle_deg - 360.0).abs() < 1e-6 {
                *angle_deg / *occurrences as f32
            } else {
                *angle_deg / (*occurrences - 1) as f32
            };
            changed |= gaps_editor(
                f,
                fx,
                angles,
                (*occurrences, even),
                Gaps {
                    toggle: "Uneven steps",
                    name: "step_angle",
                    label: "Step angle",
                    degrees: true,
                },
            );
            changed |= f.check(reversed, "Reversed", "");
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::CircularPattern {
            originals,
            axis,
            outward,
            spacing,
            counts,
            refine: _,
            boolean_tolerance: _,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            changed |= patterns::rings_editor(
                f,
                ctx,
                fx,
                (body, feature_id),
                (axis, outward),
                (spacing, counts),
                true,
            );
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::PointPattern {
            originals,
            sketch,
            refine: _,
            boolean_tolerance: _,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            if let Some(picked) = sketch_combo(f, ctx, body, feature_id, Some(*sketch), "Points") {
                *sketch = picked;
                changed = true;
            }
            taper_note(
                f,
                "A copy at each of the sketch's points after its first, moved by how far \
                 that point is from the first; construction points left out.",
            );
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::PathPattern {
            originals,
            path,
            layout,
            refine: _,
            boolean_tolerance: _,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            changed |= patterns::path_editor(f, ctx, fx, (body, feature_id), path, layout);
            taper_note(
                f,
                "The original stands for the path's start: each copy is moved from there to \
                 its place along the path.",
            );
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::MultiTransform {
            originals,
            steps,
            refine: _,
            boolean_tolerance: _,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            f.caption(
                "Steps, each applied to every result of the ones before",
                Tone::Neutral,
                false,
                false,
            );
            let mut remove = None;
            for (i, step) in steps.iter_mut().enumerate() {
                let label = match step {
                    TransformStep::Linear { .. } => "Linear",
                    TransformStep::Polar { .. } => "Polar",
                    TransformStep::Mirror { .. } => "Mirror",
                    TransformStep::Scale { .. } => "Scale",
                    TransformStep::Circular { .. } => "Circular",
                    TransformStep::Points { .. } => "Points",
                    TransformStep::Path { .. } => "Path",
                };
                f.flow("", false, |f| {
                    mono_value(f, &format!("{}. {label}", i + 1), true);
                    if small_button(f, "×", "") {
                        remove = Some(i);
                    }
                });
                // This step's numbers are `step{n}_…` to formulas.
                fx.prefix = Some(format!("step{}_", i + 1));
                match step {
                    TransformStep::Linear {
                        axis,
                        length,
                        occurrences,
                    } => {
                        changed |= pattern_axis_editor(f, ctx, (body, feature_id), axis);
                        changed |= mm_drag(f, fx, length, ("Length:", "length"));
                        changed |= count_drag(f, fx, occurrences, ("Occurrences:", "occurrences"));
                    }
                    TransformStep::Polar {
                        axis,
                        angle_deg,
                        occurrences,
                    } => {
                        changed |= pattern_axis_editor(f, ctx, (body, feature_id), axis);
                        changed |= deg_drag(f, fx, angle_deg, ("Angle:", "angle"), 1.0..=360.0);
                        changed |= count_drag(f, fx, occurrences, ("Occurrences:", "occurrences"));
                    }
                    TransformStep::Mirror { plane } => {
                        changed |= mirror_plane_editor(f, ctx, plane);
                    }
                    TransformStep::Scale {
                        factor,
                        center,
                        occurrences,
                    } => {
                        if let Some((edited, v)) =
                            fx.show(f, ("Factor:", "factor"), f64::from(*factor))
                        {
                            *factor = (v as f32).clamp(0.01, 100.0);
                            changed |= edited;
                        } else {
                            let mut q = quantity("Factor", f64::from(*factor), Kind::Plain, 0.05);
                            (q.min, q.max, q.bare) = (Some(0.01), Some(100.0), true);
                            changed |= set_f32(factor, f.quantity(q));
                        }
                        f.flow("Center", true, |f| {
                            for v in center.iter_mut() {
                                changed |= set_f32(v, bare(f, "", f64::from(*v), 0.5, ""));
                            }
                            let picked = picked_point(ctx);
                            if use_button(
                                f,
                                "Use selected",
                                picked.is_some(),
                                "A circular edge's centre, else where an edge or a face was picked",
                            ) && let Some(at) = picked
                            {
                                *center = at;
                                changed = true;
                            }
                        });
                        let points: Vec<(String, [f32; 3])> =
                            core_document::datums_of_body(ctx.document, body)
                                .into_iter()
                                .filter(|(_, _, d)| {
                                    matches!(d.shape, core_document::DatumShape::Point)
                                })
                                .map(|(_, name, d)| (name, d.frame().origin))
                                .collect();
                        if !points.is_empty() {
                            let names = points.iter().map(|(n, _)| n.clone()).collect();
                            if let Some(k) =
                                choice_shown(f, LABEL_BLANK, "A datum point…".into(), names, None)
                            {
                                *center = points[k].1;
                                changed = true;
                            }
                        }
                        changed |= count_drag(f, fx, occurrences, ("Occurrences:", "occurrences"));
                    }
                    TransformStep::Circular {
                        axis,
                        outward,
                        spacing,
                        counts,
                    } => {
                        changed |= patterns::rings_editor(
                            f,
                            ctx,
                            fx,
                            (body, feature_id),
                            (axis, outward),
                            (spacing, counts),
                            false,
                        );
                    }
                    TransformStep::Path { path, layout } => {
                        changed |=
                            patterns::path_editor(f, ctx, fx, (body, feature_id), path, layout);
                    }
                    TransformStep::Points { sketch } => {
                        if let Some(picked) =
                            sketch_combo(f, ctx, body, feature_id, Some(*sketch), "Points")
                        {
                            *sketch = picked;
                            changed = true;
                        }
                    }
                }
            }
            fx.prefix = None;
            if let Some(i) = remove {
                steps.remove(i);
                changed = true;
            }
            f.flow("", true, |f| {
                let add =
                    |f: &mut Form, label: &str| f.button(label, ButtonStyle::Secondary, true, "");
                if add(f, "+ Linear") {
                    steps.push(TransformStep::Linear {
                        axis: PatternAxis::X,
                        length: 10.0,
                        occurrences: 2,
                    });
                    changed = true;
                }
                if add(f, "+ Polar") {
                    steps.push(TransformStep::Polar {
                        axis: PatternAxis::Z,
                        angle_deg: 360.0,
                        occurrences: 4,
                    });
                    changed = true;
                }
                if add(f, "+ Mirror") {
                    steps.push(TransformStep::Mirror {
                        plane: MirrorPlane::YZ,
                    });
                    changed = true;
                }
            });
            f.flow("", true, |f| {
                let add =
                    |f: &mut Form, label: &str| f.button(label, ButtonStyle::Secondary, true, "");
                if add(f, "+ Circular") {
                    steps.push(TransformStep::Circular {
                        axis: PatternAxis::Z,
                        outward: PatternAxis::X,
                        spacing: 10.0,
                        counts: vec![6, 12],
                    });
                    changed = true;
                }
                if let Some((sketch, _)) =
                    crate::build::sketches_of_body(ctx.document, body).first()
                    && add(f, "+ Points")
                {
                    steps.push(TransformStep::Points { sketch: *sketch });
                    changed = true;
                }
                if let Some(path) = patterns::add_path_step(f, ctx, body) {
                    steps.push(TransformStep::Path {
                        path,
                        layout: Default::default(),
                    });
                    changed = true;
                }
                if add(f, "+ Scale") {
                    steps.push(TransformStep::Scale {
                        factor: 2.0,
                        center: [0.0; 3],
                        occurrences: 2,
                    });
                    changed = true;
                }
            });
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::Clone { source } => {
            let bodies: Vec<(BodyId, String)> = ctx
                .document
                .bodies()
                .iter()
                .filter(|b| b.id != body)
                .map(|b| (b.id, b.name.clone()))
                .collect();
            let current = bodies
                .iter()
                .find(|(id, _)| id == source)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "(pick body)".into());
            let at = bodies.iter().position(|(id, _)| id == source);
            let names = bodies.iter().map(|(_, n)| n.clone()).collect();
            if let Some(i) = choice_shown(f, "Source body", current, names, at) {
                *source = bodies[i].0;
                changed = true;
            }
        }
        DesignFeature::Split {
            plane,
            offset,
            sketch,
            other_side,
            split: _,
        } => {
            changed |= split_editor(f, ctx, fx, body, plane, offset, sketch, other_side);
        }
        DesignFeature::Piece { near } => {
            f.flow("Near", false, |f| {
                let near = format!("{:.2}, {:.2}, {:.2}", near[0], near[1], near[2]);
                f.caption(&near, Tone::Neutral, true, true);
            });
            taper_note(
                f,
                "Of the solid's pieces before it, this keeps the one nearest the point: the \
                 piece this body was separated as. Editing the body it came from rebuilds it.",
            );
        }
        DesignFeature::Borrow {
            source,
            frozen,
            options,
        } => {
            changed |= borrow::borrow_editor(f, ctx, body, feature_id, source, frozen, options);
        }
        DesignFeature::BodyBoolean {
            tool_body,
            kind,
            more_tools,
            refine: _,
            boolean_tolerance: _,
        } => {
            let bodies: Vec<(BodyId, String)> = ctx
                .document
                .bodies()
                .iter()
                .filter(|b| b.id != body)
                .map(|b| (b.id, b.name.clone()))
                .collect();
            let current = bodies
                .iter()
                .find(|(id, _)| id == tool_body)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "(pick body)".into());
            let at = bodies.iter().position(|(id, _)| id == tool_body);
            let names = bodies.iter().map(|(_, n)| n.clone()).collect();
            if let Some(i) = choice_shown(f, "Tool body", current, names, at) {
                *tool_body = bodies[i].0;
                changed = true;
            }
            // Further tools, taken the same way in turn.
            let mut remove = None;
            for (i, tool) in more_tools.iter().enumerate() {
                let name = bodies
                    .iter()
                    .find(|(id, _)| id == tool)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| "(gone)".into());
                f.flow(LABEL_BLANK, false, |f| {
                    mono_value(f, &name, true);
                    if small_button(f, "×", "") {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                more_tools.remove(i);
                changed = true;
            }
            let more: Vec<&(BodyId, String)> = bodies
                .iter()
                .filter(|(id, _)| id != tool_body && !more_tools.contains(id))
                .collect();
            let names = more.iter().map(|(_, n)| n.clone()).collect();
            if let Some(i) = choice_shown(f, "Add tool", "Another body…".into(), names, None) {
                more_tools.push(more[i].0);
                changed = true;
            }
            let kinds = [
                (kernel_api::BoolKind::Fuse, "Fuse"),
                (kernel_api::BoolKind::Cut, "Cut"),
                (kernel_api::BoolKind::Common, "Common"),
            ];
            let picked = f.select(
                "Operation",
                Select {
                    label: "Operation".into(),
                    options: kinds.iter().map(|(_, l)| l.to_string()).collect(),
                    selected: kinds
                        .iter()
                        .position(|(k, _)| k == kind)
                        .unwrap_or(kinds.len()),
                    segmented: true,
                    ..Default::default()
                },
            );
            if let Some(i) = picked {
                *kind = kinds[i].0;
                changed = true;
            }
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

/// A split piece's cut: a plane of the body split (or a sketch of it
/// carried through the solid) and the side kept. The other pieces of the
/// split follow the cut (`split::sync`).
#[allow(clippy::too_many_arguments)]
fn split_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    plane: &mut MirrorPlane,
    offset: &mut f32,
    sketch: &mut Option<FeatureId>,
    other_side: &mut bool,
) -> bool {
    let mut changed = false;
    let source = crate::split::source_of(ctx.document, body);
    let sketches = source
        .map(|source| crate::build::sketches_of_body(ctx.document, source))
        .unwrap_or_default();
    let shown = sketch
        .and_then(|id| sketches.iter().find(|(s, _)| *s == id))
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| "A plane".to_string());
    let mut options = vec!["A plane".to_string()];
    options.extend(sketches.iter().map(|(_, n)| n.clone()));
    let at = match sketch {
        None => Some(0),
        Some(id) => sketches.iter().position(|(s, _)| s == id).map(|i| i + 1),
    };
    match choice_shown(f, "Cut by", shown, options, at) {
        Some(0) => {
            *sketch = None;
            changed = true;
        }
        Some(i) => {
            *sketch = Some(sketches[i - 1].0);
            changed = true;
        }
        None => {}
    }
    if sketch.is_none() {
        let references = source
            .map(|source| plane_references(ctx, source))
            .unwrap_or_default();
        changed |= plane_editor(f, ctx, plane, &references);
        changed |= signed_mm(f, fx, offset, "Offset", "offset");
    }
    let side = if sketch.is_some() {
        "Keep what lies outside the profile"
    } else {
        "Keep what lies behind the plane"
    };
    changed |= f.check(other_side, side, "");
    taper_note(
        f,
        "Every piece of the split cuts the same way and keeps its own side; a change \
         here moves the cut of them all. Editing the body split rebuilds the pieces.",
    );
    changed
}

/// A Scale's factors (one, or one per axis), the point it scales about
/// and the unit presets.
#[allow(clippy::too_many_arguments)]
fn scale_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    factor: &mut f32,
    per_axis: &mut bool,
    factors: &mut [f32; 3],
    about: &mut crate::feature::ScaleCentre,
) -> bool {
    use crate::feature::ScaleCentre;
    let mut changed = f.check(
        per_axis,
        "Per axis",
        "A factor along each of the body's X, Y and Z",
    );
    if *per_axis {
        for (value, label) in factors.iter_mut().zip([
            ("Factor X", "factor_x"),
            ("Factor Y", "factor_y"),
            ("Factor Z", "factor_z"),
        ]) {
            changed |= number_drag(f, fx, value, label);
        }
    } else {
        changed |= number_drag(f, fx, factor, ("Factor", "factor"));
    }
    f.flow("Presets", false, |f| {
        for (label, value, hint) in [
            (
                "inch → mm",
                25.4,
                "A part modelled in inches, made millimetres",
            ),
            (
                "mm → inch",
                1.0 / 25.4,
                "A part modelled in millimetres, made inches",
            ),
        ] {
            if small_button(f, label, hint) {
                *per_axis = false;
                *factor = value;
                changed = true;
            }
        }
    });
    let mut centres: Vec<(ScaleCentre, String)> = [ScaleCentre::Origin, ScaleCentre::BodyCentre]
        .map(|c| (c, c.label().to_string()))
        .into();
    let datums = core_document::datums_of_body(ctx.document, body)
        .into_iter()
        .map(|(datum, name, _)| (ScaleCentre::Datum { datum }, name));
    centres.extend(datums);
    let shown = centres
        .iter()
        .skip(2)
        .find(|(c, _)| c == about)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| about.label().to_string());
    let at = centres.iter().position(|(c, _)| c == about);
    let names = centres.iter().map(|(_, n)| n.clone()).collect();
    if let Some(i) = choice_shown(f, "About", shown, names, at) {
        *about = centres[i].0;
        changed = true;
    }
    taper_note(
        f,
        "The whole solid scales about the point: the body's origin, the middle of its \
         bounding box, or a datum's origin. Factors above 1 enlarge it.",
    );
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
                core_document::panel::show(
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
