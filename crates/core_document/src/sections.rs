//! Named sections kept with a document: clipping planes the view can be
//! switched to from a list. They travel with the file and replay on peers
//! like any edit, through one op holding the whole list, but nothing builds
//! from them: saving or removing one changes no feature and no geometry.
//! Which one the view shows, if any, is the view's own state.

use serde::{Deserialize, Serialize};

use crate::BodyId;

/// How a saved section's plane was aimed, which its list names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectionAim {
    X,
    Y,
    Z,
    View,
    Face,
    Datum,
    Points,
}

/// A clipping plane by name: square to unit `normal`, `base + offset`
/// millimetres along it from the document's origin, keeping the side the
/// normal points to unless `flipped`; `formula` sets the offset when
/// given, and the bodies in `uncut` are left whole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSection {
    pub name: String,
    pub normal: [f32; 3],
    pub base: f32,
    pub offset: f32,
    #[serde(default)]
    pub flipped: bool,
    pub aim: SectionAim,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncut: Vec<BodyId>,
}

impl crate::Document {
    /// The document's saved sections, in the order they were saved.
    pub fn saved_sections(&self) -> &[SavedSection] {
        &self.saved_sections
    }

    /// The saved section named `name`.
    pub fn saved_section(&self, name: &str) -> Option<&SavedSection> {
        self.saved_sections.iter().find(|s| s.name == name)
    }

    /// Keep `section`, replacing the one of its name in place; one op.
    pub fn save_section(&mut self, section: SavedSection) {
        let mut sections = self.saved_sections.clone();
        match sections.iter_mut().find(|s| s.name == section.name) {
            Some(slot) => *slot = section,
            None => sections.push(section),
        }
        self.set_saved_sections(sections);
    }

    /// Drop the saved section named `name`; whether there was one.
    pub fn remove_saved_section(&mut self, name: &str) -> bool {
        let mut sections = self.saved_sections.clone();
        sections.retain(|s| s.name != name);
        let found = sections.len() != self.saved_sections.len();
        self.set_saved_sections(sections);
        found
    }

    /// Set the whole list, recording one op when it changes.
    pub fn set_saved_sections(&mut self, sections: Vec<SavedSection>) {
        if self.saved_sections != sections {
            self.record_and_apply(crate::op::DocumentOp::SetSavedSections { sections });
        }
    }
}
