//! The array task: a body repeated along a line, around an axis, along a
//! path or at points, each repeat a linked copy the array owns. Every
//! change applies at once, so the view shows the copies as the numbers
//! move; OK keeps the array as one undo step, Cancel puts back the array
//! as it was (or takes away the one the task made).

use bench_api::{ButtonStyle, Dim, GapSize, PanelEvent, Quantity, Tone, Widget};
use core_document::{
    ArrayId, ArrayPattern, BodyId, ComponentId, CopyArray, Document, DocumentService, FeatureId,
    PathSource, Recorded,
};
use egui::Ui;
use serde_json::{Value, json};

use super::host_tasks::{HostTaskEnd, HostTaskInputs};
use super::task_blocks::{
    Form, caption, check, choice, degrees, draw, gap, grid, heading_in, label, note, number,
    offset, quantity, row, secondary, select,
};
use crate::app::array_paths;

/// Which way a row runs, or the axis a ring turns about.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Way {
    X,
    Y,
    Z,
    Other,
}

impl Way {
    fn of(v: [f32; 3]) -> Self {
        let n = glam::Vec3::from_array(v).normalize_or_zero();
        if (n - glam::Vec3::X).length() < 1e-6 {
            Self::X
        } else if (n - glam::Vec3::Y).length() < 1e-6 {
            Self::Y
        } else if (n - glam::Vec3::Z).length() < 1e-6 {
            Self::Z
        } else {
            Self::Other
        }
    }

    fn vector(self) -> Option<[f32; 3]> {
        match self {
            Self::X => Some([1.0, 0.0, 0.0]),
            Self::Y => Some([0.0, 1.0, 0.0]),
            Self::Z => Some([0.0, 0.0, 1.0]),
            Self::Other => None,
        }
    }
}

const WAYS: &[(Way, &str)] = &[
    (Way::X, "Along X"),
    (Way::Y, "Along Y"),
    (Way::Z, "Along Z"),
    (Way::Other, "Other direction"),
];

/// How the copies are laid out.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Row,
    Ring,
    Path,
    Points,
}

const KINDS: &[(Kind, &str)] = &[
    (Kind::Row, "Row (linear)"),
    (Kind::Ring, "Ring (polar)"),
    (Kind::Path, "Along a path"),
    (Kind::Points, "At points"),
];

#[derive(Debug, Clone)]
pub struct ArrayTask {
    source: BodyId,
    /// The component repeated, every body in it copied together.
    component: Option<ComponentId>,
    /// The array, once there is one: the task makes it on its first frame
    /// when it was opened on a body without one.
    array: Option<ArrayId>,
    /// The array as it was when the task opened; `None` when the task
    /// made it.
    start: Option<CopyArray>,
    kind: Kind,
    count: f32,
    direction: [f32; 3],
    spacing: f32,
    center: [f32; 3],
    axis: [f32; 3],
    angle: f32,
    /// What a path array runs along, once picked.
    along: Option<PathSource>,
    /// Spread over the whole path, rather than `path_spacing` apart.
    spread: bool,
    path_spacing: f32,
    align: bool,
    /// The path's length, as last read.
    path_length: Option<f64>,
    /// The feature whose points a point array stands at, once picked.
    points_from: Option<FeatureId>,
    skipped: Vec<u32>,
    refused: Option<String>,
}

impl ArrayTask {
    /// The task for `body`: the array that owns it or repeats it, or a
    /// new row of three of it.
    pub fn open(document: &Document, body: BodyId) -> Self {
        let existing = document
            .array_instance(body)
            .map(|(a, _)| a.id)
            .or_else(|| document.arrays_of_source(body).first().copied())
            .and_then(|id| document.copy_array(id).cloned());
        Self::on(document, body, None, existing)
    }

    /// The task for `component`: the array that repeats it, or a new row
    /// of three of it. `None` when it holds no body.
    pub fn open_component(document: &Document, component: ComponentId) -> Option<Self> {
        let existing = document
            .array_of_component(component)
            .and_then(|id| document.copy_array(id).cloned());
        let source = match &existing {
            Some(array) => array.source,
            None => *document.component_bodies(component).first()?,
        };
        Some(Self::on(document, source, Some(component), existing))
    }

    fn on(
        document: &Document,
        body: BodyId,
        component: Option<ComponentId>,
        existing: Option<CopyArray>,
    ) -> Self {
        let mut task = Self {
            source: body,
            component,
            array: None,
            start: None,
            kind: Kind::Row,
            count: 3.0,
            direction: [1.0, 0.0, 0.0],
            spacing: document.spacing_clear_of(body, [1.0, 0.0, 0.0]),
            center: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            angle: 120.0,
            along: None,
            spread: true,
            path_spacing: 20.0,
            align: false,
            path_length: None,
            points_from: None,
            skipped: Vec::new(),
            refused: None,
        };
        if let Some(array) = existing {
            task.source = array.source;
            task.component = array.component;
            task.array = Some(array.id);
            task.count = array.count as f32;
            task.skipped = array.skipped.clone();
            match &array.pattern {
                ArrayPattern::Linear { direction, spacing } => {
                    task.direction = *direction;
                    task.spacing = *spacing;
                }
                ArrayPattern::Polar {
                    center,
                    axis,
                    angle,
                } => {
                    task.kind = Kind::Ring;
                    task.center = *center;
                    task.axis = *axis;
                    task.angle = *angle;
                }
                ArrayPattern::Path {
                    along,
                    spacing,
                    align,
                    ..
                } => {
                    task.kind = Kind::Path;
                    task.along = Some(*along);
                    task.spread = spacing.is_none();
                    task.path_spacing = spacing.unwrap_or(task.path_spacing);
                    task.align = *align;
                }
                ArrayPattern::Points { from, .. } => {
                    task.kind = Kind::Points;
                    task.points_from = Some(*from);
                }
            }
            task.start = Some(array);
        }
        task
    }

    pub fn source(&self) -> BodyId {
        self.source
    }

    /// What it repeats, by name: the component, or the body.
    pub fn subject(&self, document: &Document) -> String {
        match self.component.and_then(|c| document.component(c)) {
            Some(component) => component.name.clone(),
            None => document
                .bodies()
                .iter()
                .find(|b| b.id == self.source)
                .map(|b| b.name.clone())
                .unwrap_or_default(),
        }
    }

    /// The row the view's handle drags: the array, its direction and its
    /// spacing. `None` while the copies stand some other way.
    pub fn row_handle(&self) -> Option<(ArrayId, [f32; 3], f32)> {
        let array = self.array.filter(|_| self.kind == Kind::Row)?;
        Some((array, self.direction, self.spacing))
    }

    /// The row's spacing as the view's handle drags it, applied at once.
    pub fn drag_spacing(&mut self, document: &mut Document, spacing: f32) {
        let Some(id) = self.array.filter(|_| self.kind == Kind::Row) else {
            return;
        };
        self.spacing = spacing;
        self.refused = document
            .set_copy_array(id, self.row(), self.count(), self.skipped.clone())
            .err()
            .map(|e| format!("The count {e}."));
    }

    fn row(&self) -> ArrayPattern {
        ArrayPattern::Linear {
            direction: self.direction,
            spacing: self.spacing,
        }
    }

    fn pattern(
        &self,
        document: &Document,
        registry: &DocumentService,
    ) -> Result<ArrayPattern, String> {
        match self.kind {
            Kind::Row => Ok(self.row()),
            Kind::Ring => Ok(ArrayPattern::Polar {
                center: self.center,
                axis: self.axis,
                angle: self.angle,
            }),
            Kind::Path => {
                let along = self
                    .along
                    .ok_or("Choose a sketch, or select an edge, for the path.")?;
                let spacing = (!self.spread).then_some(self.path_spacing);
                array_paths::path_pattern(
                    document,
                    registry,
                    along,
                    self.count(),
                    spacing,
                    self.align,
                )
            }
            Kind::Points => {
                let from = self.points_from.ok_or("Choose a sketch with points.")?;
                array_paths::points_pattern(document, registry, from)
            }
        }
    }

    fn count(&self) -> u32 {
        self.count.round().clamp(1.0, 1000.0) as u32
    }

    /// The document brought to the task's values: the array made, or
    /// changed.
    fn apply(&mut self, document: &mut Document, registry: &DocumentService) {
        if let Some(along) = self.along {
            self.path_length = array_paths::path_length(document, registry, along).ok();
        }
        let pattern = match self.pattern(document, registry) {
            Ok(pattern) => pattern,
            Err(why) => {
                self.refused = Some(why);
                return;
            }
        };
        let result = match self.array {
            None => match self.component {
                Some(component) => {
                    document.create_component_array(component, pattern, self.count())
                }
                None => document.create_copy_array(self.source, pattern, self.count()),
            }
            .map(|id| {
                self.array = Some(id);
            }),
            Some(id) => document.set_copy_array(id, pattern, self.count(), self.skipped.clone()),
        };
        self.refused = result.err().map(|e| format!("The count {e}."));
    }

    /// The calls a recording keeps for the array as it stands.
    fn recorded(&self, document: &Document) -> Vec<Recorded> {
        let mut args = match self.kind {
            Kind::Row => json!({
                "kind": "linear",
                "direction": self.direction,
                "spacing": self.spacing,
            }),
            Kind::Ring => json!({
                "kind": "polar",
                "center": self.center,
                "axis": self.axis,
                "angle": self.angle,
            }),
            Kind::Path => {
                let mut args = json!({"kind": "path", "align": self.align});
                match self.along {
                    Some(PathSource::Sketch { feature, chain }) => {
                        args["sketch"] = json!(feature.0.to_string());
                        if chain > 0 {
                            args["chain"] = json!(chain + 1);
                        }
                    }
                    Some(PathSource::Edge { body, point }) => {
                        let at = document.body_placement(body).point(point);
                        args["edge"] = json!({
                            "body": body.0.to_string(),
                            "point": {"x": at[0], "y": at[1], "z": at[2]},
                        });
                    }
                    None => {}
                }
                if !self.spread {
                    args["spacing"] = json!(self.path_spacing);
                }
                args
            }
            Kind::Points => json!({
                "kind": "points",
                "sketch": self.points_from.map(|f| f.0.to_string()),
            }),
        };
        args["count"] = json!(self.count());
        if !self.skipped.is_empty() {
            args["skip"] = json!(self.skipped);
        }
        let id = match (&self.start, self.array) {
            (Some(_), Some(array)) => {
                args["array"] = json!(array.0.to_string());
                "doc.set_array"
            }
            _ => {
                match self.component {
                    Some(component) => args["component"] = json!(component.0.to_string()),
                    None => args["body"] = json!(self.source.0.to_string()),
                }
                "doc.array"
            }
        };
        vec![Recorded {
            id: id.to_string(),
            args: match args {
                Value::Object(map) => map,
                _ => Default::default(),
            },
            result: Value::Null,
        }]
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        inputs: HostTaskInputs<'_>,
        accept: bool,
        cancel: bool,
    ) -> Option<HostTaskEnd> {
        let HostTaskInputs {
            document,
            registry,
            picked_edges,
            ..
        } = inputs;
        if cancel {
            return Some(HostTaskEnd::Cancelled);
        }
        if self.array.is_none() {
            self.apply(document, registry);
        }
        if accept {
            let name = self
                .array
                .and_then(|id| document.copy_array(id))
                .map(|a| a.name.clone())
                .unwrap_or_default();
            return Some(HostTaskEnd::Accepted(
                format!("Array {name}"),
                self.recorded(document),
            ));
        }
        let widgets = self.widgets(document, registry, picked_edges);
        let mut changed = false;
        for event in draw(ui, document, widgets) {
            changed |= self.event(&event, document, registry, picked_edges);
        }
        if changed {
            self.apply(document, registry);
        }
        None
    }

    fn widgets(
        &self,
        document: &Document,
        registry: &DocumentService,
        picked_edges: &[(BodyId, [f32; 3])],
    ) -> Vec<Widget> {
        let mut form = Form::declare();
        self.clone()
            .walk(&mut form, document, registry, picked_edges);
        form.finish()
    }

    /// Whether the event changed the array's values.
    fn event(
        &mut self,
        event: &PanelEvent,
        document: &Document,
        registry: &DocumentService,
        picked_edges: &[(BodyId, [f32; 3])],
    ) -> bool {
        self.walk(&mut Form::apply(event), document, registry, picked_edges)
    }

    /// The panel, declared, or with the event applied to the field it
    /// names; whether the event changed the array's values.
    fn walk(
        &mut self,
        form: &mut Form,
        document: &Document,
        registry: &DocumentService,
        picked_edges: &[(BodyId, [f32; 3])],
    ) -> bool {
        let mut changed = false;
        form.push(note(
            "Copies of the body that follow every change to it, placed in a row, a ring, \
             along a path or at points from where it sits. Moving the body carries them.",
        ));
        heading_in(form, "Kind");
        let mut kind = KINDS.iter().position(|(k, _)| *k == self.kind).unwrap_or(0);
        let picked = choice(
            form,
            &mut kind,
            select("kind", "", KINDS.iter().map(|(_, l)| *l), 0, 200.0),
        );
        if form.put(picked)
            && let Some((kind, _)) = KINDS.get(kind)
        {
            self.choose_kind(*kind, document, registry);
            changed = true;
        }
        heading_in(form, "Count");
        let count = number(
            form,
            &mut self.count,
            Quantity {
                min: Some(1.0),
                max: Some(1000.0),
                decimals: 0,
                speed: Some(0.05),
                ..quantity("count", "", 0.0, Dim::Number)
            },
        );
        changed |= count.set;
        form.push(row([label("Occurrences"), count.widget.into()]));
        form.push(note(
            "The body itself counted: 3 is the body and two copies.",
        ));
        match self.kind {
            Kind::Ring => {
                heading_in(form, "Ring");
                let angle = number(
                    form,
                    &mut self.angle,
                    degrees("angle", "", 0.0, Some((-360.0, 360.0))),
                );
                changed |= angle.set;
                form.push(row([label("Angle"), angle.widget.into()]));
                let whole = secondary(form, "whole_turn", "Spread over a whole turn");
                if form.put(whole) {
                    self.angle = 360.0 / self.count() as f32;
                    changed = true;
                }
                changed |= way(form, "axis", &mut self.axis, "Axis");
                form.push(gap(GapSize::Small));
                form.push(label("A point on the axis"));
                changed |= vector(form, "center", &mut self.center);
            }
            Kind::Row => {
                heading_in(form, "Row");
                let spacing = number(form, &mut self.spacing, offset("spacing", "", 0.0));
                changed |= spacing.set;
                form.push(row([label("Spacing"), spacing.widget.into()]));
                form.push(note("Or drag the arrow on the first copy in the view."));
                let clear = secondary(form, "clear_spacing", "10 mm apart");
                if form.put(clear) {
                    self.spacing = document.spacing_clear_of(self.source, self.direction);
                    changed = true;
                }
                changed |= way(form, "direction", &mut self.direction, "Direction");
            }
            Kind::Path => changed |= self.path_walk(form, document, registry, picked_edges),
            Kind::Points => changed |= self.points_walk(form, document, registry),
        }
        if !self.skipped.is_empty() {
            heading_in(form, "Skipped");
            let list: Vec<String> = self.skipped.iter().map(u32::to_string).collect();
            form.push(note(format!(
                "Cop{} {} left out (deleting a copy skips it).",
                if list.len() == 1 { "y" } else { "ies" },
                list.join(", ")
            )));
            let unskip = secondary(form, "unskip", "Bring them back");
            if form.put(unskip) {
                self.skipped.clear();
                changed = true;
            }
        }
        if let Some(why) = &self.refused {
            form.push(gap(GapSize::Medium));
            form.push(caption(why.clone(), Tone::Bad));
        }
        changed
    }

    /// A new kind, with what it needs to start from: a ring spread over a
    /// whole turn, the first sketch with curves for a path, the first with
    /// points (and every point counted) for points.
    fn choose_kind(&mut self, kind: Kind, document: &Document, registry: &DocumentService) {
        self.kind = kind;
        match kind {
            Kind::Ring => self.angle = 360.0 / self.count() as f32,
            Kind::Path if self.along.is_none() => {
                self.along = layouts(document, registry)
                    .into_iter()
                    .find(|l| l.chains > 0)
                    .map(|l| PathSource::Sketch {
                        feature: l.feature,
                        chain: 0,
                    });
            }
            Kind::Points => {
                if self.points_from.is_none() {
                    self.points_from = layouts(document, registry)
                        .into_iter()
                        .find(|l| l.points > 0)
                        .map(|l| l.feature);
                }
                self.count_every_point(document, registry);
            }
            _ => {}
        }
    }

    /// The path's settings: what it runs along, how the copies spread and
    /// turn; whether the event changed them.
    fn path_walk(
        &mut self,
        form: &mut Form,
        document: &Document,
        registry: &DocumentService,
        picked_edges: &[(BodyId, [f32; 3])],
    ) -> bool {
        let mut changed = false;
        heading_in(form, "Path");
        let options = path_options(document, registry);
        let sketch = match self.along {
            Some(PathSource::Sketch { feature, .. }) => Some(feature),
            _ => None,
        };
        // Not one of them: no option is shown as chosen.
        let mut chosen = options
            .iter()
            .position(|(f, _)| *f == sketch)
            .unwrap_or(usize::MAX);
        let path = choice(
            form,
            &mut chosen,
            select("path", "", options.iter().map(|(_, l)| l.clone()), 0, 200.0),
        );
        if path.set {
            self.along = options
                .get(chosen)
                .and_then(|(f, _)| *f)
                .map(|feature| PathSource::Sketch { feature, chain: 0 });
            changed = true;
        }
        form.push(row([label("Along"), path.widget.into()]));
        if let Some(PathSource::Sketch { feature, chain }) = &mut self.along {
            let chains = layouts(document, registry)
                .iter()
                .find(|l| l.feature == *feature)
                .map_or(0, |l| l.chains);
            if chains > 1 {
                let mut shown = (*chain + 1) as f32;
                let picked = number(
                    form,
                    &mut shown,
                    Quantity {
                        min: Some(1.0),
                        max: Some(chains as f64),
                        decimals: 0,
                        speed: Some(0.05),
                        ..quantity("chain", "", 0.0, Dim::Number)
                    },
                );
                if picked.set {
                    *chain = shown.round().max(1.0) as usize - 1;
                    changed = true;
                }
                form.push(row([label("Chain"), picked.widget.into()]));
                form.push(note(format!(
                    "The sketch has {chains} separate chains of curves."
                )));
            }
        } else {
            form.push(match self.along {
                Some(PathSource::Edge { body, .. }) => note(format!(
                    "Along an edge of {}.",
                    document
                        .bodies()
                        .iter()
                        .find(|b| b.id == body)
                        .map_or("a body", |b| b.name.as_str())
                )),
                _ => note("Select an edge in the view, then take it."),
            });
            let take = form.button(
                "take_edge",
                "Use the selected edge",
                ButtonStyle::Secondary,
                !picked_edges.is_empty(),
            );
            if form.put(take)
                && let Some((body, at)) = picked_edges.last().copied()
            {
                self.along = Some(PathSource::Edge {
                    body,
                    point: document.body_placement(body).inverse().point(at),
                });
                changed = true;
            }
        }
        if let Some(length) = self.path_length.filter(|_| self.along.is_some()) {
            form.push(note(format!("The path is {length:.2} mm long.")));
        }
        form.push(gap(GapSize::Small));
        let spread = check(
            form,
            "spread",
            "Spread over the whole path",
            &mut self.spread,
        );
        changed |= form.put(spread);
        if !self.spread {
            let spacing = number(
                form,
                &mut self.path_spacing,
                offset("path_spacing", "", 0.0),
            );
            changed |= spacing.set;
            form.push(row([label("Spacing"), spacing.widget.into()]));
        }
        form.push(gap(GapSize::Small));
        let align = check(
            form,
            "align",
            "Turn each copy with the path",
            &mut self.align,
        );
        changed |= form.put(align);
        form.push(note(
            "The body stands at the path's start and the copies follow it by length. The \
             path is read again whenever the array changes.",
        ));
        if self.along.is_some() {
            let reread = secondary(form, "reread", "Read the path again");
            changed |= form.put(reread);
        }
        changed
    }

    /// The points' settings: the sketch they come from; whether the
    /// event changed them.
    fn points_walk(
        &mut self,
        form: &mut Form,
        document: &Document,
        registry: &DocumentService,
    ) -> bool {
        heading_in(form, "Points");
        let options = point_options(document, registry);
        if options.is_empty() {
            form.push(note(
                "No sketch has points of its own: draw points (not curve ends) in a sketch \
                 to place copies at.",
            ));
            return false;
        }
        let mut changed = false;
        let mut chosen = options
            .iter()
            .position(|(f, _)| *f == self.points_from)
            .unwrap_or(usize::MAX);
        let points = choice(
            form,
            &mut chosen,
            select(
                "points",
                "",
                options.iter().map(|(_, l)| l.clone()),
                0,
                200.0,
            ),
        );
        if points.set {
            self.points_from = options.get(chosen).and_then(|(f, _)| *f);
            self.count_every_point(document, registry);
            changed = true;
        }
        form.push(row([label("At"), points.widget.into()]));
        form.push(note(
            "The body stands at the first point and a copy at each of the others. The points \
             are read again whenever the array changes.",
        ));
        let every = secondary(form, "every_point", "Every point");
        if form.put(every) {
            self.count_every_point(document, registry);
            changed = true;
        }
        changed
    }

    /// The count set to every point the chosen sketch has.
    fn count_every_point(&mut self, document: &Document, registry: &DocumentService) {
        if let Some(places) = self
            .points_from
            .and_then(|f| array_paths::points_pattern(document, registry, f).ok())
            .and_then(|p| p.places())
        {
            self.count = places as f32;
        }
    }
}

/// A feature that lays out curves or points an array can follow.
struct Layout {
    feature: FeatureId,
    name: String,
    chains: usize,
    points: usize,
}

/// Every feature that lays out curves or points, in history order.
fn layouts(document: &Document, registry: &DocumentService) -> Vec<Layout> {
    let mut nodes: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .map(|(id, node)| (node.seq, *id, node.name.clone()))
        .collect();
    nodes.sort_by_key(|(seq, ..)| *seq);
    nodes
        .into_iter()
        .filter_map(|(_, feature, name)| {
            let paths = registry.feature_paths(document, feature)?;
            Some(Layout {
                feature,
                name,
                chains: paths.chains.len(),
                points: paths.points.len(),
            })
        })
        .collect()
}

/// The sketches a path can run along, and an edge of a solid.
fn path_options(
    document: &Document,
    registry: &DocumentService,
) -> Vec<(Option<FeatureId>, String)> {
    layouts(document, registry)
        .into_iter()
        .filter(|l| l.chains > 0)
        .map(|l| (Some(l.feature), l.name))
        .chain(std::iter::once((None, "An edge of a solid".to_string())))
        .collect()
}

/// The sketches with points of their own, with how many.
fn point_options(
    document: &Document,
    registry: &DocumentService,
) -> Vec<(Option<FeatureId>, String)> {
    layouts(document, registry)
        .into_iter()
        .filter(|l| l.points > 0)
        .map(|l| (Some(l.feature), format!("{} ({} points)", l.name, l.points)))
        .collect()
}

/// X, Y, Z or a direction of its own, named `name`: the choice, and the
/// numbers for one of its own; whether the event changed `value`.
fn way(form: &mut Form, name: &str, value: &mut [f32; 3], title: &str) -> bool {
    let chosen = Way::of(*value);
    let mut at = WAYS.iter().position(|(w, _)| *w == chosen).unwrap_or(0);
    form.push(gap(GapSize::Small));
    let picked = choice(
        form,
        &mut at,
        select(name, "", WAYS.iter().map(|(_, l)| *l), 0, 160.0),
    );
    let mut changed = picked.set;
    if picked.set
        && let Some((way, _)) = WAYS.get(at)
    {
        // A slant to start from, for the numbers to change.
        *value = way.vector().unwrap_or([1.0, 1.0, 0.0]);
    }
    form.push(row([label(title), picked.widget.into()]));
    if chosen == Way::Other {
        changed |= vector(form, name, value);
    }
    changed
}

/// Three numbers, X, Y and Z, named `{name}.x` and so on; whether the
/// event changed one.
fn vector(form: &mut Form, name: &str, value: &mut [f32; 3]) -> bool {
    form.nest(
        |form| {
            let mut changed = false;
            for (i, axis) in ["x", "y", "z"].iter().enumerate() {
                let field = number(
                    form,
                    &mut value[i],
                    offset(&format!("{name}.{axis}"), &axis.to_uppercase(), 0.0),
                );
                changed |= form.put(field);
            }
            changed
        },
        |fields| grid(name, true, fields),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::task_blocks::in_frame;

    /// One frame of the task over `doc`.
    fn frame(
        task: &mut ArrayTask,
        doc: &mut Document,
        registry: &DocumentService,
        accept: bool,
        cancel: bool,
    ) -> Option<HostTaskEnd> {
        let mut end = None;
        in_frame(|ui| {
            end = task.show(
                ui,
                HostTaskInputs {
                    document: doc,
                    picked_faces: &[],
                    picked_edges: &[],
                    registry,
                    custom_colors: &[],
                    printing: &settings::PrintingSettings::default(),
                    print_layout: None,
                    marking: None,
                },
                accept,
                cancel,
            );
        });
        end
    }

    #[test]
    fn the_task_makes_a_row_and_records_it() {
        let registry = DocumentService::default();
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut task = ArrayTask::open(&doc, body);
        frame(&mut task, &mut doc, &registry, false, false);
        assert_eq!(doc.bodies().len(), 3, "a row of three to start");
        task.count = 5.0;
        task.apply(&mut doc, &registry);
        assert_eq!(doc.bodies().len(), 5);
        let end = frame(&mut task, &mut doc.clone(), &registry, true, false);
        let Some(HostTaskEnd::Accepted(_, ref calls)) = end else {
            panic!("accepted");
        };
        assert_eq!(calls[0].id, "doc.array");
        assert_eq!(calls[0].args["count"], json!(5));
    }

    /// Making an array, changing an array from one of its copies and
    /// dragging a row's spacing in the view each cancel back through
    /// the journal.
    #[test]
    fn every_array_edit_cancels_back() {
        let registry = DocumentService::default();
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let cancel = |task: &mut ArrayTask, doc: &mut Document| {
            let end = frame(task, doc, &registry, false, true);
            assert!(matches!(end, Some(HostTaskEnd::Cancelled)));
        };
        core_document::history::assert_cancel_takes_back(&mut doc, |doc| {
            let mut task = ArrayTask::open(doc, body);
            frame(&mut task, doc, &registry, false, false);
            task.count = 5.0;
            task.apply(doc, &registry);
            cancel(&mut task, doc);
        });
        let row = ArrayPattern::Linear {
            direction: [0.0, 1.0, 0.0],
            spacing: 7.0,
        };
        let id = doc.create_copy_array(body, row, 3).unwrap();
        let copy = doc.copy_array(id).unwrap().instances[1].body;
        core_document::history::assert_cancel_takes_back(&mut doc, |doc| {
            let mut task = ArrayTask::open(doc, copy);
            assert_eq!((task.source(), task.spacing), (body, 7.0));
            task.count = 2.0;
            task.apply(doc, &registry);
            assert_eq!(doc.bodies().len(), 2);
            cancel(&mut task, doc);
        });
        core_document::history::assert_cancel_takes_back(&mut doc, |doc| {
            let mut task = ArrayTask::open(doc, body);
            assert_eq!(task.row_handle(), Some((id, [0.0, 1.0, 0.0], 7.0)));
            task.drag_spacing(doc, 12.5);
            assert_eq!(
                doc.copy_array(id).map(|a| a.pattern.clone()),
                Some(ArrayPattern::Linear {
                    direction: [0.0, 1.0, 0.0],
                    spacing: 12.5
                })
            );
            cancel(&mut task, doc);
            task.choose_kind(Kind::Ring, doc, &registry);
            assert_eq!(task.row_handle(), None, "a ring has no spacing handle");
        });
    }

    #[test]
    fn a_components_task_repeats_every_body_and_records_the_component() {
        let registry = DocumentService::default();
        let mut doc = Document::new("t");
        let a = doc.create_body(None);
        let b = doc.create_body(None);
        let unit = doc.create_component("Pair".into(), None).unwrap();
        doc.set_body_component(a, Some(unit)).unwrap();
        doc.set_body_component(b, Some(unit)).unwrap();
        let mut task = ArrayTask::open_component(&doc, unit).expect("a task");
        assert_eq!(task.subject(&doc), "Pair");
        frame(&mut task, &mut doc, &registry, false, false);
        assert_eq!(doc.bodies().len(), 6, "a row of three of the pair");
        let end = frame(&mut task, &mut doc, &registry, true, false);
        let Some(HostTaskEnd::Accepted(_, ref calls)) = end else {
            panic!("accepted");
        };
        assert_eq!(calls[0].args["component"], json!(unit.0.to_string()));
        let copy = doc.copy_arrays()[0].instances[0].members[0].body;
        let again = ArrayTask::open(&doc, copy);
        assert_eq!(
            again.component,
            Some(unit),
            "a copy opens its component's array"
        );
    }

    #[test]
    fn a_path_without_a_sketch_or_edge_says_what_it_needs_and_keeps_the_array() {
        let registry = DocumentService::default();
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut task = ArrayTask::open(&doc, body);
        frame(&mut task, &mut doc, &registry, false, false);
        let row = doc.copy_arrays()[0].clone();
        task.choose_kind(Kind::Path, &doc, &registry);
        assert!(task.along.is_none(), "no sketch lays out a path");
        task.apply(&mut doc, &registry);
        assert!(task.refused.as_deref().unwrap_or("").contains("sketch"));
        assert_eq!(doc.copy_arrays()[0], row, "the row stays as it was");
    }

    /// The panel's kind, count, ring axis and centre change the array;
    /// an edge taken from the view becomes the path.
    #[test]
    fn panel_events_change_the_array() {
        let registry = DocumentService::default();
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut task = ArrayTask::open(&doc, body);
        frame(&mut task, &mut doc, &registry, false, false);
        let ring = KINDS.iter().position(|(k, _)| *k == Kind::Ring).unwrap();
        let edge = [(body, [1.0, 2.0, 3.0])];
        for event in [
            PanelEvent::Choice {
                id: "kind".into(),
                index: ring,
            },
            PanelEvent::Number {
                id: "count".into(),
                value: 4.0,
            },
            PanelEvent::Choice {
                id: "axis".into(),
                index: 0,
            },
            PanelEvent::Number {
                id: "center.y".into(),
                value: 7.0,
            },
            PanelEvent::Button {
                id: "whole_turn".into(),
            },
        ] {
            assert!(task.event(&event, &doc, &registry, &edge), "{event:?}");
        }
        task.apply(&mut doc, &registry);
        assert_eq!(
            doc.copy_arrays()[0].pattern,
            ArrayPattern::Polar {
                center: [0.0, 7.0, 0.0],
                axis: [1.0, 0.0, 0.0],
                angle: 90.0
            }
        );
        assert_eq!(doc.bodies().len(), 4);
        let widgets = task.widgets(&doc, &registry, &edge);
        let json = serde_json::to_string(&widgets).unwrap();
        assert!(json.contains("whole_turn") && json.contains("center.z"));
        let path = KINDS.iter().position(|(k, _)| *k == Kind::Path).unwrap();
        task.event(
            &PanelEvent::Choice {
                id: "kind".into(),
                index: path,
            },
            &doc,
            &registry,
            &edge,
        );
        task.event(
            &PanelEvent::Button {
                id: "take_edge".into(),
            },
            &doc,
            &registry,
            &edge,
        );
        assert_eq!(
            task.along,
            Some(PathSource::Edge {
                body,
                point: [1.0, 2.0, 3.0]
            })
        );
    }
}
