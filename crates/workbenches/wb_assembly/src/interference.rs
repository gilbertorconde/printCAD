//! Interference: where solid bodies share material. Pairs whose bounds
//! meet are handed to the kernel, which answers with the solid they share.

use core_document::{BodyId, Document};
use kernel_api::KernelQueries;

/// Two bodies that share material: how much, and where (world space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clash {
    pub a: BodyId,
    pub b: BodyId,
    pub volume_mm3: f64,
    pub centre: [f32; 3],
}

/// What a check found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Interference {
    pub clashes: Vec<Clash>,
    /// Solid bodies checked.
    pub checked: usize,
    /// Visible bodies with no solid (meshes, bodies not built yet), left out.
    pub skipped: usize,
}

/// Check the visible solid bodies, or only `among` when given, against
/// each other.
pub fn interference(
    document: &Document,
    kernel: &dyn KernelQueries,
    among: Option<&[BodyId]>,
) -> Result<Interference, String> {
    let mut found = Interference::default();
    let mut solids = Vec::new();
    for body in document.bodies() {
        if among.is_some_and(|only| !only.contains(&body.id))
            || !document.imported_body_effective_visible(body.id)
        {
            continue;
        }
        let bounds = document
            .imported_geometry(body.id)
            .and_then(|g| g.bounds_mm.or_else(|| g.mesh.bounds()));
        match (document.imported_brep_blob(body.id), bounds) {
            (Some(blob), Some(bounds)) => solids.push((body.id, blob, bounds)),
            _ => found.skipped += 1,
        }
    }
    found.checked = solids.len();
    for (i, (a, blob_a, (lo_a, hi_a))) in solids.iter().enumerate() {
        for (b, blob_b, (lo_b, hi_b)) in &solids[i + 1..] {
            if (0..3).any(|k| hi_a[k] < lo_b[k] || hi_b[k] < lo_a[k]) {
                continue;
            }
            let at_a = document.body_placement(*a);
            let b_in_a = at_a.inverse().after(&document.body_placement(*b)).rows();
            let shared = kernel
                .overlap(blob_a, blob_b, &b_in_a)
                .map_err(|e| e.to_string())?;
            if let Some(shared) = shared {
                found.clashes.push(Clash {
                    a: *a,
                    b: *b,
                    volume_mm3: shared.volume_mm3,
                    centre: at_a.point(shared.centre_mm.map(|c| c as f32)),
                });
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{BodyPlacement, ImportedGeometry, TriMesh};
    use kernel_api::{KernelError, KernelResult, Overlap, ProfilePlane, ProjectedEdge};
    use std::sync::{Arc, Mutex};

    /// Answers every pair with the same shared solid, and keeps where it
    /// was told the second shape sits.
    #[derive(Default)]
    struct Fake {
        asked: Mutex<Vec<[[f64; 4]; 4]>>,
    }

    impl KernelQueries for Fake {
        fn project_edge(
            &self,
            _: &[u8],
            _: [f64; 3],
            _: &ProfilePlane,
        ) -> KernelResult<ProjectedEdge> {
            Err(KernelError::Unsupported("projection".into()))
        }

        fn overlap(
            &self,
            _: &[u8],
            _: &[u8],
            b_in_a: &[[f64; 4]; 4],
        ) -> KernelResult<Option<Overlap>> {
            self.asked.lock().unwrap().push(*b_in_a);
            Ok(Some(Overlap {
                volume_mm3: 50.0,
                centre_mm: [7.5, 5.0, 5.0],
            }))
        }
    }

    fn cube(document: &mut Document, at: [f32; 3], solid: bool) -> BodyId {
        let body = document.create_body(None);
        let corners = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 10.0]];
        document.set_imported_geometry(
            body,
            ImportedGeometry {
                mesh: Arc::new(TriMesh {
                    positions: corners.to_vec(),
                    normals: vec![[0.0, 0.0, 1.0]; 3],
                    indices: vec![0, 1, 2],
                    ..TriMesh::default()
                }),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        if solid {
            document.set_imported_brep_data(body, b"shape".to_vec(), Vec::new());
        }
        document.set_body_placement(
            body,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::from_array(at)),
        );
        body
    }

    #[test]
    fn only_solids_whose_bounds_meet_are_asked_and_clashes_sit_in_the_world() {
        let mut document = Document::new("t");
        let a = cube(&mut document, [100.0, 0.0, 0.0], true);
        let b = cube(&mut document, [105.0, 0.0, 0.0], true);
        cube(&mut document, [0.0, 0.0, 0.0], true);
        cube(&mut document, [102.0, 0.0, 0.0], false);
        let kernel = Fake::default();
        let found = interference(&document, &kernel, None).unwrap();
        assert_eq!((found.checked, found.skipped), (3, 1));
        let asked = kernel.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1, "the far body is never asked about");
        assert!((asked[0][0][3] - 5.0).abs() < 1e-6, "{asked:?}");
        assert_eq!(found.clashes.len(), 1);
        let clash = found.clashes[0];
        assert_eq!((clash.a, clash.b), (a, b));
        assert_eq!(clash.centre, [107.5, 5.0, 5.0]);
        let only_one = interference(&document, &kernel, Some(&[a])).unwrap();
        assert!(only_one.clashes.is_empty());
        document.set_body_visible(b, false);
        assert!(
            interference(&document, &kernel, None)
                .unwrap()
                .clashes
                .is_empty()
        );
    }
}
