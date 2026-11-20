//! The panel's sections other than the lists: the plane picker, the
//! solver's problems, the pickers, tool settings, the selection's array,
//! spline and text, the reference pictures, and the placement, solver and
//! display folds.

use super::*;

impl SketchWorkbench {
    /// The plane picker of a sketch about to be made: the face clicked,
    /// the body's datum planes (the selected one first), the origin's
    /// planes, and attachment modes on what is selected.
    pub(super) fn attachment_widgets(&mut self, ctx: &WorkbenchRuntimeContext) -> Vec<Widget> {
        let Some(pending) = &self.tab.pending_creation else {
            return Vec::new();
        };
        let body = pending.body;
        let datums = body.map_or_else(Vec::new, |body| {
            core_document::datums_of_body(ctx.document, body)
        });
        let selected_datum = pending
            .datum
            .filter(|id| datums.iter().any(|(d, ..)| d == id));
        let ask = match &pending.generator {
            Some(made_by) => format!("Choose the plane for the {}.", made_by.base_name()),
            None => "Choose the plane to sketch on.".to_string(),
        };
        let mut children = vec![
            panel::caption(
                &format!("{ask} An origin plane can be clicked in the view too."),
                Tone::Neutral,
                false,
            ),
            panel::gap(GapSize::Small),
        ];
        if pending.face.is_some() {
            children.push(panel::hinted(
                "Sketch on the face you clicked on the solid; the sketch follows the face",
                panel::button("attach.face", "Selected face", ButtonStyle::Secondary, true),
            ));
        }
        let tab = &mut *self.tab;
        tab.panel.datum_planes.clear();
        let mut datum_buttons = |only: bool, prefix: &str, out: &mut Vec<Widget>| {
            for (datum_id, name, datum) in &datums {
                if (Some(*datum_id) == selected_datum) != only {
                    continue;
                }
                for (label, hint, choice) in datum_planes(*datum_id, name, datum, prefix) {
                    let id = format!("attach.datum:{}", tab.panel.datum_planes.len());
                    tab.panel.datum_planes.push(choice);
                    out.push(panel::hinted(
                        hint,
                        panel::button(&id, &label, ButtonStyle::Secondary, true),
                    ));
                }
            }
        };
        datum_buttons(true, "Selected: ", &mut children);
        children.push(panel::flow_no_wrap(vec![
            panel::button("attach.base:xy", "Top (XY)", ButtonStyle::Secondary, true),
            panel::button("attach.base:xz", "Front (XZ)", ButtonStyle::Secondary, true),
            panel::button("attach.base:yz", "Side (YZ)", ButtonStyle::Secondary, true),
        ]));
        // Attached as a datum plane would be, by a mode on what is
        // selected on the body.
        tab.panel.modes.clear();
        if let Some(body) = body {
            use core_document::attach::{PICK_MODES, Picked, candidate, mode_label};
            let picked = Picked::of(ctx, body);
            let frame = core_document::DatumFrame {
                origin: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
                x_axis: [1.0, 0.0, 0.0],
            };
            children.push(panel::caption(
                "Or by a mode, on what is selected",
                Tone::Muted,
                true,
            ));
            let mut modes = Vec::new();
            for (i, (mode, needs)) in PICK_MODES.iter().enumerate() {
                let made = candidate(mode, ctx, body, frame, None, &picked);
                let enabled = made.is_some();
                tab.panel.modes.push(made);
                let mode_button = panel::button(
                    &format!("attach.mode:{i}"),
                    mode_label(mode),
                    ButtonStyle::PlainSmall,
                    enabled,
                );
                modes.push(if enabled || needs.is_empty() {
                    mode_button
                } else {
                    panel::hinted(needs, mode_button)
                });
            }
            children.push(panel::flow(modes));
        }
        // Datum planes of the target body attach the sketch to their
        // resolved frame; the selected one was offered first.
        datum_buttons(false, "", &mut children);
        children.push(panel::gap(GapSize::Small));
        children.push(panel::button(
            "attach.cancel",
            "Cancel",
            ButtonStyle::Secondary,
            true,
        ));
        vec![
            Card {
                icon: Some("sketch-map".into()),
                title: "Sketch attachment".into(),
                children,
                ..Default::default()
            }
            .into(),
        ]
    }

    /// What the header's status pill cannot do: a conflict or a
    /// redundancy names its constraints, and a click on it selects their
    /// geometry; with automatic update off, Solve now.
    pub(super) fn problem_widgets(
        &mut self,
        feature: &std::sync::Arc<crate::SketchFeature>,
    ) -> Vec<Widget> {
        // Also keeps the diagnosis the header's pill reads.
        let state = self.diagnosed_verdict(feature);
        let mut widgets = Vec::new();
        if !state.offenders.is_empty() {
            let kind = if state.kind == crate::verdict::SolverKind::Conflicting {
                NoteKind::Error
            } else {
                NoteKind::Warning
            };
            let text = match state.offenders.len() {
                1 => "Click to select the constraint's geometry.".to_string(),
                n => format!("Click to select the {n} constraints' geometry."),
            };
            widgets.push(
                bench_api::Alert {
                    id: Some("problems".into()),
                    kind,
                    title: Some(state.title),
                    text,
                }
                .into(),
            );
        }
        if !self.options.auto_update {
            widgets.push(panel::hinted(
                "Automatic update is off: solve the sketch as it stands",
                panel::button("solve_now", "Solve now", ButtonStyle::Small, true),
            ));
        }
        widgets
    }

    /// The document's other sketches, for carbon copy (a click copies one
    /// in) or merge (tick some, then merge); or, for external geometry,
    /// the other sketches and the datums. A sketch whose plane is at an
    /// angle to this one is listed but not offered.
    pub(super) fn picker_widgets(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        plane: &SketchPlane,
    ) -> Vec<Widget> {
        let Some(mode) = self.tab.sketch_picker.as_ref().map(|p| p.mode) else {
            return Vec::new();
        };
        self.tab.panel.picked_from.clear();
        let mut children = Vec::new();
        if mode == crate::SketchPickerMode::ExternalFrom {
            let mut choices: Vec<(FeatureId, String)> = self
                .other_sketches(ctx)
                .into_iter()
                .map(|(id, name, _)| (id, name))
                .collect();
            let mut datums: Vec<(u64, FeatureId, String)> = ctx
                .document
                .feature_tree()
                .all_nodes()
                .filter(|(_, n)| n.workbench_id.as_str() == core_document::DATUM_KIND)
                .map(|(id, n)| (n.seq, *id, n.name.clone()))
                .collect();
            datums.sort_by_key(|(seq, id, _)| (*seq, *id));
            choices.extend(datums.into_iter().map(|(_, id, name)| (id, name)));
            if choices.is_empty() {
                children.push(panel::note(
                    NoteKind::Info,
                    None,
                    "There is no other sketch and no datum in the document.",
                ));
            }
            for (i, (id, name)) in choices.into_iter().enumerate() {
                self.tab.panel.picked_from.push(id);
                children.push(panel::button(
                    &format!("picker.copy:{i}"),
                    &name,
                    ButtonStyle::Plain,
                    true,
                ));
            }
            children.push(panel::button(
                "picker.close",
                "Close",
                ButtonStyle::Secondary,
                true,
            ));
            children.push(panel::gap(GapSize::Medium));
            return vec![section(
                "sketch_external_from",
                "External from",
                None,
                None,
                true,
                children,
            )];
        }
        let merging = mode == crate::SketchPickerMode::Merge;
        let checked = self
            .tab
            .sketch_picker
            .as_ref()
            .map(|p| p.checked.clone())
            .unwrap_or_default();
        let others = self.other_sketches(ctx);
        if others.is_empty() {
            children.push(panel::note(
                NoteKind::Info,
                None,
                "There is no other sketch in the document.",
            ));
        }
        let why = "Its plane is at an angle to this one";
        for (i, (id, name, other)) in others.iter().enumerate() {
            self.tab.panel.picked_from.push(*id);
            let parallel = crate::plane_map(&other.plane, plane).is_ok();
            children.push(if merging {
                Check {
                    id: format!("picker.tick:{i}"),
                    text: name.clone(),
                    on: checked.contains(id),
                    enabled: parallel,
                    hint: if parallel { String::new() } else { why.into() },
                    ..Default::default()
                }
                .into()
            } else {
                let copy = panel::button(
                    &format!("picker.copy:{i}"),
                    name,
                    ButtonStyle::Plain,
                    parallel,
                );
                if parallel {
                    copy
                } else {
                    panel::hinted(why, copy)
                }
            });
        }
        if merging {
            children.push(panel::hinted(
                "A new sketch of this one and the ticked ones; they stay as they are",
                panel::button(
                    "picker.merge",
                    "Merge into a new sketch",
                    ButtonStyle::Plain,
                    !checked.is_empty(),
                ),
            ));
        }
        children.push(panel::button(
            "picker.close",
            "Close",
            ButtonStyle::Secondary,
            true,
        ));
        children.push(panel::gap(GapSize::Medium));
        let title = if merging {
            "Merge sketches"
        } else {
            "Carbon copy"
        };
        vec![section("sketch_picker", title, None, None, true, children)]
    }

    /// Settings of the active drawing tool, when it has any. What a
    /// click left to set waits here, these fields editing it, with its OK
    /// and Cancel.
    pub(super) fn tool_widgets(&self, ctx: &WorkbenchRuntimeContext) -> Vec<Widget> {
        let tool = self.tab.last_tool.as_deref();
        let p = &self.tab.tool_params;
        let fields: Vec<Widget> = match tool {
            Some("sketch.polygon") => vec![
                count("tool.polygon_sides", "Sides", p.polygon_sides, 3.0, 12.0),
                check(
                    "tool.polygon_flats",
                    "Size",
                    "Across flats",
                    p.polygon_flats,
                    true,
                    "The click is the middle of a side, the size the inscribed circle's",
                ),
            ],
            Some("sketch.text") => {
                let mut fields = text_fields(
                    "text_draft",
                    &self.tab.text_draft,
                    self.tab.panel.font_modes[0],
                );
                // Where the waiting text goes is settled by its click.
                if self.tab.corner_task.is_some() {
                    for field in &mut fields {
                        if let Widget::Toggles(t) = field
                            && t.id == "text_draft.layout"
                        {
                            for b in &mut t.buttons {
                                b.disabled = true;
                            }
                        }
                    }
                }
                fields
            }
            Some("sketch.slot" | "sketch.arc_slot") => {
                vec![mm("tool.slot_width", "Width", p.slot_width)]
            }
            Some("sketch.fillet") => vec![
                mm("tool.fillet_radius", "Radius", p.fillet_radius),
                corner_check(p.corner_keep),
            ],
            Some("sketch.rect_rounded") => {
                vec![mm("tool.fillet_radius", "Radius", p.fillet_radius)]
            }
            Some("sketch.chamfer") => vec![
                mm("tool.chamfer_length", "Length", p.chamfer_length),
                corner_check(p.corner_keep),
            ],
            Some("sketch.rect_frame") => {
                vec![mm("tool.offset_distance", "Wall", p.offset_distance)]
            }
            Some("sketch.offset") => vec![
                mm("tool.offset_distance", "Distance", p.offset_distance),
                check(
                    "tool.offset_round",
                    "Corners",
                    "Round",
                    p.offset_round,
                    true,
                    "Arcs join the copies where they part at a corner",
                ),
                check("tool.offset_both", "Sides", "Both", p.offset_both, true, ""),
                check(
                    "tool.offset_delete",
                    "Original",
                    "Replace it",
                    p.offset_delete,
                    true,
                    "",
                ),
                check(
                    "tool.offset_linked",
                    "Copy",
                    "Follows the original",
                    p.offset_linked,
                    !p.offset_delete,
                    "One offset dimension holds the copy to the original",
                ),
            ],
            Some("sketch.mirror") => vec![
                check(
                    "tool.mirror_keep",
                    "Original",
                    "Keep it",
                    p.mirror_keep,
                    true,
                    "",
                ),
                check(
                    "tool.mirror_linked",
                    "Image",
                    "Follows the original",
                    p.mirror_linked,
                    p.mirror_keep,
                    "Symmetric constraints hold the image to the original",
                ),
                check(
                    "tool.mirror_center",
                    "About",
                    "A point",
                    p.mirror_center,
                    true,
                    "One click, the centre; off, a line or two points",
                ),
            ],
            Some(t @ ("sketch.translate" | "sketch.rotate" | "sketch.scale")) => {
                let mut fields = vec![count("tool.copies", "Copies", p.copies, 0.0, 64.0)];
                if t != "sketch.scale" {
                    let full_turn = t == "sketch.rotate" && p.rotate_full_turn;
                    fields.push(check("tool.copies_total", "Count", "Includes the original",
                        p.copies_total, true,
                        "The count is every element, the original included, and the clicks span the whole pattern"));
                    fields.push(check(
                        "tool.copies_symmetric",
                        "Sides",
                        "Symmetric",
                        p.copies_symmetric,
                        !full_turn,
                        "The copies repeat on the other side of the original",
                    ));
                    if t == "sketch.rotate" {
                        fields.push(check(
                            "tool.rotate_full_turn",
                            "Turn",
                            "Full turn",
                            p.rotate_full_turn,
                            true,
                            "The elements go evenly around 360°: click only the centre",
                        ));
                    }
                    fields.push(check(
                        "tool.copies_linked",
                        "Linked",
                        "Follow the original",
                        p.copies_linked,
                        p.copies > 0,
                        "Copies stay the original's size, spaced by one pitch",
                    ));
                }
                fields
            }
            Some("sketch.bspline") => vec![
                check(
                    "tool.bspline_periodic",
                    "Closed",
                    "Periodic",
                    p.bspline_periodic,
                    true,
                    "",
                ),
                check(
                    "tool.bspline_interpolate",
                    "Clicks",
                    "Through points",
                    p.bspline_interpolate,
                    true,
                    "",
                ),
                Quantity {
                    speed: Some(0.05),
                    ..integer(
                        "tool.bspline_degree",
                        "Degree",
                        p.bspline_degree,
                        2.0,
                        f64::from(crate::spline::MAX_DEGREE),
                    )
                }
                .into(),
            ],
            _ => return Vec::new(),
        };
        vec![section(
            "sketch_tool",
            "Tool",
            None,
            None,
            true,
            [grid("sketch_tool_grid", true, fields)]
                .into_iter()
                .chain(self.pending_widgets(ctx))
                .collect(),
        )]
    }

    /// With something selected: an array of it, its rows, columns and
    /// steps set here before it is made.
    pub(super) fn array_widgets(&self) -> Vec<Widget> {
        if self.tab.selected.is_empty() || !self.tab.tool_state.is_idle() {
            return Vec::new();
        }
        let p = &self.tab.tool_params;
        let fields = vec![
            count("array.rows", "Rows", p.array_rows, 1.0, 64.0),
            count("array.cols", "Columns", p.array_cols, 1.0, 64.0),
            offset("array.dx", "Column step", p.array_dx),
            offset("array.dy", "Row step", p.array_dy),
            check(
                "array.linked",
                "Linked",
                "Follow the original",
                p.copies_linked,
                true,
                "Copies stay the original's size, spaced by one pitch along the rows and one down the columns",
            ),
        ];
        vec![section(
            "sketch_array",
            "Array",
            None,
            None,
            false,
            vec![
                grid("sketch_array_grid", false, fields),
                panel::hinted(
                    "Copy the selection into these rows and columns",
                    panel::button("array.make", "Make array", ButtonStyle::Secondary, true),
                ),
            ],
        )]
    }

    /// With one spline selected: its degree and its knots, each with how
    /// many times it stands; with one of a spline's control points, its
    /// weight.
    pub(super) fn spline_widgets(&mut self, sketch: &Sketch) -> Vec<Widget> {
        let (spline, weighted) = self.selected_spline(sketch);
        if spline.is_none() && weighted.is_none() {
            return Vec::new();
        }
        let mut fields = Vec::new();
        self.tab.panel.knots.clear();
        if let Some((id, degree)) = spline {
            fields.push(stepper(
                "spline.degree",
                "Degree",
                degree as usize,
                &degree.to_string(),
                (
                    degree > 1,
                    "Lower the degree: the nearest curve of one less",
                ),
                (
                    degree < crate::spline::MAX_DEGREE,
                    "Raise the degree, the curve unchanged",
                ),
            ));
            for (i, (knot, times)) in crate::spline_edit::knots_of(sketch, id)
                .into_iter()
                .enumerate()
            {
                self.tab.panel.knots.push((knot, times));
                let down = if times > 1 {
                    "Stand once less: a smoother curve near it"
                } else {
                    "Remove the knot: the nearest curve without it"
                };
                fields.push(stepper(
                    &format!("spline.knot:{i}"),
                    &format!("Knot {knot:.3}"),
                    times,
                    &format!("×{times}"),
                    (true, down),
                    (
                        times < degree as usize,
                        "Stand once more, the curve unchanged",
                    ),
                ));
            }
        }
        if let Some((owner, point)) = weighted {
            let now = crate::spline_edit::weight_of(sketch, owner, point).unwrap_or(1.0);
            fields.push(
                Quantity {
                    id: "spline.weight".into(),
                    label: "Weight".into(),
                    value: now,
                    dim: Dim::Number,
                    min: Some(0.01),
                    max: Some(100.0),
                    decimals: 3,
                    speed: Some(0.01),
                    ..Default::default()
                }
                .into(),
            );
        }
        vec![section(
            "sketch_spline",
            "Spline",
            None,
            None,
            true,
            vec![grid("sketch_spline_grid", false, fields)],
        )]
    }

    /// The one selected spline with its degree, and the selected control
    /// point of a spline with its owner.
    pub(super) fn selected_spline(&self, sketch: &Sketch) -> SelectedSpline {
        let splines: Vec<&sketch::BSpline> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                sketch::GeometryElement::BSpline(b) if self.tab.selected.contains(&b.id) => Some(b),
                _ => None,
            })
            .collect();
        // Each control point with the first spline that weighs on it.
        let mut weighing: std::collections::HashMap<Uuid, Uuid> = Default::default();
        for g in sketch.geometry.iter().rev() {
            if let sketch::GeometryElement::BSpline(b) = g {
                weighing.extend(b.control_points.iter().map(|p| (*p, b.id)));
            }
        }
        let weighted = (!weighing.is_empty())
            .then(|| {
                (self.tab.selected.iter())
                    .find_map(|id| weighing.get(id).map(|spline| (*spline, *id)))
            })
            .flatten();
        if splines.len() != 1 && weighted.is_none() {
            return (None, None);
        }
        (splines.first().map(|b| (b.id, b.degree)), weighted)
    }

    /// With a piece of a text block selected: what it says and how, made
    /// again when a field is changed.
    pub(super) fn text_widgets(&self, sketch: &Sketch) -> Vec<Widget> {
        let Some(block) = self
            .tab
            .selected
            .iter()
            .find_map(|id| crate::text::block_of(sketch, *id))
        else {
            return Vec::new();
        };
        let spec = crate::text::TextSpec::of(block);
        let fields = text_fields("text_block", &spec, self.tab.panel.font_modes[1]);
        vec![section(
            "sketch_text",
            "Text",
            None,
            None,
            true,
            vec![
                grid("sketch_text_grid", false, fields),
                panel::flow_no_wrap(vec![panel::hinted(
                    "Turn the text into plain lines and splines: they can be edited one by \
                     one, and the text can no longer be",
                    panel::button(
                        "text_block.explode",
                        "Explode to curves",
                        ButtonStyle::Secondary,
                        true,
                    ),
                )]),
            ],
        )]
    }

    /// The sketch's reference pictures: each one's middle, width, turn and
    /// how much of it shows, its calibration, and a way to take it away.
    pub(super) fn image_widgets(
        &self,
        ctx: &WorkbenchRuntimeContext,
        sketch: &Sketch,
    ) -> Vec<Widget> {
        if sketch.images.is_empty() {
            return Vec::new();
        }
        let mut children = Vec::new();
        for (n, image) in sketch.images.iter().enumerate() {
            children.push(panel::caption(
                &format!("Picture {}", n + 1),
                Tone::Muted,
                true,
            ));
            let field = |key: &str| format!("image:{n}.{key}");
            children.push(grid(
                &format!("sketch_image_grid:{n}"),
                false,
                vec![
                    length(&field("x"), "Middle x", image.center.x, None),
                    length(&field("y"), "Middle y", image.center.y, None),
                    length(&field("width"), "Width", image.width, Some((0.01, 1.0e6))),
                    degrees(&field("angle"), "Turn", image.angle_deg),
                    Quantity {
                        id: field("opacity"),
                        label: "Opacity".into(),
                        value: image.opacity,
                        dim: Dim::Number,
                        min: Some(0.05),
                        max: Some(1.0),
                        speed: Some(0.01),
                        ..Default::default()
                    }
                    .into(),
                ],
            ));
            match self
                .tab
                .calibrating
                .as_ref()
                .filter(|c| c.image == image.id)
            {
                Some(calibrating) => {
                    children.extend(calibration_widgets(ctx, image, calibrating));
                }
                None => children.push(panel::flow_no_wrap(vec![
                    panel::hinted(
                        "Pick two points on the picture and say how far apart they really are",
                        panel::button(
                            &field("calibrate"),
                            "Calibrate",
                            ButtonStyle::Secondary,
                            true,
                        ),
                    ),
                    panel::button(&field("remove"), "Remove", ButtonStyle::Secondary, true),
                ])),
            }
            children.push(panel::gap(GapSize::Small));
        }
        vec![section(
            "sketch_images",
            "Reference images",
            None,
            None,
            true,
            children,
        )]
    }

    /// Where the sketch sits off what it was placed on (its datum, plane,
    /// face or attachment): along the normal, across the plane and turned
    /// about it.
    pub(super) fn placement_widgets(
        &self,
        placed: crate::feature::PlaneShift,
        on_datum: bool,
    ) -> Vec<Widget> {
        if self.tab.active_sketch_id.is_none() {
            return Vec::new();
        }
        let moved = placed.offset != 0.0 || placed.shift != [0.0; 2] || placed.turn != 0.0;
        let summary = if moved {
            format!(
                "offset {:.2} mm, moved {:.2}, {:.2}, turned {:.1}°",
                placed.offset, placed.shift[0], placed.shift[1], placed.turn
            )
        } else {
            "no offset".to_string()
        };
        let mut children = Vec::new();
        if !on_datum {
            children.push(panel::caption(
                "Moves the sketch off the plane or face it was placed on",
                Tone::Muted,
                true,
            ));
        }
        children.push(grid(
            "sketch_attachment_grid",
            false,
            vec![
                offset("placement.offset", "Along normal", placed.offset),
                offset("placement.x", "Across x", placed.shift[0]),
                offset("placement.y", "Across y", placed.shift[1]),
                degrees("placement.turn", "Turned", placed.turn),
            ],
        ));
        vec![section(
            "sketch_attachment",
            "Placement",
            None,
            Some(summary),
            false,
            children,
        )]
    }

    /// What is set once and seldom changed, folded away at the panel's
    /// end: how the solver runs, and how drawing snaps and shows.
    pub(super) fn settings_widgets(&self, sketch: &Sketch) -> Vec<Widget> {
        let o = &self.options;
        let exponent = -(sketch.solver.tolerance.log10().round() as i32);
        let solver_summary = format!(
            "{} · 1e-{exponent}",
            if o.auto_update { "auto" } else { "manual" }
        );
        let last = match self.tab.last_solve {
            Some(crate::operations::SolveOutcome::Converged { iterations }) => {
                format!("Last solve: settled in {iterations} step(s)")
            }
            Some(crate::operations::SolveOutcome::NotConverged { residual }) => {
                format!("Last solve: stopped with {residual:.2e} left")
            }
            Some(crate::operations::SolveOutcome::NothingToSolve) => {
                "Last solve: nothing to solve".to_string()
            }
            None => String::new(),
        };
        let mut solver = vec![panel::flow_no_wrap(vec![
            check(
                "solver.auto_update",
                "",
                "Auto update",
                o.auto_update,
                true,
                "",
            ),
            check(
                "solver.auto_remove",
                "",
                "Auto remove redundants",
                o.auto_remove_redundant,
                true,
                "",
            ),
        ])];
        if !last.is_empty() {
            solver.push(panel::caption(&last, Tone::Muted, true));
        }
        solver.push(grid(
            "sketch_solver_grid",
            false,
            vec![
                Quantity {
                    speed: Some(1.0),
                    ..integer(
                        "solver.iterations",
                        "Most steps",
                        sketch.solver.max_iterations,
                        1.0,
                        10_000.0,
                    )
                }
                .into(),
                Select {
                    id: "solver.tolerance".into(),
                    label: "Solved within".into(),
                    options: TOLERANCES.map(|e| format!("1e-{e}")).to_vec(),
                    selected: TOLERANCES
                        .iter()
                        .position(|e| *e == exponent)
                        .unwrap_or(usize::MAX),
                    width: Some(90.0),
                    ..Default::default()
                }
                .into(),
            ],
        ));
        let snaps: Vec<&str> = [
            (!self.tab.snap_off, "snap to objects"),
            (o.grid_snap, "snap to grid"),
            (o.grid_on, "grid"),
        ]
        .into_iter()
        .filter_map(|(on, what)| on.then_some(what))
        .collect();
        let display_summary = if snaps.is_empty() {
            "no snapping".to_string()
        } else {
            snaps.join(", ")
        };
        let toggle =
            |id: &str, text: &str, on: bool, hint: &str| check(id, "", text, on, true, hint);
        let display = vec![
            panel::inline(choose(
                "display.labels",
                "Labels show",
                &crate::glyphs::DimensionLabels::ALL,
                o.dimension_labels,
                130.0,
            )),
            panel::hinted(
                "With Shift held, a segment leaves its start in steps of this angle",
                panel::inline(choose(
                    "display.angle_step",
                    "Angle snap step",
                    &crate::ANGLE_STEPS,
                    o.angle_step_deg,
                    90.0,
                )),
            ),
            toggle("display.grid_on", "Show grid", o.grid_on, ""),
            toggle("display.grid_auto", "Grid auto spacing", o.grid_auto, ""),
            toggle("display.grid_snap", "Snap to grid", o.grid_snap, ""),
            toggle("display.snap", "Snap to objects", !self.tab.snap_off, ""),
            toggle(
                "display.reuse_lines",
                "Reuse existing lines",
                o.reuse_lines,
                crate::settings::REUSE_HINT,
            ),
            toggle(
                "display.avoid_redundant",
                "Avoid redundant auto constraints",
                o.avoid_redundant_auto,
                "",
            ),
            toggle(
                "display.auto_constraints",
                "Auto constraints",
                o.auto_constraints,
                "",
            ),
            toggle(
                "display.continue_drawing",
                "Continue drawing",
                o.continue_drawing,
                "A drawing tool stays in hand after its shape; off, the pointer goes back to Select",
            ),
            toggle(
                "display.continue_constraining",
                "Continue constraining",
                o.continue_constraining,
                "A constraint tool waits for the next elements after its constraint; off, it is put away",
            ),
            toggle(
                "display.arc_helpers",
                "Arc helpers",
                o.arc_helpers,
                "While drawing, each arc's whole circle shows as a dashed guide a point can snap onto",
            ),
            panel::caption("Splines", Tone::Neutral, false),
            panel::flow(vec![
                toggle(
                    "display.spline_polygon",
                    "Control polygon",
                    o.spline_polygon,
                    "",
                ),
                toggle("display.spline_degree", "Degree", o.spline_degree, ""),
                toggle(
                    "display.spline_multiplicity",
                    "Knot multiplicity",
                    o.spline_multiplicity,
                    "",
                ),
                toggle("display.spline_weights", "Weights", o.spline_weights, ""),
            ]),
            grid(
                "sketch_edit_grid",
                false,
                vec![
                    panel::caption("Grid size", Tone::Neutral, true),
                    panel::caption("Rendering order", Tone::Neutral, true),
                    Quantity {
                        width: Some(90.0),
                        enabled: !o.grid_auto,
                        ..mm_quantity("display.grid_size", "", o.grid_size)
                    }
                    .into(),
                    choose(
                        "display.render_order",
                        "",
                        &[(false, "Normal on top"), (true, "Construction on top")],
                        o.construction_on_top,
                        130.0,
                    ),
                ],
            ),
        ];
        vec![
            section(
                "sketch_solver",
                "Solver",
                None,
                Some(solver_summary),
                false,
                solver,
            ),
            section(
                "sketch_display",
                "Display & snapping",
                None,
                Some(display_summary),
                false,
                display,
            ),
        ]
    }
}

/// The exponents the solver's tolerance is chosen among: 1e-4 to 1e-12.
pub(super) const TOLERANCES: [i32; 9] = [4, 5, 6, 7, 8, 9, 10, 11, 12];

/// The calibration of `image` under way: what to pick next, then the
/// real length (a value or a formula) and the options.
fn calibration_widgets(
    ctx: &WorkbenchRuntimeContext,
    image: &sketch::ReferenceImage,
    calibrating: &crate::Calibrating,
) -> Vec<Widget> {
    if calibrating.from.is_none() || calibrating.to.is_none() {
        let step = if calibrating.from.is_none() {
            "Click the first end of a length you know on the picture"
        } else {
            "Click the other end"
        };
        return vec![
            panel::caption(step, Tone::Neutral, true),
            panel::button("calib.cancel", "Cancel", ButtonStyle::Secondary, true),
        ];
    }
    let mut widgets = vec![grid(
        "sketch_image_calibration",
        false,
        vec![
            bench_api::Entry {
                id: "calib.length".into(),
                label: "Real length".into(),
                value: calibrating.length.clone(),
                placeholder: "A value, or a formula".into(),
                width: Some(120.0),
                mono: true,
                live: true,
                ..Default::default()
            }
            .into(),
            check(
                "calib.straighten",
                "Turn",
                "Straighten to 45°",
                calibrating.straighten,
                true,
                "Turn the picture so the line lies along the nearest 45° direction",
            ),
            check(
                "calib.centre",
                "Move",
                "Centre on the line",
                calibrating.centre,
                true,
                "Move the picture so the line's middle is the sketch's origin",
            ),
        ],
    )];
    let placed = calibrated(ctx, image, calibrating);
    if let Err(why) = &placed
        && !calibrating.length.trim().is_empty()
    {
        widgets.push(panel::caption(why, Tone::Bad, true));
    }
    widgets.push(panel::flow_no_wrap(vec![
        panel::button(
            "calib.apply",
            "Apply",
            ButtonStyle::Primary,
            placed.is_ok_and(|p| p.is_some()),
        ),
        panel::button("calib.cancel", "Cancel", ButtonStyle::Secondary, true),
    ]));
    widgets
}

/// The picture as a calibration would lay it: the length as it reads (an
/// error when it does not), then the picture when both points are picked.
pub(super) fn calibrated(
    ctx: &WorkbenchRuntimeContext,
    image: &sketch::ReferenceImage,
    calibrating: &crate::Calibrating,
) -> Result<Option<sketch::ReferenceImage>, String> {
    let length = ctx.document.evaluate_formula(
        calibrating.length.trim(),
        Some(core_document::expr::Dim::LENGTH),
    )?;
    let (Some(from), Some(to)) = (calibrating.from, calibrating.to) else {
        return Ok(None);
    };
    Ok(crate::images::calibrate(
        image,
        &crate::images::Calibration {
            from,
            to,
            length: length.value,
            straighten: calibrating.straighten,
            centre: calibrating.centre,
        },
    ))
}

/// A datum's planes in the plane picker: one for a plane, one per plane
/// of a coordinate system, each named after `prefix`, with its words on
/// hover and the sketch it makes.
fn datum_planes(
    datum_id: FeatureId,
    name: &str,
    datum: &core_document::DatumFeature,
    prefix: &str,
) -> Vec<(String, &'static str, Choice)> {
    let frame = datum.frame();
    let planes: Vec<(Option<String>, core_document::DatumFrame)> = match datum.shape {
        core_document::DatumShape::Plane { .. } => vec![(None, frame)],
        core_document::DatumShape::CoordinateSystem { .. } => frame
            .planes()
            .into_iter()
            .map(|(plane, at)| (Some(plane.to_string()), at))
            .collect(),
        _ => Vec::new(),
    };
    planes
        .into_iter()
        .map(|(plane, at)| {
            let label = match &plane {
                Some(plane) => format!("{prefix}{name} {plane}"),
                None => format!("{prefix}{name}"),
            };
            let hint = if plane.is_some() {
                "Sketch on this plane of the coordinate system"
            } else {
                "Sketch on this datum plane"
            };
            let choice = (
                SketchPlane::from_frame(widen3(at.origin), widen3(at.normal), widen3(at.x_axis)),
                Some(crate::feature::DatumSupport {
                    datum: datum_id,
                    plane,
                    offset: 0.0,
                    shift: [0.0, 0.0],
                    turn: 0.0,
                }),
                None,
            );
            (label, hint, choice)
        })
        .collect()
}
