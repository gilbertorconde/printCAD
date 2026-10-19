//! Meshes a package keeps and draws by handle (`host::mesh_create`): a
//! large mesh crosses from the package once and is then updated in part,
//! rather than sent whole in every frame.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use kernel_api::TriMesh;

/// The most positions a package may keep across its meshes: 16 million,
/// about 200 MB of positions and colours.
const POSITION_LIMIT: usize = 16 << 20;

/// One package's kept meshes, shared by all its instances.
#[derive(Default)]
pub(crate) struct KeptMeshes {
    store: Mutex<Store>,
}

#[derive(Default)]
struct Store {
    next: u64,
    meshes: HashMap<u64, Kept>,
}

struct Kept {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    colors: Vec<[f32; 3]>,
    /// Counts the changes made to it.
    revision: u64,
    /// The mesh as drawn, with its normals, and the revision it was made
    /// at.
    built: Option<(u64, Arc<TriMesh>)>,
}

fn triples(flat: &[f32], what: &str) -> Result<Vec<[f32; 3]>, String> {
    let (chunks, rest) = flat.as_chunks::<3>();
    if !rest.is_empty() {
        return Err(format!("{what} come in threes"));
    }
    Ok(chunks.to_vec())
}

impl KeptMeshes {
    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Keep a mesh; its handle.
    pub(crate) fn create(
        &self,
        positions: &[f32],
        indices: Vec<u32>,
        colors: &[f32],
    ) -> Result<u64, String> {
        let positions = triples(positions, "positions")?;
        let colors = triples(colors, "colours")?;
        if !colors.is_empty() && colors.len() != positions.len() {
            return Err("a colour is given for each position, or none".into());
        }
        if !indices.len().is_multiple_of(3) || indices.iter().any(|i| *i as usize >= positions.len()) {
            return Err("the indices name triangles of the positions given".into());
        }
        let mut store = self.store();
        let held: usize = store.meshes.values().map(|m| m.positions.len()).sum();
        if held + positions.len() > POSITION_LIMIT {
            return Err(format!(
                "a package keeps at most {POSITION_LIMIT} positions in all"
            ));
        }
        store.next += 1;
        let handle = store.next;
        store.meshes.insert(
            handle,
            Kept {
                positions,
                indices,
                colors,
                revision: 0,
                built: None,
            },
        );
        Ok(handle)
    }

    /// Replace positions from `first` on, and their colours when given.
    pub(crate) fn update(
        &self,
        handle: u64,
        first: u32,
        positions: &[f32],
        colors: &[f32],
    ) -> Result<(), String> {
        let positions = triples(positions, "positions")?;
        let colors = triples(colors, "colours")?;
        let mut store = self.store();
        let mesh = store
            .meshes
            .get_mut(&handle)
            .ok_or_else(|| format!("no kept mesh {handle}"))?;
        let first = first as usize;
        if first + positions.len() > mesh.positions.len() {
            return Err(format!(
                "positions {first} to {} pass the mesh's {}",
                first + positions.len(),
                mesh.positions.len()
            ));
        }
        if !colors.is_empty() {
            if colors.len() != positions.len() {
                return Err("a colour is given for each position updated, or none".into());
            }
            if mesh.colors.is_empty() {
                mesh.colors = vec![[1.0; 3]; mesh.positions.len()];
            }
            mesh.colors[first..first + colors.len()].copy_from_slice(&colors);
        }
        mesh.positions[first..first + positions.len()].copy_from_slice(&positions);
        mesh.revision += 1;
        Ok(())
    }

    pub(crate) fn remove(&self, handle: u64) {
        self.store().meshes.remove(&handle);
    }

    /// Forget every mesh: the instance that kept them is gone.
    pub(crate) fn clear(&self) {
        self.store().meshes.clear();
    }

    /// A kept mesh as drawn, and a revision naming this state of it.
    pub(crate) fn mesh(&self, handle: u64) -> Option<(u64, Arc<TriMesh>)> {
        let mut store = self.store();
        let mesh = store.meshes.get_mut(&handle)?;
        let revision = mesh.revision;
        if mesh.built.as_ref().is_none_or(|(r, _)| *r != revision) {
            let mut built = crate::convert::tri_mesh(mesh.positions.clone(), mesh.indices.clone());
            built.colors = mesh.colors.clone();
            mesh.built = Some((revision, Arc::new(built)));
        }
        let (_, built) = mesh.built.as_ref()?;
        // The handle and revision together name this state of the mesh.
        let key = (handle << 32) ^ revision;
        Some((key, built.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kept mesh updated in part draws the change, under a new revision;
    /// what does not fit is refused and leaves it as it was.
    #[test]
    fn a_kept_mesh_updates_in_part() {
        let kept = KeptMeshes::default();
        let flat = |points: &[[f32; 3]]| points.concat();
        let handle = kept
            .create(
                &flat(&[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
                vec![0, 1, 2],
                &[],
            )
            .unwrap();
        let (before, _) = kept.mesh(handle).unwrap();
        assert_eq!(kept.mesh(handle).unwrap().0, before, "unchanged, the same");
        kept.update(handle, 2, &[0.0, 2.0, 0.0], &[1.0, 0.0, 0.0])
            .unwrap();
        let (after, mesh) = kept.mesh(handle).unwrap();
        assert_ne!(after, before);
        assert_eq!(mesh.positions[2], [0.0, 2.0, 0.0]);
        assert_eq!(mesh.colors, [[1.0; 3], [1.0; 3], [1.0, 0.0, 0.0]]);
        assert!(
            kept.update(handle, 2, &flat(&[[0.0; 3], [0.0; 3]]), &[])
                .is_err()
        );
        assert!(
            kept.create(&[0.0; 3], vec![0, 1, 2], &[]).is_err(),
            "no such position"
        );
        assert_eq!(kept.mesh(handle).unwrap().0, after);
        kept.remove(handle);
        assert!(kept.mesh(handle).is_none());
    }
}
