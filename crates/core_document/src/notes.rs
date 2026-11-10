//! Notes: text the user, or an agent, puts on a face, an edge or a point
//! of a body, drawn as a leader from the point to a label. A note is kept
//! in the body's own frame, so it moves with the body, and names what it
//! was put on: a face by its name (by its index on a mesh, which names
//! none), an edge by the names of the two faces it runs between. A rebuild
//! that moves the face or edge carries the note along (the point is taken
//! to its nearest point, and the label with it); one that takes it away
//! leaves the note lost, where it was. A text starting with `=` is a
//! formula: the note shows what it comes to. Each note has a short number,
//! given in order across the document: a note taken away leaves its number
//! unused, and only undoing the note's adding gives it again.

use crate::mesh_query::closest_on_triangle;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use kernel_api::naming::TopoName;

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

/// Who wrote a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteAuthor {
    #[default]
    User,
    Agent,
}

impl NoteAuthor {
    pub fn word(self) -> &'static str {
        match self {
            NoteAuthor::User => "user",
            NoteAuthor::Agent => "agent",
        }
    }

    fn is_user(&self) -> bool {
        *self == NoteAuthor::User
    }
}

/// An edge of a body's mesh: the names of the two faces it runs between,
/// and its index, which finds it where the mesh names none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeKey {
    pub faces: [TopoName; 2],
    pub index: u32,
}

/// What a new note is put on, near the point given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteWant {
    Point,
    Face,
    /// The edge of that index, or the nearest edge.
    Edge(Option<u32>),
}

/// Where a note stands on a mesh as built now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotePlaced {
    pub at: [f32; 3],
    pub label: [f32; 3],
    /// Its face or edge is not on the mesh any more.
    pub lost: bool,
    /// The face it stands on, by index, where it stands on one.
    pub face: Option<u32>,
    /// The edge it stands on, by index.
    pub edge: Option<u32>,
}

/// A note on a body: a leader from `at` to a label at `label`, both in the
/// body's own frame. Two notes are equal by what they say and where, not by
/// where they were read from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: NoteId,
    /// Its short number in the document, given when it is added.
    #[serde(default, skip_serializing_if = "crate::is_zero")]
    pub number: u32,
    /// What the label says; a formula when it starts with `=`.
    pub text: String,
    /// Where the leader starts: a point on the body.
    pub at: [f32; 3],
    /// Where the label stands.
    pub label: [f32; 3],
    /// The face `at` was put on, found again by its name after a rebuild.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<FaceKey>,
    /// The edge `at` was put on, in place of a face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<EdgeKey>,
    #[serde(default, skip_serializing_if = "NoteAuthor::is_user")]
    pub author: NoteAuthor,
    /// Milliseconds since the epoch, given when it is added.
    #[serde(default, skip_serializing_if = "is_zero_time")]
    pub created_at: i64,
    /// Kept off the view, while the body shows.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// A question for the user to answer in place (an agent's, about the
    /// geometry it points at), rather than a remark.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub asks: bool,
    /// Resolved, with what was done; `None` while open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    /// Read from the file rather than written while it was open: content
    /// to show an agent as such, never as instructions to follow.
    #[serde(skip, default = "read_from_file")]
    pub from_file: bool,
}

fn read_from_file() -> bool {
    true
}

fn is_zero_time(t: &i64) -> bool {
    *t == 0
}

impl PartialEq for Note {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.number == other.number
            && self.text == other.text
            && self.at == other.at
            && self.label == other.label
            && self.face == other.face
            && self.edge == other.edge
            && self.author == other.author
            && self.created_at == other.created_at
            && self.hidden == other.hidden
            && self.asks == other.asks
            && self.resolved == other.resolved
    }
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
            number: 0,
            text,
            at,
            label,
            face,
            edge: None,
            author: NoteAuthor::User,
            created_at: 0,
            hidden: false,
            asks: false,
            resolved: None,
            from_file: false,
        }
    }

    /// Where the note stands on `mesh` (its body's own, as built now): its
    /// point on its face or edge found again, at the nearest point of it,
    /// the label moved alike. As stored, and lost, when the face or edge is
    /// gone; as stored, and not lost, without a mesh to look on.
    pub fn placed(&self, mesh: Option<&TriMesh>) -> NotePlaced {
        let stored = NotePlaced {
            at: self.at,
            label: self.label,
            lost: false,
            face: None,
            edge: None,
        };
        let Some(mesh) = mesh.filter(|m| !m.indices.is_empty()) else {
            return stored;
        };
        let at = Vec3::from(self.at);
        let moved_to = |point: Vec3| {
            let shift = point - at;
            (
                point.to_array(),
                (Vec3::from(self.label) + shift).to_array(),
            )
        };
        if let Some(edge) = self.edge {
            return match find_edge(mesh, edge, at) {
                Some((index, point)) => {
                    let (at, label) = moved_to(point);
                    NotePlaced {
                        at,
                        label,
                        edge: Some(index),
                        ..stored
                    }
                }
                None => NotePlaced {
                    lost: true,
                    ..stored
                },
            };
        }
        let Some(key) = self.face else {
            return NotePlaced {
                face: closest_on_mesh(mesh, at, None).and_then(|(_, t)| mesh.faces.get(t).copied()),
                ..stored
            };
        };
        let count = mesh.faces.iter().max().map_or(0, |m| m + 1);
        let found = (0..count)
            .find(|i| key.is_face(mesh, *i))
            .and_then(|face| Some((closest_on_mesh(mesh, at, Some(face))?.0, face)));
        match found {
            Some((point, face)) => {
                let (at, label) = moved_to(point);
                NotePlaced {
                    at,
                    label,
                    face: Some(face),
                    ..stored
                }
            }
            None => NotePlaced {
                lost: count > 0,
                ..stored
            },
        }
    }
}

/// The edge `key` names on `mesh` (by its faces' names, else its index;
/// of several between the same faces, the one nearest `p`) and its point
/// nearest `p`.
fn find_edge(mesh: &TriMesh, key: EdgeKey, p: Vec3) -> Option<(u32, Vec3)> {
    let named = key.faces.iter().all(|n| *n != 0) && !mesh.edge_faces.is_empty();
    let candidates: Vec<u32> = if named {
        let mut wanted = key.faces;
        wanted.sort_unstable();
        (0..mesh.edge_faces.len() as u32)
            .filter(|e| {
                let mut have = mesh.edge_faces[*e as usize];
                have.sort_unstable();
                have == wanted
            })
            .collect()
    } else if mesh.edge_ids.contains(&key.index) {
        vec![key.index]
    } else {
        Vec::new()
    };
    candidates
        .into_iter()
        .filter_map(|edge| Some((edge, closest_on_edge(mesh, edge, p)?)))
        .min_by(|a, b| a.1.distance_squared(p).total_cmp(&b.1.distance_squared(p)))
}

/// Where a note put near `p` (in `mesh`'s frame) lands, and the face or
/// edge it is on: the nearest point of the mesh, on the face there or the
/// edge wanted. A mesh without faces gives a point; one without edges, a
/// face.
pub fn spot_at(
    mesh: Option<&TriMesh>,
    p: [f32; 3],
    want: NoteWant,
) -> ([f32; 3], Option<FaceKey>, Option<EdgeKey>) {
    let p = Vec3::from(p);
    let Some(mesh) = mesh.filter(|m| !m.indices.is_empty()) else {
        return (p.to_array(), None, None);
    };
    if let NoteWant::Edge(index) = want {
        let edges: Vec<u32> = match index {
            Some(index) => vec![index],
            None => {
                let mut ids = mesh.edge_ids.clone();
                ids.sort_unstable();
                ids.dedup();
                ids
            }
        };
        let nearest = edges
            .into_iter()
            .filter_map(|e| Some((e, closest_on_edge(mesh, e, p)?)))
            .min_by(|a, b| a.1.distance_squared(p).total_cmp(&b.1.distance_squared(p)));
        if let Some((index, point)) = nearest {
            let faces = mesh
                .edge_faces
                .get(index as usize)
                .copied()
                .unwrap_or_default();
            return (point.to_array(), None, Some(EdgeKey { faces, index }));
        }
    }
    let Some((point, triangle)) = closest_on_mesh(mesh, p, None) else {
        return (p.to_array(), None, None);
    };
    let face = match want {
        NoteWant::Point => None,
        _ => mesh.faces.get(triangle).map(|f| FaceKey::of(mesh, *f)),
    };
    (point.to_array(), face, None)
}

/// The point of edge `edge`'s outline segments nearest `p`.
fn closest_on_edge(mesh: &TriMesh, edge: u32, p: Vec3) -> Option<Vec3> {
    mesh.edge_ids
        .iter()
        .zip(mesh.edges.as_chunks::<2>().0)
        .filter(|(id, _)| **id == edge)
        .filter_map(|(_, [a, b])| {
            let a = Vec3::from(*mesh.positions.get(*a as usize)?);
            let b = Vec3::from(*mesh.positions.get(*b as usize)?);
            let ab = b - a;
            let t = if ab.length_squared() > 0.0 {
                ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            Some(a + ab * t)
        })
        .min_by(|a, b| a.distance_squared(p).total_cmp(&b.distance_squared(p)))
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

impl Document {
    /// The notes on `body`, in the order they were made.
    pub fn body_notes(&self, body: BodyId) -> &[Note] {
        self.body(body).map_or(&[], |b| b.notes.as_slice())
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

    /// The note numbered `number`, and the body it is on.
    pub fn note_by_number(&self, number: u32) -> Option<(BodyId, &Note)> {
        self.notes().find(|(_, n)| n.number == number)
    }

    /// The number the next note added takes.
    pub fn next_note_number(&self) -> u32 {
        self.notes_numbered + 1
    }

    /// A note saying `text` on `body` by `author`, not yet added: at the
    /// point of the body nearest `at` (where the body sits in the
    /// document), or at the top of the body's middle without one, on what
    /// `want` asks for there; its label out from the face the point is on.
    pub fn new_note(
        &self,
        body: BodyId,
        text: String,
        author: NoteAuthor,
        at: Option<[f32; 3]>,
        want: NoteWant,
    ) -> Note {
        let placement = self.body_placement(body);
        let local = self.local_geometry(body);
        let mesh = local.as_ref().map(|(mesh, _)| mesh.as_ref());
        let bounds = local
            .as_ref()
            .and_then(|(mesh, bounds)| bounds.or_else(|| mesh.bounds()));
        let wanted = match (at, bounds) {
            (Some(at), _) => placement.inverse().point(at),
            (None, Some((lo, hi))) => [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, hi[2]],
            (None, None) => [0.0; 3],
        };
        let size = bounds.map_or(10.0, |(lo, hi)| Vec3::from(hi).distance(Vec3::from(lo)));
        let (point, face, edge) = spot_at(mesh, wanted, want);
        let normal = mesh
            .and_then(|mesh| {
                let (_, triangle) = closest_on_mesh(mesh, Vec3::from(point), None)?;
                let corners = mesh.indices.get(3 * triangle..3 * triangle + 3)?;
                Some(
                    corners
                        .iter()
                        .filter_map(|i| mesh.normals.get(*i as usize))
                        .map(|n| Vec3::from(*n))
                        .sum::<Vec3>(),
                )
            })
            .unwrap_or(Vec3::Z);
        Note {
            face,
            edge,
            author,
            ..Note::pinned(text, point, label_for(point, normal.to_array(), size), None)
        }
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
    /// document: on its face or edge as the body is built now, where the
    /// body sits.
    pub fn note_placed(&self, body: BodyId, note: &Note) -> NotePlaced {
        let local = self.local_geometry(body);
        let placed = note.placed(local.as_ref().map(|(mesh, _)| mesh.as_ref()));
        let placement = self.body_placement(body);
        NotePlaced {
            at: placement.point(placed.at),
            label: placement.point(placed.label),
            ..placed
        }
    }

    /// Whether a note shows: its body shown and the note not hidden.
    pub fn note_effective_visible(&self, id: NoteId) -> bool {
        self.note(id)
            .is_some_and(|(body, note)| !note.hidden && self.imported_body_effective_visible(body))
    }

    /// Set every note on `body` at once; one op when they change.
    pub fn set_body_notes(&mut self, body: BodyId, notes: Vec<Note>) {
        if self.body(body).is_some_and(|b| b.notes != notes) {
            let highest = notes.iter().map(|n| n.number).max().unwrap_or(0);
            self.record_and_apply(op::DocumentOp::SetBodyNotes {
                id: body,
                notes,
                numbered: self.notes_numbered.max(highest),
            });
        }
    }

    /// Put `note` on `body` under the document's next number, stamped now.
    /// `None` when there is no such body.
    pub fn add_note(&mut self, body: BodyId, mut note: Note) -> Option<NoteId> {
        let id = note.id;
        let mut notes = self.body(body)?.notes.clone();
        note.number = self.next_note_number();
        note.created_at = crate::epoch_ms_now();
        notes.push(note);
        self.set_body_notes(body, notes);
        Some(id)
    }

    /// Change a note to `note`, found by its id; false when there is none.
    /// Its number, author and time stay as they were given.
    pub fn set_note(&mut self, note: Note) -> bool {
        let Some((body, _)) = self.note(note.id) else {
            return false;
        };
        let notes = self
            .body_notes(body)
            .iter()
            .map(|n| {
                if n.id == note.id {
                    Note {
                        number: n.number,
                        author: n.author,
                        created_at: n.created_at,
                        ..note.clone()
                    }
                } else {
                    n.clone()
                }
            })
            .collect();
        self.set_body_notes(body, notes);
        true
    }

    /// Number the notes of a file written before notes had numbers, in
    /// body order after the highest number given. Every replica reading
    /// the file numbers them alike.
    pub(crate) fn number_unnumbered_notes(&mut self) {
        let mut next = self.notes_numbered;
        for body in &mut self.bodies {
            for note in body.notes.iter_mut().filter(|n| n.number == 0) {
                next += 1;
                note.number = next;
            }
        }
        self.notes_numbered = next;
    }

    /// Take a note away; false when there is none. Its number is not given
    /// to the next note.
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

    /// A unit square floor (face 0, named 10) and a wall at x = `x` (face
    /// 1, named 20), with the edge where they meet (edge 0) and the
    /// floor's far side (edge 1).
    fn corner(x: f32) -> TriMesh {
        TriMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [x, 0.0, 0.0],
                [x, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [x, 0.0, 1.0],
                [x, 1.0, 1.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2],
            faces: vec![0, 0, 1, 1],
            face_names: vec![10, 20],
            edges: vec![1, 2, 0, 3],
            edge_ids: vec![0, 1],
            edge_faces: vec![[10, 20], [10, 30]],
            ..Default::default()
        }
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        Vec3::from(a).distance(Vec3::from(b)) < 1e-5
    }

    #[test]
    fn a_note_follows_its_face_or_edge_by_name_and_is_lost_without_it() {
        let mesh = corner(1.0);
        let (at, face, _) = spot_at(Some(&mesh), [0.9, 0.5, 0.8], NoteWant::Face);
        assert_eq!(face, Some(FaceKey { name: 20, index: 1 }));
        assert!(near(at, [1.0, 0.5, 0.8]));
        let (at, _, edge) = spot_at(Some(&mesh), [0.9, 0.5, 0.1], NoteWant::Edge(None));
        assert_eq!(
            edge,
            Some(EdgeKey {
                faces: [10, 20],
                index: 0
            })
        );
        assert!(near(at, [1.0, 0.5, 0.0]));

        // The wall moves out: both notes go with it, their labels alike.
        let moved = corner(2.0);
        let on_face = Note {
            face: Some(FaceKey { name: 20, index: 1 }),
            ..Note::pinned(String::new(), [1.0, 0.5, 0.8], [3.0, 0.5, 0.8], None)
        };
        let placed = on_face.placed(Some(&moved));
        assert!(near(placed.at, [2.0, 0.5, 0.8]) && near(placed.label, [4.0, 0.5, 0.8]));
        assert!(!placed.lost && placed.face == Some(1));
        let on_edge = Note {
            edge: Some(EdgeKey {
                faces: [20, 10],
                index: 7,
            }),
            ..Note::pinned(String::new(), [1.0, 0.5, 0.0], [1.0, 0.5, 2.0], None)
        };
        let placed = on_edge.placed(Some(&moved));
        assert!(near(placed.at, [2.0, 0.5, 0.0]) && !placed.lost && placed.edge == Some(0));

        // A rebuild that names the wall otherwise loses both, where they were.
        let mut renamed = moved.clone();
        renamed.face_names = vec![10, 21];
        renamed.edge_faces = vec![[10, 21], [10, 30]];
        for note in [&on_face, &on_edge] {
            let placed = note.placed(Some(&renamed));
            assert!(placed.lost);
            assert_eq!((placed.at, placed.label), (note.at, note.label));
        }
        // Without a mesh to look on, nothing is lost.
        assert!(!on_face.placed(None).lost);
    }
}
