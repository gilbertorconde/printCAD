//! The parts list: every body, identical ones counted together, with the
//! size of its box in its own frame (what a print bed has to hold).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use core_document::{BodyId, Document};

/// One part: the bodies that are the same shape, and its size.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub name: String,
    pub bodies: Vec<BodyId>,
    /// Its box along its own X, Y and Z, in millimetres; `None` for a body
    /// with no geometry yet.
    pub size_mm: Option<[f32; 3]>,
    /// A mesh body rather than a solid.
    pub mesh: bool,
}

/// Every body, bodies of the same shape as one part, in name order.
pub fn parts_list(document: &Document) -> Vec<Part> {
    let mut parts: Vec<(u64, Part)> = Vec::new();
    for body in document.bodies() {
        let local = document.local_geometry(body.id);
        let size_mm = local
            .as_ref()
            .and_then(|(mesh, bounds)| bounds.or_else(|| mesh.bounds()))
            .map(|(lo, hi)| [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]);
        // The same shape is the same snapshot, or failing one the same
        // triangles; a body with neither is a part of its own.
        let mut hasher = DefaultHasher::new();
        match (document.imported_brep_blob(body.id), &local) {
            (Some(blob), _) => blob.hash(&mut hasher),
            (None, Some((mesh, _))) if !mesh.positions.is_empty() => {
                for p in &mesh.positions {
                    p.map(f32::to_bits).hash(&mut hasher);
                }
            }
            _ => body.id.hash(&mut hasher),
        }
        let key = hasher.finish();
        match parts.iter_mut().find(|(k, _)| *k == key) {
            Some((_, part)) => part.bodies.push(body.id),
            None => parts.push((
                key,
                Part {
                    name: body.name.clone(),
                    bodies: vec![body.id],
                    size_mm,
                    mesh: document.is_mesh_body(body.id),
                },
            )),
        }
    }
    let mut out: Vec<Part> = parts.into_iter().map(|(_, p)| p).collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// The list as comma-separated values, a header line first.
pub fn parts_csv(parts: &[Part]) -> String {
    let mut out = String::from("Part,Quantity,Size X (mm),Size Y (mm),Size Z (mm),Kind\n");
    for part in parts {
        let name = if part.name.contains([',', '"']) {
            format!("\"{}\"", part.name.replace('"', "\"\""))
        } else {
            part.name.clone()
        };
        let size = part.size_mm.map_or_else(
            || ",,".to_string(),
            |s| format!("{:.2},{:.2},{:.2}", s[0], s[1], s[2]),
        );
        let kind = if part.mesh { "mesh" } else { "solid" };
        out.push_str(&format!("{name},{},{size},{kind}\n", part.bodies.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_of_one_shape_are_one_part_counted() {
        let mut document = Document::new("t");
        let a = document.create_body(Some("Bolt".into()));
        let b = document.create_body(Some("Bolt 2".into()));
        let c = document.create_body(Some("Bracket, left".into()));
        document.set_imported_brep_data(a, b"bolt".to_vec(), Vec::new());
        document.set_imported_brep_data(b, b"bolt".to_vec(), Vec::new());
        document.set_imported_brep_data(c, b"bracket".to_vec(), Vec::new());
        let parts = parts_list(&document);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "Bolt");
        assert_eq!(parts[0].bodies, [a, b]);
        let csv = parts_csv(&parts);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[1], "Bolt,2,,,,solid");
        assert_eq!(lines[2], "\"Bracket, left\",1,,,,solid");
    }
}
