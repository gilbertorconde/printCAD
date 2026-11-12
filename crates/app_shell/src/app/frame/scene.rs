//! The scene's submissions, built in parts for `build_scene_submission`.

use super::*;

impl PrintCadApp {
    /// What the visible features draw, and their profiles' regions.
    pub(super) fn sketch_submissions(
        &mut self,
        editing_feature: Option<core_document::FeatureId>,
    ) -> (Vec<BodySubmission>, Vec<BodySubmission>) {
        let mut region_meshes: Vec<BodySubmission> = Vec::new();
        let mut regions_seen: Vec<(core_document::FeatureId, usize)> = Vec::new();
        let picked_parts = self.session.selected_parts();
        let hovered_curve = self
            .session
            .hover
            .part
            .clone()
            .filter(|(_, part)| matches!(part, core_document::FeaturePart::Edge(_)));
        let sketch_meshes: Vec<BodySubmission> = self
            .registry
            .passive_geometries(&self.session.document, editing_feature)
            .into_iter()
            .map(|(feature_id, geometry)| {
                // Match the in-edit overlay palette: its geometry colour, the
                // amber hover, and its selected colour, the selection's
                // blue. Color is baked directly so the tint is unmistakable
                // even on hairline geometry.
                let is_active = self.session.active_document_object == Some(feature_id)
                    || self.session.selection.tree
                        == Some(crate::ui::TreeItemId::Feature(feature_id));
                // Parts of it picked on their own leave the rest plain.
                let parts: &[core_document::FeaturePart] =
                    if self.session.active_document_object == Some(feature_id) {
                        &picked_parts
                    } else {
                        &[]
                    };
                let is_selected = is_active && parts.is_empty();
                // A curve under the cursor lights on its own, not its whole
                // sketch.
                let curve_lit = hovered_curve
                    .as_ref()
                    .is_some_and(|(f, _)| *f == feature_id);
                let is_hovered = (self.session.hover.feature == Some(feature_id) && !curve_lit)
                    || self.session.hover.tree == Some(crate::ui::TreeItemId::Feature(feature_id));
                let palette = core_document::SketchPalette::default();
                let plain = match geometry.tint {
                    core_document::PassiveTint::Plain => palette.geometry,
                    core_document::PassiveTint::External => palette.external,
                };
                let color = if is_selected {
                    palette.selected
                } else if is_hovered {
                    HOVER_PAINT
                } else {
                    plain
                };
                // The tint participates in the cache revision so hover /
                // selection transitions actually re-upload the color.
                let state_bits = (is_selected as u64) | ((is_hovered as u64) << 1);
                // What it encloses, region by region, shaded see-through
                // behind its lines and meshed once per revision; a region
                // picked or under the cursor lit in its paint.
                let strongest = geometry
                    .regions
                    .iter()
                    .map(|r| r.opacity)
                    .fold(0.0_f32, f32::max);
                for (index, region) in geometry.regions.iter().enumerate() {
                    let key = (feature_id, index);
                    regions_seen.push(key);
                    let fresh = self
                        .scene_meshes
                        .regions
                        .get(&key)
                        .is_some_and(|(revision, _)| *revision == geometry.revision);
                    if !fresh {
                        use kernel_api::KernelQueries;
                        let mesh = kernel_ogeom::QUERIES
                            .profile_mesh(&region.profile)
                            .unwrap_or_default();
                        self.scene_meshes
                            .regions
                            .insert(key, (geometry.revision, Arc::new(mesh)));
                    }
                    let Some((_, mesh)) = self.scene_meshes.regions.get(&key) else {
                        continue;
                    };
                    if mesh.indices.is_empty() {
                        continue;
                    }
                    let part = region.part.as_ref();
                    let picked = part.is_some_and(|p| parts.contains(p));
                    let under_cursor = part.is_some_and(|p| {
                        self.session
                            .hover
                            .part
                            .as_ref()
                            .is_some_and(|(f, h)| *f == feature_id && h == p)
                    });
                    let (region_color, lit, state) = if picked {
                        (palette.selected, true, 3)
                    } else if under_cursor {
                        (HOVER_PAINT, true, 4)
                    } else {
                        (color, false, state_bits)
                    };
                    region_meshes.push(BodySubmission {
                        scalar: None,
                        id: Uuid::from_u128(
                            feature_id.0.as_u128() ^ REGION_ID_SALT ^ ((index as u128) << 64),
                        ),
                        revision: geometry.revision ^ (state << 61),
                        mesh: Arc::clone(mesh),
                        color: region_color,
                        highlight: HighlightState::None,
                        is_wireframe: false,
                        opacity: if lit {
                            (strongest * LIT_REGION_GAIN).min(1.0)
                        } else {
                            region.opacity
                        },
                        pickable: false,
                        on_top: false,
                        edge_color: None,
                        front_only: false,
                        vertex_colors: false,
                        instance: None,
                        section: Default::default(),
                    });
                }
                BodySubmission {
                    scalar: None,
                    id: feature_id.0,
                    revision: geometry.revision ^ (state_bits << 62),
                    mesh: Arc::new(geometry.mesh),
                    color,
                    highlight: HighlightState::None,
                    is_wireframe: false,
                    opacity: 1.0,
                    pickable: true,
                    on_top: false,
                    edge_color: None,
                    front_only: false,
                    vertex_colors: false,
                    instance: None,
                    section: Default::default(),
                }
            })
            .collect();
        // The curves picked on their own, over the feature's lines in the
        // selection paint, and the curve under the cursor in the hover
        // paint.
        let mut sketch_meshes = sketch_meshes;
        let part_lines = |feature_id: core_document::FeatureId,
                          part: &core_document::FeaturePart,
                          id: Uuid,
                          color: [f32; 3]| {
            let lines = self
                .registry
                .part_lines(&self.session.document, feature_id, part)?;
            let revision = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for p in &lines.positions {
                    p.map(f32::to_bits).hash(&mut h);
                }
                h.finish()
            };
            Some(BodySubmission {
                scalar: None,
                id,
                revision,
                mesh: Arc::new(lines),
                color,
                highlight: HighlightState::None,
                is_wireframe: false,
                opacity: 1.0,
                pickable: false,
                on_top: false,
                edge_color: None,
                front_only: false,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            })
        };
        let palette = core_document::SketchPalette::default();
        if let Some(feature_id) = self.session.active_document_object {
            for (index, part) in picked_parts.iter().enumerate() {
                let id = Uuid::from_u128(
                    feature_id.0.as_u128() ^ PART_LINES_ID_SALT ^ ((index as u128) << 64),
                );
                sketch_meshes.extend(part_lines(feature_id, part, id, palette.selected));
            }
        }
        if let Some((feature_id, part)) = &hovered_curve {
            let picked = self.session.active_document_object == Some(*feature_id)
                && picked_parts.contains(part);
            if !picked {
                let id = Uuid::from_u128(feature_id.0.as_u128() ^ HOVERED_LINES_ID_SALT);
                sketch_meshes.extend(part_lines(*feature_id, part, id, HOVER_PAINT));
            }
        }

        self.scene_meshes
            .regions
            .retain(|id, _| regions_seen.contains(id));
        (sketch_meshes, region_meshes)
    }

    /// Every visible body as drawn: its look, its shared or textured mesh
    /// and its section; the print layout's copies while that task is open.
    pub(super) fn body_submissions(&mut self, wireframe: bool) -> Vec<BodySubmission> {
        // Imported geometry (e.g. STEP files) becomes regular renderable bodies.
        // The body id from the document is reused so picking/selection stays
        // stable, and the document's revision counter is forwarded to the
        // renderer so panning/orbiting never re-uploads the static mesh.
        let scene_clip = self.scene_clip();
        let section = self.session.view.section.clone();
        let mut imported_meshes: Vec<BodySubmission> = self
            .session
            .document
            .imported_geometries()
            .filter(|(body_id, _)| {
                self.session
                    .document
                    .imported_body_effective_visible(**body_id)
            })
            .map(|(body_id, geometry)| {
                // Selection is painted by the overlay below, never by a
                // tint; hover brightens, and a peer's selection tints
                // subordinate to it, so your own interaction always wins.
                let is_hovered = self.session.hover.body == Some(body_id.0);
                let is_peer_selected = self
                    .session
                    .connection
                    .peer_presence
                    .values()
                    .any(|p| p.selected_body == Some(body_id.0));
                let highlight = if is_hovered {
                    HighlightState::Hovered
                } else if is_peer_selected {
                    HighlightState::PeerSelected
                } else {
                    HighlightState::None
                };
                // A look the user chose wins over the material the body
                // came with; a chosen colour also drops the per-vertex
                // colours, which would tint it.
                let entry = self.session.document.body(*body_id);
                let chosen = entry.and_then(|b| b.display);
                let pickable = !entry.is_some_and(|b| b.unselectable);
                let face_colors = entry.map(|b| b.face_colors.as_slice()).unwrap_or(&[]);
                let use_vertex_albedo = chosen.is_none()
                    && geometry.mesh.colors.len() == geometry.mesh.positions.len()
                    && !geometry.mesh.colors.is_empty();
                let base_color = match chosen {
                    Some(display) => display.color,
                    None if use_vertex_albedo => [1.0, 1.0, 1.0],
                    None => core_document::BodyDisplay::default().color,
                };
                let opacity = chosen.map(|d| d.opacity.clamp(0.05, 1.0)).unwrap_or(1.0);
                // A textured body draws its textured mesh, the last one
                // made while a newer is being made. It is in the body's
                // own frame, drawn moved where the body sits.
                let pressed = self
                    .session
                    .drawn
                    .textured
                    .get(body_id)
                    .and_then(|p| p.made.as_ref());
                let textured = pressed.is_some();
                let (shown, shown_revision) = match pressed {
                    Some((key, mesh)) => (Arc::clone(mesh), *key),
                    None => (Arc::clone(&geometry.mesh), geometry.revision),
                };
                // A mesh body shaded otherwise than as meshed draws its
                // triangles reshaded, made once per change.
                let shading = entry.map(|b| b.shading).unwrap_or_default();
                let (shown, shown_revision) = if textured
                    || shading == core_document::Shading::AsMeshed
                    || !self.session.document.is_mesh_body(*body_id)
                {
                    (shown, shown_revision)
                } else {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    shown_revision.hash(&mut hasher);
                    format!("{shading:?}").hash(&mut hasher);
                    let key = hasher.finish();
                    let mesh = match self.session.drawn.shaded.get(body_id) {
                        Some((k, mesh)) if *k == key => Arc::clone(mesh),
                        _ => {
                            let crease = match shading {
                                core_document::Shading::Smooth { crease_deg } => Some(crease_deg),
                                _ => None,
                            };
                            let mesh = Arc::new(crate::app::shading::reshade(&shown, crease));
                            self.session
                                .drawn
                                .shaded
                                .insert(*body_id, (key, Arc::clone(&mesh)));
                            mesh
                        }
                    };
                    (mesh, key)
                };
                // Faces coloured on their own: the mesh with those colours
                // in it, the rest in the body's, made once per change.
                let (mesh, revision, color) = if face_colors.is_empty() {
                    (shown, shown_revision, base_color)
                } else {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    shown_revision.hash(&mut hasher);
                    for c in face_colors {
                        (c.face.name, c.face.index, c.color.map(f32::to_bits)).hash(&mut hasher);
                    }
                    chosen.map(|d| d.color.map(f32::to_bits)).hash(&mut hasher);
                    let key = hasher.finish();
                    let cached = self
                        .session
                        .drawn
                        .face_colored
                        .get(body_id)
                        .filter(|(k, _)| *k == key)
                        .map(|(_, m)| Arc::clone(m));
                    let mesh = cached.unwrap_or_else(|| {
                        // A body's own colours stay where no chosen colour
                        // covers them.
                        let base = if chosen.is_some() {
                            let mut plain = (*shown).clone();
                            plain.colors.clear();
                            core_document::mesh_with_face_colors(&plain, face_colors, base_color)
                        } else {
                            core_document::mesh_with_face_colors(&shown, face_colors, base_color)
                        };
                        let mesh = Arc::new(base);
                        self.session
                            .drawn
                            .face_colored
                            .insert(*body_id, (key, Arc::clone(&mesh)));
                        mesh
                    });
                    (mesh, key, [1.0, 1.0, 1.0])
                };
                // A linked copy drawn as its source's mesh is (no face
                // coloured, no texture, not mirrored) draws the mesh it
                // shares with the source, moved where it sits: the GPU
                // holds that mesh once for every copy. A textured body
                // draws its pressed mesh alike, which a source and its
                // copies share when their textures are the same.
                let shared = if textured {
                    Some(Arc::clone(&mesh))
                } else if face_colors.is_empty() {
                    shared_mesh(&self.session.document, *body_id)
                } else {
                    None
                };
                let (mesh, revision, instance) = match shared {
                    Some(shared) => {
                        let ptr = Arc::as_ptr(&shared) as usize;
                        let key = self
                            .session
                            .drawn
                            .shared_meshes
                            .entry(ptr)
                            .or_insert_with(|| (Arc::downgrade(&shared), Uuid::new_v4()))
                            .1;
                        let transform = self.session.document.body_placement(*body_id).matrix();
                        (
                            shared,
                            0,
                            Some(render_wgpu::MeshInstance { key, transform }),
                        )
                    }
                    None => (mesh, revision, None),
                };
                // Under the clipping plane, an open mesh shows its inside
                // as the shell it is; a closed one is capped. A body the
                // toolbar's section leaves whole is neither.
                let uncut = scene_clip.is_some()
                    && crate::camera::section::clip_for(scene_clip, section.as_ref(), *body_id)
                        .is_none();
                let open = !uncut && {
                    let closed = match self.session.drawn.closed.get(body_id) {
                        Some((at, closed)) if *at == geometry.revision => *closed,
                        _ => {
                            let closed = crate::app::scans::closes(&geometry.mesh);
                            self.session
                                .drawn
                                .closed
                                .insert(*body_id, (geometry.revision, closed));
                            closed
                        }
                    };
                    !closed
                };
                BodySubmission {
                    scalar: None,
                    id: body_id.0,
                    revision,
                    mesh,
                    color,
                    opacity,
                    highlight,
                    is_wireframe: wireframe,
                    pickable,
                    on_top: false,
                    edge_color: None,
                    front_only: false,
                    vertex_colors: false,
                    instance,
                    section: render_wgpu::SectionLook {
                        open,
                        hatch: None,
                        uncut,
                    },
                }
            })
            .collect();
        // Where several closed bodies are cut, each cap hatches at its own
        // angle, so nested parts read apart.
        if self.frame_submission.clip_plane.is_some() {
            let capped: Vec<usize> = (0..imported_meshes.len())
                .filter(|i| {
                    let look = imported_meshes[*i].section;
                    !look.open && !look.uncut
                })
                .collect();
            if capped.len() > 1 {
                const ANGLES: [f32; 4] = [0.785, -0.785, 0.262, -1.309];
                for (n, i) in capped.into_iter().enumerate() {
                    imported_meshes[i].section.hatch = Some(ANGLES[n % ANGLES.len()]);
                }
            }
        }
        // A shared mesh no copy draws any more is let go.
        self.session
            .drawn
            .shared_meshes
            .retain(|_, (mesh, _)| mesh.strong_count() > 0);
        // What a pick's triangle numbers: each body drawn with the
        // document's mesh, or a copy of it in its order with faces
        // coloured; a textured body's triangles are split.
        self.session.drawn.meshes.clear();
        for submitted in &imported_meshes {
            let body = core_document::BodyId(submitted.id);
            let textured = self
                .session
                .drawn
                .textured
                .get(&body)
                .is_some_and(|p| p.made.is_some());
            if let Some(geometry) = self.session.document.imported_geometry(body)
                && !textured
            {
                self.session
                    .drawn
                    .meshes
                    .insert(submitted.id, (submitted.revision, geometry.revision));
            }
        }
        // While the Print layout task is open the scene is the layout: the
        // copies on the bed, drawn but not picked, the bodies left where
        // they are.
        if let Some(shown) = &self.session.tools.print_layout {
            let document = &self.session.document;
            imported_meshes = shown
                .layout
                .pieces
                .iter()
                .zip(&shown.preview)
                .map(|(piece, (id, mesh))| BodySubmission {
                    scalar: None,
                    id: *id,
                    revision: shown.key,
                    mesh: Arc::clone(mesh),
                    color: document
                        .body(piece.body)
                        .and_then(|b| b.display)
                        .unwrap_or_default()
                        .color,
                    opacity: 1.0,
                    highlight: HighlightState::None,
                    is_wireframe: wireframe,
                    pickable: false,
                    on_top: false,
                    edge_color: None,
                    front_only: false,
                    vertex_colors: false,
                    instance: None,
                    section: Default::default(),
                })
                .collect();
        }
        imported_meshes
    }

    /// The handles and readouts drawn over the scene: the selection box,
    /// the clipping plane's and the array's handles, the measurement and
    /// peers' cursors.
    pub(super) fn overlay_handles(&mut self, data: &mut ViewportData) {
        let screen_space_labels = &mut data.labels;

        // The selection box being dragged, in the colour the sketcher's
        // box draws in.
        if let Some(drag) = self.session.gestures.boxed() {
            data.overlays
                .extend(drag.outline(core_document::SketchPalette::default().preview));
        }
        data.overlays
            .extend(self.mark_outline(core_document::SketchPalette::default().preview));

        // The clipping plane's handle, an arrow along its normal.
        if let Some(handle) = self.section_handle() {
            let hot = self.section_handle_hot(&handle);
            let (lines, marks, label) = handle.drawn(hot, self.session.document.display_unit());
            data.overlays.extend(lines);
            data.marks.extend(marks);
            screen_space_labels.push(label);
        }

        // The array task's spacing handle on its row's first copy.
        if let Some(handle) = self.array_handle() {
            let hot = self.array_handle_hot(&handle);
            let (lines, marks, label) = handle.drawn(hot, self.session.document.display_unit());
            data.overlays.extend(lines);
            data.marks.extend(marks);
            screen_space_labels.push(label);
        }

        // The measurement in progress: its picks pinned and numbered, the
        // path joining them and the readout of the last two, drawn over the
        // scene; a pick on a body the split view's side pane shows is
        // pinned there.
        if let Some(picks) = &self.session.tools.measure {
            let unit = self.measure_unit();
            let color = HOVER_PAINT;
            let vp = self.session.view.camera.viewport_info();
            let vp = (vp.0 as u32, vp.1 as u32, vp.2, vp.3);
            let side = self.side_pane_view(vp);
            let px: Vec<Option<(f32, f32)>> = picks
                .iter()
                .map(|p| match (&side, p.body()) {
                    (Some(side), Some(body)) if side.bodies.contains(&body.0) => {
                        core_document::runtime::world_to_viewport(side.view_proj, vp, p.point())
                            .map(|(x, y)| (x + side.offset.0, y + side.offset.1))
                    }
                    _ => self
                        .session
                        .view
                        .camera
                        .world_to_viewport(Vec3::from_array(p.point())),
                })
                .collect();
            let [r, g, b, _] = ui_kit::tokens::WARNING.to_normalized_gamma_f32();
            let (marks, labels) = core_document::numbered_pins(
                px.iter()
                    .enumerate()
                    .filter_map(|(i, at)| at.map(|(x, y)| ([x, y], i + 1))),
                [r, g, b],
                self.pixels_per_point(),
            );
            data.marks.extend(marks);
            screen_space_labels.extend(labels);
            let mut lines = crate::app::measure::readout(picks, unit);
            // The kernel's least distance between the last two picks, with
            // the closest points joined.
            if let Some(gap) = &self.session.tools.measure_gap {
                match (&gap.found, gap.in_world()) {
                    (_, Some((distance, a, b))) => {
                        lines.insert(
                            0,
                            format!(
                                "Least distance {}",
                                crate::app::measure::length(distance, unit)
                            ),
                        );
                        let camera = &self.session.view.camera;
                        if let (Some(a), Some(b)) =
                            (camera.world_to_viewport(a), camera.world_to_viewport(b))
                        {
                            let mut line = core_document::ScreenSpaceOverlay::new(
                                [a.0, a.1],
                                [b.0, b.1],
                                color,
                                2.0,
                            );
                            line.dash = Some((5.0, 3.0));
                            data.overlays.push(line);
                        }
                    }
                    (None, _) => lines.insert(0, "Least distance …".to_string()),
                    (Some(Err(why)), _) => {
                        lines.insert(0, format!("Least distance unknown: {why}"));
                    }
                    _ => {}
                }
            }
            for pair in px.windows(2) {
                if let [Some(pa), Some(pb)] = pair {
                    data.overlays.push(core_document::ScreenSpaceOverlay::new(
                        [pa.0, pa.1],
                        [pb.0, pb.1],
                        color,
                        1.5,
                    ));
                }
            }
            // The readout between the last two picks, one line a row.
            let anchor = match px.as_slice() {
                [.., Some(pa), Some(pb)] => Some(((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0)),
                [.., last] => *last,
                [] => None,
            };
            if let Some(anchor) = anchor {
                for (i, line) in lines.into_iter().enumerate() {
                    screen_space_labels.push(
                        core_document::ScreenSpaceLabel::new(
                            [anchor.0 + 14.0, anchor.1 - 12.0 + 20.0 * i as f32],
                            line,
                            color,
                            12.0,
                        )
                        .pill(),
                    );
                }
            }
            if let Some((x, y)) = self.input.cursor_in_viewport {
                let prompt = match picks.len() {
                    0 => "Measure: pick a point, an edge, a face or a datum",
                    1 => "Measure: pick a second one to measure between",
                    _ => "Measure: pick another to go on, or empty space to start over",
                };
                screen_space_labels.push(
                    core_document::ScreenSpaceLabel::new(
                        [x + 14.0, y + 14.0],
                        prompt.to_string(),
                        color,
                        11.0,
                    )
                    .pill(),
                );
            }
        }

        // Peers' cursors: a named marker where each other editor points.
        // Same projection the overlays use; a cursor behind the camera or
        // outside the model simply has no marker this frame.
        for state in self.session.connection.peer_presence.values() {
            let Some(world) = state.cursor_world else {
                continue;
            };
            let Some((x, y)) = self
                .session
                .view
                .camera
                .world_to_viewport(Vec3::new(world[0], world[1], world[2]))
            else {
                continue;
            };
            screen_space_labels.push(
                core_document::ScreenSpaceLabel::new(
                    [x, y - 14.0],
                    format!("⯆ {}", state.display_name),
                    [0.35, 0.75, 0.95],
                    12.0,
                )
                .pill(),
            );
        }
    }

    /// The selection and hover paints, each recorded in `overlay_bodies`.
    pub(super) fn selection_paints(&mut self, all_meshes: &mut Vec<BodySubmission>) {
        // The selection overlay, in the selection paint at the chosen
        // opacity: a selected face is its own triangles, slightly lifted off
        // the surface; a selected body is the whole body's mesh again,
        // drawn over itself.
        let paint = self.user_settings.rendering.selection_color;
        let opacity = self
            .user_settings
            .rendering
            .selection_opacity
            .min(settings::MAX_SELECTION_OPACITY);
        // Each paint's submission by the body it is drawn for, so it draws
        // in that body's pane of a split view.
        let mut overlay_bodies: Vec<(Uuid, Uuid)> = Vec::new();
        // The faces Ctrl added before the last, each in a slot of its own.
        for (i, picked) in self.session.selection.earlier_faces.iter().enumerate() {
            let face = &picked.highlight;
            let (hi, lo) = face.body.as_u64_pair();
            let id = Uuid::from_u128(self.overlay_ids.face_highlight.as_u128() ^ (i as u128 + 1));
            overlay_bodies.push((id, face.body));
            all_meshes.push(BodySubmission {
                scalar: None,
                id,
                revision: face.revision ^ hi ^ lo ^ (u64::from(face.face.unwrap_or(0)) << 32),
                mesh: Arc::clone(&face.mesh),
                color: paint,
                opacity,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
                edge_color: None,
                front_only: false,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            });
        }
        if let Some(face) = &self.session.selection.face_highlight {
            overlay_bodies.push((self.overlay_ids.face_highlight, face.body));
            all_meshes.push(BodySubmission {
                scalar: None,
                id: self.overlay_ids.face_highlight,
                revision: face.revision,
                mesh: Arc::clone(&face.mesh),
                color: paint,
                opacity,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
                edge_color: None,
                front_only: false,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            });
        } else if let Some(geometry) = self
            .session
            .selection
            .whole_body()
            // A body showing a feature's preview is not painted over: the
            // preview is what it shows.
            .filter(|id| {
                !self
                    .session
                    .builds
                    .previews
                    .contains_key(&core_document::BodyId(*id))
            })
            .and_then(|id| {
                self.session
                    .document
                    .imported_geometry(core_document::BodyId(id))
            })
        {
            // One slot serves every body; the body's id in the revision
            // keeps two bodies at the same revision from sharing buffers.
            let selected = self.session.selection.body.unwrap_or_default();
            overlay_bodies.push((self.overlay_ids.body_highlight, selected));
            let (hi, lo) = selected.as_u64_pair();
            all_meshes.push(BodySubmission {
                scalar: None,
                id: self.overlay_ids.body_highlight,
                revision: geometry.revision ^ hi ^ lo,
                mesh: Arc::clone(&geometry.mesh),
                color: paint,
                opacity,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
                edge_color: None,
                front_only: false,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            });
        }

        // The bodies a box took beside the last, each in a slot of its own.
        for (i, body) in self.session.selection.earlier_bodies.iter().enumerate() {
            let id = core_document::BodyId(*body);
            if self.session.builds.previews.contains_key(&id) {
                continue;
            }
            let Some(geometry) = self.session.document.imported_geometry(id) else {
                continue;
            };
            let (hi, lo) = body.as_u64_pair();
            let slot = Uuid::from_u128(self.overlay_ids.body_highlight.as_u128() ^ (i as u128 + 1));
            overlay_bodies.push((slot, *body));
            all_meshes.push(BodySubmission {
                scalar: None,
                id: slot,
                revision: geometry.revision ^ hi ^ lo,
                mesh: Arc::clone(&geometry.mesh),
                color: paint,
                opacity,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
                edge_color: None,
                front_only: false,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            });
        }

        // The body a tree row under the pointer stands for, translucent in
        // the hover paint, unless it is selected already.
        if let Some(body) = self.tree_hovered_body()
            && self.session.selection.body != Some(body.0)
            && !self.session.builds.previews.contains_key(&body)
            && let Some(geometry) = self.session.document.imported_geometry(body)
        {
            let (hi, lo) = body.0.as_u64_pair();
            overlay_bodies.push((self.overlay_ids.tree_hover, body.0));
            all_meshes.push(BodySubmission {
                scalar: None,
                id: self.overlay_ids.tree_hover,
                revision: geometry.revision ^ hi ^ lo,
                mesh: Arc::clone(&geometry.mesh),
                color: HOVER_PAINT,
                opacity: opacity * 0.5,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
                edge_color: None,
                front_only: false,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            });
        }

        // The hovered face, translucent in the hover paint, unless it is the
        // selected face already.
        if let Some(hover) = &self.session.hover.face
            && self.session.hover.edge.is_none()
            && self.session.view.pick_filter.faces()
            && !self
                .session
                .selection
                .face_highlight
                .iter()
                .chain(
                    self.session
                        .selection
                        .earlier_faces
                        .iter()
                        .map(|p| &p.highlight),
                )
                .any(|f| f.body == hover.body && f.face == Some(hover.face))
        {
            let (hi, lo) = hover.body.as_u64_pair();
            overlay_bodies.push((self.overlay_ids.face_hover, hover.body));
            all_meshes.push(BodySubmission {
                scalar: None,
                id: self.overlay_ids.face_hover,
                revision: hover.revision ^ hi ^ lo ^ (u64::from(hover.face) << 32),
                mesh: Arc::clone(&hover.mesh),
                color: HOVER_PAINT,
                opacity: opacity * 0.5,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
                edge_color: None,
                front_only: false,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            });
        }

        // The hovered edge and the picked edges, drawn as line bodies over
        // the outline in the hover and selection paints.
        if let Some(hit) = &self.session.hover.edge
            && !self
                .session
                .selection
                .edges
                .iter()
                .any(|s| s.body == hit.body && s.edge == hit.edge)
            && let Some(submission) = crate::app::edges::highlight_submission(
                self,
                self.overlay_ids.edge_hover,
                hit.body,
                &[hit.edge],
                HOVER_PAINT,
            )
        {
            overlay_bodies.push((self.overlay_ids.edge_hover, hit.body));
            all_meshes.push(submission);
        }
        let mut by_body: std::collections::BTreeMap<uuid::Uuid, Vec<u32>> = Default::default();
        for hit in &self.session.selection.edges {
            by_body.entry(hit.body).or_default().push(hit.edge);
        }
        for (i, (body, edges)) in by_body.iter().enumerate() {
            // One slot per body: its id folded into the submission id.
            let id = uuid::Uuid::from_u64_pair(
                self.overlay_ids.edge_select.as_u64_pair().0 ^ body.as_u64_pair().0,
                self.overlay_ids
                    .edge_select
                    .as_u64_pair()
                    .1
                    .wrapping_add(i as u64),
            );
            if let Some(submission) =
                crate::app::edges::highlight_submission(self, id, *body, edges, paint)
            {
                overlay_bodies.push((id, *body));
                all_meshes.push(submission);
            }
        }
        // A paint over a body the section leaves whole is left whole too.
        if self.frame_submission.clip_plane.is_some() {
            for (slot, body) in &overlay_bodies {
                if self.clip_of(core_document::BodyId(*body)).is_none()
                    && let Some(paint) = all_meshes.iter_mut().find(|m| m.id == *slot)
                {
                    paint.section.uncut = true;
                }
            }
        }
        self.session.drawn.overlay_bodies = overlay_bodies;
    }
}
