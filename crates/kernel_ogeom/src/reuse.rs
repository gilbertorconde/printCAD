//! What one build of a body may take from the one before: the faces it
//! left as they were keep their meshes, boxes and edge outlines.
//!
//! A face is known by its content as the kernel writes it out
//! ([`face_content`]), never by its node: an op rebuilds every face of the
//! solid it makes, the ones it leaves as they were too, so the nodes of one
//! build's solid are never the last's. The meshes are the kernel's own
//! [`FaceMeshCache`]; the boxes and outlines are kept here under the same
//! content.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use ogeom::math::Point;
use ogeom::mesh::{Deflection, FaceMeshCache};
use ogeom::topo::{EdgeRepr, Model, NodeData, Shape, face_content};

/// How many faces the kept meshes may hold, as a multiple of the faces of
/// the shape meshed last, before they start afresh: an edit drawing new
/// faces build after build would otherwise keep every one it ever drew.
const KEPT_PER_FACE: usize = 8;

/// The meshes, boxes and outlines a meshing drew, for the next to take
/// what it draws alike.
#[derive(Default)]
pub(crate) struct FaceMeshes {
    meshes: FaceMeshCache,
    /// The faces drawn into `meshes`, by [`FaceKeys::face`].
    held: HashSet<u64>,
    /// Each face's smallest box, by [`FaceKeys::face`], from the last build.
    boxes: HashMap<u64, Option<(Point, Point)>>,
    /// Each edge's outline, by [`FaceKeys::edge`], from the last build.
    edges: HashMap<u64, Arc<Vec<Point>>>,
    /// The smallest box holding the shape meshed last, from its faces'.
    bounds: Option<(Point, Point)>,
}

impl FaceMeshes {
    /// The kernel's meshes, to draw through.
    pub fn meshes(&self) -> &FaceMeshCache {
        &self.meshes
    }

    /// How many faces' meshes are kept.
    pub fn len(&self) -> usize {
        self.held.len()
    }

    /// Note the faces a meshing just drew, starting afresh when what is
    /// kept has grown past [`KEPT_PER_FACE`] times this shape.
    pub fn drew(&mut self, keys: &FaceKeys) {
        self.held.extend(keys.faces.iter().flatten());
        if self.held.len() > KEPT_PER_FACE * keys.faces.len().max(1) {
            self.meshes = FaceMeshCache::new();
            self.held.clear();
        }
    }

    pub fn face_box(&self, key: u64) -> Option<Option<(Point, Point)>> {
        self.boxes.get(&key).copied()
    }

    pub fn replace_boxes(&mut self, boxes: HashMap<u64, Option<(Point, Point)>>) {
        self.boxes = boxes;
    }

    pub fn edge(&self, key: u64) -> Option<Arc<Vec<Point>>> {
        self.edges.get(&key).cloned()
    }

    pub fn replace_edges(&mut self, edges: HashMap<u64, Arc<Vec<Point>>>) {
        self.edges = edges;
    }

    /// The smallest box holding the shape meshed last, the union of its
    /// faces' boxes; `None` when a face's could not be had.
    pub fn take_bounds(&mut self) -> Option<(Point, Point)> {
        self.bounds.take()
    }

    pub fn set_bounds(&mut self, bounds: Option<(Point, Point)>) {
        self.bounds = bounds;
    }
}

/// The keys of one shape's faces, in face order, and where each edge is
/// first met among them.
pub(crate) struct FaceKeys {
    /// Each face's content and the deflection, hashed; `None` where the
    /// kernel cannot write the face out.
    pub faces: Vec<Option<u64>>,
    /// Each edge node's first face, wire and place in the wire.
    first_met: HashMap<u32, (usize, usize, usize)>,
}

impl FaceKeys {
    pub fn new(model: &Model, faces: &[Shape], deflection: Deflection) -> Self {
        let keys = ogeom::core::parallel::map_ordered(faces, |_, face| {
            let content = face_content(model, face)?;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            content.hash(&mut hasher);
            (deflection.chord.to_bits(), deflection.angular.to_bits()).hash(&mut hasher);
            Some(hasher.finish())
        });
        let mut first_met = HashMap::new();
        for (f, face) in faces.iter().enumerate() {
            if keys[f].is_none() {
                continue;
            }
            let Ok(wires) = model.ordered_children_of(face) else {
                continue;
            };
            for (w, wire) in wires.iter().enumerate() {
                let Ok(edges) = model.ordered_children_of(wire) else {
                    continue;
                };
                for (e, edge) in edges.iter().enumerate() {
                    first_met.entry(edge.node().index()).or_insert((f, w, e));
                }
            }
        }
        Self {
            faces: keys,
            first_met,
        }
    }

    /// What an edge's outline depends on: the edge as its first face's
    /// content writes it, where and which way it sits, and the deflection
    /// it is drawn to. `None` for an edge no face names, or one drawn from
    /// a curve on a surface, which the face's content names but does not
    /// write out where it is another face's.
    pub fn edge(&self, model: &Model, edge: &Shape, drawn_to: Deflection) -> Option<u64> {
        let NodeData::Edge(data) = model.node(edge)?.data() else {
            return None;
        };
        if !data
            .representations
            .iter()
            .any(|r| matches!(r, EdgeRepr::Curve3d { .. }))
        {
            return None;
        }
        let (f, w, e) = *self.first_met.get(&edge.node().index())?;
        let face = self.faces[f]?;
        let placement = edge.transform(model.datums()).ok()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (face, w, e).hash(&mut hasher);
        format!("{:?}{placement:?}", edge.orientation()).hash(&mut hasher);
        (drawn_to.chord.to_bits(), drawn_to.angular.to_bits()).hash(&mut hasher);
        Some(hasher.finish())
    }
}
