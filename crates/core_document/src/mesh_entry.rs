//! A mesh kept as an entry of its own in a `.prtcad` (`mesh/<uuid>.bin`):
//! MessagePack, written with its field names.
//!
//! Older files hold it as a bare array, a field per place, with the face
//! and edge tables at the end left out when empty. Those places therefore
//! move: with no face surfaces, the face names stand where the surfaces
//! would, and a table added between two others (the face makers, before
//! the edge faces) shifts the one after it. Such an array is read by the
//! fixed fields' places and the tables' shapes instead.

use kernel_api::{FaceSurface, TopoName, TriMesh};
use serde::de::{self, Deserialize, Deserializer, SeqAccess, Visitor};

/// The bytes of a mesh entry.
pub(crate) fn write(mesh: &TriMesh) -> Result<Vec<u8>, rmp_serde::encode::Error> {
    rmp_serde::to_vec_named(mesh)
}

/// A mesh entry, in either layout.
pub(crate) fn read(bytes: &[u8]) -> Result<TriMesh, rmp_serde::decode::Error> {
    let positional = bytes
        .first()
        .is_some_and(|&marker| matches!(marker, 0x90..=0x9f | 0xdc | 0xdd));
    if positional {
        rmp_serde::from_slice::<Positional>(bytes).map(|read| read.0)
    } else {
        rmp_serde::from_slice(bytes)
    }
}

/// A mesh as a bare array: positions, normals, indices, edges, colours,
/// faces and edge ids in their places, then whichever tables were not
/// empty, in their order.
struct Positional(TriMesh);

/// One of the tables after the fixed fields, told apart by its shape. Of
/// two lists of names, the first is the faces' names and the second their
/// makers: a mesh with makers has names.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Table {
    EdgeFaces(Vec<[TopoName; 2]>),
    Names(Vec<TopoName>),
    Surfaces(Vec<FaceSurface>),
}

impl<'de> Deserialize<'de> for Positional {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(PositionalVisitor)
    }
}

struct PositionalVisitor;

impl<'de> Visitor<'de> for PositionalVisitor {
    type Value = Positional;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a mesh as an array of its fields")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Positional, A::Error> {
        let mut mesh = TriMesh {
            positions: required(&mut seq, 0)?,
            normals: required(&mut seq, 1)?,
            indices: required(&mut seq, 2)?,
            ..TriMesh::default()
        };
        mesh.edges = seq.next_element()?.unwrap_or_default();
        mesh.colors = seq.next_element()?.unwrap_or_default();
        mesh.faces = seq.next_element()?.unwrap_or_default();
        mesh.edge_ids = seq.next_element()?.unwrap_or_default();
        let mut names_seen = false;
        while let Some(table) = seq.next_element::<Table>()? {
            match table {
                Table::Surfaces(surfaces) => mesh.face_surfaces = surfaces,
                Table::Names(names) if !names_seen => {
                    names_seen = true;
                    mesh.face_names = names;
                }
                Table::Names(makers) => mesh.face_makers = makers,
                Table::EdgeFaces(pairs) => mesh.edge_faces = pairs,
            }
        }
        Ok(Positional(mesh))
    }
}

fn required<'de, A: SeqAccess<'de>, T: Deserialize<'de>>(
    seq: &mut A,
    at: usize,
) -> Result<T, A::Error> {
    seq.next_element()?
        .ok_or_else(|| de::Error::invalid_length(at, &"positions, normals and indices"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh() -> TriMesh {
        TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            edges: vec![0, 1],
            faces: vec![0],
            edge_ids: vec![0],
            ..TriMesh::default()
        }
    }

    /// An array written with some tables left out still puts each table
    /// read where it belongs, and the named form reads as it was written.
    #[test]
    fn a_mesh_reads_whichever_tables_its_array_left_out() {
        let mut full = mesh();
        full.face_surfaces = vec![FaceSurface::Plane {
            origin: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
        }];
        full.face_names = vec![7];
        full.face_makers = vec![9];
        full.edge_faces = vec![[7, 8]];
        let mut no_surfaces = full.clone();
        no_surfaces.face_surfaces.clear();
        let mut no_makers = full.clone();
        no_makers.face_makers.clear();
        let mut only_pairs = mesh();
        only_pairs.edge_faces = vec![[3, 4]];

        let key = |m: &TriMesh| {
            format!(
                "{:?}",
                (
                    &m.positions,
                    &m.indices,
                    &m.edges,
                    &m.face_surfaces,
                    &m.face_names,
                    &m.face_makers,
                    &m.edge_faces,
                )
            )
        };
        for case in [full, no_surfaces, no_makers, only_pairs, mesh()] {
            let array = rmp_serde::to_vec(&case).expect("array");
            assert_eq!(key(&read(&array).expect("array reads")), key(&case));
            let named = write(&case).expect("named");
            assert_eq!(key(&read(&named).expect("named reads")), key(&case));
        }
    }
}
