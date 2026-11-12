//! What the bench draws over the view: point crosses, labels kept apart,
//! the maps it paints, faded references and the status line.

use crate::{
    ID, InspectionWorkbench, READING_REACH, SPOT_LEAST, clearance, compare, compare::Comparison,
    construction, dimension, element, element_map, hotspots, making::dimension_at,
    making::element_at, making::tolerance_at, overhang, picks, pins, section,
    sectioning::shadows_on, shadow, thickness, tolerance, volume,
};
use core_document::units::show_mm_signed;
use core_document::{BodyId, Document, WorkbenchFeature, WorkbenchRuntimeContext};
use glam::DVec3;
use kernel_api::FitKind;

/// Hot spots labelled on the part, the largest first.
const LABELLED_SPOTS: usize = 5;

/// A point element's cross across, in logical pixels.
const POINT_CROSS_PT: f32 = 14.0;

/// Move labels down until none covers another, in the order given.
fn spread(labels: &mut [core_document::ScreenSpaceLabel]) {
    let half = |l: &core_document::ScreenSpaceLabel| {
        (
            l.text.chars().count() as f32 * l.size * 0.32 + 6.0,
            l.size * 0.9 + 3.0,
        )
    };
    for i in 1..labels.len() {
        let (w, h) = half(&labels[i]);
        for _ in 0..labels.len() {
            let hit = labels[..i].iter().find(|o| {
                let (ow, oh) = half(o);
                (o.pos[0] - labels[i].pos[0]).abs() < w + ow - 0.5
                    && (o.pos[1] - labels[i].pos[1]).abs() < h + oh - 0.5
            });
            match hit {
                Some(o) => labels[i].pos[1] = o.pos[1] + half(o).1 + h,
                None => break,
            }
        }
    }
}

impl InspectionWorkbench {
    /// The map shown on `scan`, as `scalar_maps` picks it (comparisons,
    /// then maps against elements, then wall thickness, the latest of each
    /// first), with its values when they are measured.
    pub(crate) fn shown_map(
        &self,
        document: &Document,
        scan: BodyId,
    ) -> Option<(core_document::FeatureId, std::sync::Arc<[f32]>)> {
        self.tab
            .maps
            .shown::<Comparison>(document, scan)
            .or_else(|| {
                self.tab
                    .maps
                    .shown::<element_map::ElementMap>(document, scan)
            })
            .or_else(|| {
                self.tab
                    .maps
                    .shown::<thickness::WallThickness>(document, scan)
            })
            .flatten()
    }

    /// The shown map's value under the cursor, and where, in the scan's
    /// frame.
    pub(crate) fn hovered_reading(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<(BodyId, f64, [f64; 3])> {
        let scan = ctx.hovered_body_id.map(BodyId)?;
        let face = ctx.hovered_face?;
        let (_, values) = self.shown_map(ctx.document, scan)?;
        let (mesh, _) = ctx.document.local_geometry(scan)?;
        let at = ctx
            .document
            .body_placement(scan)
            .inverse()
            .point(face.point)
            .map(f64::from);
        Some((scan, pins::read(&mesh, &values, at, READING_REACH)?, at))
    }

    fn armed_status(&self, _ctx: &WorkbenchRuntimeContext) -> Option<core_document::StatusItems> {
        let kind = self.tab.armed?;
        Some(core_document::StatusItems {
            mode: Some(format!(
                "Click a scan {} to fit a {} there · Shift-click pins one more seed · Esc to stop",
                if matches!(kind, FitKind::Circle | FitKind::Line) {
                    "by a sharp edge"
                } else {
                    "on a surface"
                },
                element::kind_word(kind).to_lowercase()
            )),
            ..Default::default()
        })
    }

    pub(crate) fn status_items(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<core_document::StatusItems> {
        if self.tab.fitting.running() {
            return Some(core_document::StatusItems {
                mode: Some("Fitting the scan onto its reference… · Esc to stop".to_string()),
                ..Default::default()
            });
        }
        if self.tab.picks.aligning {
            let n = self.tab.picks.pairs();
            return Some(core_document::StatusItems {
                mode: Some(format!(
                    "Align by points: click a feature on the scan, then the same on the \
                     reference · {n} pair{} · Enter aligns from {} · Esc to stop",
                    if n == 1 { "" } else { "s" },
                    picks::PAIRS_NEEDED
                )),
                ..Default::default()
            });
        }
        self.armed_status(ctx)
    }

    pub(crate) fn faded_bodies(&self, ctx: &WorkbenchRuntimeContext) -> Vec<BodyId> {
        let document = &*ctx.document;
        document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == compare::KIND && n.visible)
            .filter_map(|(id, _)| document.feature_as::<Comparison>(*id).map(|c| c.reference))
            .collect()
    }

    pub(crate) fn scalar_maps(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Vec<core_document::ScalarMap> {
        let Some(kernel) = ctx.kernel else {
            return Vec::new();
        };
        let _ = self.kernel.set(kernel);
        let mut maps = Vec::new();
        self.tab
            .maps
            .paint_all::<Comparison>(ctx, kernel, &mut maps);
        self.tab
            .maps
            .paint_all::<element_map::ElementMap>(ctx, kernel, &mut maps);
        self.tab
            .maps
            .paint_all::<thickness::WallThickness>(ctx, kernel, &mut maps);
        self.tab
            .maps
            .paint_all::<clearance::Clearance>(ctx, kernel, &mut maps);
        self.tab
            .maps
            .paint_all::<overhang::Overhang>(ctx, kernel, &mut maps);
        maps
    }

    /// Every point element, a cross the same size at any zoom, and the
    /// clicks of an element in progress, pinned.
    pub(crate) fn screen_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<core_document::FeatureId>,
    ) -> Vec<core_document::ScreenSpaceMark> {
        let document = &*ctx.document;
        let mut marks: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| {
                n.visible && matches!(n.workbench_id.as_str(), ID | construction::KIND)
            })
            .filter_map(|(id, _)| {
                let e = element_at(document, *id)?;
                let kernel_api::FittedShape::Sphere { centre, radius } = e.result.shape else {
                    return None;
                };
                if radius != 0.0 || !document.imported_body_effective_visible(e.scan) {
                    return None;
                }
                let world = document
                    .body_placement(e.scan)
                    .point(centre.map(|c| c as f32));
                let (x, y) = ctx.world_to_viewport_on(Some(e.scan.0), world)?;
                Some(core_document::ScreenSpaceMark::crosshair(
                    [x, y],
                    POINT_CROSS_PT * ctx.pixels_per_point,
                    ctx.sketch_palette.external,
                ))
            })
            .collect();
        marks.extend(self.tab.picks.drawn(ctx).0);
        marks
    }

    pub(crate) fn screen_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<core_document::FeatureId>,
    ) -> Vec<core_document::ScreenSpaceLabel> {
        let palette = &ctx.sketch_palette;
        let document = &*ctx.document;
        let mut labels = self.tab.picks.drawn(ctx).1;
        let mut put = |body: BodyId, at: [f64; 3], text: String, color: [f32; 3]| {
            if !document.imported_body_effective_visible(body) {
                return;
            }
            let world = document.body_placement(body).point(at.map(|c| c as f32));
            if let Some((x, y)) = ctx.world_to_viewport(world) {
                labels.push(core_document::ScreenSpaceLabel {
                    pos: [x, y],
                    text,
                    color,
                    size: 11.0,
                    background: true,
                    mono: true,
                });
            }
        };
        let mut dims: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == dimension::KIND && n.visible)
            .collect();
        dims.sort_by_key(|(id, n)| (n.seq, **id));
        for (id, _) in dims {
            let Some((dim, Ok(reading))) = dimension_at(document, *id) else {
                continue;
            };
            let color = match dim.passes(reading.value) {
                Some(true) => palette.fully_constrained,
                Some(false) => palette.conflict,
                None => palette.constraint,
            };
            let middle = ((DVec3::from(reading.from) + DVec3::from(reading.to)) * 0.5).to_array();
            put(
                dim.scan,
                middle,
                dim.label(&reading, ctx.document.display_unit()),
                color,
            );
        }
        let mut sections: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == section::KIND && n.visible)
            .collect();
        sections.sort_by_key(|(id, n)| (n.seq, **id));
        for (id, node) in sections {
            let Some((s, cuts, _)) = self.cuts_of(document, *id) else {
                continue;
            };
            // A stack's top cut says how its loops run through it.
            let tracked = section::tracks(&cuts, s.normal);
            let summary = match tracked.as_slice() {
                [] => String::new(),
                [t] => format!(
                    " · draft {:+.2}° · tilt {:.2}° · R {:.2}→{:.2}",
                    t.draft_deg + 0.0,
                    t.tilt_deg,
                    t.radius[0],
                    t.radius[1]
                ),
                many => format!(" · {} loops tracked", many.len()),
            };
            for (k, c) in cuts.iter().enumerate() {
                let Some(at) = c
                    .chains
                    .iter()
                    .max_by(|a, b| a.points.len().cmp(&b.points.len()))
                    .and_then(|chain| chain.points.first())
                else {
                    continue;
                };
                let mut text = format!("{} · {:.1} mm", node.name, c.length());
                if s.count > 1 {
                    text = format!("{} {:+.1} · {:.1} mm", node.name, c.offset, c.length());
                }
                if c.closed() > 0 {
                    text.push_str(&format!(" · {:.1} mm²", c.area(s.normal)));
                }
                if k + 1 == cuts.len() {
                    text.push_str(&summary);
                }
                put(s.scan, *at, text, palette.constraint);
            }
        }
        for (_, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() != shadow::KIND || !node.visible {
                continue;
            }
            let Some(body) = node.body else {
                continue;
            };
            let Some((_, s)) = shadows_on(document, body).into_iter().next() else {
                continue;
            };
            let Some(made) = s.made.as_ref() else {
                continue;
            };
            let Some(at) = made
                .outline
                .first()
                .and_then(|w| w.segments.first())
                .map(|seg| {
                    let [u, v] = match *seg {
                        kernel_api::ProfileSegment::Line { start, .. }
                        | kernel_api::ProfileSegment::Arc { start, .. } => start,
                        kernel_api::ProfileSegment::Circle { center, radius } => {
                            [center[0] + radius, center[1]]
                        }
                        _ => [0.0; 2],
                    };
                    shadow::on_plane(&made.plane, [u, v])
                })
            else {
                continue;
            };
            let loops = made.outline.len();
            let text = format!(
                "{} · {loops} loop{} · {:.1} mm² · clear {:.2}–{:.2} mm",
                node.name,
                if loops == 1 { "" } else { "s" },
                made.area,
                made.band[0],
                made.band[1]
            );
            put(
                body,
                ctx.document
                    .body_placement(body)
                    .inverse()
                    .point(at.map(|c| c as f32))
                    .map(f64::from),
                text,
                palette.constraint,
            );
        }
        let mut tols: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.workbench_id.as_str() == tolerance::KIND && n.visible)
            .collect();
        tols.sort_by_key(|(id, n)| (n.seq, **id));
        for (id, _) in tols {
            let Some((t, Ok(e))) = tolerance_at(
                document,
                *id,
                &self.tab.tolerance_readings,
                self.kernel.get().copied(),
                false,
            ) else {
                continue;
            };
            let color = if t.accepts(&e) {
                palette.fully_constrained
            } else {
                palette.conflict
            };
            put(
                t.scan,
                e.at,
                t.label(&e, ctx.document.display_unit()),
                color,
            );
        }
        for (_, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() != pins::KIND || !node.visible {
                continue;
            }
            let Ok(pin) = pins::Pin::from_json(&node.data) else {
                continue;
            };
            let Some((map, values)) = self.shown_map(document, pin.scan) else {
                continue;
            };
            if map != pin.map {
                continue;
            }
            let Some((mesh, _)) = document.local_geometry(pin.scan) else {
                continue;
            };
            let text = pins::read(&mesh, &values, pin.at, READING_REACH).map_or_else(
                || "unmeasured".to_string(),
                |v| show_mm_signed(v, ctx.document.display_unit()),
            );
            put(pin.scan, pin.at, text, palette.constraint);
        }
        if let Some((scan, value, at)) = self.hovered_reading(ctx) {
            put(
                scan,
                at,
                show_mm_signed(value, ctx.document.display_unit()),
                palette.selected,
            );
        }
        // The largest hot spots of each shown comparison with a tolerance.
        for (id, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() != compare::KIND || !node.visible {
                continue;
            }
            let Some(c) = document.feature_as::<Comparison>(*id) else {
                continue;
            };
            let (Some(tolerance), Ok(inputs)) = (c.tolerance, compare::inputs(document, &c)) else {
                continue;
            };
            let Some(spots) = self.tab.maps.hot_spots(*id, inputs.key, tolerance, || {
                self.tab
                    .maps
                    .values(*id, inputs.key)
                    .map(|v| hotspots::find(inputs.scan_mesh(), &v, tolerance, SPOT_LEAST))
            }) else {
                continue;
            };
            for spot in spots.iter().take(LABELLED_SPOTS) {
                let color = if spot.side > 0 {
                    palette.conflict
                } else {
                    palette.constraint
                };
                put(
                    c.scan,
                    spot.at,
                    format!("{:+.3} mm · {:.0} mm²", spot.extreme, spot.area),
                    color,
                );
            }
        }
        // The largest volume errors of each shown comparison that reads them.
        for (id, node) in document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() != compare::KIND || !node.visible {
                continue;
            }
            let Some(c) = document.feature_as::<Comparison>(*id) else {
                continue;
            };
            let (Some(cell), Ok(inputs)) = (c.volume_cell, compare::inputs(document, &c)) else {
                continue;
            };
            let key = inputs.key;
            let back = inputs.scan_to_reference().inverse();
            let Some(found) = self.tab.maps.volumes(*id, key, cell) else {
                if let Some(kernel) = self.kernel.get() {
                    self.tab.maps.request_volumes(*id, inputs, cell, *kernel);
                }
                continue;
            };
            let Ok(regions) = found else {
                continue;
            };
            for r in regions.iter().take(LABELLED_SPOTS) {
                let at = back.point(volume::centre(r).map(|v| v as f32));
                let color = if r.missing {
                    palette.constraint
                } else {
                    palette.conflict
                };
                put(
                    c.scan,
                    at.map(f64::from),
                    volume::label(r, document.display_unit()),
                    color,
                );
            }
        }
        spread(&mut labels);
        labels
    }
}
