//! Interference: where solid bodies share material. Pairs whose bounds
//! meet are handed to the kernel, which answers with the solid they share.
//! [`plan`] reads the document; [`Check::run`] asks the kernel and needs
//! nothing else, so it can run away from the window.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use core_document::{BodyId, BodyPlacement, Document};
use kernel_api::{KernelQueries, TriMesh};

/// Two bodies that share material: how much, where (world space), and
/// the shared solid to draw.
#[derive(Debug, Clone)]
pub struct Clash {
    pub a: BodyId,
    pub b: BodyId,
    pub volume_mm3: f64,
    pub centre: [f32; 3],
    pub mesh: Arc<TriMesh>,
}

/// What a check found.
#[derive(Debug, Clone, Default)]
pub struct Interference {
    pub clashes: Vec<Clash>,
    /// Solid bodies checked.
    pub checked: usize,
    /// Visible bodies with no solid (meshes, bodies not built yet), left out.
    pub skipped: usize,
    /// Stopped before every pair was asked about.
    pub stopped: bool,
}

/// A body as a check needs it: its shape and where it sits.
struct Solid {
    body: BodyId,
    blob: Arc<Vec<u8>>,
    placement: BodyPlacement,
}

/// The pairs a check asks the kernel about, read from the document.
pub struct Check {
    solids: Vec<Solid>,
    pairs: Vec<(usize, usize)>,
    skipped: usize,
}

/// The visible solid bodies, or only `among` when given, and the pairs of
/// them whose bounds meet.
pub fn plan(document: &Document, among: Option<&[BodyId]>) -> Check {
    let mut solids = Vec::new();
    let mut bounds = Vec::new();
    let mut skipped = 0;
    for body in document.bodies() {
        if among.is_some_and(|only| !only.contains(&body.id))
            || !document.imported_body_effective_visible(body.id)
        {
            continue;
        }
        let placed = document
            .imported_geometry(body.id)
            .and_then(|g| g.bounds_mm.or_else(|| g.mesh.bounds()));
        match (document.imported_brep_blob_arc(body.id), placed) {
            (Some(blob), Some(placed)) => {
                solids.push(Solid {
                    body: body.id,
                    blob,
                    placement: document.body_placement(body.id),
                });
                bounds.push(placed);
            }
            _ => skipped += 1,
        }
    }
    let mut pairs = Vec::new();
    for i in 0..bounds.len() {
        for j in i + 1..bounds.len() {
            let ((lo_a, hi_a), (lo_b, hi_b)) = (bounds[i], bounds[j]);
            if (0..3).all(|k| hi_a[k] >= lo_b[k] && hi_b[k] >= lo_a[k]) {
                pairs.push((i, j));
            }
        }
    }
    Check {
        solids,
        pairs,
        skipped,
    }
}

impl Check {
    /// How many pairs the kernel is asked about.
    pub fn pairs(&self) -> usize {
        self.pairs.len()
    }

    /// Ask the kernel about each pair, counting them off in `done`, until
    /// `stop` is set. Pairs are shared among a few threads; the clashes
    /// come back in pair order whichever thread found them.
    pub fn run(
        &self,
        kernel: &dyn KernelQueries,
        done: &AtomicUsize,
        stop: &AtomicBool,
    ) -> Result<Interference, String> {
        let next = AtomicUsize::new(0);
        let found: Mutex<Vec<(usize, Clash)>> = Mutex::default();
        let failed: Mutex<Option<String>> = Mutex::default();
        let workers = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, 8)
            .min(self.pairs.len().max(1));
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        if stop.load(Ordering::Relaxed) || failed.lock().unwrap().is_some() {
                            return;
                        }
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&(i, j)) = self.pairs.get(k) else {
                            return;
                        };
                        match self.pair(kernel, i, j) {
                            Ok(Some(clash)) => found.lock().unwrap().push((k, clash)),
                            Ok(None) => {}
                            Err(why) => {
                                *failed.lock().unwrap() = Some(why);
                                return;
                            }
                        }
                        done.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        });
        if let Some(why) = failed.into_inner().unwrap() {
            return Err(why);
        }
        let mut clashes = found.into_inner().unwrap();
        clashes.sort_by_key(|(k, _)| *k);
        Ok(Interference {
            clashes: clashes.into_iter().map(|(_, c)| c).collect(),
            checked: self.solids.len(),
            skipped: self.skipped,
            stopped: done.load(Ordering::Relaxed) < self.pairs.len(),
        })
    }

    /// What solids `i` and `j` share, if anything.
    fn pair(
        &self,
        kernel: &dyn KernelQueries,
        i: usize,
        j: usize,
    ) -> Result<Option<Clash>, String> {
        let (a, b) = (&self.solids[i], &self.solids[j]);
        let b_in_a = a.placement.inverse().after(&b.placement).rows();
        let shared = kernel
            .overlap(&a.blob, &b.blob, &b_in_a)
            .map_err(|e| e.to_string())?;
        Ok(shared.map(|shared| Clash {
            a: a.body,
            b: b.body,
            volume_mm3: shared.volume_mm3,
            centre: a.placement.point(shared.centre_mm.map(|c| c as f32)),
            mesh: Arc::new(a.placement.mesh(&shared.mesh)),
        }))
    }
}

/// Check the visible solid bodies, or only `among` when given, against
/// each other, here and now.
pub fn interference(
    document: &Document,
    kernel: &dyn KernelQueries,
    among: Option<&[BodyId]>,
) -> Result<Interference, String> {
    plan(document, among).run(kernel, &AtomicUsize::new(0), &AtomicBool::new(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{ImportedGeometry, TriMesh};
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
                mesh: TriMesh {
                    positions: vec![[7.5, 5.0, 5.0]; 3],
                    normals: vec![[0.0, 0.0, 1.0]; 3],
                    indices: vec![0, 1, 2],
                    ..TriMesh::default()
                },
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
        let clash = &found.clashes[0];
        assert_eq!((clash.a, clash.b), (a, b));
        assert_eq!(clash.centre, [107.5, 5.0, 5.0]);
        assert_eq!(
            clash.mesh.positions[0],
            [107.5, 5.0, 5.0],
            "drawn where it is"
        );
        let stopped =
            plan(&document, None).run(&kernel, &AtomicUsize::new(0), &AtomicBool::new(true));
        assert!(stopped.unwrap().stopped);
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
