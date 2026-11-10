//! The application's own tasks in the right panel, beside the benches':
//! a body's placement, its appearance (colour, faces, see-through,
//! material), a note on it, a feature's place in its body's history, an
//! array of copies (`array_task`), and the print layout. Edits apply to
//! the document as they are made, so the view shows them; OK keeps them
//! as one undo step, Cancel puts back what was there when the task
//! opened. The print layout edits nothing: the view
//! shows the layout while it is open, and Close puts the model back.

use bench_api::{ButtonStyle, Dim, GapSize, NoteKind, PanelEvent, Tone, Widget};
use core_document::{
    BodyDisplay, BodyId, BodyPlacement, Document, FaceColor, FeatureId, Material, Note, NoteId,
    Recorded,
};
use egui::Ui;
use serde_json::{Value, json};

use super::UiCommand;
use super::task_blocks::{
    button, caption, degrees, draw, gap, grid, heading, hinted, indexed, label, length, note,
    offset, quantity, row, secondary, select, srgb,
};

/// The colours offered first, a row of neutrals and two of hues.
pub const PALETTE: &[[f32; 3]] = &[
    [0.97, 0.97, 0.97],
    [0.78, 0.79, 0.81],
    [0.58, 0.61, 0.66],
    [0.38, 0.40, 0.43],
    [0.20, 0.21, 0.23],
    [0.06, 0.06, 0.07],
    [0.86, 0.20, 0.20],
    [0.95, 0.50, 0.15],
    [0.96, 0.80, 0.18],
    [0.55, 0.78, 0.25],
    [0.20, 0.62, 0.32],
    [0.16, 0.66, 0.66],
    [0.20, 0.58, 0.90],
    [0.22, 0.34, 0.80],
    [0.52, 0.34, 0.84],
    [0.84, 0.36, 0.72],
    [0.93, 0.62, 0.66],
    [0.55, 0.38, 0.22],
];

/// Materials offered, densities in g/cm³.
pub const MATERIALS: &[(&str, f32)] = &[
    ("PLA", 1.24),
    ("PETG", 1.27),
    ("ABS", 1.04),
    ("ASA", 1.07),
    ("TPU", 1.21),
    ("Nylon", 1.14),
    ("Polycarbonate", 1.20),
    ("Resin", 1.15),
    ("Aluminium", 2.70),
    ("Steel", 7.85),
    ("Stainless steel", 8.00),
    ("Brass", 8.50),
    ("Copper", 8.96),
    ("Wood", 0.60),
];

/// A task the application runs in the right panel.
#[derive(Debug, Clone)]
pub enum HostTask {
    Placement(PlacementTask),
    Appearance(AppearanceTask),
    History(HistoryTask),
    Texture(super::texture_task::TextureTask),
    Note(Box<NoteTask>),
    /// Every part laid flat on the bed, for export or the slicer.
    PrintLayout,
    /// A body repeated as linked copies in a row or a ring.
    Array(Box<super::array_task::ArrayTask>),
    /// A scan's surface marked by hand.
    Mark(MarkTask),
}

/// The Mark surface task: the tool in hand, which the app reads each
/// frame. The marking itself is the app's, kept when the task closes.
#[derive(Debug, Clone)]
pub struct MarkTask {
    pub arm: crate::app::marking::MarkArm,
}

impl MarkTask {
    pub fn new(body: BodyId) -> Self {
        Self {
            arm: crate::app::marking::MarkArm {
                body,
                tool: crate::app::marking::MarkTool::Window,
                brush_mm: 5.0,
                through: false,
            },
        }
    }

    fn show(
        &mut self,
        ui: &mut Ui,
        document: &Document,
        marking: Option<(usize, usize)>,
        close: bool,
        commands: &mut Vec<UiCommand>,
    ) -> Option<HostTaskEnd> {
        if close {
            return Some(HostTaskEnd::Cancelled);
        }
        let widgets = self.widgets(marking);
        for event in draw(ui, document, widgets) {
            self.event(&event, commands);
        }
        None
    }

    fn widgets(&self, marking: Option<(usize, usize)>) -> Vec<Widget> {
        use crate::app::marking::MarkTool;
        let arm = &self.arm;
        let mut rows = vec![select(
            "tool",
            "Tool",
            MarkTool::ALL.map(|t| t.label()),
            MarkTool::ALL
                .iter()
                .position(|t| *t == arm.tool)
                .unwrap_or(0),
            180.0,
        )];
        if arm.tool == MarkTool::Brush {
            rows.push(length("brush", "Brush diameter", arm.brush_mm, 0.1, 500.0));
        }
        let (vertices, triangles) = marking.unwrap_or_default();
        vec![
            bench_api::SettingsGroup {
                title: "Marking".into(),
                children: rows,
            }
            .into(),
            bench_api::Check {
                id: "through".into(),
                text: "Include faces pointing away".into(),
                on: arm.through,
                ..Default::default()
            }
            .into(),
            row([secondary("invert", "Invert"), secondary("clear", "Clear")]),
            note(format!("{triangles} triangles marked ({vertices} points)")),
            note(
                "Drag over the scan to mark; drag with the right button or Alt to rub out; \
                 Shift-drag turns the view. The marking stays when the task closes.",
            ),
        ]
    }

    fn event(&mut self, event: &PanelEvent, commands: &mut Vec<UiCommand>) {
        use crate::app::marking::{MarkEdit, MarkTool};
        match event {
            PanelEvent::Choice { id, index } if id == "tool" => {
                if let Some(tool) = MarkTool::ALL.get(*index) {
                    self.arm.tool = *tool;
                }
            }
            PanelEvent::Number { id, value } if id == "brush" => self.arm.brush_mm = *value as f32,
            PanelEvent::Toggle { id, on } if id == "through" => self.arm.through = *on,
            PanelEvent::Button { id } if id == "invert" => {
                commands.push(UiCommand::MarkEdit(MarkEdit::Invert));
            }
            PanelEvent::Button { id } if id == "clear" => {
                commands.push(UiCommand::MarkEdit(MarkEdit::Clear));
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct NoteTask {
    body: BodyId,
    /// The note as it was when the task opened; `None` when the task
    /// makes it.
    start: Option<Note>,
    note: Note,
    /// The note is in the document: a new one goes in on the task's
    /// first frame.
    placed: bool,
}

#[derive(Debug, Clone)]
pub struct PlacementTask {
    body: BodyId,
    /// Every body that moves with it, where it was when the task opened.
    start: Vec<(BodyId, BodyPlacement)>,
    offset: [f32; 3],
    /// Turn about X, then Y, then Z, degrees.
    angles: [f32; 3],
    /// The axes a move by numbers goes along and turns about.
    frame: Frame,
    /// The point a move by numbers turns about.
    pivot: Pivot,
    /// The move by numbers: along the frame's X, Y and Z, millimetres.
    shift: [f32; 3],
    /// And about the frame's X, then Y, then Z through the pivot, degrees.
    turn: [f32; 3],
}

/// The axes a placement moves along by numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Frame {
    World,
    /// Another body's own axes, where it sits.
    Body(BodyId),
    /// A datum's axes, where its body sits: a plane's or a coordinate
    /// system's.
    Datum(FeatureId),
}

/// The point a placement turns about by numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pivot {
    /// The body's own origin.
    Origin,
    /// The middle of the box around the body.
    BoxCentre,
    /// The centre of the body's volume, from its mesh.
    Mass,
}

#[derive(Debug, Clone)]
pub struct AppearanceTask {
    body: BodyId,
    start_display: Option<BodyDisplay>,
    start_material: Option<Material>,
    start_faces: Vec<FaceColor>,
    /// The colour the custom picker holds.
    custom: [f32; 3],
    /// The material is typed in rather than picked from the list.
    own_material: bool,
    /// The faces being coloured: the one the task was opened for, until
    /// faces are selected in the view.
    faces: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct HistoryTask {
    feature: FeatureId,
    /// What it came after when the task opened; `None` when first.
    start_after: Option<FeatureId>,
    /// Why the last move was refused.
    refused: Option<String>,
}

/// How a task ended.
pub enum HostTaskEnd {
    /// Kept: the undo step's label and the calls a recording keeps.
    Accepted(String, Vec<Recorded>),
    Cancelled,
}

/// What a task reads of the moment.
pub struct HostTaskInputs<'a> {
    pub document: &'a mut Document,
    /// The faces selected in the view, in the order picked: each its body
    /// and index.
    pub picked_faces: &'a [(BodyId, u32)],
    /// The edges selected in the view: each its body and a point on it
    /// where the body sits.
    pub picked_edges: &'a [(BodyId, [f32; 3])],
    pub registry: &'a core_document::DocumentService,
    pub custom_colors: &'a [[f32; 3]],
    pub printing: &'a settings::PrintingSettings,
    /// The print layout the view shows, once made.
    pub print_layout: Option<&'a crate::app::print_layout::Layout>,
    /// How many vertices and triangles of the marked scan are marked.
    pub marking: Option<(usize, usize)>,
}

fn call(id: &str, args: Value) -> Recorded {
    Recorded {
        id: id.to_string(),
        args: match args {
            Value::Object(map) => map,
            _ => Default::default(),
        },
        result: Value::Null,
    }
}

fn body_name(document: &Document, body: BodyId) -> String {
    document
        .bodies()
        .iter()
        .find(|b| b.id == body)
        .map(|b| b.name.clone())
        .unwrap_or_default()
}

fn same_color(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

/// A mesh body's shadings, as the select offers them.
const SHADINGS: [&str; 3] = ["As meshed", "Sharp: every triangle flat", "Smooth"];

/// The palette, then the user's own colours under their caption, each
/// ringed when it is `current`; ids `{prefix}palette` and `{prefix}yours`.
fn swatches(prefix: &str, current: Option<[f32; 3]>, custom: &[[f32; 3]]) -> Vec<Widget> {
    let set = |id: &str, colors: &[[f32; 3]]| -> Widget {
        bench_api::Swatches {
            id: format!("{prefix}{id}"),
            swatches: colors
                .iter()
                .map(|c| bench_api::Swatch {
                    color: srgb(*c),
                    chosen: current.is_some_and(|now| same_color(now, *c)),
                })
                .collect(),
            keys: false,
        }
        .into()
    };
    let mut w = vec![set("palette", PALETTE)];
    if !custom.is_empty() {
        w.push(gap(GapSize::Small));
        w.push(
            bench_api::Caption {
                text: "Your colours".into(),
                tone: Tone::Muted,
                small: true,
                ..Default::default()
            }
            .into(),
        );
        w.push(set("yours", custom));
    }
    w
}

impl HostTask {
    pub fn placement(document: &Document, body: BodyId) -> Self {
        let placement = document.body_placement(body);
        let (x, y, z) = placement.quat().to_euler(glam::EulerRot::XYZ);
        Self::Placement(PlacementTask {
            body,
            start: document
                .rigid_unit_of(body)
                .into_iter()
                .map(|b| (b, document.body_placement(b)))
                .collect(),
            offset: placement.offset().to_array(),
            angles: [x, y, z].map(f32::to_degrees),
            frame: Frame::World,
            pivot: Pivot::Origin,
            shift: [0.0; 3],
            turn: [0.0; 3],
        })
    }

    pub fn appearance(document: &Document, body: BodyId, face: Option<u32>) -> Self {
        let entry = document.bodies().iter().find(|b| b.id == body);
        let display = entry.and_then(|b| b.display);
        let material = entry.and_then(|b| b.material.clone());
        let own_material = material
            .as_ref()
            .is_some_and(|m| !MATERIALS.iter().any(|(name, _)| *name == m.name));
        Self::Appearance(AppearanceTask {
            body,
            start_display: display,
            start_material: material,
            start_faces: entry.map(|b| b.face_colors.clone()).unwrap_or_default(),
            custom: display.unwrap_or_default().color,
            own_material,
            faces: face.into_iter().collect(),
        })
    }

    /// A new note on `body` at `at`, where the body sits in the
    /// document, or where the body offers one.
    pub fn add_note(document: &Document, body: BodyId, at: Option<[f32; 3]>) -> Self {
        Self::Note(Box::new(NoteTask {
            body,
            start: None,
            note: document.new_note(
                body,
                "Note".to_string(),
                core_document::NoteAuthor::User,
                at,
                core_document::NoteWant::Face,
            ),
            placed: false,
        }))
    }

    /// Note `id` to edit; `None` when there is no such note.
    pub fn edit_note(document: &Document, id: NoteId) -> Option<Self> {
        let (body, note) = document.note(id)?;
        Some(Self::Note(Box::new(NoteTask {
            body,
            start: Some(note.clone()),
            note: note.clone(),
            placed: true,
        })))
    }

    pub fn history(document: &Document, feature: FeatureId) -> Self {
        let order = document.body_history_of(feature);
        let at = order.iter().position(|f| *f == feature).unwrap_or(0);
        Self::History(HistoryTask {
            feature,
            start_after: at.checked_sub(1).map(|i| order[i]),
            refused: None,
        })
    }

    /// The panel's title.
    pub fn title(&self, document: &Document) -> String {
        match self {
            Self::Placement(t) => format!("Placement · {}", body_name(document, t.body)),
            Self::Appearance(t) => format!("Appearance · {}", body_name(document, t.body)),
            Self::Texture(t) => format!("Surface texture · {}", body_name(document, t.body())),
            Self::Note(t) => format!("Note · {}", body_name(document, t.body)),
            Self::PrintLayout => "Print layout".to_string(),
            Self::Mark(t) => format!("Mark surface · {}", body_name(document, t.arm.body)),
            Self::Array(t) => format!("Array · {}", t.subject(document)),
            Self::History(t) => format!(
                "Move · {}",
                document
                    .get_feature_meta(t.feature)
                    .map(|n| n.name.clone())
                    .unwrap_or_default()
            ),
        }
    }

    /// Whether what the task works on is still in the document.
    pub fn still_there(&self, document: &Document) -> bool {
        match self {
            Self::Placement(PlacementTask { body, .. })
            | Self::Appearance(AppearanceTask { body, .. }) => {
                document.bodies().iter().any(|b| b.id == *body)
            }
            Self::History(t) => document.get_feature_meta(t.feature).is_some(),
            Self::Texture(t) => document.bodies().iter().any(|b| b.id == t.body()),
            Self::Note(t) => !t.placed || document.note(t.note.id).is_some(),
            Self::PrintLayout => true,
            Self::Mark(t) => document.body_scan(t.arm.body).is_some(),
            Self::Array(t) => document.bodies().iter().any(|b| b.id == t.source()),
        }
    }

    /// Draw the task and apply its edits; how it ended, once it has.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        inputs: HostTaskInputs<'_>,
        accept: bool,
        cancel: bool,
        commands: &mut Vec<UiCommand>,
    ) -> Option<HostTaskEnd> {
        match self {
            Self::Placement(t) => t.show(ui, inputs.document, accept, cancel),
            Self::Appearance(t) => t.show(ui, inputs, accept, cancel, commands),
            Self::History(t) => t.show(ui, inputs.document, accept, cancel),
            Self::Texture(t) => t.show(
                ui,
                inputs.document,
                inputs.picked_faces,
                accept,
                cancel,
                commands,
            ),
            Self::Note(t) => t.show(ui, inputs.document, accept, cancel),
            Self::PrintLayout => print_layout(ui, &inputs, accept || cancel, commands),
            Self::Mark(t) => t.show(
                ui,
                inputs.document,
                inputs.marking,
                accept || cancel,
                commands,
            ),
            Self::Array(t) => t.show(ui, inputs, accept, cancel),
        }
    }

    /// Whether the task has OK and Cancel, rather than Close alone.
    pub fn confirmable(&self) -> bool {
        !matches!(self, Self::PrintLayout)
    }
}

/// The print layout's panel: how many copies on how many plates, what
/// does not fit, the gap, and export or the slicer with the layout.
fn print_layout(
    ui: &mut Ui,
    inputs: &HostTaskInputs<'_>,
    close: bool,
    commands: &mut Vec<UiCommand>,
) -> Option<HostTaskEnd> {
    if close {
        return Some(HostTaskEnd::Cancelled);
    }
    let widgets = print_layout_widgets(inputs.print_layout, inputs.printing);
    for event in draw(ui, inputs.document, widgets) {
        print_layout_event(&event, commands);
    }
    None
}

fn print_layout_widgets(
    layout: Option<&crate::app::print_layout::Layout>,
    printing: &settings::PrintingSettings,
) -> Vec<Widget> {
    let Some(layout) = layout else {
        return vec![note("Laying out…")];
    };
    let mut w = Vec::new();
    let card = |kind, title: &str, text: String| Widget::Note {
        kind,
        title: Some(title.into()),
        text,
    };
    if layout.pieces.is_empty() {
        w.push(card(
            NoteKind::Info,
            "Nothing to print",
            "No visible part has a count to print and a shape.".into(),
        ));
    } else {
        let s = |n: usize| if n == 1 { "" } else { "s" };
        let pieces = layout.pieces.len();
        w.push(Widget::Heading {
            text: format!(
                "{pieces} piece{} on {} plate{}",
                s(pieces),
                layout.plates,
                s(layout.plates)
            ),
        });
        if layout.plates > 1 {
            for plate in 0..layout.plates {
                let on = layout.pieces.iter().filter(|p| p.plate == plate).count();
                w.push(note(format!("Plate {}: {on} piece{}", plate + 1, s(on))));
            }
        }
    }
    for (title, names) in [
        (
            "Wider than the bed, each on a plate of its own",
            &layout.too_big,
        ),
        ("Taller than the build height", &layout.too_tall),
        ("Not built yet, left out", &layout.unbuilt),
    ] {
        if !names.is_empty() {
            w.push(card(NoteKind::Warning, title, names.join(", ")));
        }
    }
    w.extend(heading("Spacing"));
    w.push(row([
        label("Gap"),
        length("gap", "", printing.layout_gap_mm, 0.0, 50.0),
    ]));
    w.push(
        bench_api::Check {
            id: "slicer_layout".into(),
            text: "Send to slicer sends the layout".into(),
            on: printing.slicer_layout,
            hint: "File › Send to slicer writes this layout rather than the bodies where they sit"
                .into(),
            ..Default::default()
        }
        .into(),
    );
    w.push(gap(GapSize::Medium));
    w.push(note(
        "Each part rests on its largest flat face with nothing below it, turned to its \
         smallest footprint. How many of each comes from the parts list's Print column, \
         every visible body once without one. The model stays where it is.",
    ));
    w.push(gap(GapSize::Section));
    w.push(row([
        button("send", "Send to slicer", ButtonStyle::Primary, true),
        secondary("export", "Export…"),
    ]));
    w
}

fn print_layout_event(event: &PanelEvent, commands: &mut Vec<UiCommand>) {
    match event {
        PanelEvent::Number { id, value } if id == "gap" => {
            commands.push(UiCommand::SetLayoutGap(*value as f32));
        }
        PanelEvent::Toggle { id, on } if id == "slicer_layout" => {
            commands.push(UiCommand::SetSlicerLayout(*on));
        }
        PanelEvent::Button { id } if id == "send" => {
            commands.push(UiCommand::File(super::FileCommand::SendLaidOutToSlicer));
        }
        PanelEvent::Button { id } if id == "export" => {
            commands.push(UiCommand::File(super::FileCommand::ExportLaidOut));
        }
        _ => {}
    }
}

impl NoteTask {
    fn show(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        accept: bool,
        cancel: bool,
    ) -> Option<HostTaskEnd> {
        if !self.placed {
            if document.add_note(self.body, self.note.clone()).is_none() {
                return Some(HostTaskEnd::Cancelled);
            }
            self.placed = true;
        }
        if cancel {
            match &self.start {
                Some(start) => {
                    document.set_note(start.clone());
                }
                None => {
                    document.remove_note(self.note.id);
                }
            }
            return Some(HostTaskEnd::Cancelled);
        }
        if accept {
            return Some(self.accepted(document));
        }
        let widgets = self.widgets(document);
        let mut changed = false;
        for event in draw(ui, document, widgets) {
            changed |= self.event(&event);
        }
        if changed {
            document.set_note(self.note.clone());
        }
        None
    }

    fn widgets(&self, document: &Document) -> Vec<Widget> {
        let mut w = vec![note(
            "A leader from the point on the body to the label. It moves with the body, \
             and follows its face when the body is rebuilt.",
        )];
        w.extend(heading("Text"));
        w.push(
            bench_api::Fill {
                child: Box::new(
                    bench_api::Entry {
                        id: "text".into(),
                        value: self.note.text.clone(),
                        live: true,
                        formulas: true,
                        large: true,
                        ..Default::default()
                    }
                    .into(),
                ),
            }
            .into(),
        );
        w.push(if self.note.text.trim_start().starts_with('=') {
            note(format!("Shows {}", document.note_text(&self.note)))
        } else {
            note("Start with = to show what a formula comes to, as =Pad.length does.")
        });
        w.extend(heading("Label"));
        w.push(note(
            "Where the label stands from the point, in the body's own axes.",
        ));
        w.push(grid(
            "label",
            true,
            ["X", "Y", "Z"]
                .iter()
                .enumerate()
                .map(|(i, axis)| offset(axis, axis, self.note.label[i] - self.note.at[i])),
        ));
        w.push(gap(GapSize::Medium));
        w.push(
            bench_api::Check {
                id: "shown".into(),
                text: "Shown in the view".into(),
                on: !self.note.hidden,
                ..Default::default()
            }
            .into(),
        );
        w
    }

    /// Whether the event changed the note.
    fn event(&mut self, event: &PanelEvent) -> bool {
        match event {
            PanelEvent::Text { id, value } if id == "text" => self.note.text = value.clone(),
            PanelEvent::Number { id, value } => {
                let Some(i) = ["X", "Y", "Z"].iter().position(|a| a == id) else {
                    return false;
                };
                self.note.label[i] = self.note.at[i] + *value as f32;
            }
            PanelEvent::Toggle { id, on } if id == "shown" => self.note.hidden = !on,
            _ => return false,
        }
        true
    }

    /// The task kept: its undo step's label and the call a recording
    /// keeps, with the point and label where the body sits.
    fn accepted(&self, document: &Document) -> HostTaskEnd {
        let placed = document.note_placed(self.body, &self.note);
        let (at, label) = (placed.at, placed.label);
        match &self.start {
            None => HostTaskEnd::Accepted(
                format!("Add note to {}", body_name(document, self.body)),
                vec![call(
                    "doc.note_add",
                    json!({
                        "body": self.body.0.to_string(),
                        "text": self.note.text,
                        "point": at,
                        "label": label,
                    }),
                )],
            ),
            Some(_) => HostTaskEnd::Accepted(
                "Edit note".to_string(),
                vec![call(
                    "doc.note_set",
                    json!({
                        "id": self.note.id.0.to_string(),
                        "text": self.note.text,
                        "label": label,
                        "visible": !self.note.hidden,
                    }),
                )],
            ),
        }
    }
}

impl PlacementTask {
    /// The axes a move by numbers can go along: the world, every other
    /// body and every datum, with their names.
    fn frames(&self, document: &Document) -> Vec<(Frame, String)> {
        let mut frames: Vec<(Frame, String)> = vec![(Frame::World, "World".to_string())];
        frames.extend(
            document
                .bodies()
                .iter()
                .filter(|b| b.id != self.body)
                .map(|b| (Frame::Body(b.id), b.name.clone())),
        );
        frames.extend(
            document
                .feature_tree()
                .all_nodes()
                .filter(|(_, n)| n.workbench_id.as_str() == core_document::DATUM_KIND)
                .map(|(id, n)| (Frame::Datum(*id), n.name.clone())),
        );
        frames
    }

    /// The move by numbers: a frame, a pivot, a shift and a turn, applied
    /// to where the body is by Move.
    fn move_by(&mut self, document: &Document) {
        let Some((axes, _)) = frame_of(document, self.frame) else {
            return;
        };
        let pivot = pivot_of(document, self.body, self.pivot);
        let moved = moved_by(self.placement(), axes, pivot, self.shift, self.turn);
        let (x, y, z) = moved.quat().to_euler(glam::EulerRot::XYZ);
        self.offset = moved.offset().to_array();
        self.angles = [x, y, z].map(f32::to_degrees);
        self.shift = [0.0; 3];
        self.turn = [0.0; 3];
    }

    fn placement(&self) -> BodyPlacement {
        let [x, y, z] = self.angles.map(f32::to_radians);
        BodyPlacement::new(
            glam::Quat::from_euler(glam::EulerRot::XYZ, x, y, z),
            glam::Vec3::from_array(self.offset),
        )
    }

    fn show(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        accept: bool,
        cancel: bool,
    ) -> Option<HostTaskEnd> {
        if cancel {
            for (body, placement) in &self.start {
                document.set_body_placement(*body, *placement);
            }
            return Some(HostTaskEnd::Cancelled);
        }
        if accept {
            let placement = document.body_placement(self.body);
            return Some(HostTaskEnd::Accepted(
                format!("Place {}", body_name(document, self.body)),
                vec![call(
                    "doc.set_body",
                    json!({
                        "body": self.body.0.to_string(),
                        "translation": placement.translation,
                        "rotation": placement.rotation,
                    }),
                )],
            ));
        }
        let widgets = self.widgets(document);
        let mut changed = false;
        for event in draw(ui, document, widgets) {
            changed |= self.event(&event, document);
        }
        if changed {
            document.place_with_unit(self.body, self.placement());
        }
        None
    }

    fn widgets(&self, document: &Document) -> Vec<Widget> {
        const TURNS: [&str; 3] = ["About X", "About Y", "About Z"];
        let mut w = vec![note(
            "Where the body's origin goes and how it is turned. The view follows as you type or drag.",
        )];
        if self.start.len() > 1 {
            w.push(gap(GapSize::Small));
            w.push(note(format!(
                "{} other bodies of its component move with it.",
                self.start.len() - 1
            )));
        }
        w.extend(heading("Position"));
        w.push(grid(
            "position",
            true,
            (0..3).map(|i| offset(AXES[i], AXES[i], self.offset[i])),
        ));
        w.extend(heading("Turn"));
        w.push(grid(
            "turn",
            true,
            (0..3).map(|i| degrees(TURN_IDS[i], TURNS[i], self.angles[i], Some((-360.0, 360.0)))),
        ));
        w.extend(heading("Move by"));
        let frames = self.frames(document);
        let frame = frames
            .iter()
            .position(|(f, _)| *f == self.frame)
            .unwrap_or(0);
        let pivot = PIVOTS
            .iter()
            .position(|(p, _)| *p == self.pivot)
            .unwrap_or(0);
        let mut by = vec![
            select(
                "frame",
                "Relative to",
                frames.into_iter().map(|(_, name)| name),
                frame,
                180.0,
            ),
            select("pivot", "Turn about", PIVOTS.map(|(_, l)| l), pivot, 180.0),
        ];
        by.extend((0..3).map(|i| offset(SHIFT_IDS[i], ALONG[i], self.shift[i])));
        by.extend(
            (0..3).map(|i| degrees(SPIN_IDS[i], TURNS[i], self.turn[i], Some((-360.0, 360.0)))),
        );
        w.push(grid("move_by", true, by));
        let still = self.shift == [0.0; 3] && self.turn == [0.0; 3];
        w.push(hinted(
            "Move the body by these numbers, from where it is now",
            button("move", "Move", ButtonStyle::Secondary, !still),
        ));
        w.push(gap(GapSize::Section));
        w.push(row([
            secondary("made", "Where it was made"),
            secondary("was", "Where it was"),
        ]));
        w
    }

    /// Whether the event moved the body's fields.
    fn event(&mut self, event: &PanelEvent, document: &Document) -> bool {
        match event {
            PanelEvent::Number { id, value } => {
                let v = *value as f32;
                let at = |ids: &[&str; 3]| ids.iter().position(|a| a == id);
                if let Some(i) = at(&AXES) {
                    self.offset[i] = v;
                    return true;
                }
                if let Some(i) = at(&TURN_IDS) {
                    self.angles[i] = v;
                    return true;
                }
                if let Some(i) = at(&SHIFT_IDS) {
                    self.shift[i] = v;
                } else if let Some(i) = at(&SPIN_IDS) {
                    self.turn[i] = v;
                }
                false
            }
            PanelEvent::Choice { id, index } if id == "frame" => {
                if let Some((frame, _)) = self.frames(document).get(*index) {
                    self.frame = *frame;
                }
                false
            }
            PanelEvent::Choice { id, index } if id == "pivot" => {
                if let Some((pivot, _)) = PIVOTS.get(*index) {
                    self.pivot = *pivot;
                }
                false
            }
            PanelEvent::Button { id } if id == "move" => {
                self.move_by(document);
                true
            }
            PanelEvent::Button { id } if id == "made" => {
                self.offset = [0.0; 3];
                self.angles = [0.0; 3];
                true
            }
            PanelEvent::Button { id } if id == "was" => {
                let start = self
                    .start
                    .iter()
                    .find(|(b, _)| *b == self.body)
                    .map(|(_, p)| *p)
                    .unwrap_or_default();
                let (x, y, z) = start.quat().to_euler(glam::EulerRot::XYZ);
                self.offset = start.offset().to_array();
                self.angles = [x, y, z].map(f32::to_degrees);
                true
            }
            _ => false,
        }
    }
}

/// The position fields' names, which are their ids too.
const AXES: [&str; 3] = ["X", "Y", "Z"];
const TURN_IDS: [&str; 3] = ["turn_x", "turn_y", "turn_z"];
const ALONG: [&str; 3] = ["Along X", "Along Y", "Along Z"];
const SHIFT_IDS: [&str; 3] = ["shift_x", "shift_y", "shift_z"];
const SPIN_IDS: [&str; 3] = ["spin_x", "spin_y", "spin_z"];
const PIVOTS: [(Pivot, &str); 3] = [
    (Pivot::Origin, "Body origin"),
    (Pivot::BoxCentre, "Middle of its box"),
    (Pivot::Mass, "Centre of mass"),
];

impl AppearanceTask {
    fn show(
        &mut self,
        ui: &mut Ui,
        inputs: HostTaskInputs<'_>,
        accept: bool,
        cancel: bool,
        commands: &mut Vec<UiCommand>,
    ) -> Option<HostTaskEnd> {
        let HostTaskInputs {
            document,
            picked_faces,
            custom_colors,
            ..
        } = inputs;
        let body = self.body;
        if cancel {
            document.set_body_display(body, self.start_display);
            document.set_body_material(body, self.start_material.clone());
            let now: Vec<FaceColor> = document
                .bodies()
                .iter()
                .find(|b| b.id == body)
                .map(|b| b.face_colors.clone())
                .unwrap_or_default();
            for c in now {
                document.set_face_color(body, c.face.index, c.face.name, None);
            }
            for c in &self.start_faces {
                document.set_face_color(body, c.face.index, c.face.name, Some(c.color));
            }
            return Some(HostTaskEnd::Cancelled);
        }
        let Some(entry) = document.bodies().iter().find(|b| b.id == body).cloned() else {
            return Some(HostTaskEnd::Cancelled);
        };
        if accept {
            return Some(HostTaskEnd::Accepted(
                format!("Appearance of {}", entry.name),
                self.recorded(&entry),
            ));
        }
        let picked: Vec<u32> = picked_faces
            .iter()
            .filter(|(b, _)| *b == body)
            .map(|(_, index)| *index)
            .collect();
        if !picked.is_empty() {
            self.faces = picked;
        }
        let widgets = self.widgets(document, &entry, custom_colors);
        for event in draw(ui, document, widgets) {
            self.event(&event, document, &entry, custom_colors, commands);
        }
        None
    }

    fn widgets(
        &self,
        document: &Document,
        entry: &core_document::Body,
        custom_colors: &[[f32; 3]],
    ) -> Vec<Widget> {
        let body = self.body;
        let display = entry.display;
        let mut w = Vec::new();
        w.extend(heading("Colour"));
        w.extend(swatches("", display.map(|d| d.color), custom_colors));
        w.push(gap(GapSize::Medium));
        let kept = custom_colors.iter().any(|c| same_color(*c, self.custom));
        w.push(row([
            label("Custom"),
            bench_api::ColorField {
                id: "custom".into(),
                color: self.custom,
                ..Default::default()
            }
            .into(),
            if kept {
                hinted(
                    "Take it out of your colours",
                    button("forget", "Forget", ButtonStyle::Small, true),
                )
            } else {
                hinted(
                    "Add it to your colours, offered for every body",
                    button("keep", "Keep", ButtonStyle::Small, true),
                )
            },
        ]));
        if display.is_some() {
            w.push(secondary("uncolor", "The colour it came with"));
        }

        w.extend(heading("See-through"));
        w.push(
            bench_api::Range {
                id: "through".into(),
                value: f64::from((1.0 - display.map_or(1.0, |d| d.opacity)) * 100.0),
                min: 0.0,
                max: 90.0,
                decimals: 0,
                suffix: " %".into(),
                ..Default::default()
            }
            .into(),
        );

        if document.is_mesh_body(body) {
            w.extend(heading("Shading"));
            let (kind, crease) = match entry.shading {
                core_document::Shading::AsMeshed => (0, 30.0),
                core_document::Shading::Sharp => (1, 30.0),
                core_document::Shading::Smooth { crease_deg } => (2, crease_deg),
            };
            w.push(select("shading", "", SHADINGS, kind, 240.0));
            if kind == 2 {
                w.push(row([
                    label("Crease"),
                    bench_api::Quantity {
                        unit: Some("°".into()),
                        min: Some(0.0),
                        max: Some(180.0),
                        ..quantity("crease", "", crease, Dim::Number)
                    }
                    .into(),
                ]));
            }
        }

        w.extend(heading("Faces"));
        match self.faces.as_slice() {
            [] => w.push(note(
                "Click a face of the body in the view to give it a colour of its own \
                 (Ctrl adds more faces).",
            )),
            faces => {
                // The colour they share, when they all have the same one.
                let first = document.face_color(body, faces[0]);
                let own = faces
                    .iter()
                    .all(|f| document.face_color(body, *f) == first)
                    .then_some(first)
                    .flatten();
                w.push(
                    bench_api::Caption {
                        text: match faces {
                            [one] => format!("Selected face {}", one + 1),
                            many => format!("{} selected faces", many.len()),
                        },
                        strong: true,
                        ..Default::default()
                    }
                    .into(),
                );
                w.extend(swatches("face_", own, custom_colors));
                if faces
                    .iter()
                    .any(|f| document.face_color(body, *f).is_some())
                {
                    w.push(secondary("face_uncolor", "The body's colour"));
                }
            }
        }
        let coloured = entry.face_colors.len();
        if coloured > 0 {
            w.push(gap(GapSize::Small));
            w.push(row([
                note(format!(
                    "{coloured} face{} coloured",
                    if coloured == 1 { "" } else { "s" }
                )),
                button("clear_faces", "Clear", ButtonStyle::Small, true),
            ]));
        }

        w.extend(heading("Material"));
        let choice: usize = match &entry.material {
            None => 0,
            Some(_) if self.own_material => MATERIALS.len() + 1,
            Some(m) => MATERIALS
                .iter()
                .position(|(name, _)| *name == m.name)
                .map_or(MATERIALS.len() + 1, |i| i + 1),
        };
        let options = std::iter::once("None".to_string())
            .chain(
                MATERIALS
                    .iter()
                    .map(|(name, density)| format!("{name}  ·  {density:.2} g/cm³")),
            )
            .chain(std::iter::once("Other…".to_string()));
        w.push(select("material", "", options, choice, 240.0));
        if self.own_material
            && let Some(material) = &entry.material
        {
            w.push(grid(
                "own_material",
                true,
                [
                    bench_api::Entry {
                        id: "material_name".into(),
                        label: "Name".into(),
                        value: material.name.clone(),
                        width: Some(140.0),
                        live: true,
                        ..Default::default()
                    }
                    .into(),
                    bench_api::Quantity {
                        unit: Some("g/cm³".into()),
                        min: Some(0.01),
                        max: Some(30.0),
                        speed: Some(0.01),
                        ..quantity("density", "Density", material.density, Dim::Number)
                    }
                    .into(),
                ],
            ));
        }
        if entry.material.is_some() {
            w.push(note("The Physical group of its properties gives its mass."));
        }
        w
    }

    fn event(
        &mut self,
        event: &PanelEvent,
        document: &mut Document,
        entry: &core_document::Body,
        custom_colors: &[[f32; 3]],
        commands: &mut Vec<UiCommand>,
    ) {
        let body = self.body;
        let opacity = entry.display.map_or(1.0, |d| d.opacity);
        // A swatch's colour: the palette's, then the user's own.
        let swatch = |prefix: &str, id: &str, index: usize| match id.strip_prefix(prefix) {
            Some("palette") => PALETTE.get(index).copied(),
            Some("yours") => custom_colors.get(index).copied(),
            _ => None,
        };
        match event {
            PanelEvent::Choice { id, index } if let Some(color) = swatch("", id, *index) => {
                self.custom = color;
                document.set_body_display(body, Some(BodyDisplay { color, opacity }));
            }
            PanelEvent::Choice { id, index } if let Some(color) = swatch("face_", id, *index) => {
                for face in &self.faces {
                    document.color_face(body, *face, Some(color));
                }
            }
            PanelEvent::Color { id, color } if id == "custom" => {
                self.custom = *color;
                document.set_body_display(
                    body,
                    Some(BodyDisplay {
                        color: *color,
                        opacity,
                    }),
                );
            }
            PanelEvent::Button { id } => match id.as_str() {
                "keep" => {
                    let mut all = custom_colors.to_vec();
                    all.push(self.custom);
                    commands.push(UiCommand::SetCustomColors(all));
                }
                "forget" => {
                    let all = custom_colors
                        .iter()
                        .copied()
                        .filter(|c| !same_color(*c, self.custom))
                        .collect();
                    commands.push(UiCommand::SetCustomColors(all));
                }
                "uncolor" => document.set_body_display(body, None),
                "face_uncolor" => {
                    for face in &self.faces {
                        document.color_face(body, *face, None);
                    }
                }
                "clear_faces" => {
                    for c in &entry.face_colors {
                        document.set_face_color(body, c.face.index, c.face.name, None);
                    }
                }
                _ => {}
            },
            PanelEvent::Number { id, value } if id == "through" => {
                let mut next = entry.display.unwrap_or_default();
                next.opacity = 1.0 - *value as f32 / 100.0;
                document.set_body_display(body, Some(next));
            }
            PanelEvent::Choice { id, index } if id == "shading" => {
                let crease = match entry.shading {
                    core_document::Shading::Smooth { crease_deg } => crease_deg,
                    _ => 30.0,
                };
                document.set_body_shading(
                    body,
                    match index {
                        1 => core_document::Shading::Sharp,
                        2 => core_document::Shading::Smooth { crease_deg: crease },
                        _ => core_document::Shading::AsMeshed,
                    },
                );
            }
            PanelEvent::Number { id, value } if id == "crease" => {
                document.set_body_shading(
                    body,
                    core_document::Shading::Smooth {
                        crease_deg: *value as f32,
                    },
                );
            }
            PanelEvent::Choice { id, index } if id == "material" => {
                self.own_material = *index == MATERIALS.len() + 1;
                let material = match *index {
                    0 => None,
                    i if i <= MATERIALS.len() => Some(Material {
                        name: MATERIALS[i - 1].0.to_string(),
                        density: MATERIALS[i - 1].1,
                    }),
                    _ => Some(entry.material.clone().unwrap_or(Material {
                        name: "Material".into(),
                        density: 1.0,
                    })),
                };
                document.set_body_material(body, material);
            }
            PanelEvent::Text { id, value } if id == "material_name" => {
                if let Some(material) = &entry.material
                    && !value.trim().is_empty()
                {
                    document.set_body_material(
                        body,
                        Some(Material {
                            name: value.clone(),
                            ..material.clone()
                        }),
                    );
                }
            }
            PanelEvent::Number { id, value } if id == "density" => {
                if let Some(material) = &entry.material {
                    document.set_body_material(
                        body,
                        Some(Material {
                            density: *value as f32,
                            ..material.clone()
                        }),
                    );
                }
            }
            _ => {}
        }
    }

    /// The calls that set the body as it stands, for a recording.
    fn recorded(&self, entry: &core_document::Body) -> Vec<Recorded> {
        let id = entry.id.0.to_string();
        let mut calls = Vec::new();
        if entry.display != self.start_display || entry.material != self.start_material {
            let mut args = json!({"body": id});
            match entry.display {
                Some(d) => {
                    args["color"] = json!(d.color);
                    args["opacity"] = json!(d.opacity);
                }
                None => args["color"] = Value::Null,
            }
            args["material"] = entry.material.as_ref().map_or(
                Value::Null,
                |m| json!({"name": m.name, "density": m.density}),
            );
            calls.push(call("doc.set_body", args));
        }
        if entry.face_colors != self.start_faces {
            calls.push(call("doc.set_body", json!({"body": id, "face_colors": []})));
            for c in &entry.face_colors {
                calls.push(call(
                    "doc.set_face_color",
                    json!({"body": id, "face": c.face.index, "color": c.color}),
                ));
            }
        }
        calls
    }
}

impl HistoryTask {
    fn show(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        accept: bool,
        cancel: bool,
    ) -> Option<HostTaskEnd> {
        let order = document.body_history_of(self.feature);
        let at = order.iter().position(|f| *f == self.feature);
        let after = at.and_then(|i| i.checked_sub(1)).map(|i| order[i]);
        if cancel {
            let _ = document.move_feature_after(self.feature, self.start_after);
            return Some(HostTaskEnd::Cancelled);
        }
        if accept {
            let mut args = json!({"id": self.feature.0.to_string()});
            if let Some(after) = after {
                args["after"] = json!(after.0.to_string());
            }
            return Some(HostTaskEnd::Accepted(
                "Reorder history".into(),
                if after == self.start_after {
                    Vec::new()
                } else {
                    vec![call("doc.move_after", args)]
                },
            ));
        }
        let widgets = self.widgets(document, &order, at);
        for event in draw(ui, document, widgets) {
            self.event(&event, document, &order);
        }
        None
    }

    fn widgets(&self, document: &Document, order: &[FeatureId], at: Option<usize>) -> Vec<Widget> {
        let mut w = vec![
            note(
                "Click the feature it should come after. A feature never goes before one it is built from.",
            ),
            gap(GapSize::Medium),
            button("start", "At the start", ButtonStyle::Plain, at != Some(0)),
        ];
        for (i, id) in order.iter().enumerate() {
            let Some(node) = document.get_feature_meta(*id) else {
                continue;
            };
            w.push(
                bench_api::RowButton {
                    id: format!("after:{i}"),
                    text: format!("{}. {}", i + 1, node.name),
                    current: *id == self.feature,
                }
                .into(),
            );
        }
        if let Some(why) = &self.refused {
            w.push(gap(GapSize::Medium));
            w.push(caption(why.clone(), Tone::Warning));
        }
        w
    }

    fn event(&mut self, event: &PanelEvent, document: &mut Document, order: &[FeatureId]) {
        let PanelEvent::Button { id } = event else {
            return;
        };
        let target = match indexed(id, "after:") {
            Some(i) => match order.get(i) {
                Some(feature) => Some(*feature),
                None => return,
            },
            None if id == "start" => None,
            None => return,
        };
        self.refused = document
            .move_feature_after(self.feature, target)
            .err()
            .map(|refused| match refused {
                core_document::MoveRefused::Dependency { neighbour } => format!(
                    "It stops by {}: one is built from the other.",
                    document
                        .get_feature_meta(neighbour)
                        .map(|n| n.name.clone())
                        .unwrap_or_default()
                ),
                _ => "It cannot go there.".to_string(),
            });
    }
}

/// The axes and origin of `frame`, in the world.
fn frame_of(document: &Document, frame: Frame) -> Option<(glam::Quat, glam::Vec3)> {
    match frame {
        Frame::World => Some((glam::Quat::IDENTITY, glam::Vec3::ZERO)),
        Frame::Body(body) => {
            let placed = document.body_placement(body);
            Some((placed.quat(), placed.offset()))
        }
        Frame::Datum(id) => {
            let datum =
                <core_document::DatumFeature as core_document::WorkbenchFeature>::from_json(
                    document.feature_values(id)?,
                )
                .ok()?;
            let own = datum.frame();
            let v = glam::Vec3::from_array;
            let axes = glam::Quat::from_mat3(&glam::Mat3::from_cols(
                v(own.x_axis).normalize(),
                v(own.y_axis()).normalize(),
                v(own.normal).normalize(),
            ));
            let placed = document
                .get_feature_meta(id)
                .and_then(|n| n.body)
                .map(|b| document.body_placement(b))
                .unwrap_or_default();
            Some((
                (placed.quat() * axes).normalize(),
                v(placed.point(own.origin)),
            ))
        }
    }
}

/// Where `pivot` is for `body`, in the world.
fn pivot_of(document: &Document, body: BodyId, pivot: Pivot) -> glam::Vec3 {
    let origin = document.body_placement(body).offset();
    let mesh = document.imported_geometry(body).map(|g| &g.mesh);
    match pivot {
        Pivot::Origin => origin,
        Pivot::BoxCentre => mesh
            .and_then(|m| m.bounds())
            .map(|(lo, hi)| (glam::Vec3::from_array(lo) + glam::Vec3::from_array(hi)) * 0.5)
            .unwrap_or(origin),
        Pivot::Mass => mesh.and_then(|m| volume_centre(m)).unwrap_or(origin),
    }
}

/// The centre of the volume a closed mesh bounds: the tetrahedra from the
/// origin to each triangle, weighted by their signed volumes. `None` for
/// a mesh that encloses nothing.
fn volume_centre(mesh: &kernel_api::TriMesh) -> Option<glam::Vec3> {
    let mut volume = 0.0f64;
    let mut moment = glam::DVec3::ZERO;
    for [i, j, k] in mesh.indices.as_chunks::<3>().0 {
        let p = |i: u32| glam::Vec3::from_array(mesh.positions[i as usize]).as_dvec3();
        let (a, b, c) = (p(*i), p(*j), p(*k));
        let v = a.dot(b.cross(c)) / 6.0;
        volume += v;
        moment += (a + b + c) * (v / 4.0);
    }
    (volume.abs() > 1e-12).then(|| (moment / volume).as_vec3())
}

/// `placement` moved along `axes` by `shift`, then turned about `axes`'
/// X, then Y, then Z by `turn` degrees about `pivot`, all in the world.
fn moved_by(
    placement: BodyPlacement,
    axes: glam::Quat,
    pivot: glam::Vec3,
    shift: [f32; 3],
    turn: [f32; 3],
) -> BodyPlacement {
    let [x, y, z] = turn.map(f32::to_radians);
    let local = glam::Quat::from_euler(glam::EulerRot::XYZ, x, y, z);
    let spin = (axes * local * axes.inverse()).normalize();
    let along = axes * glam::Vec3::from_array(shift);
    BodyPlacement::new(
        (spin * placement.quat()).normalize(),
        spin * (placement.offset() - pivot) + pivot + along,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::task_blocks::in_frame;

    fn show(
        task: &mut HostTask,
        doc: &mut Document,
        accept: bool,
        cancel: bool,
    ) -> Option<HostTaskEnd> {
        show_picking(task, doc, &[], accept, cancel)
    }

    /// One frame of the task with these faces picked.
    fn show_picking(
        task: &mut HostTask,
        doc: &mut Document,
        picked_faces: &[(BodyId, u32)],
        accept: bool,
        cancel: bool,
    ) -> Option<HostTaskEnd> {
        let mut end = None;
        in_frame(|ui| {
            end = task.show(
                ui,
                HostTaskInputs {
                    document: doc,
                    picked_faces,
                    picked_edges: &[],
                    registry: &core_document::DocumentService::default(),
                    custom_colors: &[],
                    printing: &settings::PrintingSettings::default(),
                    print_layout: None,
                    marking: None,
                },
                accept,
                cancel,
                &mut Vec::new(),
            );
        });
        end
    }

    #[test]
    fn a_new_note_goes_in_while_its_task_is_open_and_out_on_cancel() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut task = HostTask::add_note(&doc, body, Some([1.0, 2.0, 3.0]));
        assert!(show(&mut task, &mut doc, false, false).is_none());
        assert_eq!(doc.body_notes(body).len(), 1, "drawn while it is edited");
        assert!(matches!(
            show(&mut task, &mut doc, false, true),
            Some(HostTaskEnd::Cancelled)
        ));
        assert!(doc.body_notes(body).is_empty());

        let mut task = HostTask::add_note(&doc, body, Some([1.0, 2.0, 3.0]));
        show(&mut task, &mut doc, false, false);
        let Some(HostTaskEnd::Accepted(_, calls)) = show(&mut task, &mut doc, true, false) else {
            panic!("accepted");
        };
        assert_eq!(calls[0].id, "doc.note_add");
        assert_eq!(calls[0].args["point"], json!([1.0, 2.0, 3.0]));
    }

    #[test]
    fn editing_a_note_records_its_change_and_cancel_puts_it_back() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let note = Note::pinned("Before".into(), [0.0; 3], [0.0, 0.0, 5.0], None);
        let id = doc.add_note(body, note).unwrap();
        let note = doc.note(id).unwrap().1.clone();
        let mut task = HostTask::edit_note(&doc, id).expect("the note");
        if let HostTask::Note(t) = &mut task {
            t.note.text = "After".into();
            doc.set_note(t.note.clone());
        }
        assert!(matches!(
            show(&mut task, &mut doc, false, true),
            Some(HostTaskEnd::Cancelled)
        ));
        assert_eq!(doc.note(id).unwrap().1, &note);

        let mut task = HostTask::edit_note(&doc, id).expect("the note");
        if let HostTask::Note(t) = &mut task {
            t.note.text = "After".into();
            doc.set_note(t.note.clone());
        }
        let Some(HostTaskEnd::Accepted(_, calls)) = show(&mut task, &mut doc, true, false) else {
            panic!("accepted");
        };
        assert_eq!(calls[0].id, "doc.note_set");
        assert_eq!(calls[0].args["text"], json!("After"));
    }

    #[test]
    fn appearance_takes_every_face_picked_on_its_body() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let other = doc.create_body(None);
        let mut task = HostTask::appearance(&doc, body, Some(1));
        let picks = [(body, 2), (other, 4), (body, 5)];
        show_picking(&mut task, &mut doc, &picks, false, false);
        let HostTask::Appearance(t) = &task else {
            panic!("appearance");
        };
        assert_eq!(
            t.faces,
            [2, 5],
            "its own body's, in order, for the swatches to colour"
        );
    }

    /// A quarter turn about Z through a pivot beside the origin swings the
    /// origin round it; a shift goes along the frame's axes, not the
    /// world's.
    #[test]
    fn move_by_turns_about_the_pivot_and_shifts_along_the_frame() {
        let close = |a: glam::Vec3, b: [f32; 3]| (a - glam::Vec3::from_array(b)).length() < 1e-4;
        let turned = moved_by(
            BodyPlacement::IDENTITY,
            glam::Quat::IDENTITY,
            glam::Vec3::new(10.0, 0.0, 0.0),
            [0.0; 3],
            [0.0, 0.0, 90.0],
        );
        assert!(
            close(turned.offset(), [10.0, -10.0, 0.0]),
            "{:?}",
            turned.offset()
        );
        assert!(close(turned.quat() * glam::Vec3::X, [0.0, 1.0, 0.0]));

        let frame = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let shifted = moved_by(
            BodyPlacement::IDENTITY,
            frame,
            glam::Vec3::ZERO,
            [5.0, 0.0, 0.0],
            [0.0; 3],
        );
        assert!(
            close(shifted.offset(), [0.0, 5.0, 0.0]),
            "{:?}",
            shifted.offset()
        );
        // About the frame's X, which is the world's Y.
        let tipped = moved_by(
            BodyPlacement::IDENTITY,
            frame,
            glam::Vec3::ZERO,
            [0.0; 3],
            [90.0, 0.0, 0.0],
        );
        assert!(
            close(tipped.quat() * glam::Vec3::Z, [1.0, 0.0, 0.0]),
            "{:?}",
            tipped.quat() * glam::Vec3::Z
        );
    }

    /// A box's volume centre is its middle, wherever it sits.
    #[test]
    fn the_centre_of_mass_of_a_box_is_its_middle() {
        let (lo, hi) = (
            glam::Vec3::new(2.0, 4.0, 6.0),
            glam::Vec3::new(12.0, 8.0, 7.0),
        );
        let mesh = crate::test_support::block((hi - lo).to_array(), lo.to_array());
        let centre = volume_centre(&mesh).unwrap();
        assert!((centre - (lo + hi) * 0.5).length() < 1e-4, "{centre:?}");
    }

    #[test]
    fn placement_moves_live_and_cancel_puts_it_back() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut task = HostTask::placement(&doc, body);
        if let HostTask::Placement(t) = &mut task {
            t.offset = [-5.0, 2.0, 0.0];
            t.angles = [0.0, 0.0, 90.0];
            doc.place_with_unit(body, t.placement());
        }
        assert_eq!(doc.body_placement(body).translation, [-5.0, 2.0, 0.0]);
        assert!(matches!(
            show(&mut task, &mut doc, false, true),
            Some(HostTaskEnd::Cancelled)
        ));
        assert_eq!(doc.body_placement(body), BodyPlacement::IDENTITY);

        let mut task = HostTask::placement(&doc, body);
        if let HostTask::Placement(t) = &mut task {
            t.offset = [-5.0, 0.0, 0.0];
            doc.place_with_unit(body, t.placement());
        }
        let Some(HostTaskEnd::Accepted(_, calls)) = show(&mut task, &mut doc, true, false) else {
            panic!("accepted");
        };
        assert_eq!(calls[0].id, "doc.set_body");
        assert_eq!(calls[0].args["translation"], json!([-5.0, 0.0, 0.0]));
    }

    #[test]
    fn appearance_cancel_puts_back_colour_material_and_faces() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let mut task = HostTask::appearance(&doc, body, None);
        doc.set_body_display(
            body,
            Some(BodyDisplay {
                color: PALETTE[6],
                opacity: 0.5,
            }),
        );
        doc.set_body_material(
            body,
            Some(Material {
                name: "PLA".into(),
                density: 1.24,
            }),
        );
        doc.set_face_color(body, 3, 9, Some([1.0, 0.0, 0.0]));
        let Some(HostTaskEnd::Accepted(_, calls)) =
            show(&mut task.clone(), &mut doc.clone(), true, false)
        else {
            panic!("accepted");
        };
        let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["doc.set_body", "doc.set_body", "doc.set_face_color"]);
        assert!(matches!(
            show(&mut task, &mut doc, false, true),
            Some(HostTaskEnd::Cancelled)
        ));
        let entry = &doc.bodies()[0];
        assert!(entry.display.is_none() && entry.material.is_none());
        assert!(entry.face_colors.is_empty());
    }

    #[test]
    fn a_history_move_cancels_back_to_where_it_was() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let ids = crate::test_support::datum_points(&mut doc, body, &["a", "b", "c"]);
        let mut task = HostTask::history(&doc, ids[1]);
        doc.move_feature_after(ids[1], Some(ids[2])).unwrap();
        assert_eq!(doc.body_history_of(ids[0]), [ids[0], ids[2], ids[1]]);
        assert!(matches!(
            show(&mut task, &mut doc, false, true),
            Some(HostTaskEnd::Cancelled)
        ));
        assert_eq!(doc.body_history_of(ids[0]), ids);
    }

    fn button(id: &str) -> PanelEvent {
        PanelEvent::Button { id: id.into() }
    }

    fn number(id: &str, value: f64) -> PanelEvent {
        PanelEvent::Number {
            id: id.into(),
            value,
        }
    }

    fn choice(id: &str, index: usize) -> PanelEvent {
        PanelEvent::Choice {
            id: id.into(),
            index,
        }
    }

    /// Whether a widget, or one inside it, has `id`.
    fn has(widgets: &[Widget], id: &str) -> bool {
        serde_json::to_string(widgets)
            .unwrap()
            .contains(&format!("\"id\":\"{id}\""))
    }

    /// The appearance panel's swatches, picker, Keep, see-through, faces
    /// and material each change what they name.
    #[test]
    fn appearance_events_colour_the_body_its_faces_and_its_material() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let HostTask::Appearance(mut task) = HostTask::appearance(&doc, body, Some(2)) else {
            panic!("appearance");
        };
        let mut commands = Vec::new();
        let mut send = |task: &mut AppearanceTask, doc: &mut Document, event: PanelEvent| {
            let entry = doc.bodies()[0].clone();
            task.event(&event, doc, &entry, &[[0.1, 0.2, 0.3]], &mut commands);
        };
        send(&mut task, &mut doc, choice("palette", 6));
        assert_eq!(doc.bodies()[0].display.unwrap().color, PALETTE[6]);
        send(&mut task, &mut doc, choice("yours", 0));
        assert_eq!(doc.bodies()[0].display.unwrap().color, [0.1, 0.2, 0.3]);
        send(&mut task, &mut doc, number("through", 40.0));
        assert!((doc.bodies()[0].display.unwrap().opacity - 0.6).abs() < 1e-6);
        send(&mut task, &mut doc, button("keep"));
        doc.set_face_color(body, 2, 9, Some(PALETTE[7]));
        let entry = doc.bodies()[0].clone();
        assert!(has(&task.widgets(&doc, &entry, &[]), "clear_faces"));
        send(&mut task, &mut doc, button("clear_faces"));
        assert!(doc.bodies()[0].face_colors.is_empty());
        send(&mut task, &mut doc, choice("material", MATERIALS.len() + 1));
        assert!(task.own_material);
        send(
            &mut task,
            &mut doc,
            PanelEvent::Text {
                id: "material_name".into(),
                value: "Foam".into(),
            },
        );
        send(&mut task, &mut doc, number("density", 0.3));
        let material = doc.bodies()[0].material.clone().unwrap();
        assert_eq!((material.name.as_str(), material.density), ("Foam", 0.3));
        assert!(matches!(&commands[..], [UiCommand::SetCustomColors(c)] if c.len() == 2));
        let entry = doc.bodies()[0].clone();
        let widgets = task.widgets(&doc, &entry, &[]);
        for id in ["uncolor", "material_name", "density"] {
            assert!(has(&widgets, id), "{id}");
        }
    }

    /// A move by numbers in another body's axes, and Where it was.
    #[test]
    fn placement_events_move_by_numbers_and_put_back() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let other = doc.create_body(None);
        let turned = BodyPlacement::new(
            glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            glam::Vec3::ZERO,
        );
        doc.set_body_placement(other, turned);
        let HostTask::Placement(mut task) = HostTask::placement(&doc, body) else {
            panic!("placement");
        };
        assert!(
            task.event(&number("X", 4.0), &doc),
            "the position moves the body"
        );
        let frame = task
            .frames(&doc)
            .iter()
            .position(|(f, _)| *f == Frame::Body(other));
        assert!(!task.event(&choice("frame", frame.unwrap()), &doc));
        assert!(
            !task.event(&number("shift_x", 5.0), &doc),
            "nothing moves until Move"
        );
        assert!(task.event(&button("move"), &doc));
        let at = glam::Vec3::from_array(task.offset);
        assert!(
            (at - glam::Vec3::new(4.0, 5.0, 0.0)).length() < 1e-4,
            "{at:?}"
        );
        assert!(task.event(&button("was"), &doc));
        assert_eq!(task.offset, [0.0; 3]);
    }

    /// A row of the history moves the feature, or says why it cannot.
    #[test]
    fn history_rows_move_the_feature_or_say_why_not() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let ids = crate::test_support::datum_points(&mut doc, body, &["a", "b", "c"]);
        let HostTask::History(mut task) = HostTask::history(&doc, ids[0]) else {
            panic!("history");
        };
        let order = doc.body_history_of(ids[0]);
        task.event(&button("after:2"), &mut doc, &order);
        assert_eq!(doc.body_history_of(ids[0]), [ids[1], ids[2], ids[0]]);
        let order = doc.body_history_of(ids[0]);
        task.event(&button("start"), &mut doc, &order);
        assert_eq!(doc.body_history_of(ids[0]), ids);
        let widgets = task.widgets(&doc, &ids, Some(0));
        assert!(has(&widgets, "after:2") && task.refused.is_none());
    }

    /// The note's text, label offset and shown box.
    #[test]
    fn note_events_change_the_note() {
        let doc = Document::new("t");
        let mut task = NoteTask {
            body: BodyId(uuid::Uuid::nil()),
            start: None,
            note: Note::pinned("A".into(), [1.0, 0.0, 0.0], [1.0, 0.0, 5.0], None),
            placed: true,
        };
        assert!(task.event(&PanelEvent::Text {
            id: "text".into(),
            value: "=Pad.length".into()
        }));
        assert!(task.event(&number("Y", 3.0)));
        assert!(task.event(&PanelEvent::Toggle {
            id: "shown".into(),
            on: false
        }));
        assert_eq!(task.note.label, [1.0, 3.0, 5.0]);
        assert!(task.note.hidden && task.note.text == "=Pad.length");
        assert!(has(&task.widgets(&doc), "text"));
    }

    /// The marking's tool, brush, faces away, Invert and Clear; the print
    /// layout's gap, slicer switch and buttons.
    #[test]
    fn mark_and_print_layout_events_reach_the_app() {
        use crate::app::marking::{MarkEdit, MarkTool};
        let mut task = MarkTask::new(BodyId(uuid::Uuid::nil()));
        let mut commands = Vec::new();
        assert!(!has(&task.widgets(None), "brush"));
        for event in [
            choice("tool", 1),
            number("brush", 2.5),
            PanelEvent::Toggle {
                id: "through".into(),
                on: true,
            },
            button("invert"),
            button("clear"),
        ] {
            task.event(&event, &mut commands);
        }
        assert_eq!(task.arm.tool, MarkTool::Brush);
        assert!(task.arm.through && task.arm.brush_mm == 2.5);
        assert!(has(&task.widgets(None), "brush"));
        assert!(matches!(
            &commands[..],
            [
                UiCommand::MarkEdit(MarkEdit::Invert),
                UiCommand::MarkEdit(MarkEdit::Clear)
            ]
        ));
        let mut commands = Vec::new();
        for event in [
            number("gap", 4.0),
            PanelEvent::Toggle {
                id: "slicer_layout".into(),
                on: true,
            },
            button("send"),
            button("export"),
        ] {
            print_layout_event(&event, &mut commands);
        }
        assert!(matches!(
            &commands[..],
            [
                UiCommand::SetLayoutGap(4.0),
                UiCommand::SetSlicerLayout(true),
                UiCommand::File(super::super::FileCommand::SendLaidOutToSlicer),
                UiCommand::File(super::super::FileCommand::ExportLaidOut),
            ]
        ));
    }

    /// The print layout edits nothing: it closes with Close alone, and
    /// leaves the document as it was.
    #[test]
    fn the_print_layout_closes_without_a_change() {
        let mut doc = Document::new("t");
        doc.create_body(None);
        let seq = doc.mutation_seq();
        let mut task = HostTask::PrintLayout;
        assert!(!task.confirmable());
        assert!(show(&mut task, &mut doc, false, false).is_none());
        assert!(matches!(
            show(&mut task, &mut doc, true, false),
            Some(HostTaskEnd::Cancelled)
        ));
        assert_eq!(doc.mutation_seq(), seq);
    }
}
