//! What the sketcher hands the viewport each frame: the HUD, the status
//! items, the clip plane and grid, pictures, overlay meshes, lines, marks
//! and labels, and the frame's reading of the edited sketch they are drawn
//! from (its overlays and constraint glyphs).

use std::collections::HashSet;

use core_document::{
    FeatureId, ScreenSpaceLabel, ScreenSpaceMark, SketchPalette, StatusItems, ToolHint,
    ViewportHud, WorkbenchFeature, WorkbenchRuntimeContext,
};
use uuid::Uuid;

use crate::boundary::{narrow, narrow3, widen3};
use crate::overlay::SketchProjector;
use crate::registration::{idle_hint, tool_icon};
use crate::sketch::{GeometryElement, Sketch, Vec2D};
use crate::tools::ToolParams;
use crate::tools::ToolState;
use crate::{
    POLYLINE_ARC_ACTION, REGION_LIFT_MM, SNAP_TOLERANCE_PX, SPLINE_LABEL_PT, SketchFeature,
    SketchWorkbench, candidates, frame_memo, glyphs, images, overlay, ovp, picture_edges,
    relations, scan, section_equation, sketch, snap, spline_edit, style, tools, walls,
};

impl SketchWorkbench {
    pub(crate) fn viewport_hud(&self, ctx: &WorkbenchRuntimeContext) -> Option<ViewportHud> {
        let feature = self.frame_displayed(ctx)?;
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let tool = self.tab.last_tool.as_deref().unwrap_or("sketch.select");
        let calibrating = self.tab.calibrating.as_ref().map(|c| match (c.from, c.to) {
            (None, _) => "Click the first end of a length you know on the picture",
            (Some(_), None) => "Click the other end",
            _ => "Type the real length in the panel and apply it",
        });
        let waiting = self.tab.corner_task.is_some();
        let (name, prompt) = match self.tab.tool_state.hint() {
            _ if calibrating.is_some() => ("Calibrate picture", calibrating.unwrap_or_default()),
            _ if waiting => (
                idle_hint(tool).0,
                "Set it in the Tool section: OK keeps it, Cancel drops it",
            ),
            Some((name, prompt)) => (name, prompt),
            None if tool == "sketch.external" && self.tab.external_intersection => (
                "Intersection",
                "Click faces of a solid to add where they cross the sketch plane",
            ),
            None if tool == "sketch.rotate" && self.tab.tool_params.rotate_full_turn => {
                ("Rotate", "Click the centre: the copies go evenly around it")
            }
            None => idle_hint(tool),
        };
        let mut keys: Vec<(String, &'static str)> = Vec::new();
        let mut key = |key: &str, meaning: &'static str| keys.push((key.to_string(), meaning));
        // Enter locks a typed value, and otherwise finishes a chain.
        let typed = self.tab.dim_capture.has_typed_input();
        if self.tab.dim_capture.is_active() {
            key("Tab", "next field");
        }
        if typed {
            key("Enter", "lock value");
        }
        if matches!(
            self.tab.tool_state,
            ToolState::LineFrom { chain: true, .. }
                | ToolState::BSplineDraw { .. }
                | ToolState::PathDraw { .. }
        ) {
            if !typed {
                key("Enter", "finish");
            }
        } else if let ToolState::PolylineFrom {
            segment, heading, ..
        } = self.tab.tool_state
        {
            if heading.is_some()
                && let Some(switch) = self.action_keys.get(POLYLINE_ARC_ACTION)
            {
                key(
                    switch,
                    match segment.next() {
                        tools::PolySegment::Line => "lines",
                        tools::PolySegment::Tangent => "tangent arcs",
                        tools::PolySegment::Perpendicular => "square arcs",
                        tools::PolySegment::Reverse => "reversed arcs",
                    },
                );
            }
            if !typed {
                key("Enter", "finish");
            }
        }
        let armed = self.armed_hint();
        if armed.is_some() || waiting {
            key("Enter", "apply");
        }
        // Selecting is what the view does at rest: it takes no card.
        let resting =
            armed.is_none() && tool == "sketch.select" && self.tab.tool_state.hint().is_none();
        if waiting {
            key("Esc", "keep");
        } else if !resting {
            key("Esc", "cancel");
        }
        let verdict = self.solver_verdict(&feature);
        let zoom = 1.0 / proj.units_per_px().max(1e-6);
        let card = self.tab.cursor.and_then(|cursor| {
            // The card reads where the click lands, snapped, and stands
            // by the cursor.
            let landing = self.click_landing(ctx, &feature, cursor).0;
            let widgets = ovp::readout_fields(
                &self.tab.dim_capture,
                &self.tab.tool_state,
                self.tab.last_tool.as_deref(),
                &feature.sketch,
                landing,
                &self.tab.tool_params,
                ctx.document,
            );
            if widgets.is_empty() {
                return None;
            }
            let px = proj.to_px(cursor)?;
            // Some steps take no constraint: typed, they only place the click.
            let hint = if ovp::only_places(&self.tab.tool_state) {
                "Tab next · Enter places it"
            } else {
                "Tab next · Enter constrains · click keeps free"
            };
            Some(bench_api::FloatingCard {
                id: format!("sketch.readout:{tool}"),
                screen: Some(px),
                widgets,
                hint: Some(hint.to_string()),
                ..Default::default()
            })
        });
        Some(ViewportHud {
            tool: (!resting).then(|| match armed {
                Some(name) => ToolHint {
                    icon: "external-geometry",
                    name,
                    prompt: "Click sketch elements, or edges and vertices of the model".to_string(),
                    keys,
                },
                None => ToolHint {
                    icon: tool_icon(tool),
                    name: name.to_string(),
                    prompt: prompt.to_string(),
                    keys,
                },
            }),
            badge: Some((verdict.kind.color(&pal), format!("{} DoF", verdict.dof))),
            legend: vec![
                (pal.geometry, "Normal"),
                (pal.construction, "Construction"),
                (pal.external, "External"),
                (pal.locked, "Locked"),
                (pal.fully_constrained, "Fully constrained"),
                (pal.constraint, "Constraint"),
            ],
            footer: [
                style::placement_label(&feature),
                if self.tab.snap_off {
                    "Snap: off".to_string()
                } else {
                    "Snap: objects".to_string()
                },
                format!("Zoom {zoom:.1}×"),
            ]
            .into_iter()
            .chain(
                self.tab
                    .parked_layer
                    .then(|| "Constraints: parked layer".to_string()),
            )
            .chain(self.shown_wall_check().map(walls::WallCheck::summary))
            .collect(),
            card,
        })
    }

    /// A sketch edited in section view cuts the scene at its plane, keeping
    /// the side away from the viewer, and a hair beyond the plane so a face
    /// the sketch lies on stays.
    pub(crate) fn clip_plane(&self, ctx: &WorkbenchRuntimeContext) -> Option<[f32; 4]> {
        let feature = self.frame_sketch(ctx)?;
        if !feature.shows_section(self.options.section_on_open) {
            return None;
        }
        Some(section_equation(&feature.plane, widen3(ctx.camera_position)).map(narrow))
    }

    /// The grid on the edited sketch's plane while it is switched on, its
    /// lines along the sketch's axes from its origin.
    pub(crate) fn view_grid(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<core_document::PlaneGrid> {
        if !self.options.grid_on {
            return None;
        }
        let plane = self.frame_sketch(ctx)?.plane;
        let look = &self.options.appearance;
        Some(core_document::PlaneGrid {
            origin: narrow3(plane.origin),
            x: narrow3(plane.x_axis),
            y: narrow3(plane.y_axis),
            base: self.grid_base() as f32,
            color: look.grid_color,
            opacity: look.grid_opacity,
            dash: look.grid_lines.dash(),
        })
    }

    pub(crate) fn status_items(&self, ctx: &WorkbenchRuntimeContext) -> Option<StatusItems> {
        let feature = self.frame_displayed(ctx)?;
        let pal = ctx.sketch_palette;
        let verdict = self.solver_verdict(&feature);
        // Elements are named by their ordinals, worked out only when one
        // is selected.
        let mut names: Vec<String> = if self.tab.selected.is_empty() {
            Vec::new()
        } else {
            let named = style::ElementNames::new(&feature.sketch);
            feature
                .sketch
                .geometry
                .iter()
                .filter(|g| self.tab.selected.contains(&g.id()))
                .map(|g| named.name(g.id()))
                .collect()
        };
        names.extend(
            feature
                .sketch
                .constraints
                .iter()
                .filter(|c| self.tab.selected_constraints.contains(&c.id))
                .map(|c| {
                    c.name
                        .clone()
                        .unwrap_or_else(|| sketch::constraint_label(&c.kind))
                }),
        );
        let selection = match names.len() {
            0 => None,
            n if n <= 3 => Some(names.join(" · ")),
            n => Some(format!("{} · {} more", names[..2].join(" · "), n - 2)),
        };
        Some(StatusItems {
            state: Some((verdict.kind.color(&pal), verdict.title)),
            selection,
            coords: self
                .tab
                .cursor
                .map(|c| format!("X {:.2} · Y {:.2} mm", c.x, c.y)),
            mode: Some(match self.scan_fit(&feature.sketch, ctx) {
                Some((within, all)) => format!(
                    "Sketch edit mode · scan {within} of {all} points within {} mm",
                    self.options.scan_range
                ),
                None => "Sketch edit mode".to_string(),
            }),
        })
    }

    pub(crate) fn screen_images(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceImage> {
        let Some(feature) = self.frame_displayed(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let loupe = self.loupe(ctx, &feature, &proj).map(|l| l.image);
        feature
            .sketch
            .images
            .iter()
            .filter_map(|image| {
                let decoded = self.tab.image_cache.get(image.asset, || {
                    ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
                })?;
                let aspect = decoded.height as f64 / decoded.width.max(1) as f64;
                let [a, b, c, d] = images::corners(image, aspect);
                let (hi, lo) = image.asset.as_u64_pair();
                Some(core_document::ScreenSpaceImage {
                    key: hi ^ lo,
                    size: [decoded.width, decoded.height],
                    rgba: std::sync::Arc::clone(&decoded.rgba),
                    corners: [
                        proj.to_px(a)?,
                        proj.to_px(b)?,
                        proj.to_px(c)?,
                        proj.to_px(d)?,
                    ],
                    opacity: narrow(image.opacity),
                })
            })
            .chain(loupe)
            .collect()
    }

    /// The regions the edited sketch's curves close, shaded on its plane
    /// as the view shades the sketch out of editing
    /// (`closed_parts::Kept::passive_regions`): every region, the ones a
    /// plain profile takes stronger.
    pub(crate) fn overlay_meshes(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::OverlayMesh> {
        if !self.options.shade_regions {
            return Vec::new();
        }
        let (Some(id), Some(feature), Some(kernel)) = (
            self.tab.active_sketch_id,
            self.frame_displayed(ctx),
            ctx.kernel,
        ) else {
            return Vec::new();
        };
        let opacity = self.options.appearance.region_opacity;
        // The stored sketch is known by its data, as out of editing; an
        // open card's draft by what it holds.
        let drafting = self.tab.corner_task.is_some() || self.tab.dim_edit.is_some();
        let revision = match ctx.document.values_revision(id) {
            Some(revision) if !drafting => revision,
            _ => core_document::data_revision(&feature.to_json()),
        };
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            let plane = feature.plane;
            let axes = [plane.origin, plane.normal, plane.x_axis, plane.y_axis];
            (
                revision,
                axes.map(|v| v.map(f64::to_bits)),
                opacity.to_bits(),
            )
                .hash(&mut hasher);
            hasher.finish()
        };
        let mut cached = self.tab.regions.borrow_mut();
        // A drag that moves the geometry keeps the regions shaded where it
        // started; its release has them worked out for the sketch it leaves.
        let moving = self.tab.dragging.as_ref().is_some_and(|drag| drag.moved);
        if cached.as_ref().is_none_or(|(k, _)| *k != key && !moving) {
            let meshes = self
                .tab
                .closed_parts
                .passive_regions(id, revision, &feature, REGION_LIFT_MM, opacity)
                .into_iter()
                .filter_map(|region| {
                    let mesh = kernel.profile_mesh(&region.profile).ok()?;
                    (!mesh.indices.is_empty()).then(|| (std::sync::Arc::new(mesh), region.opacity))
                })
                .collect();
            *cached = Some((key, meshes));
        }
        let Some((key, meshes)) = cached.as_ref() else {
            return Vec::new();
        };
        meshes
            .iter()
            .enumerate()
            .map(|(index, (mesh, opacity))| core_document::OverlayMesh {
                mesh: std::sync::Arc::clone(mesh),
                revision: key.wrapping_add(index as u64),
                color: ctx.sketch_palette.geometry,
                wireframe: false,
                opacity: *opacity,
                on_top: false,
            })
            .collect()
    }

    pub(crate) fn screen_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceOverlay> {
        let Some(feature) = self.frame_displayed(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let mut out = Vec::new();
        // The loupe's frame, its edges and a cross on the snapped point.
        if let Some(loupe) = self.loupe(ctx, &feature, &proj) {
            let c = loupe.image.corners;
            for i in 0..4 {
                out.push(core_document::ScreenSpaceOverlay::new(
                    c[i],
                    c[(i + 1) % 4],
                    pal.preselect,
                    1.5,
                ));
            }
            for (p, q) in loupe.edges {
                out.push(core_document::ScreenSpaceOverlay::new(
                    p,
                    q,
                    pal.external,
                    1.0,
                ));
            }
            let arm = 8.0 * ctx.pixels_per_point;
            let [x, y] = loupe.middle;
            for (p, q) in [([x - arm, y], [x + arm, y]), ([x, y - arm], [x, y + arm])] {
                out.push(core_document::ScreenSpaceOverlay::new(
                    p,
                    q,
                    pal.preselect,
                    1.0,
                ));
            }
        }
        // The reference pictures' edges, as thin guides over them.
        for image in &feature.sketch.images {
            let Some(set) = self.tab.picture_edges.get(image.asset, || {
                ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
            }) else {
                continue;
            };
            let step = (set.width.max(set.height) / 1024).max(1);
            for chain in &set.chains {
                let points: Vec<_> = chain
                    .points
                    .iter()
                    .step_by(step)
                    .filter_map(|p| proj.to_px(picture_edges::to_sketch(image, &set, *p)))
                    .collect();
                for pair in points.windows(2) {
                    out.push(core_document::ScreenSpaceOverlay::new(
                        pair[0],
                        pair[1],
                        pal.external,
                        1.0,
                    ));
                }
            }
        }
        // The scans' section, dashed under the sketch's own lines, every
        // fifth of a millimetre at most: in the fitting colour where it
        // lies within range of the sketch's curves, the conflict colour
        // where it does not.
        let guides = self.scan_guides(ctx);
        let off = std::cell::LazyCell::new(|| scan::distance_to_sketch(&feature.sketch));
        let range = self.options.scan_range;
        for chain in guides.iter() {
            let mut kept: Vec<[f64; 2]> = Vec::new();
            for p in &chain.points {
                if kept
                    .last()
                    .is_none_or(|q| (p[0] - q[0]).hypot(p[1] - q[1]) >= 0.2)
                {
                    kept.push(*p);
                }
            }
            if chain.closed
                && let Some(first) = kept.first().copied()
            {
                kept.push(first);
            }
            for pair in kept.windows(2) {
                let at = |p: [f64; 2]| proj.to_px(Vec2D::new(p[0], p[1]));
                if let (Some(a), Some(b)) = (at(pair[0]), at(pair[1])) {
                    let color = match (*off)(pair[0]).zip((*off)(pair[1])) {
                        Some((d, e)) if d.max(e) <= range => pal.fully_constrained,
                        Some(_) => pal.conflict,
                        None => pal.external,
                    };
                    out.push(
                        core_document::ScreenSpaceOverlay::new(a, b, color, 1.5).dashed(4.0, 3.0),
                    );
                }
            }
        }
        out.extend(
            self.frame_overlays(ctx, &feature, &proj, &pal)
                .lines()
                .cloned(),
        );
        // The calibration's line, to the cursor until its second point.
        if let Some(calibrating) = &self.tab.calibrating
            && let Some(from) = calibrating.from
            && let Some(to) = calibrating.to.or(self.tab.cursor)
            && let (Some(a), Some(b)) = (proj.to_px(from), proj.to_px(to))
        {
            out.push(core_document::ScreenSpaceOverlay::new(
                a,
                b,
                pal.preview,
                2.0,
            ));
        }
        if let Some(check) = self.shown_wall_check() {
            out.extend(walls::overlays(check, &proj, &pal));
        }
        if self.options.spline_comb {
            for spline in feature.sketch.geometry.iter().filter_map(|g| match g {
                GeometryElement::BSpline(b) => Some(b.id),
                _ => None,
            }) {
                for (a, b) in spline_edit::comb(&feature.sketch, spline) {
                    if let (Some(a), Some(b)) = (proj.to_px(a), proj.to_px(b)) {
                        out.push(core_document::ScreenSpaceOverlay::new(
                            a,
                            b,
                            pal.preview,
                            1.0,
                        ));
                    }
                }
            }
        }
        if !self.options.constraints_hidden {
            out.extend(
                self.frame_glyphs(ctx, &feature, &proj)
                    .lines
                    .iter()
                    .cloned(),
            );
        }
        out
    }

    pub(crate) fn screen_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceMark> {
        let Some(feature) = self.frame_displayed(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let mut out: Vec<ScreenSpaceMark> = self
            .frame_overlays(ctx, &feature, &proj, &pal)
            .marks()
            .cloned()
            .collect();
        if let Some(check) = self.shown_wall_check() {
            out.extend(walls::marks(check, &proj, &pal));
        }
        if self.options.spline_knots {
            for spline in feature.sketch.geometry.iter().filter_map(|g| match g {
                GeometryElement::BSpline(b) => Some(b.id),
                _ => None,
            }) {
                for (at, multiplicity) in spline_edit::knot_points(&feature.sketch, spline) {
                    if let Some(px) = proj.to_px(at) {
                        let radius = 2.5 + multiplicity as f32;
                        out.push(ScreenSpaceMark::dot(px, radius, pal.constraint));
                    }
                }
            }
        }
        if !self.options.constraints_hidden {
            out.extend(
                self.frame_glyphs(ctx, &feature, &proj)
                    .marks
                    .iter()
                    .cloned(),
            );
        }
        out
    }

    pub(crate) fn screen_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<ScreenSpaceLabel> {
        let Some(feature) = self.frame_displayed(ctx) else {
            return Vec::new();
        };
        let proj = SketchProjector::new(ctx, feature.plane);
        let pal = ctx.sketch_palette;
        let mut labels: Vec<ScreenSpaceLabel> = if self.options.constraints_hidden {
            Vec::new()
        } else {
            self.frame_glyphs(ctx, &feature, &proj).labels.clone()
        };
        // What the cursor would snap to, by name.
        labels.extend(
            self.frame_overlays(ctx, &feature, &proj, &pal)
                .labels()
                .cloned(),
        );
        labels.extend(overlay::spline_labels(
            &proj,
            &pal,
            &feature.sketch,
            overlay::SplineLabels {
                degree: self.options.spline_degree,
                multiplicity: self.options.spline_multiplicity,
                weights: self.options.spline_weights,
            },
            SPLINE_LABEL_PT * ctx.pixels_per_point,
            ctx.pixels_per_point,
        ));
        if let Some(check) = self.shown_wall_check() {
            labels.extend(walls::labels(check, &proj, &pal));
        }
        // A picture not measured against something of known size says so.
        for image in &feature.sketch.images {
            let text = match &image.scale {
                sketch::ImageScale::Calibrated { .. } => continue,
                sketch::ImageScale::Nominal { dpi } => {
                    format!("UNCALIBRATED · {dpi:.0} dpi as the file says")
                }
                sketch::ImageScale::Given => "UNCALIBRATED · width as given".to_string(),
            };
            let aspect = self
                .tab
                .image_cache
                .get(image.asset, || {
                    ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
                })
                .map_or(1.0, |d| d.height as f64 / d.width.max(1) as f64);
            let [top_left, ..] = images::corners(image, aspect);
            if let Some([x, y]) = proj.to_px(top_left) {
                // Labels stand centred on their point: inside the corner.
                let half = text.chars().count() as f32 * 3.4;
                labels.push(
                    ScreenSpaceLabel::new([x + half + 8.0, y + 14.0], text, pal.conflict, 11.0)
                        .pill()
                        .mono(),
                );
            }
        }
        // The probable design value of each selected unsized curve, at its
        // middle.
        let sizes = if self.tab.selected.is_empty() {
            Vec::new()
        } else {
            candidates::sizes(&feature.sketch)
        };
        for size in sizes {
            if !self.tab.selected.contains(&size.element) {
                continue;
            }
            let middle = feature
                .sketch
                .geometry
                .iter()
                .find(|g| g.id() == size.element)
                .and_then(|g| match g {
                    GeometryElement::Line(l) => {
                        let (a, b) = (
                            feature.sketch.point_position(l.start)?,
                            feature.sketch.point_position(l.end)?,
                        );
                        Some(Vec2D::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0))
                    }
                    GeometryElement::Arc(a) => feature.sketch.point_position(a.start),
                    GeometryElement::Circle(c) => {
                        let o = feature.sketch.point_position(c.center)?;
                        Some(Vec2D::new(o.x + c.radius, o.y))
                    }
                    _ => None,
                });
            if let Some([x, y]) = middle.and_then(|m| proj.to_px(m)) {
                let word = if size.kind == "radius" { "R " } else { "" };
                labels.push(
                    ScreenSpaceLabel::new(
                        [x, y - 16.0],
                        format!("{:.2} ≈ {word}{}", size.measured, size.design),
                        pal.formula,
                        12.0,
                    )
                    .pill()
                    .mono(),
                );
            }
        }
        labels
    }
}

impl SketchWorkbench {
    /// The constraint glyphs of the layer shown, in viewport pixels.
    pub(crate) fn glyphs(
        &self,
        ctx: &WorkbenchRuntimeContext,
        sketch: &Sketch,
        proj: &SketchProjector,
    ) -> Vec<glyphs::Glyph> {
        let lit = self.lit_constraints(|| std::sync::Arc::new(relations::Relations::of(sketch)));
        self.build_glyphs(ctx, sketch, proj, &lit, &self.bound_dimensions(ctx))
    }

    /// The constraints whose glyphs draw lit: the selected ones, the one
    /// in hand, and an element in hand in the panel lights its
    /// constraints, as its row's badges and the constraint rows do.
    fn lit_constraints(
        &self,
        relations: impl FnOnce() -> std::sync::Arc<relations::Relations>,
    ) -> HashSet<Uuid> {
        let mut lit = self.tab.selected_constraints.clone();
        lit.extend(self.tab.hovered_constraint);
        if self.tab.hover_from_panel
            && let Some(id) = self.tab.hovered
        {
            lit.extend(relations().constraints_on(id));
        }
        lit
    }

    fn build_glyphs(
        &self,
        ctx: &WorkbenchRuntimeContext,
        sketch: &Sketch,
        proj: &SketchProjector,
        lit: &HashSet<Uuid>,
        bound: &HashSet<Uuid>,
    ) -> Vec<glyphs::Glyph> {
        glyphs::build(
            sketch,
            proj,
            lit,
            bound,
            &ctx.sketch_palette,
            self.tab.parked_layer,
            self.options.dimension_labels,
            self.options.appearance.label_px(ctx.pixels_per_point),
            &mut self.frame.arrangement(),
        )
    }

    /// What the cursor would snap to in select mode, shown without moving
    /// anything: the object snap a drag or a drawing tool would take there
    /// (a point, a centre, the origin, a crossing, a line's middle, a curve
    /// or an axis). None while a gesture is held, a box is drawn, a
    /// dimension's label is under the cursor or a constraint tool is armed.
    fn hover_snap(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
    ) -> Option<snap::Snap> {
        if !matches!(self.tab.last_tool.as_deref(), None | Some("sketch.select"))
            || self.tab.snap_off
            || self.tab.box_select.is_some()
            || self.tab.dragging.is_some()
            || self.tab.label_drag.is_some()
            || self.tab.hovered_constraint.is_some()
            || self.tab.armed.is_some()
        {
            return None;
        }
        let tol = Self::snap_tolerance(ctx, &feature.plane);
        let snap = snap::resolve(&feature.sketch, self.tab.cursor?, tol, &Default::default());
        snap.kind.is_some().then_some(snap)
    }

    /// The edited sketch as [`Self::read_active_sketch`] reads it, read
    /// once for every hook of a frame and every read while nothing it was
    /// read from has changed.
    pub(crate) fn frame_sketch(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<std::sync::Arc<SketchFeature>> {
        let feature = self
            .frame
            .active(self.active_key(ctx), || self.read_active_sketch(ctx));
        feature.inspect(snap::share)
    }

    /// What [`Self::frame_sketch`] reads the sketch under.
    pub(crate) fn active_key(&self, ctx: &WorkbenchRuntimeContext) -> frame_memo::ActiveKey {
        let label = self.tab.label_drag.as_ref().and_then(|drag| {
            drag.current
                .map(|offset| (drag.constraint, [offset.x, offset.y]))
        });
        frame_memo::ActiveKey::of(ctx, self.tab.active_sketch_id, label)
    }

    /// The sketch the view shows: an open card's draft
    /// ([`Self::drafted_sketch`]), else the stored sketch. Worked out once
    /// for every hook of a frame; writes nothing.
    pub(crate) fn frame_displayed(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<std::sync::Arc<SketchFeature>> {
        let key = frame_memo::SketchKey::of(self.tab.stamp(), ctx);
        let feature = self
            .frame
            .displayed(key, || match self.drafted_sketch(ctx) {
                Some(draft) => Some(std::sync::Arc::new(draft)),
                None => self.frame_sketch(ctx),
            });
        feature.inspect(snap::share)
    }

    /// [`relations::Relations::of`] `feature`'s sketch, worked out once
    /// while its shape stands: the panel's list, the overlays and the
    /// glyphs all ask.
    pub(crate) fn frame_relations(
        &self,
        feature: &std::sync::Arc<SketchFeature>,
    ) -> std::sync::Arc<relations::Relations> {
        self.frame.relations(self.frame.content(feature).shape, || {
            relations::Relations::of(&feature.sketch)
        })
    }

    pub(crate) fn draw_key(&self, ctx: &WorkbenchRuntimeContext) -> frame_memo::DrawKey {
        let sketch = frame_memo::SketchKey::of(self.tab.stamp(), ctx);
        frame_memo::DrawKey::of(sketch, self.options, ctx)
    }

    /// The overlays of the frame's displayed sketch: what is drawn
    /// wherever the pointer is, kept while what it reads holds, then what
    /// follows the pointer, built once a frame for its lines, marks and
    /// labels.
    fn frame_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &std::sync::Arc<SketchFeature>,
        proj: &SketchProjector,
        pal: &SketchPalette,
    ) -> FrameOverlays {
        let (base, pointer) = self.frame.overlays(self.draw_key(ctx), || {
            let base = self.base_overlays(ctx, feature, proj, pal);
            let pointer = std::sync::Arc::new(self.build_pointer(ctx, feature, proj, pal));
            (base, pointer)
        });
        FrameOverlays { base, pointer }
    }

    /// What is drawn wherever the pointer is, kept while what it reads
    /// holds.
    fn base_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &std::sync::Arc<SketchFeature>,
        proj: &SketchProjector,
        pal: &SketchPalette,
    ) -> std::sync::Arc<overlay::Overlays> {
        let key = frame_memo::BaseKey {
            shape: self.frame.content(feature).shape,
            view: frame_memo::ViewKey::of(ctx),
            options: self.options,
            palette: *pal,
            selected: (*self.tab.selected).clone(),
            constraints: self.tab.selected_constraints.clone(),
            constraint: self.tab.hovered_constraint,
            hovered: self.tab.hovered,
            idle: self.tab.tool_state.is_idle(),
            drawing: self
                .tab
                .last_tool
                .as_deref()
                .is_some_and(crate::is_draw_tool),
            nodes: self.node_view(feature),
        };
        self.frame.base(key.clone(), || {
            // Selected constraints show the geometry they hold, in their
            // own colour: it is not selected, so tools do not act on it.
            let relations = self.frame_relations(feature);
            let mut referenced: HashSet<Uuid> = HashSet::new();
            for c in &feature.sketch.constraints {
                if key.constraints.contains(&c.id) || key.constraint == Some(c.id) {
                    referenced.extend(relations.held_by(c.id));
                }
            }
            overlay::build_base(
                proj,
                pal,
                &feature.sketch,
                &key.selected,
                &referenced,
                &self.locked_elements(feature),
                key.hovered,
                key.idle,
                key.drawing,
                self.options.construction_on_top,
                &self.options.appearance,
                self.options.arc_helpers,
                self.options.spline_polygon,
                key.nodes.as_ref(),
            )
        })
    }

    /// The constraint glyphs of the frame's displayed sketch and what the
    /// view draws of them, kept while what they read holds.
    fn frame_glyphs(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &std::sync::Arc<SketchFeature>,
        proj: &SketchProjector,
    ) -> std::sync::Arc<frame_memo::GlyphDraw> {
        self.frame
            .frame_glyphs(self.draw_key(ctx), || self.kept_glyphs(ctx, feature, proj))
    }

    fn kept_glyphs(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &std::sync::Arc<SketchFeature>,
        proj: &SketchProjector,
    ) -> std::sync::Arc<frame_memo::GlyphDraw> {
        let key = frame_memo::GlyphKey {
            content: self.frame.content(feature),
            view: frame_memo::ViewKey::of(ctx),
            options: self.options,
            palette: ctx.sketch_palette,
            parked: self.tab.parked_layer,
            lit: self.lit_constraints(|| self.frame_relations(feature)),
            bound: self.bound_dimensions(ctx),
        };
        self.frame.glyphs(key.clone(), || {
            let glyphs = self.build_glyphs(ctx, &feature.sketch, proj, &key.lit, &key.bound);
            let look = &self.options.appearance;
            let size = look.label_px(ctx.pixels_per_point);
            frame_memo::GlyphDraw {
                lines: glyphs::dimension_overlays(&glyphs, look),
                marks: glyphs.iter().filter_map(|g| g.mark(size)).collect(),
                labels: glyphs
                    .iter()
                    .filter_map(|g| g.label(look, ctx.pixels_per_point))
                    .collect(),
            }
        })
    }

    /// What follows the pointer in one frame: the tool's preview, the
    /// snap's marker, the selection box.
    fn build_pointer(
        &self,
        ctx: &WorkbenchRuntimeContext,
        feature: &SketchFeature,
        proj: &SketchProjector,
        pal: &SketchPalette,
    ) -> overlay::Overlays {
        let snap_tol = SNAP_TOLERANCE_PX * proj.units_per_px();
        // Where a click would land, as the click itself works it out.
        let landed = self.tab.cursor.map(|c| self.click_landing(ctx, feature, c));
        let snap = landed
            .and_then(|(_, snap)| snap)
            .or_else(|| self.hover_snap(ctx, feature))
            .or_else(|| self.armed_snap(ctx, feature));
        // Typed values place the preview where they will place the click.
        let typed = self.tab.dim_capture.typed(ctx.document);
        let preview_cursor = landed.map(|(cursor, _)| {
            if typed.is_empty() {
                cursor
            } else {
                let cursor =
                    self.tab
                        .dim_capture
                        .steer(&self.tab.tool_state, &feature.sketch, cursor);
                ovp::override_cursor(&self.tab.tool_state, &feature.sketch, cursor, &typed)
            }
        });
        let params = ovp::typed_params(&self.tab.tool_params, &self.tab.tool_state, &typed);
        overlay::build_pointer(
            proj,
            pal,
            &feature.sketch,
            &self.tab.selected,
            &self.tab.tool_state,
            preview_cursor,
            &ToolParams {
                // The copy tool makes one copy at least.
                copies: self.effective_copies(&params),
                ..params
            },
            self.tab.box_select.as_ref().map(|b| (b.anchor, b.current)),
            self.tab.last_tool.as_deref(),
            snap_tol,
            snap,
            self.options.hint_lines,
            self.tool_proposal(ctx, feature).as_ref(),
        )
    }
}

/// A frame's overlays: what is drawn wherever the pointer is, then what
/// follows the pointer over it.
struct FrameOverlays {
    base: std::sync::Arc<overlay::Overlays>,
    pointer: std::sync::Arc<overlay::Overlays>,
}

impl FrameOverlays {
    fn lines(&self) -> impl Iterator<Item = &core_document::ScreenSpaceOverlay> {
        self.base.lines.iter().chain(&self.pointer.lines)
    }

    fn marks(&self) -> impl Iterator<Item = &ScreenSpaceMark> {
        self.base.marks.iter().chain(&self.pointer.marks)
    }

    fn labels(&self) -> impl Iterator<Item = &ScreenSpaceLabel> {
        self.base.labels.iter().chain(&self.pointer.labels)
    }
}
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use core_document::{Document, Workbench, WorkbenchInputEvent};
    use glam::{Mat4, Vec3};

    use super::*;
    use crate::placement::stored_sketch;
    use crate::sketch::{ConstraintKind, Line, Point, SketchPlane};

    type Drawn = (Arc<overlay::Overlays>, Arc<frame_memo::GlyphDraw>);

    /// What one frame draws wherever the pointer is, and its glyphs,
    /// after a pointer move to `x` with the view `distance` off the sketch.
    fn frame(
        doc: &mut Document,
        wb: &mut SketchWorkbench,
        id: FeatureId,
        x: f32,
        distance: f32,
    ) -> Drawn {
        let cam = [2.5, 1.0, distance];
        let proj = glam::camera::rh::proj::directx::perspective(1.0, 800.0 / 600.0, 0.1, 1e4);
        let view = glam::camera::rh::view::look_at_mat4(
            Vec3::from(cam),
            Vec3::new(2.5, 1.0, 0.0),
            Vec3::Y,
        );
        let mut ctx = WorkbenchRuntimeContext::new(doc, cam, [2.5, 1.0, 0.0], (0, 0, 800, 600));
        ctx.view_proj =
            Some((Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)) * proj * view).to_cols_array_2d());
        ctx.active_document_object = Some(id);
        let viewport_pos = (x, 300.0);
        let event = WorkbenchInputEvent::MouseMove { viewport_pos };
        wb.on_input(&event, Some("sketch.line"), &mut ctx);
        wb.on_frame(0.016, &mut ctx);
        let feature = wb.frame_displayed(&ctx).expect("the edited sketch");
        let proj = SketchProjector::new(&ctx, feature.plane);
        let drawn = wb.frame_overlays(&ctx, &feature, &proj, &ctx.sketch_palette);
        (drawn.base, wb.frame_glyphs(&ctx, &feature, &proj))
    }

    /// The elements and the glyphs are kept while the pointer moves and
    /// worked out again when what they read changes: the selection the
    /// elements alone, the view both, a label's place the glyphs alone,
    /// an edit both.
    #[test]
    fn the_drawing_is_kept_until_what_it_reads_changes() {
        let mut sketch = Sketch::new("s");
        let [a, b] = [0.0, 5.0]
            .map(|x| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 1.0)))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        let mut doc = Document::new("t");
        let feature = SketchFeature::new(sketch, SketchPlane::xy());
        let id = doc.add_feature(feature, "s".into()).unwrap();
        let mut wb = SketchWorkbench::default();
        let mut x = 400.0;
        let mut next = |doc: &mut Document, wb: &mut SketchWorkbench, distance: f32| {
            x += 7.0;
            frame(doc, wb, id, x, distance)
        };
        let kept = |a: &Drawn, b: &Drawn| (Arc::ptr_eq(&a.0, &b.0), Arc::ptr_eq(&a.1, &b.1));
        let first = next(&mut doc, &mut wb, 20.0);
        let moved = next(&mut doc, &mut wb, 20.0);
        assert_eq!(kept(&first, &moved), (true, true), "a pointer move");
        wb.tab.selected.insert(line);
        let selected = next(&mut doc, &mut wb, 20.0);
        assert_eq!(kept(&moved, &selected), (false, true), "a selection");
        let turned = next(&mut doc, &mut wb, 30.0);
        assert_eq!(kept(&selected, &turned), (false, false), "the view");
        let mut held = stored_sketch(&doc, id).unwrap();
        held.sketch.constraints[0].label_offset = Some(Vec2D::new(0.0, 3.0));
        doc.update_feature_data(id, held.to_json()).unwrap();
        let labelled = next(&mut doc, &mut wb, 30.0);
        assert_eq!(kept(&turned, &labelled), (true, false), "a label's place");
        let mut edited = held;
        let free = Point::new(Vec2D::new(2.0, 3.0));
        edited.sketch.add_geometry(GeometryElement::Point(free));
        doc.update_feature_data(id, edited.to_json()).unwrap();
        let edited = next(&mut doc, &mut wb, 30.0);
        assert_eq!(kept(&labelled, &edited), (false, false), "an edit");
        let again = next(&mut doc, &mut wb, 30.0);
        assert_eq!(kept(&edited, &again), (true, true), "nothing changed");
    }
}
