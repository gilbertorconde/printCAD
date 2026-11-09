//! Pins: numbered notes put on a face, an edge or a point of a body, for
//! the AI agent working on the document and for people. A pin is kept in
//! its body's own frame, so it moves with the body, and names what it was
//! put on: a face by its name (by its index on a mesh, which names none),
//! an edge by the names of the two faces it runs between. After a rebuild
//! the point is taken to the nearest point of the face or edge found
//! again; a pin whose face or edge is gone is lost, shown where it was and
//! said to be lost rather than taken to whatever lies nearest. Numbers are
//! given in order across the document; a pin taken away leaves its number
//! unused, and only undoing the pin's adding gives it again.

use glam::Vec3;
use kernel_api::naming::TopoName;
use serde::{Deserialize, Serialize};

use crate::notes::closest_on_mesh;
use crate::{BodyId, Document, FaceKey, TriMesh, op};

/// Who put a pin down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinAuthor {
    #[default]
    User,
    Agent,
}

impl PinAuthor {
    pub fn word(self) -> &'static str {
        match self {
            PinAuthor::User => "user",
            PinAuthor::Agent => "agent",
        }
    }
}

/// What a pin is on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinSpot {
    /// A point that follows nothing but the body.
    Point,
    /// A face, found by its name, by its index where the mesh names none.
    Face(FaceKey),
    /// An edge, found by the names of the faces it runs between, by its
    /// index where the mesh names none.
    Edge { faces: [TopoName; 2], index: u32 },
}

/// What a new pin is put on, near the point given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinWant {
    Point,
    Face,
    /// The edge of that index, or the nearest edge.
    Edge(Option<u32>),
}

/// A numbered note on a body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    pub number: u32,
    /// Where it was put, in the body's own frame.
    pub at: [f32; 3],
    pub on: PinSpot,
    pub note: String,
    #[serde(default)]
    pub author: PinAuthor,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    /// Kept off the view, while the body shows.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Milliseconds since the epoch.
    #[serde(default)]
    pub created_at: i64,
}

/// Where a pin stands on a mesh as built now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinPlaced {
    pub at: [f32; 3],
    /// Its face or edge is not on the mesh any more.
    pub lost: bool,
    /// The face it stands on, by index, where it stands on one.
    pub face: Option<u32>,
    /// The edge it stands on, by index.
    pub edge: Option<u32>,
}

impl Pin {
    /// Where the pin stands on `mesh` (its body's own, as built now): on
    /// its face or edge found again, at the point of it nearest where the
    /// pin was put. As stored, and not lost, without a mesh to look on.
    pub fn placed(&self, mesh: Option<&TriMesh>) -> PinPlaced {
        let stored = PinPlaced {
            at: self.at,
            lost: false,
            face: None,
            edge: None,
        };
        let Some(mesh) = mesh.filter(|m| !m.indices.is_empty()) else {
            return stored;
        };
        let at = Vec3::from(self.at);
        match self.on {
            PinSpot::Point => PinPlaced {
                face: closest_on_mesh(mesh, at, None).and_then(|(_, t)| mesh.faces.get(t).copied()),
                ..stored
            },
            PinSpot::Face(key) => {
                let count = mesh.faces.iter().max().map_or(0, |m| m + 1);
                let found = (0..count)
                    .find(|i| key.is_face(mesh, *i))
                    .and_then(|face| Some((closest_on_mesh(mesh, at, Some(face))?.0, face)));
                match found {
                    Some((point, face)) => PinPlaced {
                        at: point.to_array(),
                        face: Some(face),
                        ..stored
                    },
                    None => PinPlaced {
                        lost: true,
                        ..stored
                    },
                }
            }
            PinSpot::Edge { faces, index } => {
                let named = faces.iter().all(|n| *n != 0) && !mesh.edge_faces.is_empty();
                let candidates: Vec<u32> = if named {
                    let mut wanted = faces;
                    wanted.sort_unstable();
                    let mut ids: Vec<u32> = (0..mesh.edge_faces.len() as u32)
                        .filter(|e| {
                            let mut have = mesh.edge_faces[*e as usize];
                            have.sort_unstable();
                            have == wanted
                        })
                        .collect();
                    ids.dedup();
                    ids
                } else if mesh.edge_ids.contains(&index) {
                    vec![index]
                } else {
                    Vec::new()
                };
                let best = candidates
                    .into_iter()
                    .filter_map(|edge| {
                        let point = closest_on_edge(mesh, edge, at)?;
                        Some((point.distance_squared(at), point, edge))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                match best {
                    Some((_, point, edge)) => PinPlaced {
                        at: point.to_array(),
                        edge: Some(edge),
                        ..stored
                    },
                    None => PinPlaced {
                        lost: true,
                        ..stored
                    },
                }
            }
        }
    }
}

/// Where a pin put near `p` (in `mesh`'s frame) lands, and what it is on:
/// the nearest point of the mesh, on the face there or the edge wanted.
/// A mesh without faces gives a point; one without edges, a face.
pub fn spot_at(mesh: Option<&TriMesh>, p: [f32; 3], want: PinWant) -> ([f32; 3], PinSpot) {
    let p = Vec3::from(p);
    let Some(mesh) = mesh.filter(|m| !m.indices.is_empty()) else {
        return (p.to_array(), PinSpot::Point);
    };
    if let PinWant::Edge(index) = want {
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
            return (point.to_array(), PinSpot::Edge { faces, index });
        }
    }
    let Some((point, triangle)) = closest_on_mesh(mesh, p, None) else {
        return (p.to_array(), PinSpot::Point);
    };
    let spot = match (want, mesh.faces.get(triangle)) {
        (PinWant::Point, _) | (_, None) => PinSpot::Point,
        (_, Some(face)) => PinSpot::Face(FaceKey::of(mesh, *face)),
    };
    (point.to_array(), spot)
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

impl Document {
    /// The pins on `body`, in the order they were put.
    pub fn body_pins(&self, body: BodyId) -> &[Pin] {
        self.bodies
            .iter()
            .find(|b| b.id == body)
            .map_or(&[], |b| b.pins.as_slice())
    }

    /// Every pin, with its body, in body order.
    pub fn pins(&self) -> impl Iterator<Item = (BodyId, &Pin)> {
        self.bodies
            .iter()
            .flat_map(|b| b.pins.iter().map(move |p| (b.id, p)))
    }

    /// The pin numbered `number`, and the body it is on.
    pub fn pin(&self, number: u32) -> Option<(BodyId, &Pin)> {
        self.pins().find(|(_, p)| p.number == number)
    }

    /// A pin saying `note` on `body`, not yet added (it takes its number
    /// when it is): near `at` (where the body sits in the document) on
    /// what `want` asks for, or at the top of the body's middle.
    pub fn new_pin(
        &self,
        body: BodyId,
        note: String,
        author: crate::PinAuthor,
        at: Option<[f32; 3]>,
        want: PinWant,
    ) -> Pin {
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
        let (at, on) = spot_at(mesh, wanted, want);
        Pin {
            number: 0,
            at,
            on,
            note,
            author,
            done: false,
            hidden: false,
            created_at: 0,
        }
    }

    /// Where a pin stands as `body` is built now, in the document.
    pub fn pin_placed(&self, body: BodyId, pin: &Pin) -> PinPlaced {
        let local = self.local_geometry(body);
        let placed = pin.placed(local.as_ref().map(|(mesh, _)| mesh.as_ref()));
        PinPlaced {
            at: self.body_placement(body).point(placed.at),
            ..placed
        }
    }

    /// Whether a pin shows: its body shown and the pin not hidden.
    pub fn pin_effective_visible(&self, number: u32) -> bool {
        self.pin(number)
            .is_some_and(|(body, pin)| !pin.hidden && self.imported_body_effective_visible(body))
    }

    /// Set every pin on `body` at once; one op when they change.
    pub fn set_body_pins(&mut self, body: BodyId, pins: Vec<Pin>) {
        if self.bodies.iter().any(|b| b.id == body && b.pins != pins) {
            let highest = pins.iter().map(|p| p.number).max().unwrap_or(0);
            self.record_and_apply(op::DocumentOp::SetBodyPins {
                id: body,
                pins,
                numbered: self.pins_numbered.max(highest),
            });
        }
    }

    /// The number the next pin put down takes.
    pub fn next_pin_number(&self) -> u32 {
        self.pins_numbered + 1
    }

    /// Put `pin` on `body` under the document's next number, stamped now;
    /// that number, or `None` when there is no such body.
    pub fn add_pin(&mut self, body: BodyId, mut pin: Pin) -> Option<u32> {
        let mut pins = self.bodies.iter().find(|b| b.id == body)?.pins.clone();
        pin.number = self.next_pin_number();
        pin.created_at = crate::epoch_ms_now();
        let number = pin.number;
        pins.push(pin);
        self.set_body_pins(body, pins);
        Some(number)
    }

    /// Change a pin to `pin`, found by its number; false when there is none.
    pub fn set_pin(&mut self, pin: Pin) -> bool {
        let Some((body, _)) = self.pin(pin.number) else {
            return false;
        };
        let pins = self
            .body_pins(body)
            .iter()
            .map(|p| {
                if p.number == pin.number {
                    pin.clone()
                } else {
                    p.clone()
                }
            })
            .collect();
        self.set_body_pins(body, pins);
        true
    }

    /// Take a pin away; false when there is none. Its number is not given
    /// to the next pin.
    pub fn remove_pin(&mut self, number: u32) -> bool {
        let Some((body, _)) = self.pin(number) else {
            return false;
        };
        let pins = self
            .body_pins(body)
            .iter()
            .filter(|p| p.number != number)
            .cloned()
            .collect();
        self.set_body_pins(body, pins);
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

    fn pin(at: [f32; 3], on: PinSpot) -> Pin {
        Pin {
            number: 1,
            at,
            on,
            note: String::new(),
            author: PinAuthor::User,
            done: false,
            hidden: false,
            created_at: 0,
        }
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        Vec3::from(a).distance(Vec3::from(b)) < 1e-5
    }

    #[test]
    fn a_pin_follows_its_face_or_edge_by_name_and_is_lost_without_it() {
        let mesh = corner(1.0);
        let (at, on) = spot_at(Some(&mesh), [0.9, 0.5, 0.8], PinWant::Face);
        assert_eq!(on, PinSpot::Face(FaceKey { name: 20, index: 1 }));
        assert!(near(at, [1.0, 0.5, 0.8]));
        let (at, on) = spot_at(Some(&mesh), [0.9, 0.5, 0.1], PinWant::Edge(None));
        assert_eq!(
            on,
            PinSpot::Edge {
                faces: [10, 20],
                index: 0
            }
        );
        assert!(near(at, [1.0, 0.5, 0.0]));

        // The wall moves out: both pins go with it.
        let moved = corner(2.0);
        let face = pin(
            [1.0, 0.5, 0.8],
            PinSpot::Face(FaceKey { name: 20, index: 1 }),
        );
        let placed = face.placed(Some(&moved));
        assert!(near(placed.at, [2.0, 0.5, 0.8]) && !placed.lost && placed.face == Some(1));
        let edge = pin(
            [1.0, 0.5, 0.0],
            PinSpot::Edge {
                faces: [20, 10],
                index: 7,
            },
        );
        let placed = edge.placed(Some(&moved));
        assert!(near(placed.at, [2.0, 0.5, 0.0]) && !placed.lost && placed.edge == Some(0));

        // A rebuild that names the wall otherwise loses both, where they were.
        let mut renamed = moved.clone();
        renamed.face_names = vec![10, 21];
        renamed.edge_faces = vec![[10, 21], [10, 30]];
        for pin in [&face, &edge] {
            let placed = pin.placed(Some(&renamed));
            assert!(placed.lost);
            assert_eq!(placed.at, pin.at);
        }
        // Without a mesh to look on, nothing is lost.
        assert!(!face.placed(None).lost);
    }
}
