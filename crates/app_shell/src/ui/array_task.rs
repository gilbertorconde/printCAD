//! The array task: a body repeated along a line, around an axis, along a
//! path or at points, each repeat a linked copy the array owns. Every
//! change applies at once, so the view shows the copies as the numbers
//! move; OK keeps the array as one undo step, Cancel puts back the array
//! as it was (or takes away the one the task made).

use core_document::{
    ArrayId, ArrayPattern, BodyId, ComponentId, CopyArray, Document, DocumentService, FeatureId,
    PathSource, Recorded,
};
use egui::{RichText, Ui};
use serde_json::{Value, json};
use ui_kit::sans;
use ui_kit::tokens::*;
use ui_kit::widgets::{QtyField, check_row, overline, secondary_button, select_field};

use super::host_tasks::{HostTaskEnd, HostTaskInputs};
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
            if let Some(id) = self.array {
                document.restore_copy_array(id, self.start.clone());
            }
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
        let mut changed = false;
        note(
            ui,
            "Copies of the body that follow every change to it, placed in a row, a ring, \
             along a path or at points from where it sits. Moving the body carries them.",
        );
        heading(ui, "Kind");
        let mut kind = self.kind;
        if select_field(ui, "array_kind", &mut kind, KINDS, 200.0) {
            self.choose_kind(kind, document, registry);
            changed = true;
        }
        heading(ui, "Count");
        ui.horizontal(|ui| {
            label(ui, "Occurrences");
            changed |= QtyField::new(&mut self.count)
                .range(1.0..=1000.0)
                .decimals(0)
                .speed(0.05)
                .show(ui);
        });
        note(ui, "The body itself counted: 3 is the body and two copies.");
        match self.kind {
            Kind::Ring => {
                heading(ui, "Ring");
                ui.horizontal(|ui| {
                    label(ui, "Angle");
                    changed |= QtyField::degrees(&mut self.angle)
                        .range(-360.0..=360.0)
                        .show(ui);
                });
                if secondary_button(ui, "Spread over a whole turn").clicked() {
                    self.angle = 360.0 / self.count() as f32;
                    changed = true;
                }
                changed |= way(ui, "array_axis", &mut self.axis, "Axis");
                ui.add_space(SPACE_1);
                label(ui, "A point on the axis");
                changed |= vector(ui, "array_center", &mut self.center);
            }
            Kind::Row => {
                heading(ui, "Row");
                ui.horizontal(|ui| {
                    label(ui, "Spacing");
                    changed |= QtyField::offset(&mut self.spacing).show(ui);
                });
                note(ui, "Or drag the arrow on the first copy in the view.");
                if secondary_button(ui, "10 mm apart").clicked() {
                    self.spacing = document.spacing_clear_of(self.source, self.direction);
                    changed = true;
                }
                changed |= way(ui, "array_direction", &mut self.direction, "Direction");
            }
            Kind::Path => {
                changed |= self.path_section(ui, document, registry, picked_edges);
            }
            Kind::Points => {
                changed |= self.points_section(ui, document, registry);
            }
        }
        if !self.skipped.is_empty() {
            heading(ui, "Skipped");
            let list: Vec<String> = self.skipped.iter().map(u32::to_string).collect();
            note(
                ui,
                &format!(
                    "Cop{} {} left out (deleting a copy skips it).",
                    if list.len() == 1 { "y" } else { "ies" },
                    list.join(", ")
                ),
            );
            if secondary_button(ui, "Bring them back").clicked() {
                self.skipped.clear();
                changed = true;
            }
        }
        if let Some(why) = &self.refused {
            ui.add_space(SPACE_2);
            ui.label(RichText::new(why).font(sans(FONT_SM)).color(DANGER));
        }
        if changed {
            self.apply(document, registry);
        }
        None
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
    /// turn. Whether one changed.
    fn path_section(
        &mut self,
        ui: &mut Ui,
        document: &Document,
        registry: &DocumentService,
        picked_edges: &[(BodyId, [f32; 3])],
    ) -> bool {
        let mut changed = false;
        heading(ui, "Path");
        let all = layouts(document, registry);
        let options: Vec<(Option<FeatureId>, String)> = all
            .iter()
            .filter(|l| l.chains > 0)
            .map(|l| (Some(l.feature), l.name.clone()))
            .chain(std::iter::once((None, "An edge of a solid".to_string())))
            .collect();
        let mut sketch = match self.along {
            Some(PathSource::Sketch { feature, .. }) => Some(feature),
            _ => None,
        };
        ui.horizontal(|ui| {
            label(ui, "Along");
            if pick_list(ui, "array_path", &mut sketch, &options) {
                self.along = sketch.map(|feature| PathSource::Sketch { feature, chain: 0 });
                changed = true;
            }
        });
        if let Some(PathSource::Sketch { feature, chain }) = &mut self.along {
            let chains = all
                .iter()
                .find(|l| l.feature == *feature)
                .map_or(0, |l| l.chains);
            if chains > 1 {
                let mut which = (*chain + 1) as f32;
                ui.horizontal(|ui| {
                    label(ui, "Chain");
                    if QtyField::new(&mut which)
                        .range(1.0..=chains as f64)
                        .decimals(0)
                        .speed(0.05)
                        .show(ui)
                    {
                        *chain = which.round().max(1.0) as usize - 1;
                        changed = true;
                    }
                });
                note(
                    ui,
                    &format!("The sketch has {chains} separate chains of curves."),
                );
            }
        } else {
            match self.along {
                Some(PathSource::Edge { body, .. }) => note(
                    ui,
                    &format!(
                        "Along an edge of {}.",
                        document
                            .bodies()
                            .iter()
                            .find(|b| b.id == body)
                            .map_or("a body", |b| b.name.as_str())
                    ),
                ),
                _ => note(ui, "Select an edge in the view, then take it."),
            }
            let edge = picked_edges.last().copied();
            let take = ui
                .add_enabled_ui(edge.is_some(), |ui| {
                    secondary_button(ui, "Use the selected edge")
                })
                .inner
                .clicked();
            if take && let Some((body, at)) = edge {
                self.along = Some(PathSource::Edge {
                    body,
                    point: document.body_placement(body).inverse().point(at),
                });
                changed = true;
            }
        }
        if let Some(length) = self.path_length.filter(|_| self.along.is_some()) {
            note(ui, &format!("The path is {length:.2} mm long."));
        }
        ui.add_space(SPACE_1);
        changed |= check_row(ui, &mut self.spread, "Spread over the whole path").changed();
        if !self.spread {
            ui.horizontal(|ui| {
                label(ui, "Spacing");
                changed |= QtyField::offset(&mut self.path_spacing).show(ui);
            });
        }
        ui.add_space(SPACE_1);
        changed |= check_row(ui, &mut self.align, "Turn each copy with the path").changed();
        note(
            ui,
            "The body stands at the path's start and the copies follow it by length. The \
             path is read again whenever the array changes.",
        );
        if self.along.is_some() && secondary_button(ui, "Read the path again").clicked() {
            changed = true;
        }
        changed
    }

    /// The points' settings: the sketch they come from. Whether it
    /// changed.
    fn points_section(
        &mut self,
        ui: &mut Ui,
        document: &Document,
        registry: &DocumentService,
    ) -> bool {
        let mut changed = false;
        heading(ui, "Points");
        let options: Vec<(Option<FeatureId>, String)> = layouts(document, registry)
            .into_iter()
            .filter(|l| l.points > 0)
            .map(|l| (Some(l.feature), format!("{} ({} points)", l.name, l.points)))
            .collect();
        if options.is_empty() {
            note(
                ui,
                "No sketch has points of its own: draw points (not curve ends) in a sketch \
                 to place copies at.",
            );
            return false;
        }
        let mut from = self.points_from;
        ui.horizontal(|ui| {
            label(ui, "At");
            if pick_list(ui, "array_points", &mut from, &options) {
                self.points_from = from;
                self.count_every_point(document, registry);
                changed = true;
            }
        });
        note(
            ui,
            "The body stands at the first point and a copy at each of the others. The points \
             are read again whenever the array changes.",
        );
        if secondary_button(ui, "Every point").clicked() {
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

/// A choice among `options` by their labels; whether it changed.
fn pick_list(
    ui: &mut Ui,
    id: &str,
    value: &mut Option<FeatureId>,
    options: &[(Option<FeatureId>, String)],
) -> bool {
    let labels: Vec<(Option<FeatureId>, &str)> =
        options.iter().map(|(v, l)| (*v, l.as_str())).collect();
    select_field(ui, id, value, &labels, 200.0)
}

fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(sans(FONT_SM)).color(TEXT3));
}

fn label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(sans(FONT_SM)).color(TEXT2));
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(SPACE_3);
    overline(ui, text);
    ui.add_space(SPACE_1);
}

/// X, Y, Z or a direction of its own, typed; whether it changed.
fn way(ui: &mut Ui, id: &str, value: &mut [f32; 3], title: &str) -> bool {
    let mut changed = false;
    let mut chosen = Way::of(*value);
    ui.add_space(SPACE_1);
    ui.horizontal(|ui| {
        label(ui, title);
        if select_field(ui, id, &mut chosen, WAYS, 160.0) {
            if let Some(v) = chosen.vector() {
                *value = v;
            } else {
                // A slant to start from, for the numbers to change.
                *value = [1.0, 1.0, 0.0];
            }
            changed = true;
        }
    });
    if chosen == Way::Other {
        changed |= vector(ui, id, value);
    }
    changed
}

/// Three numbers, X, Y and Z; whether one changed.
fn vector(ui: &mut Ui, id: &str, value: &mut [f32; 3]) -> bool {
    let mut changed = false;
    egui::Grid::new((id, "xyz"))
        .num_columns(2)
        .spacing([SPACE_3, SPACE_1])
        .show(ui, |ui| {
            for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                label(ui, axis);
                changed |= QtyField::offset(&mut value[i]).show(ui);
                ui.end_row();
            }
        });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `f` in a frame of a panel, as the task panel would.
    fn in_frame(f: impl FnOnce(&mut Ui)) {
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        let mut f = Some(f);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(f) = f.take() {
                f(ui);
            }
        });
        output.textures_delta.clear();
    }

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
                },
                accept,
                cancel,
            );
        });
        end
    }

    #[test]
    fn the_task_makes_a_row_and_cancel_takes_it_away() {
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
        let end = frame(&mut task, &mut doc, &registry, false, true);
        assert!(matches!(end, Some(HostTaskEnd::Cancelled)));
        assert_eq!(doc.bodies().len(), 1, "only the body is left");
        assert!(doc.copy_arrays().is_empty());
    }

    #[test]
    fn a_copy_opens_its_array_and_cancel_puts_it_back() {
        let registry = DocumentService::default();
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let row = ArrayPattern::Linear {
            direction: [0.0, 1.0, 0.0],
            spacing: 7.0,
        };
        let id = doc.create_copy_array(body, row, 3).unwrap();
        let before = doc.copy_array(id).cloned();
        let copy = before.as_ref().unwrap().instances[1].body;
        let mut task = ArrayTask::open(&doc, copy);
        assert_eq!(task.source(), body);
        assert_eq!(task.spacing, 7.0);
        task.count = 2.0;
        task.apply(&mut doc, &registry);
        assert_eq!(doc.bodies().len(), 2);
        let end = frame(&mut task, &mut doc, &registry, false, true);
        assert!(matches!(end, Some(HostTaskEnd::Cancelled)));
        assert_eq!(doc.copy_array(id).cloned(), before, "the same copies again");
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
    fn the_views_handle_changes_a_rows_spacing_and_cancel_puts_it_back() {
        let registry = DocumentService::default();
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let row = ArrayPattern::Linear {
            direction: [0.0, 1.0, 0.0],
            spacing: 7.0,
        };
        let id = doc.create_copy_array(body, row, 3).unwrap();
        let before = doc.copy_array(id).cloned();
        let mut task = ArrayTask::open(&doc, body);
        assert_eq!(task.row_handle(), Some((id, [0.0, 1.0, 0.0], 7.0)));
        task.drag_spacing(&mut doc, 12.5);
        assert_eq!(
            doc.copy_array(id).map(|a| a.pattern.clone()),
            Some(ArrayPattern::Linear {
                direction: [0.0, 1.0, 0.0],
                spacing: 12.5
            })
        );
        frame(&mut task, &mut doc, &registry, false, true);
        assert_eq!(doc.copy_array(id).cloned(), before);
        task.choose_kind(Kind::Ring, &doc, &registry);
        assert_eq!(task.row_handle(), None, "a ring has no spacing handle");
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
}
