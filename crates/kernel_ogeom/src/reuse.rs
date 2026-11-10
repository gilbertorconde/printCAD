//! What one build of a body may take from the one before: the faces and
//! edges it left as they were keep their meshes and outlines.
//!
//! A face is known by its geometry, not by the node it is: an op rebuilds
//! every face of the solid it makes, the ones it leaves as they were too,
//! so the nodes of one build's solid are never the last's. What a face's
//! mesh depends on is its surface, the curves round it and the ranges of
//! them it covers, where it sits, the deflection and the chords its edges
//! are drawn to; two faces alike in all of that mesh alike, and two edges
//! alike in their curves, ends and chord draw alike.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::convert::array;
use ogeom::math::Point;
use ogeom::mesh::{Deflection, EdgeChords};
use ogeom::topo::{
    CurveId, EdgeRepr, Location, Model, NodeData, Orientation, PCurveId, Shape, ShapeType,
    SurfaceId, Triangulation, explore_unique,
};

/// A face as a meshing drew it: its triangles, and the smallest box
/// holding it.
pub(crate) struct FaceDrawn {
    pub triangles: Triangulation,
    pub bounds: Option<(Point, Point)>,
}

/// The faces and the outlines of the edges a meshing drew, by what makes
/// each what it is (see [`Keys`]).
#[derive(Default)]
pub(crate) struct FaceMeshes {
    faces: HashMap<u64, Arc<FaceDrawn>>,
    edges: HashMap<u64, Arc<Vec<Point>>>,
    /// The smallest box holding the shape meshed last, from its faces'.
    bounds: Option<(Point, Point)>,
}

impl FaceMeshes {
    pub fn face(&self, key: u64) -> Option<Arc<FaceDrawn>> {
        self.faces.get(&key).cloned()
    }

    /// The smallest box holding the shape meshed last, the union of its
    /// faces' boxes; `None` when a face's could not be had.
    pub fn take_bounds(&mut self) -> Option<(Point, Point)> {
        self.bounds.take()
    }

    pub fn set_bounds(&mut self, bounds: Option<(Point, Point)>) {
        self.bounds = bounds;
    }

    pub fn edge(&self, key: u64) -> Option<Arc<Vec<Point>>> {
        self.edges.get(&key).cloned()
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    /// The meshes of the faces drawn now, in place of the last ones.
    pub fn replace_faces(&mut self, faces: HashMap<u64, Arc<FaceDrawn>>) {
        self.faces = faces;
    }

    /// The outlines of the edges drawn now, in place of the last ones.
    pub fn replace_edges(&mut self, edges: HashMap<u64, Arc<Vec<Point>>>) {
        self.edges = edges;
    }
}

/// Works out the keys of one shape's faces and edges, each surface, curve
/// and placement read once however many faces and edges share it.
pub(crate) struct Keys<'a> {
    model: &'a Model,
    surfaces: HashMap<SurfaceId, u64>,
    curves: HashMap<CurveId, u64>,
    pcurves: HashMap<PCurveId, u64>,
}

/// One thing's description, hashed.
fn hash_of(value: impl std::fmt::Debug) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{value:?}").hash(&mut hasher);
    hasher.finish()
}

impl<'a> Keys<'a> {
    pub fn new(model: &'a Model) -> Self {
        Self {
            model,
            surfaces: HashMap::new(),
            curves: HashMap::new(),
            pcurves: HashMap::new(),
        }
    }

    fn surface(&mut self, id: SurfaceId) -> Option<u64> {
        if let Some(h) = self.surfaces.get(&id) {
            return Some(*h);
        }
        let h = hash_of(self.model.geometry().surface(id)?);
        self.surfaces.insert(id, h);
        Some(h)
    }

    fn curve(&mut self, id: CurveId) -> Option<u64> {
        if let Some(h) = self.curves.get(&id) {
            return Some(*h);
        }
        let h = hash_of(self.model.geometry().curve(id)?);
        self.curves.insert(id, h);
        Some(h)
    }

    fn pcurve(&mut self, id: PCurveId) -> Option<u64> {
        if let Some(h) = self.pcurves.get(&id) {
            return Some(*h);
        }
        let h = hash_of(self.model.geometry().pcurve(id)?);
        self.pcurves.insert(id, h);
        Some(h)
    }

    /// A placement, as the numbers of its transform.
    fn placement(&self, location: &Location) -> Option<u64> {
        if location.chain().is_empty() {
            return Some(0);
        }
        Some(hash_of(location.composed(self.model.datums()).ok()?))
    }

    /// What an edge's drawing depends on but the chord: its curves, the
    /// ranges of them it covers, its ends and where it sits.
    fn edge_shape(&mut self, edge: &Shape) -> Option<u64> {
        let model = self.model;
        let NodeData::Edge(data) = model.node(edge)?.data() else {
            return None;
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        matches!(edge.orientation(), Orientation::Forward).hash(&mut hasher);
        self.placement(edge.location())?.hash(&mut hasher);
        data.degenerate.hash(&mut hasher);
        hash_of(data.tolerance).hash(&mut hasher);
        for repr in &data.representations {
            match repr {
                EdgeRepr::Curve3d {
                    curve,
                    location,
                    range,
                } => {
                    (1u8, self.curve(*curve)?, self.placement(location)?).hash(&mut hasher);
                    (range.0.to_bits(), range.1.to_bits()).hash(&mut hasher);
                }
                EdgeRepr::PCurve {
                    curve,
                    surface,
                    location,
                    range,
                } => {
                    (2u8, self.pcurve(*curve)?, self.surface(*surface)?).hash(&mut hasher);
                    self.placement(location)?.hash(&mut hasher);
                    (range.0.to_bits(), range.1.to_bits()).hash(&mut hasher);
                }
                EdgeRepr::Seam {
                    forward,
                    reversed,
                    surface,
                    location,
                    range,
                } => {
                    (3u8, self.pcurve(*forward)?, self.pcurve(*reversed)?).hash(&mut hasher);
                    (self.surface(*surface)?, self.placement(location)?).hash(&mut hasher);
                    (range.0.to_bits(), range.1.to_bits()).hash(&mut hasher);
                }
                // Cached drawings say nothing an edge's drawing is made from.
                _ => {}
            }
        }
        for vertex in explore_unique(model, edge, ShapeType::Vertex).ok()? {
            let NodeData::Vertex(v) = model.node(&vertex)?.data() else {
                return None;
            };
            for c in array(v.point) {
                c.to_bits().hash(&mut hasher);
            }
            hash_of(v.tolerance).hash(&mut hasher);
            self.placement(vertex.location())?.hash(&mut hasher);
        }
        Some(hasher.finish())
    }

    /// What an edge's outline depends on: the edge and the deflection it is
    /// drawn to.
    pub fn edge(&mut self, edge: &Shape, deflection: Deflection) -> Option<u64> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.edge_shape(edge)?.hash(&mut hasher);
        (deflection.chord.to_bits(), deflection.angular.to_bits()).hash(&mut hasher);
        Some(hasher.finish())
    }

    /// What a face's mesh depends on: its surface and where it sits, its
    /// orientation, every edge round it with the chord it is drawn to
    /// (which the faces across it have a say in), and the deflection.
    pub fn face(
        &mut self,
        face: &Shape,
        deflection: Deflection,
        chords: &EdgeChords,
    ) -> Option<u64> {
        let model = self.model;
        let NodeData::Face(data) = model.node(face)?.data() else {
            return None;
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.surface(data.surface)?.hash(&mut hasher);
        self.placement(&data.location)?.hash(&mut hasher);
        self.placement(face.location())?.hash(&mut hasher);
        matches!(face.orientation(), Orientation::Forward).hash(&mut hasher);
        data.natural_restriction.hash(&mut hasher);
        hash_of(data.tolerance).hash(&mut hasher);
        (deflection.chord.to_bits(), deflection.angular.to_bits()).hash(&mut hasher);
        for edge in explore_unique(model, face, ShapeType::Edge).ok()? {
            self.edge_shape(&edge)?.hash(&mut hasher);
            let chord = chords
                .get(&edge.node().index())
                .copied()
                .unwrap_or(f64::NAN);
            chord.to_bits().hash(&mut hasher);
        }
        Some(hasher.finish())
    }
}
