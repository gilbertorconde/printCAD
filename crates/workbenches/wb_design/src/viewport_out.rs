//! What the view draws while the bench is active: the scans' deviation
//! map, overlays, marks, labels and the datums' meshes.

use crate::{DesignWorkbench, face_snap, handles, instances, region_pick, scan_deviation};
use core_document::{FeatureId, Workbench, WorkbenchRuntimeContext};

impl DesignWorkbench {
    pub(crate) fn scalar_maps(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Vec<core_document::ScalarMap> {
        let (true, Some(kernel)) = (self.tab.scan_deviation, ctx.kernel) else {
            return Vec::new();
        };
        let document = &*ctx.document;
        // The body being edited, else the one selected.
        let body = self
            .editing_feature()
            .and_then(|f| document.get_feature_meta(f))
            .and_then(|n| n.body)
            .or_else(|| Self::target_body(ctx))
            .filter(|b| document.body_scan(*b).is_none());
        let Some(body) = body else {
            return Vec::new();
        };
        let range = scan_deviation::RANGE;
        let ramp = metrology::ramp::Ramp::Jet;
        let mut out = Vec::new();
        for scan in document.bodies().iter().map(|b| b.id) {
            if document.body_scan(scan).is_none() || !document.imported_body_effective_visible(scan)
            {
                continue;
            }
            let Ok(inputs) = scan_deviation::inputs(document, scan, body) else {
                continue;
            };
            let key = inputs.key;
            let measured = self.tab.deviations.ensure(scan, key, move || {
                Ok(scan_deviation::measure(kernel, &inputs)
                    .unwrap_or_else(|_| std::sync::Arc::from([])))
            });
            let Some(values) = measured else {
                continue;
            };
            if values.is_empty() {
                continue;
            }
            let r = scan_deviation::reading(&values, range);
            let share = |k: &str| format!("{:.1} %", r[k].as_f64().unwrap_or(0.0) * 100.0);
            out.push(core_document::ScalarMap {
                body: scan,
                revision: key,
                values: scan_deviation::drawn(&values, range),
                range: [-range as f32, range as f32],
                bands: 0,
                ramp: (0..32).map(|i| ramp.at(f64::from(i) / 31.0)).collect(),
                // Deeper inside the body: violet.
                under: metrology::ramp::Ramp::Plasma.at(0.15),
                over: ramp.caps().1,
                none: ctx.sketch_palette.inactive,
                legend: core_document::MapLegend {
                    title: "Deviation from scan".to_string(),
                    unit: "mm".to_string(),
                    stats: vec![
                        (format!("Within ±{range}"), share("within")),
                        ("Inside the body".to_string(), share("inside")),
                        (
                            "RMS on it".to_string(),
                            core_document::units::show_mm(
                                r["rms"].as_f64().unwrap_or(0.0),
                                document.display_unit(),
                            ),
                        ),
                    ],
                    histogram: Vec::new(),
                    tolerance: None,
                },
            });
        }
        out
    }

    pub(crate) fn viewport_hud(
        &self,
        _ctx: &WorkbenchRuntimeContext,
    ) -> Option<core_document::ViewportHud> {
        self.centre_hud()
    }

    pub(crate) fn screen_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceOverlay> {
        let mut lines = self.centre_overlays(ctx);
        for handle in self.handles(ctx) {
            lines.extend(handles::overlays(ctx, &handle));
        }
        lines.extend(self.failing_edge_overlays(ctx, active_feature));
        lines
    }

    pub(crate) fn screen_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceMark> {
        let mut marks = self.centre_marks(ctx);
        for handle in self.handles(ctx) {
            marks.extend(handles::marks(ctx, &handle, self.tab.held.is_some()));
        }
        marks.extend(instances::marks(
            ctx,
            &self.occurrences(ctx),
            self.tab.hovered_occurrence,
        ));
        marks
    }

    pub(crate) fn status_items(
        &self,
        _ctx: &WorkbenchRuntimeContext,
    ) -> Option<core_document::StatusItems> {
        let held = self.tab.held.as_ref()?;
        let (step, fine) = self.handle_steps(held);
        Some(core_document::StatusItems {
            mode: Some(handles::hint(held, step, fine)),
            ..Default::default()
        })
    }

    pub(crate) fn screen_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceLabel> {
        let mut labels = self.centre_labels(ctx);
        if let Some((handle, typed)) = &self.tab.typed {
            labels.extend(handles::typed_label(ctx, handle, typed));
        } else if let Some(handle) = &self.tab.held {
            labels.extend(handles::labels(ctx, handle));
        }
        labels.extend(instances::labels(
            ctx,
            &self.occurrences(ctx),
            self.tab.hovered_occurrence,
        ));
        labels
    }

    pub(crate) fn overlay_meshes(
        &self,
        ctx: &WorkbenchRuntimeContext,
        active_feature: Option<FeatureId>,
    ) -> Vec<core_document::OverlayMesh> {
        let Some(body) = Self::target_body(ctx) else {
            return Vec::new();
        };
        let mut meshes = Vec::new();
        // Datums live in their body's frame and draw where the body sits.
        let placement = ctx.document.body_placement(body);
        for (id, _, datum) in core_document::datums_of_body(ctx.document, body) {
            let visible = ctx
                .document
                .get_feature_meta(id)
                .map(|n| n.visible)
                .unwrap_or(true);
            if !visible {
                continue;
            }
            let color = if active_feature == Some(id) {
                [1.0, 0.75, 0.2]
            } else {
                [0.55, 0.55, 0.95]
            };
            meshes.push(core_document::OverlayMesh::wireframe(
                placement.mesh(&datum_mesh(&datum)),
                color,
            ));
        }
        meshes.extend(
            self.task_feature()
                .and_then(|task| region_pick::overlay(ctx, task)),
        );
        meshes.extend(
            self.tab
                .snapped
                .as_ref()
                .and_then(|snap| face_snap::lit(ctx, snap)),
        );
        meshes
    }
}

/// Wireframe mesh for a datum, in its body's frame.
pub(crate) fn datum_mesh(datum: &core_document::DatumFeature) -> kernel_api::TriMesh {
    let frame = datum.frame();
    let o = frame.origin;
    let x = frame.x_axis;
    let y = frame.y_axis();
    let n = frame.normal;
    let at = |sx: f32, sy: f32, sn: f32| -> [f32; 3] {
        [
            o[0] + x[0] * sx + y[0] * sy + n[0] * sn,
            o[1] + x[1] * sx + y[1] * sy + n[1] * sn,
            o[2] + x[2] * sx + y[2] * sy + n[2] * sn,
        ]
    };
    let mut mesh = kernel_api::TriMesh::default();
    match datum.shape {
        core_document::DatumShape::Plane { size } => {
            let h = size * 0.5;
            mesh.positions = vec![
                at(-h, -h, 0.0),
                at(h, -h, 0.0),
                at(h, h, 0.0),
                at(-h, h, 0.0),
            ];
            mesh.normals = vec![n; 4];
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            // Border + diagonal edges make the plane readable as wireframe.
            mesh.edges = vec![0, 1, 1, 2, 2, 3, 3, 0, 0, 2];
        }
        core_document::DatumShape::Line { length } => {
            let h = length * 0.5;
            // A degenerate-thin triangle along the x-axis; the edge list is
            // what the viewer reads.
            mesh.positions = vec![at(-h, 0.0, 0.0), at(h, 0.0, 0.0), at(h, 0.2, 0.0)];
            mesh.normals = vec![n; 3];
            mesh.indices = vec![0, 1, 2];
            mesh.edges = vec![0, 1];
        }
        core_document::DatumShape::Point => {
            let s = 1.5;
            mesh.positions = vec![
                at(-s, 0.0, 0.0),
                at(s, 0.0, 0.0),
                at(0.0, -s, 0.0),
                at(0.0, s, 0.0),
                at(0.0, 0.0, -s),
                at(0.0, 0.0, s),
            ];
            mesh.normals = vec![n; 6];
            mesh.indices = vec![0, 1, 2, 3, 4, 5];
            mesh.edges = vec![0, 1, 2, 3, 4, 5];
        }
        core_document::DatumShape::CoordinateSystem { size } => {
            // Three axes from the origin, each with an arrowhead, and a
            // corner square between x and y marking the frame's XY plane.
            let s = size;
            let h = size * 0.12;
            let c = size * 0.3;
            mesh.positions = vec![
                at(0.0, 0.0, 0.0),
                at(s, 0.0, 0.0),
                at(0.0, s, 0.0),
                at(0.0, 0.0, s),
                at(s - h, h * 0.5, 0.0),
                at(s - h, -h * 0.5, 0.0),
                at(h * 0.5, s - h, 0.0),
                at(-h * 0.5, s - h, 0.0),
                at(h * 0.5, 0.0, s - h),
                at(-h * 0.5, 0.0, s - h),
                at(c, 0.0, 0.0),
                at(c, c, 0.0),
                at(0.0, c, 0.0),
            ];
            mesh.normals = vec![n; mesh.positions.len()];
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            mesh.edges = vec![
                0, 1, 0, 2, 0, 3, 1, 4, 1, 5, 2, 6, 2, 7, 3, 8, 3, 9, 10, 11, 11, 12,
            ];
        }
    }
    mesh
}
