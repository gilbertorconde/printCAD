//! The application's own tasks in the right panel, beside the benches':
//! a body's placement, its appearance (colour, faces, see-through,
//! material), a note on it, a feature's place in its body's history, an
//! array of copies (`array_task`), and the print layout. Edits apply to
//! the document as they are made, so the view shows them; OK keeps them
//! as one undo step, Cancel puts back what was there when the task
//! opened. The print layout edits nothing: the view
//! shows the layout while it is open, and Close puts the model back.

use core_document::{
    BodyDisplay, BodyId, BodyPlacement, Document, FaceColor, FeatureId, Material, Note, NoteId,
    Recorded,
};
use egui::{RichText, Ui, Vec2};
use serde_json::{Value, json};
use ui_kit::tokens::*;
use ui_kit::widgets::{QtyField, check_row, overline, secondary_button, small_secondary_button};
use ui_kit::{sans, sans_semibold};

use super::UiCommand;

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
    Note(NoteTask),
    /// Every part laid flat on the bed, for export or the slicer.
    PrintLayout,
    /// A body repeated as linked copies in a row or a ring.
    Array(Box<super::array_task::ArrayTask>),
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

fn color32(c: [f32; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8,
    )
}

fn same_color(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

/// One colour to click, ringed when it is the one in use.
fn swatch(ui: &mut Ui, color: [f32; 3], chosen: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect.shrink(2.0), RADIUS_SM, color32(color));
    let stroke = if chosen {
        egui::Stroke::new(2.0, ACCENT)
    } else if response.hovered() {
        egui::Stroke::new(1.0, TEXT2)
    } else {
        egui::Stroke::new(1.0, BORDER)
    };
    painter.rect_stroke(
        rect.shrink(1.0),
        RADIUS_SM,
        stroke,
        egui::StrokeKind::Inside,
    );
    response
}

/// The palette, then the user's own colours; the one clicked.
fn swatches(ui: &mut Ui, current: Option<[f32; 3]>, custom: &[[f32; 3]]) -> Option<[f32; 3]> {
    let mut picked = None;
    let is = |c: [f32; 3]| current.is_some_and(|now| same_color(now, c));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(2.0);
        for color in PALETTE {
            if swatch(ui, *color, is(*color)).clicked() {
                picked = Some(*color);
            }
        }
    });
    if !custom.is_empty() {
        ui.add_space(SPACE_1);
        ui.label(
            RichText::new("Your colours")
                .font(sans(FONT_XS))
                .color(TEXT3),
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
            for color in custom {
                if swatch(ui, *color, is(*color)).clicked() {
                    picked = Some(*color);
                }
            }
        });
    }
    picked
}

fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(sans(FONT_SM)).color(TEXT3));
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(SPACE_3);
    overline(ui, text);
    ui.add_space(SPACE_1);
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
        Self::Note(NoteTask {
            body,
            start: None,
            note: document.new_note(body, "Note".to_string(), at),
            placed: false,
        })
    }

    /// Note `id` to edit; `None` when there is no such note.
    pub fn edit_note(document: &Document, id: NoteId) -> Option<Self> {
        let (body, note) = document.note(id)?;
        Some(Self::Note(NoteTask {
            body,
            start: Some(note.clone()),
            note: note.clone(),
            placed: true,
        }))
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
    use ui_kit::widgets::{Note, check_row, note_card, primary_button};
    if close {
        return Some(HostTaskEnd::Cancelled);
    }
    let Some(layout) = inputs.print_layout else {
        note(ui, "Laying out…");
        return None;
    };
    if layout.pieces.is_empty() {
        note_card(
            ui,
            Note::Info,
            Some("Nothing to print"),
            "No visible part has a count to print and a shape.",
        );
    } else {
        let s = |n: usize| if n == 1 { "" } else { "s" };
        let pieces = layout.pieces.len();
        ui.label(
            RichText::new(format!(
                "{pieces} piece{} on {} plate{}",
                s(pieces),
                layout.plates,
                s(layout.plates)
            ))
            .font(sans_semibold(FONT_MD))
            .color(TEXT1),
        );
        if layout.plates > 1 {
            for plate in 0..layout.plates {
                let on = layout.pieces.iter().filter(|p| p.plate == plate).count();
                note(ui, &format!("Plate {}: {on} piece{}", plate + 1, s(on)));
            }
        }
    }
    let warn = |ui: &mut Ui, title: &str, names: &[String]| {
        if !names.is_empty() {
            note_card(ui, Note::Warning, Some(title), &names.join(", "));
        }
    };
    warn(
        ui,
        "Wider than the bed, each on a plate of its own",
        &layout.too_big,
    );
    warn(ui, "Taller than the build height", &layout.too_tall);
    warn(ui, "Not built yet, left out", &layout.unbuilt);

    heading(ui, "Spacing");
    let mut gap = inputs.printing.layout_gap_mm;
    let changed = ui
        .horizontal(|ui| {
            ui.label(RichText::new("Gap").font(sans(FONT_SM)).color(TEXT2));
            QtyField::mm(&mut gap).range(0.0..=50.0).show(ui)
        })
        .inner;
    if changed {
        commands.push(UiCommand::SetLayoutGap(gap));
    }
    let mut laid_out = inputs.printing.slicer_layout;
    if check_row(ui, &mut laid_out, "Send to slicer sends the layout")
        .on_hover_text(
            "File › Send to slicer writes this layout rather than the bodies where they sit",
        )
        .changed()
    {
        commands.push(UiCommand::SetSlicerLayout(laid_out));
    }
    ui.add_space(SPACE_2);
    note(
        ui,
        "Each part rests on its largest flat face with nothing below it, turned to its \
         smallest footprint. How many of each comes from the parts list's Print column, \
         every visible body once without one. The model stays where it is.",
    );
    ui.add_space(SPACE_3);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_2;
        if primary_button(ui, "Send to slicer").clicked() {
            commands.push(UiCommand::File(super::FileCommand::SendLaidOutToSlicer));
        }
        if secondary_button(ui, "Export…").clicked() {
            commands.push(UiCommand::File(super::FileCommand::ExportLaidOut));
        }
    });
    None
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
        note(
            ui,
            "A leader from the point on the body to the label. It moves with the body, \
             and follows its face when the body is rebuilt.",
        );
        let mut changed = false;
        heading(ui, "Text");
        let id = ui.id().with(("host_note_text", self.note.id));
        let candidates = || core_document::formula_candidates(document);
        let edit = ui_kit::completion::completing_text_edit(
            ui,
            id,
            &mut self.note.text,
            &candidates,
            |e| e.font(sans(FONT_MD)).desired_width(f32::INFINITY),
        );
        changed |= edit.response.changed();
        if self.note.text.trim_start().starts_with('=') {
            note(ui, &format!("Shows {}", document.note_text(&self.note)));
        } else {
            note(
                ui,
                "Start with = to show what a formula comes to, as =Pad.length does.",
            );
        }
        heading(ui, "Label");
        note(
            ui,
            "Where the label stands from the point, in the body's own axes.",
        );
        let mut offset: [f32; 3] = std::array::from_fn(|i| self.note.label[i] - self.note.at[i]);
        egui::Grid::new("host_note_label")
            .num_columns(2)
            .spacing([SPACE_3, SPACE_1])
            .show(ui, |ui| {
                for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.label(RichText::new(*axis).font(sans(FONT_SM)).color(TEXT2));
                    if QtyField::offset(&mut offset[i]).show(ui) {
                        self.note.label[i] = self.note.at[i] + offset[i];
                        changed = true;
                    }
                    ui.end_row();
                }
            });
        ui.add_space(SPACE_2);
        let mut shown = !self.note.hidden;
        if check_row(ui, &mut shown, "Shown in the view").changed() {
            self.note.hidden = !shown;
            changed = true;
        }
        if changed {
            document.set_note(self.note.clone());
        }
        None
    }

    /// The task kept: its undo step's label and the call a recording
    /// keeps, with the point and label where the body sits.
    fn accepted(&self, document: &Document) -> HostTaskEnd {
        let (at, label) = document.note_placed(self.body, &self.note);
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
    /// The move by numbers: a frame, a pivot, a shift and a turn, applied
    /// to where the body is by Move. Whether it moved the body's fields.
    fn move_by(&mut self, ui: &mut Ui, document: &Document) -> bool {
        heading(ui, "Move by");
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
        let frame_labels: Vec<(Frame, &str)> =
            frames.iter().map(|(f, n)| (*f, n.as_str())).collect();
        let pivots = [
            (Pivot::Origin, "Body origin"),
            (Pivot::BoxCentre, "Middle of its box"),
            (Pivot::Mass, "Centre of mass"),
        ];
        egui::Grid::new("host_placement_move_by")
            .num_columns(2)
            .spacing([SPACE_3, SPACE_1])
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Relative to")
                        .font(sans(FONT_SM))
                        .color(TEXT2),
                );
                ui_kit::widgets::select_field(
                    ui,
                    "host_placement_frame",
                    &mut self.frame,
                    &frame_labels,
                    180.0,
                );
                ui.end_row();
                ui.label(RichText::new("Turn about").font(sans(FONT_SM)).color(TEXT2));
                ui_kit::widgets::select_field(
                    ui,
                    "host_placement_pivot",
                    &mut self.pivot,
                    &pivots,
                    180.0,
                );
                ui.end_row();
                for (i, axis) in ["Along X", "Along Y", "Along Z"].iter().enumerate() {
                    ui.label(RichText::new(*axis).font(sans(FONT_SM)).color(TEXT2));
                    QtyField::offset(&mut self.shift[i]).show(ui);
                    ui.end_row();
                }
                for (i, axis) in ["About X", "About Y", "About Z"].iter().enumerate() {
                    ui.label(RichText::new(*axis).font(sans(FONT_SM)).color(TEXT2));
                    QtyField::degrees(&mut self.turn[i])
                        .range(-360.0..=360.0)
                        .show(ui);
                    ui.end_row();
                }
            });
        let still = self.shift == [0.0; 3] && self.turn == [0.0; 3];
        if !ui
            .add_enabled_ui(!still, |ui| secondary_button(ui, "Move"))
            .inner
            .on_hover_text("Move the body by these numbers, from where it is now")
            .clicked()
        {
            return false;
        }
        let Some((axes, _)) = frame_of(document, self.frame) else {
            return false;
        };
        let pivot = pivot_of(document, self.body, self.pivot);
        let moved = moved_by(self.placement(), axes, pivot, self.shift, self.turn);
        let (x, y, z) = moved.quat().to_euler(glam::EulerRot::XYZ);
        self.offset = moved.offset().to_array();
        self.angles = [x, y, z].map(f32::to_degrees);
        self.shift = [0.0; 3];
        self.turn = [0.0; 3];
        true
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
        note(
            ui,
            "Where the body's origin goes and how it is turned. The view follows as you type or drag.",
        );
        if self.start.len() > 1 {
            ui.add_space(SPACE_1);
            note(
                ui,
                &format!(
                    "{} other bodies of its component move with it.",
                    self.start.len() - 1
                ),
            );
        }
        let mut changed = false;
        heading(ui, "Position");
        egui::Grid::new("host_placement_position")
            .num_columns(2)
            .spacing([SPACE_3, SPACE_1])
            .show(ui, |ui| {
                for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.label(RichText::new(*axis).font(sans(FONT_SM)).color(TEXT2));
                    changed |= QtyField::offset(&mut self.offset[i]).show(ui);
                    ui.end_row();
                }
            });
        heading(ui, "Turn");
        egui::Grid::new("host_placement_turn")
            .num_columns(2)
            .spacing([SPACE_3, SPACE_1])
            .show(ui, |ui| {
                for (i, axis) in ["About X", "About Y", "About Z"].iter().enumerate() {
                    ui.label(RichText::new(*axis).font(sans(FONT_SM)).color(TEXT2));
                    changed |= QtyField::degrees(&mut self.angles[i])
                        .range(-360.0..=360.0)
                        .show(ui);
                    ui.end_row();
                }
            });
        changed |= self.move_by(ui, document);
        ui.add_space(SPACE_3);
        ui.horizontal(|ui| {
            if secondary_button(ui, "Where it was made").clicked() {
                self.offset = [0.0; 3];
                self.angles = [0.0; 3];
                changed = true;
            }
            if secondary_button(ui, "Where it was").clicked() {
                let start = self
                    .start
                    .iter()
                    .find(|(b, _)| *b == self.body)
                    .map(|(_, p)| *p)
                    .unwrap_or_default();
                let (x, y, z) = start.quat().to_euler(glam::EulerRot::XYZ);
                self.offset = start.offset().to_array();
                self.angles = [x, y, z].map(f32::to_degrees);
                changed = true;
            }
        });
        if changed {
            document.place_with_unit(self.body, self.placement());
        }
        None
    }
}

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
                document.set_face_color(body, c.index, c.name, None);
            }
            for c in &self.start_faces {
                document.set_face_color(body, c.index, c.name, Some(c.color));
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

        heading(ui, "Colour");
        let display = entry.display;
        if let Some(color) = swatches(ui, display.map(|d| d.color), custom_colors) {
            self.custom = color;
            document.set_body_display(
                body,
                Some(BodyDisplay {
                    color,
                    opacity: display.map_or(1.0, |d| d.opacity),
                }),
            );
        }
        ui.add_space(SPACE_2);
        ui.horizontal(|ui| {
            let before = self.custom;
            ui.label(RichText::new("Custom").font(sans(FONT_SM)).color(TEXT2));
            egui::color_picker::color_edit_button_rgb(ui, &mut self.custom);
            if self.custom != before {
                document.set_body_display(
                    body,
                    Some(BodyDisplay {
                        color: self.custom,
                        opacity: display.map_or(1.0, |d| d.opacity),
                    }),
                );
            }
            let kept = custom_colors.iter().any(|c| same_color(*c, self.custom));
            if !kept
                && small_secondary_button(ui, "Keep")
                    .on_hover_text("Add it to your colours, offered for every body")
                    .clicked()
            {
                let mut all = custom_colors.to_vec();
                all.push(self.custom);
                commands.push(UiCommand::SetCustomColors(all));
            }
            if kept
                && small_secondary_button(ui, "Forget")
                    .on_hover_text("Take it out of your colours")
                    .clicked()
            {
                let all = custom_colors
                    .iter()
                    .copied()
                    .filter(|c| !same_color(*c, self.custom))
                    .collect();
                commands.push(UiCommand::SetCustomColors(all));
            }
        });
        if display.is_some() && secondary_button(ui, "The colour it came with").clicked() {
            document.set_body_display(body, None);
        }

        heading(ui, "See-through");
        let mut through = (1.0 - display.map_or(1.0, |d| d.opacity)) * 100.0;
        let slider = ui.add(
            egui::Slider::new(&mut through, 0.0..=90.0)
                .suffix(" %")
                .fixed_decimals(0),
        );
        if slider.changed() {
            let mut next = display.unwrap_or_default();
            next.opacity = 1.0 - through / 100.0;
            document.set_body_display(body, Some(next));
        }

        heading(ui, "Faces");
        let picked: Vec<u32> = picked_faces
            .iter()
            .filter(|(b, _)| *b == body)
            .map(|(_, index)| *index)
            .collect();
        if !picked.is_empty() {
            self.faces = picked;
        }
        match self.faces.as_slice() {
            [] => note(
                ui,
                "Click a face of the body in the view to give it a colour of its own \
                 (Ctrl adds more faces).",
            ),
            faces => {
                // The colour they share, when they all have the same one.
                let first = document.face_color(body, faces[0]);
                let own = faces
                    .iter()
                    .all(|f| document.face_color(body, *f) == first)
                    .then_some(first)
                    .flatten();
                let title = match faces {
                    [one] => format!("Selected face {}", one + 1),
                    many => format!("{} selected faces", many.len()),
                };
                ui.label(
                    RichText::new(title)
                        .font(sans_semibold(FONT_SM))
                        .color(TEXT1),
                );
                if let Some(color) = swatches(ui, own, custom_colors) {
                    for face in faces {
                        document.color_face(body, *face, Some(color));
                    }
                }
                let any_own = faces
                    .iter()
                    .any(|f| document.face_color(body, *f).is_some());
                if any_own && secondary_button(ui, "The body's colour").clicked() {
                    for face in faces {
                        document.color_face(body, *face, None);
                    }
                }
            }
        }
        let coloured = entry.face_colors.len();
        if coloured > 0 {
            ui.add_space(SPACE_1);
            ui.horizontal(|ui| {
                note(
                    ui,
                    &format!(
                        "{coloured} face{} coloured",
                        if coloured == 1 { "" } else { "s" }
                    ),
                );
                if small_secondary_button(ui, "Clear").clicked() {
                    for c in &entry.face_colors {
                        document.set_face_color(body, c.index, c.name, None);
                    }
                }
            });
        }

        heading(ui, "Material");
        let mut choice: usize = match &entry.material {
            None => 0,
            Some(_) if self.own_material => MATERIALS.len() + 1,
            Some(m) => MATERIALS
                .iter()
                .position(|(name, _)| *name == m.name)
                .map_or(MATERIALS.len() + 1, |i| i + 1),
        };
        let mut options: Vec<(usize, String)> = vec![(0, "None".to_string())];
        options.extend(
            MATERIALS
                .iter()
                .enumerate()
                .map(|(i, (name, density))| (i + 1, format!("{name}  ·  {density:.2} g/cm³"))),
        );
        options.push((MATERIALS.len() + 1, "Other…".to_string()));
        let labels: Vec<(usize, &str)> = options.iter().map(|(i, l)| (*i, l.as_str())).collect();
        if ui_kit::widgets::select_field(ui, "host_material", &mut choice, &labels, 240.0) {
            self.own_material = choice == MATERIALS.len() + 1;
            let material = match choice {
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
        if self.own_material
            && let Some(mut material) = entry.material.clone()
        {
            let before = material.clone();
            egui::Grid::new("host_material_own")
                .num_columns(2)
                .spacing([SPACE_3, SPACE_1])
                .show(ui, |ui| {
                    ui.label(RichText::new("Name").font(sans(FONT_SM)).color(TEXT2));
                    ui.add(egui::TextEdit::singleline(&mut material.name).desired_width(140.0));
                    ui.end_row();
                    ui.label(RichText::new("Density").font(sans(FONT_SM)).color(TEXT2));
                    QtyField::new(&mut material.density)
                        .unit("g/cm³")
                        .range(0.01..=30.0)
                        .speed(0.01)
                        .show(ui);
                    ui.end_row();
                });
            if material != before && !material.name.trim().is_empty() {
                document.set_body_material(body, Some(material));
            }
        }
        if entry.material.is_some() {
            note(ui, "The Physical group of its properties gives its mass.");
        }
        None
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
                    json!({"body": id, "face": c.index, "color": c.color}),
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
        note(
            ui,
            "Click the feature it should come after. A feature never goes before one it is built from.",
        );
        ui.add_space(SPACE_2);
        let mut target: Option<Option<FeatureId>> = None;
        if ui
            .add_enabled(at != Some(0), egui::Button::new("At the start"))
            .clicked()
        {
            target = Some(None);
        }
        for (i, id) in order.iter().enumerate() {
            let Some(node) = document.get_feature_meta(*id) else {
                continue;
            };
            let this = *id == self.feature;
            let text = RichText::new(format!("{}. {}", i + 1, node.name)).font(sans(FONT_SM));
            let text = if this {
                text.color(ACCENT).strong()
            } else {
                text.color(TEXT1)
            };
            let row = ui.add_enabled(
                !this,
                egui::Button::new(text)
                    .frame(false)
                    .min_size(Vec2::new(ui.available_width(), 22.0)),
            );
            if row.clicked() {
                target = Some(Some(*id));
            }
        }
        if let Some(after) = target {
            self.refused = document
                .move_feature_after(self.feature, after)
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
        if let Some(why) = &self.refused {
            ui.add_space(SPACE_2);
            ui.label(RichText::new(why).font(sans(FONT_SM)).color(WARNING));
        }
        None
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

    /// Run `f` in a frame of a panel, as the task panel would.
    fn in_frame(f: impl FnOnce(&mut Ui)) {
        let ctx = egui::Context::default();
        // The design's fonts, which a panel naming them needs.
        ui_kit::theme::apply_theme(&ctx);
        let mut f = Some(f);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(f) = f.take() {
                f(ui);
            }
        });
        output.textures_delta.clear();
    }

    fn show(
        task: &mut HostTask,
        doc: &mut Document,
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
                    registry: &core_document::DocumentService::default(),
                    custom_colors: &[],
                    printing: &settings::PrintingSettings::default(),
                    print_layout: None,
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
        let id = doc.add_note(body, note.clone()).unwrap();
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
        in_frame(|ui| {
            task.show(
                ui,
                HostTaskInputs {
                    document: &mut doc,
                    picked_faces: &picks,
                    picked_edges: &[],
                    registry: &core_document::DocumentService::default(),
                    custom_colors: &[],
                    printing: &settings::PrintingSettings::default(),
                    print_layout: None,
                },
                false,
                false,
                &mut Vec::new(),
            );
        });
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
        let corner = |i: usize| {
            [
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            ]
        };
        // Outward-facing triangles of the box's six sides.
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let mesh = kernel_api::TriMesh {
            positions: (0..8).map(corner).collect(),
            indices: quads
                .iter()
                .flat_map(|[a, b, c, d]| [*a, *b, *c, *a, *c, *d])
                .collect(),
            ..Default::default()
        };
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
        let ids: Vec<FeatureId> = ["a", "b", "c"]
            .iter()
            .map(|n| {
                doc.add_feature_in_body(
                    core_document::DatumFeature {
                        shape: core_document::DatumShape::Point,
                        attachment: core_document::DatumAttachment::BasePlane(
                            core_document::BasePlane::XY,
                        ),
                        offset: Default::default(),
                    },
                    n.to_string(),
                    Some(body),
                )
                .unwrap()
            })
            .collect();
        let mut task = HostTask::history(&doc, ids[1]);
        doc.move_feature_after(ids[1], Some(ids[2])).unwrap();
        assert_eq!(doc.body_history_of(ids[0]), [ids[0], ids[2], ids[1]]);
        assert!(matches!(
            show(&mut task, &mut doc, false, true),
            Some(HostTaskEnd::Cancelled)
        ));
        assert_eq!(doc.body_history_of(ids[0]), ids);
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
