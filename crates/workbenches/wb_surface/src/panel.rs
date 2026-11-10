//! The task panels of the Surface bench, declared as widgets and drawn by
//! `core_document::panel` as a package's panel is, and its Preferences
//! page.
//!
//! A step's inputs are lists: each row names a sketch or an edge, with a
//! cross to take it out; "Add the selection" takes what is selected now
//! (a sketch in the tree, edges and faces in the view; a list of curves or
//! edges takes a face's edges). Every edit writes the step at once and it
//! builds again.
//!
//! One walk serves both ends of a panel: a [`Form`] without an event lists
//! the widgets, and with one applies it to the values the walk passes, so
//! what is drawn and what a change does read the same code.

use bench_api::{
    ButtonStyle, Caption, Cells, Check, Fill, Flow, GapSize, NoteKind, PanelAction, PanelEvent,
    Quantity, Select, Stack, Swatch, Swatches, Tone, Widget,
};
use core_document::Document;
use kernel_api::Continuity;

use crate::feature::{Axis, CurveRef, Direction, EdgePick, FacePick, PlaneRef, SurfaceFeature};

/// The width of a choice.
const FIELD: f32 = 150.0;

/// What is selected now: curves and faces a step could take, and the
/// edges round the faces.
pub type Selection = (Vec<CurveRef>, Vec<FacePick>, Vec<CurveRef>);

/// One walk over a panel: the widgets it lists, and with an event, that
/// event applied to the values it walks.
pub struct Form<'e> {
    widgets: Vec<Widget>,
    event: Option<&'e PanelEvent>,
    changed: bool,
    /// Ids given so far, so two fields of one label stay apart.
    ids: Vec<String>,
}

impl<'e> Form<'e> {
    pub fn new(event: Option<&'e PanelEvent>) -> Self {
        Self {
            widgets: Vec::new(),
            event,
            changed: false,
            ids: Vec::new(),
        }
    }

    /// The widgets, at the theme's close spacing.
    pub fn widgets(self) -> Vec<Widget> {
        vec![
            Stack {
                children: self.widgets,
                tight: true,
                ..Default::default()
            }
            .into(),
        ]
    }

    /// Whether the event changed a value.
    pub fn changed(&self) -> bool {
        self.changed
    }

    fn push(&mut self, widget: impl Into<Widget>) {
        self.widgets.push(widget.into());
    }

    /// An id for a field labelled `label`, unique in the panel.
    fn id(&mut self, label: &str) -> String {
        let mut id = label.to_string();
        let mut n = 1;
        while self.ids.contains(&id) {
            n += 1;
            id = format!("{label}#{n}");
        }
        self.ids.push(id.clone());
        id
    }

    fn gap(&mut self) {
        self.push(bench_api::Gap {
            size: GapSize::Small,
        });
    }

    fn note(&mut self, kind: NoteKind, title: &str, text: &str) {
        self.push(Widget::Note {
            kind,
            title: Some(title.into()),
            text: text.into(),
        });
    }

    /// Dim small words, then a little room.
    fn hint(&mut self, text: &str) {
        self.push(Caption {
            text: text.into(),
            tone: Tone::Muted,
            small: true,
            ..Default::default()
        });
        self.gap();
    }

    /// `start` at the row's start and `end` at its end.
    fn row(&mut self, start: Vec<Widget>, end: Vec<Widget>) {
        let mut children = start;
        children.push(
            Fill {
                child: Box::new(Stack::default().into()),
            }
            .into(),
        );
        children.extend(end);
        self.push(Flow {
            children,
            no_wrap: true,
            ..Default::default()
        });
    }

    /// A field's label at the row's start, its control at the end.
    fn field(&mut self, label: &str, control: Widget) {
        let label = Caption {
            text: label.into(),
            ..Default::default()
        };
        self.row(vec![label.into()], vec![control]);
    }

    fn number_event(&mut self, id: &str) -> Option<f64> {
        match self.event {
            Some(PanelEvent::Number { id: e, value }) if e == id => {
                self.changed = true;
                Some(*value)
            }
            _ => None,
        }
    }

    /// A number at the row's end, as `shape` says it; whether it changed.
    fn number(&mut self, label: &str, value: &mut f64, shape: Quantity) -> bool {
        let id = self.id(label);
        let new = self.number_event(&id);
        if let Some(v) = new {
            *value = v;
        }
        self.field(
            label,
            Quantity {
                id,
                value: *value,
                ..shape
            }
            .into(),
        );
        new.is_some()
    }

    fn length(&mut self, label: &str, value: &mut f32) -> bool {
        let mut v = f64::from(*value);
        let changed = self.number(label, &mut v, Quantity::default());
        if changed {
            *value = v as f32;
        }
        changed
    }

    fn check(&mut self, text: &str, on: &mut bool) -> bool {
        let id = self.id(text);
        let toggled = match self.event {
            Some(PanelEvent::Toggle { id: e, on: now }) if *e == id => Some(*now),
            _ => None,
        };
        if let Some(now) = toggled {
            *on = now;
            self.changed = true;
        }
        self.push(Check {
            id,
            text: text.into(),
            on: *on,
            ..Default::default()
        });
        toggled.is_some()
    }

    /// One of `options` at the row's end; whether the choice changed.
    fn select<T: PartialEq + Copy>(
        &mut self,
        label: &str,
        value: &mut T,
        options: &[(T, &str)],
    ) -> bool {
        let id = self.id(label);
        let chosen = match self.event {
            Some(PanelEvent::Choice { id: e, index }) if *e == id => options.get(*index),
            _ => None,
        };
        let changed = chosen.is_some_and(|(v, _)| *v != *value);
        if let Some((v, _)) = chosen {
            *value = *v;
            self.changed |= changed;
        }
        // A value none of the options names (set by a script) shows none.
        let selected = options
            .iter()
            .position(|(v, _)| *v == *value)
            .unwrap_or(options.len());
        self.field(
            label,
            Select {
                id,
                options: options.iter().map(|(_, o)| o.to_string()).collect(),
                selected,
                width: Some(FIELD),
                ..Default::default()
            }
            .into(),
        );
        changed
    }

    fn clicked(&self, id: &str) -> bool {
        matches!(self.event, Some(PanelEvent::Button { id: e }) if e == id)
    }

    /// A title with an "Add the selection" button, then one row per item
    /// with a cross to take it out; whether the list changed.
    fn list<T: Clone + PartialEq>(
        &mut self,
        title: &str,
        items: &mut Vec<T>,
        selection: &[T],
        name: impl Fn(&T) -> String,
    ) -> bool {
        let key = self.id(title);
        let add = format!("{key}:add");
        let fresh: Vec<T> = selection
            .iter()
            .filter(|s| !items.contains(s))
            .cloned()
            .collect();
        let mut changed = false;
        if self.clicked(&add) && !fresh.is_empty() {
            items.extend(fresh.iter().cloned());
            changed = true;
        }
        let button = Widget::Button {
            id: add,
            label: "Add the selection".into(),
            style: ButtonStyle::Small,
            enabled: !fresh.is_empty(),
        };
        let button = if fresh.is_empty() {
            Widget::Hinted {
                hint: "Select a sketch in the tree, or edges or a face in the view".into(),
                widget: Box::new(button),
            }
        } else {
            button
        };
        let heading = Caption {
            text: title.into(),
            medium: true,
            primary: true,
            ..Default::default()
        };
        self.row(vec![heading.into()], vec![button]);
        if items.is_empty() {
            self.hint("Nothing picked yet.");
        }
        let out = (0..items.len()).find(|i| self.clicked(&format!("{key}:out:{i}")));
        for (index, item) in items.iter().enumerate() {
            let label = Caption {
                text: name(item),
                ..Default::default()
            };
            let cross = PanelAction {
                id: format!("{key}:out:{index}"),
                icon: Some("close".into()),
                label: "Take it out".into(),
                ..Default::default()
            };
            let indent = bench_api::Gap {
                size: GapSize::Medium,
            };
            self.row(vec![indent.into(), label.into()], vec![cross.into()]);
        }
        if let Some(index) = out {
            items.remove(index);
            changed = true;
        }
        self.gap();
        self.changed |= changed;
        changed
    }
}

/// Walk the step's editor.
pub fn editor(
    form: &mut Form,
    document: &Document,
    error: Option<&str>,
    feature: &mut SurfaceFeature,
    selection: Option<Selection>,
) {
    let (picked, faces, outline) = selection.unwrap_or_default();
    // A list of curves or edges takes a picked face's edges too.
    let mut around = picked.clone();
    around.extend(outline.into_iter().filter(|c| !picked.contains(c)));
    if let Some(waits) = feature.kind().waits {
        form.note(
            NoteKind::Warning,
            "Waits on the geometry kernel",
            &format!("{waits}. The step keeps its settings and builds once the kernel can."),
        );
    }
    if let Some(error) = error {
        form.note(NoteKind::Error, "Does not build", error);
    }
    form.gap();
    let curve = |c: &CurveRef| curve_name(document, c);
    match feature {
        SurfaceFeature::Extrude {
            curves,
            direction,
            length,
            symmetric,
            reversed,
        } => {
            form.list("Curves", curves, &around, curve);
            form.length("Length", length);
            direction_row(form, direction);
            form.check("Both ways, half each", symmetric);
            form.check("Reversed", reversed);
        }
        SurfaceFeature::Revolve {
            curves,
            axis,
            angle_deg,
        } => {
            form.list("Curves", curves, &around, curve);
            axis_row(form, axis);
            let mut angle = f64::from(*angle_deg);
            let degrees = Quantity {
                dim: bench_api::Dim::Angle,
                speed: Some(0.5),
                min: Some(0.1),
                max: Some(360.0),
                ..Default::default()
            };
            if form.number("Angle", &mut angle, degrees) {
                *angle_deg = angle as f32;
            }
        }
        SurfaceFeature::PlanarFill { curves } => {
            form.hint("Closed loops in one plane; a loop inside another is a hole.");
            form.list("Loops", curves, &around, curve);
        }
        SurfaceFeature::Fill {
            boundary,
            continuity,
        } => {
            form.hint(
                "Curves meeting end to end round the hole: four today. Pick edges of the \
                 body's sheets to fill between them.",
            );
            form.list("Boundary", boundary, &around, curve);
            continuity_row(form, continuity);
        }
        SurfaceFeature::Ruled { first, second } => {
            one(form, "First curve", first, &first_of(&picked), curve);
            one(form, "Second curve", second, &first_of(&picked), curve);
        }
        SurfaceFeature::Loft {
            sections,
            closed,
            guides,
        } => {
            form.hint("One curve per section, in the order the surface passes them.");
            form.list("Sections", sections, &picked, curve);
            form.hint(
                "Guides shape the surface between the sections: each crosses every \
                 section once, in order. A guided loft is one face, through sections of \
                 one curve each, and does not close.",
            );
            form.list("Guides", guides, &picked, curve);
            form.check("Closed back to the first", closed);
        }
        SurfaceFeature::Sweep {
            profile,
            path,
            second_rail,
        } => {
            form.list("Profile", profile, &picked, curve);
            form.list("Path", path, &picked, curve);
            form.hint(
                "With a second rail, the profile runs from the path's start to the \
                 rail's and rides both, scaled to the width between them.",
            );
            form.list("Second rail", second_rail, &picked, curve);
        }
        SurfaceFeature::Offset { faces: f, distance } => {
            form.list("Faces", f, &faces, face_name);
            form.length("Distance", distance);
        }
        SurfaceFeature::Extend {
            edges,
            length,
            continuity,
        } => {
            form.list("Edges", edges, &edges_of(&around), edge_name);
            form.length("Length", length);
            continuity_row(form, continuity);
        }
        SurfaceFeature::Blend {
            first,
            second,
            continuity,
        } => {
            let one_edge: Vec<EdgePick> = edges_of(&picked).into_iter().take(1).collect();
            one(form, "First edge", first, &one_edge, edge_name);
            one(form, "Second edge", second, &one_edge, edge_name);
            continuity_row(form, continuity);
        }
        SurfaceFeature::Split { faces: f, curves } => {
            form.list("Faces", f, &faces, face_name);
            form.list("Cut along", curves, &picked, curve);
        }
        SurfaceFeature::Sew { gap } => {
            form.hint(
                "Joins the body's surfaces where their edges meet. A shell that closes \
                 becomes a solid.",
            );
            let mut v = f64::from(*gap);
            let shape = Quantity {
                min: Some(0.0),
                max: Some(5.0),
                ..Default::default()
            };
            if form.number("Close gaps up to", &mut v, shape) {
                *gap = v as f32;
            }
        }
        SurfaceFeature::Fillet { edges, radius } => {
            form.hint("Edges where two faces of the body's surfaces meet.");
            form.list("Edges", edges, &edges_of(&around), edge_name);
            form.length("Radius", radius);
        }
        SurfaceFeature::FilletFaces {
            first,
            second,
            radius,
            flip_first,
            flip_second,
        } => {
            form.hint(
                "Two faces that share no edge, such as two separate surfaces. The round \
                 rolls on the side each face's normal points to; flip a face to roll it on \
                 the other.",
            );
            one(form, "First face", first, &first_of(&faces), face_name);
            form.check("Round on its other side", flip_first);
            // The second slot offers the face picked last.
            let last: Vec<FacePick> = faces.iter().rev().take(1).copied().collect();
            one(form, "Second face", second, &last, face_name);
            form.check("Round on its other side", flip_second);
            form.length("Radius", radius);
        }
        SurfaceFeature::Thicken {
            thickness,
            both_sides,
        } => {
            form.length("Thickness", thickness);
            form.check("Both sides, half each", both_sides);
        }
        SurfaceFeature::Trim {
            plane,
            offset,
            flip,
        } => {
            plane_row(form, plane);
            form.length("Offset", offset);
            form.check("Keep the other side", flip);
        }
        SurfaceFeature::Mirror { plane, offset } => {
            plane_row(form, plane);
            form.length("Offset", offset);
        }
        SurfaceFeature::TrimScan {
            tolerance, share, ..
        } => {
            form.hint(
                "The body's surfaces are cut where they cross, and each face kept where enough \
                 of it lies on the scan; what is kept is sewn, a closed shell a solid.",
            );
            let mut near = tolerance.unwrap_or(0.0);
            if form.length("Within (0: from the fits)", &mut near) {
                *tolerance = (near > 0.0).then_some(near);
            }
            let mut percent = f64::from(*share) * 100.0;
            if form.number("Of its area", &mut percent, percent_of(1.0, 100.0)) {
                *share = (percent / 100.0) as f32;
            }
        }
        SurfaceFeature::FitNetwork {
            curves,
            polylines,
            patches,
            chained,
            tolerance,
            ..
        } => {
            form.hint(&format!(
                "{} patches over {} curves, fitted to the scan together: they meet \
                 across every curve without a gap, tangent unless it is a crease.",
                patches.len() + chained.len(),
                curves.len() + polylines.len()
            ));
            form.length("Tolerance", tolerance);
        }
        SurfaceFeature::FitScanFree {
            points, tolerance, ..
        } => {
            form.hint(&format!(
                "A B-spline fitted to {} points of the scan, refined until it keeps within \
                 the tolerance of them or only noise is left.",
                points.len()
            ));
            form.length("Tolerance", tolerance);
        }
        SurfaceFeature::FitScan { sigma, extend, .. } => {
            form.hint(&format!(
                "Fitted to the scan's points within {sigma:.3} mm RMS; it reaches past them \
                 by the share below, so neighbouring fits cross and can be trimmed."
            ));
            let mut share = f64::from(*extend) * 100.0;
            if form.number("Extend", &mut share, percent_of(0.0, 200.0)) {
                *extend = (share / 100.0) as f32;
            }
        }
    }
}

/// A share in percent, from `min` to `max`.
fn percent_of(min: f64, max: f64) -> Quantity {
    Quantity {
        unit: Some("%".into()),
        min: Some(min),
        max: Some(max),
        ..Default::default()
    }
}

/// The first of `picked`, as a single slot offers it.
fn first_of<T: Copy>(picked: &[T]) -> Vec<T> {
    picked.iter().take(1).copied().collect()
}

/// A single slot as a list of one, filled from `offered`.
fn one<T: Copy + PartialEq>(
    form: &mut Form,
    title: &str,
    slot: &mut Option<T>,
    offered: &[T],
    name: impl Fn(&T) -> String,
) {
    let mut items: Vec<T> = slot.iter().copied().collect();
    if form.list(title, &mut items, offered, name) {
        *slot = items.last().copied();
    }
}

fn continuity_row(form: &mut Form, continuity: &mut Continuity) {
    form.select(
        "Continuity",
        continuity,
        &[
            (Continuity::G0, "Touching (G0)"),
            (Continuity::G1, "Tangent (G1)"),
            (Continuity::G2, "Curvature (G2)"),
        ],
    );
}

fn direction_row(form: &mut Form, direction: &mut Direction) {
    let mut choice = match *direction {
        Direction::Custom(_) => Direction::SketchNormal,
        other => other,
    };
    let custom = matches!(direction, Direction::Custom(_));
    let changed = form.select(
        "Direction",
        &mut choice,
        &[
            (Direction::SketchNormal, "Square to the sketch"),
            (Direction::X, "X"),
            (Direction::Y, "Y"),
            (Direction::Z, "Z"),
        ],
    );
    if changed || !custom {
        *direction = choice;
    }
}

fn axis_row(form: &mut Form, axis: &mut Axis) {
    if matches!(axis, Axis::Custom { .. }) {
        form.hint("A custom axis, set by a script.");
    }
    form.select(
        "Axis",
        axis,
        &[
            (Axis::SketchVertical, "Sketch's vertical"),
            (Axis::SketchHorizontal, "Sketch's horizontal"),
            (Axis::X, "X"),
            (Axis::Y, "Y"),
            (Axis::Z, "Z"),
        ],
    );
}

fn plane_row(form: &mut Form, plane: &mut PlaneRef) {
    if matches!(plane, PlaneRef::Custom { .. }) {
        form.hint("A custom plane, set by a script.");
    }
    form.select(
        "Plane",
        plane,
        &[
            (PlaneRef::YZ, "YZ"),
            (PlaneRef::XZ, "XZ"),
            (PlaneRef::XY, "XY"),
        ],
    );
}

fn curve_name(document: &Document, curve: &CurveRef) -> String {
    match curve {
        CurveRef::Sketch(id) => document
            .get_feature_meta(*id)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "A sketch that is gone".into()),
        CurveRef::Edge(edge) => edge_name(edge),
        CurveRef::BodyEdge { body, edge } => {
            let name = document
                .bodies()
                .iter()
                .find(|b| b.id == *body)
                .map(|b| b.name.clone())
                .unwrap_or_else(|| "a body that is gone".into());
            format!("{} of {name}", edge_name(edge))
        }
    }
}

fn edge_name(edge: &EdgePick) -> String {
    if edge.length > 0.0 {
        format!("Edge, {:.2} mm", edge.length)
    } else {
        "Edge".into()
    }
}

fn face_name(_: &FacePick) -> String {
    "Face".into()
}

/// The edges among `picked`.
fn edges_of(picked: &[CurveRef]) -> Vec<EdgePick> {
    picked
        .iter()
        .filter_map(|c| match c {
            CurveRef::Edge(e) => Some(*e),
            CurveRef::Sketch(_) | CurveRef::BodyEdge { .. } => None,
        })
        .collect()
}

/// A continuity check's findings: how many edges, how many meet tangent,
/// and each edge from the sharpest crease down.
pub fn continuity(
    body: &str,
    joins: &[([f32; 3], kernel_api::EdgeContinuity)],
    tangent_deg: f64,
    apart_mm: f64,
) -> Vec<Widget> {
    let apart = joins.iter().filter(|(_, j)| j.gap > apart_mm).count();
    let tangent = joins
        .iter()
        .filter(|(_, j)| j.gap <= apart_mm && j.angle_deg < tangent_deg)
        .count();
    let mut form = Form::new(None);
    form.hint(&format!(
        "{body}: {} shared edge(s); {tangent} tangent, {} creased, {apart} apart. \
         The view labels each edge.",
        joins.len(),
        joins.len() - tangent - apart,
    ));
    let mut sorted: Vec<&kernel_api::EdgeContinuity> = joins.iter().map(|(_, j)| j).collect();
    sorted.sort_by(|a, b| b.angle_deg.total_cmp(&a.angle_deg));
    let cell = |text: String, primary: bool| -> Widget {
        Caption {
            text,
            small: true,
            mono: true,
            primary,
            ..Default::default()
        }
        .into()
    };
    let mut children: Vec<Widget> = ["Edge at", "Crease", "Gap"]
        .into_iter()
        .map(|title| {
            Caption {
                text: title.into(),
                small: true,
                medium: true,
                ..Default::default()
            }
            .into()
        })
        .collect();
    for join in sorted {
        let [x, y, z] = join.point;
        children.push(cell(format!("{x:.1}, {y:.1}, {z:.1}"), false));
        children.push(cell(format!("{:.2}°", join.angle_deg), true));
        children.push(cell(format!("{:.4} mm", join.gap), true));
    }
    form.push(Cells {
        id: "surface_continuity".into(),
        columns: 3,
        children,
    });
    form.widgets()
}

/// Walk an analysis's settings and, for a curvature map, its legend.
pub fn analysis(
    form: &mut Form,
    body: &str,
    display: &mut crate::analysis::Display,
    range: Option<(f64, f64)>,
    palette: &core_document::SketchPalette,
) {
    use crate::analysis::{Display, Measure, StripeAxis};
    match display {
        Display::Curvature { measure, limit } => {
            let sign = if *measure == Measure::Gaussian {
                "positive on a dome or in a bowl, negative on a saddle, none where a \
                 surface unrolls flat"
            } else {
                "negative where they bulge out, positive in a hollow"
            };
            form.hint(&format!(
                "{body}, painted by how its surfaces bend: {sign}."
            ));
            let options: Vec<(Measure, &str)> =
                Measure::ALL.iter().map(|m| (*m, m.label())).collect();
            form.select("Curvature", measure, &options);
            let mut auto = *limit <= 0.0;
            if form.check("Colour range from the body", &mut auto) {
                *limit = if auto {
                    0.0
                } else {
                    range.map_or(0.1, |(lo, hi)| lo.abs().max(hi.abs()).max(1e-6))
                };
            }
            if !auto {
                let shape = Quantity {
                    unit: Some(measure.unit().into()),
                    min: Some(1e-6),
                    max: Some(1e3),
                    ..Default::default()
                };
                let mut value = f64::from(*limit as f32);
                if form.number("Colours reach", &mut value, shape) {
                    *limit = f64::from(value as f32);
                }
            }
            form.gap();
            let mut legend = Vec::new();
            for (color, text) in [
                (palette.analysis_low, "−"),
                (palette.analysis_mid, "0"),
                (palette.analysis_high, "+"),
            ] {
                let key = Swatch {
                    color: srgb(color),
                    chosen: false,
                };
                legend.push(
                    Swatches {
                        swatches: vec![key],
                        keys: true,
                        ..Default::default()
                    }
                    .into(),
                );
                legend.push(
                    Caption {
                        text: text.into(),
                        small: true,
                        mono: true,
                        ..Default::default()
                    }
                    .into(),
                );
            }
            form.push(Flow {
                children: legend,
                no_wrap: true,
                ..Default::default()
            });
            match range {
                Some((low, high)) => form.hint(&format!(
                    "From {low:.4} to {high:.4} {} over the body.",
                    measure.unit()
                )),
                None => form.hint("The kernel read no curvature of the body."),
            }
        }
        Display::Zebra { stripes, axis } => {
            form.hint(&format!(
                "{body}, striped by the way its surfaces face. A stripe kinks where \
                 faces meet at a crease (G0), bends sharply where they meet tangent \
                 (G1), and runs on smoothly where they bend alike (G2)."
            ));
            let mut count = f64::from(*stripes);
            let shape = Quantity {
                dim: bench_api::Dim::Number,
                min: Some(1.0),
                max: Some(64.0),
                ..Default::default()
            };
            if form.number("Stripes", &mut count, shape) {
                *stripes = count.round().clamp(1.0, 64.0) as u32;
            }
            let options: Vec<(StripeAxis, &str)> =
                StripeAxis::ALL.iter().map(|a| (*a, a.label())).collect();
            form.select("Along", axis, &options);
        }
    }
}

/// A palette colour as sRGB bytes.
fn srgb(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (v * 255.0).round() as u8)
}

/// The Surface preferences page, declared.
#[cfg(feature = "egui")]
pub fn settings(options: &crate::Options) -> Vec<Widget> {
    vec![Widget::Settings(bench_api::SettingsGroup {
        title: "New surfaces".into(),
        children: vec![Widget::Hinted {
            hint: "Keep the sketches a new surface is built from out of the view".into(),
            widget: Box::new(
                bench_api::Check {
                    id: "hide_used_sketches".into(),
                    text: "Hide the sketch a surface uses".into(),
                    on: options.hide_used_sketches,
                    ..Default::default()
                }
                .into(),
            ),
        }],
    })]
}

/// What a change on the Preferences page sets.
#[cfg(feature = "egui")]
pub fn settings_event(options: &mut crate::Options, event: &PanelEvent) {
    if let PanelEvent::Toggle { id, on } = event
        && id == "hide_used_sketches"
    {
        options.hide_used_sketches = *on;
    }
}
