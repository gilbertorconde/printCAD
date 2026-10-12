//! Notes: text the user pins to a point on a body, drawn as a leader from
//! the point to a label. A note is kept in the body's own frame, so it
//! moves with the body, and names the face it was put on, so a rebuild
//! that moves the face carries the note along (the point is taken to the
//! nearest point of the face found by its name, and the label with it).
//! A text starting with `=` is a formula: the note shows what it comes to.

use glam::Vec3;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{BodyId, Document, FaceKey, TriMesh, op};

/// Unique identifier for a note in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct NoteId(pub Uuid);

impl NoteId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for NoteId {
    fn default() -> Self {
        Self::new()
    }
}

/// A note on a body: a leader from `at` to a label at `label`, both in the
/// body's own frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: NoteId,
    /// What the label says; a formula when it starts with `=`.
    pub text: String,
    /// Where the leader starts: a point on the body.
    pub at: [f32; 3],
    /// Where the label stands.
    pub label: [f32; 3],
    /// The face `at` was put on, found again by its name after a rebuild.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<FaceKey>,
    /// Kept off the view, while the body shows.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

impl Note {
    /// A note at `at` on `mesh` (the body's own), its label `label`; the
    /// face it is on taken from the mesh where the point lies on one.
    pub fn pinned(text: String, at: [f32; 3], label: [f32; 3], mesh: Option<&TriMesh>) -> Self {
        let face = mesh.and_then(|mesh| {
            let (_, triangle) = closest_on_mesh(mesh, Vec3::from(at), None)?;
            let index = *mesh.faces.get(triangle)?;
            Some(FaceKey::of(mesh, index))
        });
        Self {
            id: NoteId::new(),
            text,
            at,
            label,
            face,
            hidden: false,
        }
    }

    /// Where the leader starts and the label stands on `mesh` (the body's
    /// own, as built now): the stored point taken to the nearest point of
    /// its face, the label moved alike. As stored when the face is not
    /// found.
    pub fn resolved(&self, mesh: Option<&TriMesh>) -> ([f32; 3], [f32; 3]) {
        let (Some(mesh), Some(key)) = (mesh, self.face) else {
            return (self.at, self.label);
        };
        let count = mesh.faces.iter().max().map_or(0, |m| m + 1);
        let Some(face) = (0..count).find(|i| key.is_face(mesh, *i)) else {
            return (self.at, self.label);
        };
        let Some((point, _)) = closest_on_mesh(mesh, Vec3::from(self.at), Some(face)) else {
            return (self.at, self.label);
        };
        let moved = point - Vec3::from(self.at);
        (
            point.to_array(),
            (Vec3::from(self.label) + moved).to_array(),
        )
    }
}

/// Where a label goes for a note at `at` on a face facing `normal`, on a
/// body `size` across: out along the normal, a fifth of the body away.
pub fn label_for(at: [f32; 3], normal: [f32; 3], size: f32) -> [f32; 3] {
    let normal = Vec3::from(normal).normalize_or(Vec3::Z);
    let reach = (size * 0.2).clamp(2.0, 50.0);
    (Vec3::from(at) + normal * reach).to_array()
}

/// The point of `mesh` nearest `p`, and the triangle it lies on; only
/// face `face`'s triangles when given.
pub fn closest_on_mesh(mesh: &TriMesh, p: Vec3, face: Option<u32>) -> Option<(Vec3, usize)> {
    let mut best: Option<(f32, Vec3, usize)> = None;
    for (t, corners) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        if let Some(face) = face
            && mesh.faces.get(t) != Some(&face)
        {
            continue;
        }
        let corner = |i: usize| {
            mesh.positions
                .get(corners[i] as usize)
                .map(|p| Vec3::from(*p))
        };
        let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else {
            continue;
        };
        let q = closest_on_triangle(p, a, b, c);
        let d = q.distance_squared(p);
        if best.is_none_or(|(least, _, _)| d < least) {
            best = Some((d, q, t));
        }
    }
    best.map(|(_, q, t)| (q, t))
}

/// The point of triangle `abc` nearest `p`.
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = va + vb + vc;
    if denom.abs() <= f32::EPSILON {
        return a;
    }
    a + ab * (vb / denom) + ac * (vc / denom)
}

impl Document {
    /// The notes on `body`, in the order they were made.
    pub fn body_notes(&self, body: BodyId) -> &[Note] {
        self.bodies
            .iter()
            .find(|b| b.id == body)
            .map_or(&[], |b| b.notes.as_slice())
    }

    /// Every note, with its body, in body order.
    pub fn notes(&self) -> impl Iterator<Item = (BodyId, &Note)> {
        self.bodies
            .iter()
            .flat_map(|b| b.notes.iter().map(move |n| (b.id, n)))
    }

    /// A note and the body it is on.
    pub fn note(&self, id: NoteId) -> Option<(BodyId, &Note)> {
        self.notes().find(|(_, n)| n.id == id)
    }

    /// A note saying `text` on `body`, not yet added: at the point of the
    /// body nearest `at` (where the body sits in the document), or at the
    /// top of the body's middle without one; its label out from the face
    /// the point is on.
    pub fn new_note(&self, body: BodyId, text: String, at: Option<[f32; 3]>) -> Note {
        let placement = self.body_placement(body);
        let local = self.local_geometry(body);
        let mesh = local.as_ref().map(|(mesh, _)| mesh.as_ref());
        let bounds = local
            .as_ref()
            .and_then(|(mesh, bounds)| bounds.or_else(|| mesh.bounds()));
        let wanted = match (at, bounds) {
            (Some(at), _) => Vec3::from(placement.inverse().point(at)),
            (None, Some((lo, hi))) => {
                Vec3::new((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, hi[2])
            }
            (None, None) => Vec3::ZERO,
        };
        let size = bounds.map_or(10.0, |(lo, hi)| Vec3::from(hi).distance(Vec3::from(lo)));
        let hit = mesh.and_then(|mesh| {
            let (point, triangle) = closest_on_mesh(mesh, wanted, None)?;
            let corners = mesh.indices.get(3 * triangle..3 * triangle + 3)?;
            let normal: Vec3 = corners
                .iter()
                .filter_map(|i| mesh.normals.get(*i as usize))
                .map(|n| Vec3::from(*n))
                .sum();
            Some((point, normal))
        });
        let (point, normal) = hit.unwrap_or((wanted, Vec3::Z));
        let at = point.to_array();
        Note::pinned(text, at, label_for(at, normal.to_array(), size), mesh)
    }

    /// What a note's label shows: its text, or what its formula comes to.
    pub fn note_text(&self, note: &Note) -> String {
        let Some(formula) = note.text.trim_start().strip_prefix('=') else {
            return note.text.clone();
        };
        match self.evaluate_formula(formula, None) {
            Ok(value) => value.display(self.display_unit(), 3),
            Err(why) => format!("{} ({why})", note.text),
        }
    }

    /// Where a note's leader starts and its label stands, in the
    /// document: on its face as the body is built now, where the body
    /// sits.
    pub fn note_placed(&self, body: BodyId, note: &Note) -> ([f32; 3], [f32; 3]) {
        let local = self.local_geometry(body);
        let (at, label) = note.resolved(local.as_ref().map(|(mesh, _)| mesh.as_ref()));
        let placement = self.body_placement(body);
        (placement.point(at), placement.point(label))
    }

    /// Whether a note shows: its body shown and the note not hidden.
    pub fn note_effective_visible(&self, id: NoteId) -> bool {
        self.note(id)
            .is_some_and(|(body, note)| !note.hidden && self.imported_body_effective_visible(body))
    }

    /// Set every note on `body` at once.
    pub fn set_body_notes(&mut self, body: BodyId, notes: Vec<Note>) {
        if self.bodies.iter().any(|b| b.id == body && b.notes != notes) {
            self.record_and_apply(op::DocumentOp::SetBodyNotes { id: body, notes });
        }
    }

    /// Put `note` on `body`. `None` when there is no such body.
    pub fn add_note(&mut self, body: BodyId, note: Note) -> Option<NoteId> {
        let id = note.id;
        let mut notes = self.bodies.iter().find(|b| b.id == body)?.notes.clone();
        notes.push(note);
        self.set_body_notes(body, notes);
        Some(id)
    }

    /// Change a note to `note`, found by its id; false when there is none.
    pub fn set_note(&mut self, note: Note) -> bool {
        let Some((body, _)) = self.note(note.id) else {
            return false;
        };
        let notes = self
            .body_notes(body)
            .iter()
            .map(|n| {
                if n.id == note.id {
                    note.clone()
                } else {
                    n.clone()
                }
            })
            .collect();
        self.set_body_notes(body, notes);
        true
    }

    /// Take a note away; false when there is none.
    pub fn remove_note(&mut self, id: NoteId) -> bool {
        let Some((body, _)) = self.note(id) else {
            return false;
        };
        let notes = self
            .body_notes(body)
            .iter()
            .filter(|n| n.id != id)
            .cloned()
            .collect();
        self.set_body_notes(body, notes);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two unit squares, faces 0 (z = 0) and 1 (z = h), named 10 and 20.
    fn slab(h: f32) -> TriMesh {
        TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, h],
                [1.0, 0.0, h],
                [1.0, 1.0, h],
                [0.0, 1.0, h],
            ],
            indices: vec![0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7],
            faces: vec![0, 0, 1, 1],
            face_names: vec![10, 20],
            ..Default::default()
        }
    }

    #[test]
    fn a_note_follows_its_face_by_name_and_its_label_with_it() {
        let note = Note::pinned(
            "top".into(),
            [0.5, 0.5, 1.0],
            [0.5, 0.5, 3.0],
            Some(&slab(1.0)),
        );
        assert_eq!(note.face, Some(FaceKey { name: 20, index: 1 }));
        let (at, label) = note.resolved(Some(&slab(4.0)));
        assert_eq!(at, [0.5, 0.5, 4.0]);
        assert_eq!(label, [0.5, 0.5, 6.0]);
        // A mesh that no longer names the face leaves the note where it was.
        let mut unnamed = slab(4.0);
        unnamed.face_names = vec![10, 30];
        assert_eq!(note.resolved(Some(&unnamed)), (note.at, note.label));
    }

    #[test]
    fn the_nearest_point_of_a_triangle_is_inside_or_on_its_rim() {
        let (a, b, c) = (Vec3::ZERO, Vec3::X, Vec3::Y);
        let inside = closest_on_triangle(Vec3::new(0.25, 0.25, 2.0), a, b, c);
        assert!(inside.distance(Vec3::new(0.25, 0.25, 0.0)) < 1e-6);
        let past_corner = closest_on_triangle(Vec3::new(2.0, -1.0, 0.0), a, b, c);
        assert!(past_corner.distance(b) < 1e-6);
        let off_edge = closest_on_triangle(Vec3::new(1.0, 1.0, 0.0), a, b, c);
        assert!(off_edge.distance(Vec3::new(0.5, 0.5, 0.0)) < 1e-6);
    }
}
