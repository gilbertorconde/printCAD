//! The entries a body's menu has, in the tree (on the body's row and its
//! features') and in the view alike: its appearance, placement and notes
//! (tasks in the right panel), freezing, whether clicks pick it, copies and a
//! rebuild. What only changes the window (a task, the property panel's
//! page, the console's input) comes back as a [`MenuLocal`] for the UI to
//! act on.

use core_document::{BodyId, Document, FeatureId, NoteId};
use egui::Ui;

use super::property_panel::PropertyTab;
use super::{BodyEdit, TreeItemId, UiCommand};

/// What a menu entry changes in the window rather than the document.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuLocal {
    /// Select the item and show this page of the property panel.
    Properties(TreeItemId, PropertyTab),
    /// Select the item and put its name up for editing.
    Rename(TreeItemId),
    /// Put this text in the script console's input.
    Console(String),
    /// Open one of the application's tasks in the right panel.
    Task(OpenTask),
}

/// A task a menu opens in the right panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpenTask {
    Placement(BodyId),
    /// The body's look, with the face the menu was opened on, if any.
    Appearance(BodyId, Option<u32>),
    /// A feature's place in its body's history.
    History(FeatureId),
    /// The patterns pressed into the body's faces, with the face the menu
    /// was opened on, if any.
    Texture(BodyId, Option<u32>),
    /// Every part laid flat on the bed, for export or the slicer.
    PrintLayout,
    /// A new note on the body, at a point where it sits in the document
    /// (the one the menu was opened on), or where the body offers one.
    AddNote(BodyId, Option<[f32; 3]>),
    /// A note's text and where its label stands.
    EditNote(NoteId),
    /// The array that owns or repeats the body, or a new one of it.
    Array(BodyId),
}

/// A body as a script names it: its id, with its name beside.
pub fn body_reference(name: &str, body: BodyId) -> String {
    format!("\"{}\" --[[ {} ]]", body.0, name.replace("]]", "] ]"))
}

fn edit(out: &mut Vec<UiCommand>, body: BodyId, edit: BodyEdit) {
    out.push(UiCommand::BodyEdit { body, edit });
}

/// The body entries of a menu, after whatever came before. `face` is the
/// face the menu was opened on and `at` the point, where it sits in the
/// document, when it was. Returns whether an entry was picked, so the
/// caller can close the menu.
pub fn body_entries(
    ui: &mut Ui,
    document: &Document,
    body: BodyId,
    face: Option<u32>,
    at: Option<[f32; 3]>,
    out: &mut Vec<UiCommand>,
    local: &mut Option<MenuLocal>,
) -> bool {
    let Some(entry) = document.bodies().iter().find(|b| b.id == body) else {
        return false;
    };
    let mut picked = false;
    if ui
        .button("Appearance…")
        .on_hover_text("Colour, see-through, face colours and material, in the task panel")
        .clicked()
    {
        *local = Some(MenuLocal::Task(OpenTask::Appearance(body, None)));
        picked = true;
    }
    if let Some(face) = face
        && ui
            .button("Face colour…")
            .on_hover_text("A colour of its own for the face under the pointer")
            .clicked()
    {
        *local = Some(MenuLocal::Task(OpenTask::Appearance(body, Some(face))));
        picked = true;
    }
    if ui
        .button("Surface texture…")
        .on_hover_text(
            "Press a pattern into the body's faces for printing: knurling, ribs, a picture",
        )
        .clicked()
    {
        *local = Some(MenuLocal::Task(OpenTask::Texture(body, face)));
        picked = true;
    }
    if ui
        .button("Add note")
        .on_hover_text(if at.is_some() {
            "A note at the point under the pointer: a leader to a label, in the task panel"
        } else {
            "A note on the body: a leader to a label, in the task panel"
        })
        .clicked()
    {
        *local = Some(MenuLocal::Task(OpenTask::AddNote(body, at)));
        picked = true;
    }
    if ui
        .button("Placement…")
        .on_hover_text("Move or turn the body by numbers, in the task panel")
        .clicked()
    {
        *local = Some(MenuLocal::Task(OpenTask::Placement(body)));
        picked = true;
    }
    ui.separator();
    let (freeze, hint) = if entry.frozen {
        ("Thaw", "Rebuild the body from its history again")
    } else {
        (
            "Freeze",
            "Keep the body as it stands: its features are not rebuilt until it thaws",
        )
    };
    if ui.button(freeze).on_hover_text(hint).clicked() {
        edit(out, body, BodyEdit::Frozen(!entry.frozen));
        picked = true;
    }
    let (select, hint) = if entry.unselectable {
        (
            "Make selectable",
            "Let clicks in the view pick the body again",
        )
    } else {
        (
            "Make unselectable",
            "Clicks in the view pass through the body to what is behind it",
        )
    };
    if ui.button(select).on_hover_text(hint).clicked() {
        edit(out, body, BodyEdit::Selectable(entry.unselectable));
        picked = true;
    }
    if ui
        .button("Linked copy")
        .on_hover_text("A body of the same shape beside this one, following every change to it")
        .clicked()
    {
        edit(out, body, BodyEdit::LinkedCopy);
        picked = true;
    }
    let array = document.array_instance(body).map(|(a, _)| a.id);
    let label = if array.is_some() || !document.arrays_of_source(body).is_empty() {
        "Edit array…"
    } else {
        "Array of copies…"
    };
    if ui
        .button(label)
        .on_hover_text(
            "Copies of the body in a row or a ring, following every change to it, in the \
             task panel",
        )
        .clicked()
    {
        *local = Some(MenuLocal::Task(OpenTask::Array(body)));
        picked = true;
    }
    if array.is_some()
        && ui
            .button("Skip this copy")
            .on_hover_text("Leave this occurrence out of its array; Edit array brings it back")
            .clicked()
    {
        out.push(UiCommand::DeleteTreeItem(TreeItemId::Body(body)));
        picked = true;
    }
    if let Some(source) = entry.copy_of
        && ui
            .button("Select the original")
            .on_hover_text("The body this one is a copy of")
            .clicked()
    {
        out.push(UiCommand::SelectTreeItem(TreeItemId::Body(source)));
        out.push(UiCommand::RevealInTree(source));
        picked = true;
    }
    if document.can_replace_shape(body)
        && ui
            .button("Replace shape…")
            .on_hover_text(
                "Read the shape from another file: its first solid, which the body's \
                 features then build on",
            )
            .clicked()
    {
        out.push(UiCommand::ReplaceShape(body));
        picked = true;
    }
    ui.separator();
    if ui
        .button("Recompute")
        .on_hover_text("Build the body again from its history")
        .clicked()
    {
        edit(out, body, BodyEdit::Recompute);
        picked = true;
    }
    if ui
        .button("Send to console")
        .on_hover_text("Put the body's id in the script console's input")
        .clicked()
    {
        *local = Some(MenuLocal::Console(body_reference(&entry.name, body)));
        picked = true;
    }
    if ui.button("Properties").clicked() {
        *local = Some(MenuLocal::Properties(
            TreeItemId::Body(body),
            PropertyTab::Data,
        ));
        picked = true;
    }
    picked
}
