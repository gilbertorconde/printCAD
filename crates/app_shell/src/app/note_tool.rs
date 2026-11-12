//! The note tool and the note card. With the tool armed a click on a part
//! opens a card by the point asking for the note's text (Enter puts the
//! note down on the edge or face there, Escape drops it); a click on a
//! note opens the card on it, to reword it, answer its question, resolve
//! or reopen it, or delete it. Also what the note commands and an agent's
//! context say of a note, and whether the part hides a note's point.

use core_document::{BodyId, Document, Note, NoteAuthor, NoteId, NoteWant, TriMesh};
use glam::{Mat4, Vec2, Vec3, Vec4};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::PrintCadApp;
use crate::app::face_pick::MeshStamp;

/// How near the cursor a note's point or label must be to be under it, in
/// logical pixels.
const REACH: f32 = 9.0;

/// The most triangles a body is searched for what hides a note's point;
/// past it the point counts as seen.
const OCCLUSION_TRIANGLES: usize = 400_000;

/// The note card: the body, and the new note waiting for its text or the
/// note it is open on.
#[derive(Debug, Clone)]
pub(crate) struct NoteCard {
    pub body: BodyId,
    pub editing: NoteEditing,
}

#[derive(Debug, Clone)]
pub(crate) enum NoteEditing {
    New(Box<Note>),
    Existing(NoteId),
}

/// What the UI draws of the note card.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteCardView {
    pub id: NoteId,
    /// The note's number, or the one a new note will take.
    pub number: u32,
    pub new: bool,
    /// What the field opens with: the note's text, or the answer to its
    /// question so far.
    pub text: String,
    /// The note is a question still open: the field answers it.
    pub answering: bool,
    /// The question, when the field answers one.
    pub question: String,
    pub resolved: bool,
    pub author: NoteAuthor,
    pub lost: bool,
    pub from_file: bool,
    /// Where the note's point is, in viewport pixels; `None` off screen.
    pub at: Option<[f32; 2]>,
}

impl PrintCadApp {
    /// A click with the note tool armed: on a note, its card opens; on a
    /// part, a new note waits on the card for its text, on the edge there
    /// or else the face; on nothing, the card goes. Whether to draw again.
    pub(crate) fn note_click(&mut self) -> bool {
        if let Some(id) = self.note_under_cursor() {
            self.open_note_card(id);
            return true;
        }
        let (Some(body), Some(point)) = (self.session.hovered_body, self.session.hovered_world_pos)
        else {
            return self.session.note_card.take().is_some();
        };
        let body = BodyId(body);
        let want = match self.session.hovered_edge {
            Some(edge) if edge.body == body.0 => NoteWant::Edge(Some(edge.edge)),
            _ => NoteWant::Face,
        };
        let note = self.session.document.new_note(
            body,
            String::new(),
            NoteAuthor::User,
            Some(point),
            want,
        );
        self.session.note_card = Some(NoteCard {
            body,
            editing: NoteEditing::New(Box::new(note)),
        });
        true
    }

    /// Open the card on note `id`.
    pub(crate) fn open_note_card(&mut self, id: NoteId) {
        if let Some((body, _)) = self.session.document.note(id) {
            self.session.note_card = Some(NoteCard {
                body,
                editing: NoteEditing::Existing(id),
            });
        }
    }

    /// The card's text saved: the new note put down with it, the open
    /// question answered with it, or the note reworded. A new note with
    /// nothing to say is not put down.
    pub(crate) fn save_note_card(&mut self, text: String) {
        let Some(card) = self.session.note_card.take() else {
            return;
        };
        let text = text.trim().to_string();
        let label = match card.editing {
            NoteEditing::New(mut note) => {
                if text.is_empty() {
                    return;
                }
                note.text = text;
                let Some(id) = self.session.document.add_note(card.body, *note) else {
                    return;
                };
                let number = self.session.document.note(id).map_or(0, |(_, n)| n.number);
                format!("Note #{number}")
            }
            NoteEditing::Existing(id) => {
                let Some((_, note)) = self.session.document.note(id) else {
                    return;
                };
                let mut note = note.clone();
                let label = if note.asks && note.resolved.is_none() {
                    note.resolved = Some(text);
                    format!("Answer note #{}", note.number)
                } else {
                    note.text = text;
                    format!("Edit note #{}", note.number)
                };
                self.session.document.set_note(note);
                label
            }
        };
        self.session.journal.label_next(label);
        self.close_gesture();
    }

    /// Resolve a note, saying what was done, or open it again (`None`).
    pub(crate) fn resolve_note(&mut self, id: NoteId, resolved: Option<String>) {
        let Some((_, note)) = self.session.document.note(id) else {
            return;
        };
        let mut note = note.clone();
        let label = match resolved {
            Some(_) => format!("Resolve note #{}", note.number),
            None => format!("Reopen note #{}", note.number),
        };
        note.resolved = resolved;
        self.session.document.set_note(note);
        self.session.journal.label_next(label);
        self.close_gesture();
    }

    /// What the UI draws of the note card, while it is open on a note that
    /// is there.
    pub(crate) fn note_card_view(&self) -> Option<NoteCardView> {
        let card = self.session.note_card.as_ref()?;
        let document = &self.session.document;
        let (number, new, note, placed) = match &card.editing {
            NoteEditing::New(note) => (
                document.next_note_number(),
                true,
                note.as_ref(),
                document.note_placed(card.body, note),
            ),
            NoteEditing::Existing(id) => {
                let (body, note) = document.note(*id)?;
                (note.number, false, note, document.note_placed(body, note))
            }
        };
        let answering = !new && note.asks && note.resolved.is_none();
        Some(NoteCardView {
            id: note.id,
            number,
            new,
            text: if answering {
                String::new()
            } else {
                note.text.clone()
            },
            answering,
            question: document.note_text(note),
            resolved: note.resolved.is_some(),
            author: note.author,
            lost: placed.lost,
            from_file: note.from_file,
            at: self
                .session
                .camera
                .world_to_viewport(Vec3::from_array(placed.at))
                .map(|(x, y)| [x, y]),
        })
    }

    /// The shown note whose point or label is nearest the cursor, within
    /// reach.
    fn note_under_cursor(&self) -> Option<NoteId> {
        let cursor = Vec2::from(self.input.cursor_in_viewport?);
        let reach = REACH * self.pixels_per_point();
        let document = &self.session.document;
        let camera = &self.session.camera;
        document
            .notes()
            .filter(|(_, note)| document.note_effective_visible(note.id))
            .flat_map(|(body, note)| {
                let placed = document.note_placed(body, note);
                [placed.at, placed.label].map(|p| {
                    camera
                        .world_to_viewport(Vec3::from_array(p))
                        .map(|(x, y)| (Vec2::new(x, y).distance(cursor), note.id))
                })
            })
            .flatten()
            .filter(|(d, _)| *d <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }

    /// Whether `body`'s own shape hides the world point `at` of `note`
    /// from the view.
    pub(crate) fn hidden_by_part(&self, note: NoteId, body: BodyId, at: [f32; 3]) -> bool {
        let Some(geometry) = self.session.document.imported_geometry(body) else {
            return false;
        };
        self.session.note_occlusions.hidden(
            note,
            &geometry.mesh,
            geometry.revision,
            self.session.camera.view_projection(),
            at,
        )
    }
}

/// Whether each note's point is hidden by its body ([`hides`]), kept by
/// note with everything the answer reads: the body's mesh, the view and
/// the point. A frame that moves none of them reads the answer rather
/// than casting the ray through every triangle again.
#[derive(Default)]
pub(crate) struct Occlusions(std::cell::RefCell<HashMap<NoteId, (OcclusionKey, bool)>>);

#[derive(PartialEq)]
struct OcclusionKey {
    mesh: MeshStamp,
    view: [u32; 16],
    at: [u32; 3],
}

impl Occlusions {
    /// Whether `mesh`, a body's placed mesh at `revision`, hides `at` from
    /// the view `view` (a view-projection, columns first).
    pub fn hidden(
        &self,
        note: NoteId,
        mesh: &Arc<TriMesh>,
        revision: u64,
        view: [[f32; 4]; 4],
        at: [f32; 3],
    ) -> bool {
        let view = Mat4::from_cols_array_2d(&view);
        let key = OcclusionKey {
            mesh: MeshStamp::of(mesh, revision),
            view: view.to_cols_array().map(f32::to_bits),
            at: at.map(f32::to_bits),
        };
        let mut kept = self.0.borrow_mut();
        if let Some((_, hidden)) = kept.get(&note).filter(|(k, _)| *k == key) {
            return *hidden;
        }
        let hidden = hides(mesh, view, Vec3::from_array(at));
        kept.insert(note, (key, hidden));
        hidden
    }

    /// Let go of the answers for notes not among `shown`.
    pub fn retain(&self, shown: &HashSet<NoteId>) {
        self.0.borrow_mut().retain(|note, _| shown.contains(note));
    }
}

/// Whether a triangle of `mesh` (in the world) lies between the view and
/// `at`: on the ray from the near plane through `at`, short of it by more
/// than the point's own surface.
fn hides(mesh: &TriMesh, view: Mat4, at: Vec3) -> bool {
    if mesh.indices.len() / 3 > OCCLUSION_TRIANGLES {
        return false;
    }
    let clip = view * at.extend(1.0);
    if clip.w.abs() < 1e-12 {
        return false;
    }
    let ndc = clip.truncate() / clip.w;
    let near = view.inverse() * Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
    if near.w.abs() < 1e-12 {
        return false;
    }
    let from = near.truncate() / near.w;
    let span = at - from;
    let length = span.length();
    if length < 1e-6 {
        return false;
    }
    let dir = span / length;
    // The point lies on the mesh: a hit within this of it is its own face.
    let slack = 1e-3 * length + 1e-3;
    mesh.indices.as_chunks::<3>().0.iter().any(|[a, b, c]| {
        let corner = |i: &u32| mesh.positions.get(*i as usize).map(|p| Vec3::from(*p));
        let (Some(a), Some(b), Some(c)) = (corner(a), corner(b), corner(c)) else {
            return false;
        };
        core_document::mesh_query::ray_triangle(from, dir, [a, b, c])
            .is_some_and(|t| t < length - slack)
    })
}

/// A note as the note commands and an agent's context give it: what it
/// says and how it stands, where it is now, and what it is on.
pub(crate) fn note_json(document: &Document, body: BodyId, note: &Note) -> Value {
    let placed = document.note_placed(body, note);
    let mut out = json!({
        "id": note.id.0.to_string(),
        "number": note.number,
        "body": body.0.to_string(),
        "body_name": document
            .body(body)
            .map(|b| b.name.clone())
            .unwrap_or_default(),
        "text": note.text,
        "shows": document.note_text(note),
        "author": note.author.word(),
        "point": placed.at,
        "label": placed.label,
        "visible": !note.hidden,
        "on": if note.edge.is_some() {
            "edge"
        } else if note.face.is_some() {
            "face"
        } else {
            "point"
        },
        "on_face": note.face.is_some(),
        "lost": placed.lost,
        "asks": note.asks,
        "resolved": note.resolved.is_some(),
        "done": note.resolved,
        "from_file": note.from_file,
    });
    let Some(geometry) = document.imported_geometry(body).filter(|_| !placed.lost) else {
        return out;
    };
    let mesh = &geometry.mesh;
    if let Some(face) = placed.face {
        if let Some(surface) = mesh.face_surfaces.get(face as usize) {
            let mut described = crate::app::scripts::surface_json(*surface);
            described["words"] = json!(surface_words(surface));
            out["surface"] = described;
        }
        let maker = mesh
            .face_names
            .get(face as usize)
            .and_then(|name| document.face_maker(body, *name));
        if let Some(feature) = maker {
            out["feature"] = json!(feature.0.to_string());
            if let Some(node) = document.get_feature_meta(feature) {
                out["feature_name"] = json!(node.name);
            }
        }
    }
    if let Some(edge) = placed.edge
        && let Some(hit) = crate::app::edges::edge_hit(mesh, body.0, edge)
    {
        let mut described = json!({"length": hit.length_mm, "direction": hit.direction});
        if let Some(circle) = hit.circle {
            described["centre"] = json!(circle.center);
            described["radius"] = json!(circle.radius);
            described["normal"] = json!(circle.normal);
        }
        out["edge"] = described;
    }
    out
}

/// A face's surface in a few words: "cylinder, diameter 4.2, axis
/// (0, 0, 1) through (10, 5, 0)".
fn surface_words(surface: &kernel_api::FaceSurface) -> String {
    use kernel_api::FaceSurface as S;
    let r = |x: f32| (x * 1000.0).round() / 1000.0;
    let v = |p: [f32; 3]| {
        let n = |x: f32| {
            let x = r(x);
            if x == 0.0 {
                "0".to_string()
            } else {
                x.to_string()
            }
        };
        format!("({}, {}, {})", n(p[0]), n(p[1]), n(p[2]))
    };
    match *surface {
        S::Plane { origin, normal } => format!("plane facing {} through {}", v(normal), v(origin)),
        S::Cylinder {
            origin,
            axis,
            radius,
        } => format!(
            "cylinder, diameter {}, axis {} through {}",
            r(2.0 * radius),
            v(axis),
            v(origin)
        ),
        S::Cone {
            apex,
            axis,
            half_angle,
        } => format!(
            "cone, half angle {}°, axis {} from apex {}",
            r(half_angle.to_degrees()),
            v(axis),
            v(apex)
        ),
        S::Sphere { center, radius } => {
            format!("sphere, diameter {}, centre {}", r(2.0 * radius), v(center))
        }
        S::Torus {
            center,
            axis,
            major_radius,
            minor_radius,
        } => format!(
            "torus, radii {} and {}, axis {} through {}",
            r(major_radius),
            r(minor_radius),
            v(axis),
            v(center)
        ),
        S::Other => "free-form surface".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box from 0 to 10 (its top and bottom), and a view looking down
    /// from z = 50 onto x and y from -15 to 25: depth grows downwards,
    /// nearest at the top.
    fn box_from_above() -> (TriMesh, Mat4) {
        let mut mesh = TriMesh::default();
        let corners = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 10.0],
            [10.0, 0.0, 10.0],
            [10.0, 10.0, 10.0],
            [0.0, 10.0, 10.0],
        ];
        mesh.positions = corners.to_vec();
        mesh.indices = vec![0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7];
        let view = Mat4::from_cols(
            Vec4::new(0.05, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 0.05, 0.0, 0.0),
            Vec4::new(0.0, 0.0, -0.01, 0.0),
            Vec4::new(-0.25, -0.25, 0.5, 1.0),
        );
        (mesh, view)
    }

    /// Seen from straight above, a point on the box's top is seen, one on
    /// its bottom or inside is hidden.
    #[test]
    fn the_part_hides_a_point_behind_its_near_side() {
        let (mesh, view) = box_from_above();
        assert!(!hides(&mesh, view, Vec3::new(5.0, 5.0, 10.0)), "on top");
        assert!(hides(&mesh, view, Vec3::new(5.0, 5.0, 0.0)), "underneath");
        assert!(hides(&mesh, view, Vec3::new(5.0, 5.0, 4.0)), "inside");
        assert!(!hides(&mesh, view, Vec3::new(15.0, 5.0, 0.0)), "beside it");
    }

    /// The kept answer is the one worked out afresh after any of what it
    /// reads moves: the point, the view, the mesh (another at the same
    /// revision, or the same at another).
    #[test]
    fn a_kept_occlusion_follows_the_point_the_view_and_the_mesh() {
        let (mesh, view) = box_from_above();
        let mesh = Arc::new(mesh);
        let mut open = (*mesh).clone();
        open.indices.truncate(6);
        let open = Arc::new(open);
        // The same box seen from below: z mirrored.
        let below =
            view * Mat4::from_cols(Vec4::X, Vec4::Y, -Vec4::Z, Vec4::new(0.0, 0.0, 10.0, 1.0));
        let note = NoteId(uuid::Uuid::new_v4());
        let kept = Occlusions::default();
        let bottom = [5.0, 5.0, 0.0];
        let top = [5.0, 5.0, 10.0];
        for (mesh, revision, view, at) in [
            (&mesh, 1, view, bottom),
            (&mesh, 1, view, bottom),
            (&mesh, 1, view, top),
            (&mesh, 1, below, top),
            (&mesh, 1, below, bottom),
            (&mesh, 1, view, bottom),
            (&open, 1, view, bottom),
            (&open, 2, view, bottom),
            (&mesh, 2, view, bottom),
        ] {
            assert_eq!(
                kept.hidden(note, mesh, revision, view.to_cols_array_2d(), at),
                hides(mesh, view, Vec3::from_array(at)),
                "{at:?} at revision {revision}"
            );
        }
        assert!(kept.hidden(note, &mesh, 2, view.to_cols_array_2d(), bottom));
        assert!(!kept.hidden(note, &open, 2, view.to_cols_array_2d(), bottom));
    }
}
