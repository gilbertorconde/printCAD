//! Where a mesh crosses itself: the kernel's pairs of triangles that meet
//! away from the corners they share, on the mesh a body draws now,
//! counted by `mesh.intersections` and shown on the selected body as a
//! map, the crossing triangles' corners coloured over grey.

use super::*;

/// The triangles in any of `pairs`, each once, in order.
pub(crate) fn triangles(pairs: &[[u32; 2]]) -> Vec<u32> {
    let mut out: Vec<u32> = pairs.iter().flatten().copied().collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// One value per vertex of `mesh`: one at a corner of a crossing
/// triangle, none elsewhere.
pub(crate) fn values(mesh: &TriMesh, pairs: &[[u32; 2]]) -> Vec<f32> {
    let mut out = vec![f32::NAN; mesh.positions.len()];
    let corners = mesh.indices.as_chunks::<3>().0;
    for t in triangles(pairs) {
        for v in corners.get(t as usize).into_iter().flatten() {
            if let Some(slot) = out.get_mut(*v as usize) {
                *slot = 1.0;
            }
        }
    }
    out
}

/// What `mesh.intersections` answers.
pub(crate) fn reading(pairs: &[[u32; 2]]) -> Value {
    json!({"pairs": pairs.len(), "triangles": triangles(pairs)})
}

/// The crossings of `mesh`, by the kernel.
pub(crate) fn find(
    kernel: &dyn kernel_api::KernelQueries,
    mesh: &TriMesh,
) -> Result<Vec<[u32; 2]>, String> {
    kernel.self_intersections(mesh).map_err(|e| e.to_string())
}

/// What a body's crossings are found from: its mesh as drawn now.
pub(crate) fn key(document: &Document, body: BodyId) -> Option<u64> {
    Some(document.derived().imported_geometry(body)?.revision)
}

impl MeshWorkbench {
    /// The selected body's crossings as a map, once found.
    pub(crate) fn crossing_maps(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Vec<core_document::ScalarMap> {
        let (true, Some(kernel), Some(body)) = (
            self.tab.crossings,
            ctx.kernel,
            ctx.selected_body_id.map(BodyId),
        ) else {
            return Vec::new();
        };
        let document = &*ctx.document;
        let (Some(key), Some((mesh, _))) =
            (key(document, body), document.derived().local_geometry(body))
        else {
            return Vec::new();
        };
        let drawn = Arc::clone(&mesh);
        let Some(pairs) = self
            .tab
            .crossing_pairs
            .ensure(body, key, move || find(kernel, &drawn).map(Arc::from))
        else {
            return Vec::new();
        };
        let ramp = metrology::ramp::Ramp::Jet;
        let hot = ramp.at(1.0);
        vec![core_document::ScalarMap {
            body,
            revision: key,
            values: values(&mesh, &pairs).into(),
            range: [0.0, 1.0],
            bands: 0,
            ramp: vec![hot; 2].into(),
            under: hot,
            over: hot,
            none: ctx.sketch_palette.inactive,
            legend: core_document::MapLegend {
                title: "Self-intersections".to_string(),
                unit: String::new(),
                stats: vec![
                    ("Crossing pairs".to_string(), pairs.len().to_string()),
                    ("Triangles".to_string(), triangles(&pairs).len().to_string()),
                ],
                histogram: Vec::new(),
                tolerance: None,
            },
        }]
    }

    /// Say once what the search on the selected body found, or why it
    /// could not run; a search that could not run turns the map off.
    pub(crate) fn report_crossings(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(body) = ctx.selected_body_id.map(BodyId) else {
            return;
        };
        let Some(key) = key(ctx.document, body).filter(|_| self.tab.crossings) else {
            return;
        };
        if self.tab.crossings_said == Some((body, key)) {
            return;
        }
        if let Some(why) = self.tab.crossing_pairs.failure(body, key) {
            ctx.log_warn(format!("Self-intersections: {why}"));
            self.tab.crossings = false;
        } else if let Some(pairs) = self.tab.crossing_pairs.get(body, key) {
            if pairs.is_empty() {
                ctx.log_info("The mesh does not cross itself");
            } else {
                ctx.log_warn(format!(
                    "{} triangles cross others, in {} pairs",
                    triangles(&pairs).len(),
                    pairs.len()
                ));
            }
        } else {
            return;
        }
        self.tab.crossings_said = Some((body, key));
    }
}
