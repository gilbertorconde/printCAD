//! What the view draws while the bench is active: faded bodies, the
//! status, joints, clashes and near pairs, the centre of mass, explode
//! lines, handles, marks and labels.

use crate::{
    Anchor, AssemblyWorkbench, Clash, Joint, JointKind, Motion, Rigid, Task, exploded, joints,
    motion,
};
use core_document::{BodyId, FeatureId, ToolHint, ViewportHud, WorkbenchRuntimeContext};

/// A joint as the view draws it: its lines, the dots at its ends and its
/// name.
type JointDrawing = (
    Vec<core_document::ScreenSpaceOverlay>,
    Vec<[f32; 2]>,
    String,
);

impl AssemblyWorkbench {
    /// The pairs nearer than the clearance shown: the line between their
    /// nearest points on screen, and how near.
    fn near_on_screen(&self, ctx: &WorkbenchRuntimeContext) -> Vec<([f32; 2], [f32; 2], f64)> {
        let near = match &self.tab.task {
            Some(Task::Interference {
                found: Some(found), ..
            }) => found.near.as_slice(),
            _ => &[],
        };
        near.iter()
            .filter_map(|n| {
                let (ax, ay) = ctx.world_to_viewport(n.on_a)?;
                let (bx, by) = ctx.world_to_viewport(n.on_b)?;
                Some(([ax, ay], [bx, by], n.distance_mm))
            })
            .collect()
    }

    /// The joint open in its task, else the joints selected, drawn.
    pub(crate) fn joint_drawings(&self, ctx: &WorkbenchRuntimeContext) -> Vec<JointDrawing> {
        let ids: Vec<FeatureId> = match &self.tab.task {
            Some(Task::Joint { id, .. }) => vec![*id],
            _ if !ctx.selected_features.is_empty() => ctx.selected_features.clone(),
            _ => Self::selected_joint(ctx).into_iter().collect(),
        };
        let all = joints(ctx.document);
        ids.iter()
            .filter_map(|id| all.iter().find(|j| j.id == *id))
            .filter_map(|joint| Self::joint_drawing(ctx, joint))
            .collect()
    }

    /// A joint drawn: at each end a dot where it takes hold, its direction
    /// (a flat face's normal, an axis both ways), a square in a flat
    /// face's plane, and a dashed link between the ends.
    fn joint_drawing(ctx: &WorkbenchRuntimeContext, joint: &Joint) -> Option<JointDrawing> {
        if joint.feature.kind == JointKind::Ground {
            return None;
        }
        let color = ctx.sketch_palette.selected;
        let mut lines = Vec::new();
        let mut dots = Vec::new();
        let ends = [
            (joint.feature.moving, joint.body),
            (joint.feature.fixed, joint.feature.other_body),
        ];
        let screen = |p: glam::DVec3| ctx.world_to_viewport(p.as_vec3().to_array());
        for (anchor, body) in ends {
            let at: Rigid = ctx.document.body_placement(body).into();
            let (p, d) = anchor.placed(&at);
            let Some(centre) = screen(p) else {
                continue;
            };
            dots.push([centre.0, centre.1]);
            // About 40 pixels long wherever the camera stands.
            let Some(unit) = screen(p + d) else {
                continue;
            };
            let px = (unit.0 - centre.0).hypot(unit.1 - centre.1).max(1e-3);
            let reach = f64::from(40.0 / px);
            let line = |a: glam::DVec3, b: glam::DVec3| {
                Some(core_document::ScreenSpaceOverlay::new(
                    screen(a).map(|(x, y)| [x, y])?,
                    screen(b).map(|(x, y)| [x, y])?,
                    color,
                    1.5,
                ))
            };
            match anchor {
                Anchor::Plane { .. } => {
                    lines.extend(line(p, p + d * reach));
                    let side = d.any_orthonormal_pair();
                    let (u, v) = (side.0 * reach * 0.5, side.1 * reach * 0.5);
                    let corners = [p + u + v, p - u + v, p - u - v, p + u - v];
                    for k in 0..4 {
                        lines.extend(line(corners[k], corners[(k + 1) % 4]));
                    }
                }
                Anchor::Axis { .. } => {
                    lines.extend(
                        line(p - d * reach * 1.5, p + d * reach * 1.5).map(|l| l.dashed(6.0, 4.0)),
                    );
                }
                // A point is its dot.
                Anchor::Point { .. } => {}
            }
        }
        if let [a, b] = dots.as_slice() {
            lines.push(
                core_document::ScreenSpaceOverlay::new(*a, *b, color, 1.0)
                    .dashed(3.0, 3.0)
                    .with_alpha(0.8),
            );
        }
        Some((lines, dots, joint.name.clone()))
    }

    /// The clashes shown, each where it sits on screen.
    fn clashes_on_screen<'a>(
        &'a self,
        ctx: &'a WorkbenchRuntimeContext,
    ) -> impl Iterator<Item = ([f32; 2], &'a Clash)> + 'a {
        let clashes = match &self.tab.task {
            Some(Task::Interference {
                found: Some(found), ..
            }) => found.clashes.as_slice(),
            _ => &[],
        };
        clashes.iter().filter_map(|clash| {
            let (x, y) = ctx.world_to_viewport(clash.centre)?;
            Some(([x, y], clash))
        })
    }

    /// The centre of mass shown, on screen.
    fn centre_of_mass_on_screen(&self, ctx: &WorkbenchRuntimeContext) -> Option<[f32; 2]> {
        let Some(Task::Mass {
            found: Some(report),
            density,
        }) = &self.tab.task
        else {
            return None;
        };
        let c = report.centre(f64::from(*density))?;
        let (x, y) = ctx.world_to_viewport(c.map(|v| v as f32))?;
        Some([x, y])
    }

    /// The explode lines of the open view: each moved body's middle from
    /// where it sits to where the view has it.
    fn explode_lines(&self, ctx: &WorkbenchRuntimeContext) -> Vec<([f32; 3], [f32; 3])> {
        let Some(Task::Explode {
            placements, steps, ..
        }) = &self.tab.task
        else {
            return Vec::new();
        };
        let Some(view) = steps
            .view
            .and_then(|id| exploded::view_of(ctx.document, id))
        else {
            return Vec::new();
        };
        view.placed_at(ctx.document, placements, steps.at)
            .into_iter()
            .zip(placements)
            .filter(|((_, now), (_, was))| !now.after(&was.inverse()).is_identity())
            .filter_map(|((body, now), (_, was))| {
                Some((
                    exploded::centre(ctx.document, body, was)?,
                    exploded::centre(ctx.document, body, &now)?,
                ))
            })
            .collect()
    }

    /// While a joint's faces are picked, every body but the one under the
    /// cursor and the one already picked fades, so faces behind show.
    pub(crate) fn faded_bodies(&self, ctx: &WorkbenchRuntimeContext) -> Vec<BodyId> {
        let Some(picking) = &self.tab.picking else {
            return Vec::new();
        };
        let keep = [
            ctx.hovered_body_id.map(BodyId),
            picking.first.map(|(body, ..)| body),
        ];
        ctx.document
            .bodies()
            .iter()
            .map(|b| b.id)
            .filter(|b| !keep.contains(&Some(*b)))
            .collect()
    }

    /// Whether the assembly is fully placed, or how many motions its
    /// joints leave open; the selected body's, in words.
    pub(crate) fn status_items(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<core_document::StatusItems> {
        let free = self.freedom_now(ctx);
        if free.is_empty() {
            return None;
        }
        let pal = ctx.sketch_palette;
        let open: usize = free.iter().map(|(_, m)| m.len()).sum();
        let extra = self.redundant_now(ctx).len();
        let extra = match extra {
            0 => String::new(),
            1 => ", 1 joint redundant".to_string(),
            n => format!(", {n} joints redundant"),
        };
        let state = if open == 0 {
            (pal.fully_constrained, format!("Fully placed{extra}"))
        } else {
            (
                pal.constraint,
                format!(
                    "{open} motion{} free{extra}",
                    if open == 1 { "" } else { "s" }
                ),
            )
        };
        let selection = Self::body_to_move(ctx).and_then(|body| {
            let (_, motions) = free.iter().find(|(b, _)| *b == body)?;
            let name = ctx
                .document
                .body(body)
                .map(|b| b.name.clone())
                .unwrap_or_default();
            Some(if motions.is_empty() {
                format!("{name}: fully placed")
            } else {
                let words: Vec<String> = motions.iter().map(Motion::describe).collect();
                format!("{name}: {}", words.join(", "))
            })
        });
        Some(core_document::StatusItems {
            state: Some(state),
            selection,
            coords: None,
            mode: None,
        })
    }

    /// The material each clash shares, drawn over everything.
    pub(crate) fn overlay_meshes(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::OverlayMesh> {
        let Some(Task::Interference {
            found: Some(found), ..
        }) = &self.tab.task
        else {
            return Vec::new();
        };
        found
            .clashes
            .iter()
            .map(|clash| {
                core_document::OverlayMesh::on_top(
                    std::sync::Arc::clone(&clash.mesh),
                    clash.revision,
                    ctx.sketch_palette.conflict,
                    0.6,
                )
            })
            .collect()
    }

    /// Each pair nearer than the clearance, joined between its nearest
    /// points; the joint drawn, the move handles, the motion's traced
    /// points and the explode lines.
    pub(crate) fn screen_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceOverlay> {
        let mut lines: Vec<core_document::ScreenSpaceOverlay> = self
            .near_on_screen(ctx)
            .into_iter()
            .map(|(a, b, _)| {
                core_document::ScreenSpaceOverlay::new(a, b, ctx.sketch_palette.conflict, 2.0)
            })
            .collect();
        for (joint, _, _) in self.joint_drawings(ctx) {
            lines.extend(joint);
        }
        if let Some(body) = self.handled_body(ctx) {
            lines.extend(self.tab.handles.drawing(ctx, body).overlays);
        }
        if let Some(Task::Motion(studying)) = &self.tab.task
            && let Some(frames) = &studying.frames
        {
            for (body, point) in &studying.traces {
                let path = motion::trace(frames, *body, *point);
                let px: Vec<Option<(f32, f32)>> = path
                    .iter()
                    .map(|(_, p, _)| ctx.world_to_viewport(*p))
                    .collect();
                for w in px.windows(2) {
                    if let (Some(a), Some(b)) = (w[0], w[1]) {
                        lines.push(core_document::ScreenSpaceOverlay::new(
                            [a.0, a.1],
                            [b.0, b.1],
                            ctx.sketch_palette.selected,
                            1.5,
                        ));
                    }
                }
            }
        }
        for (from, to) in self.explode_lines(ctx) {
            if let (Some(a), Some(b)) = (ctx.world_to_viewport(from), ctx.world_to_viewport(to)) {
                lines.push(
                    core_document::ScreenSpaceOverlay::new(
                        [a.0, a.1],
                        [b.0, b.1],
                        ctx.sketch_palette.preview,
                        1.0,
                    )
                    .dashed(5.0, 4.0),
                );
            }
        }
        lines
    }

    /// Each clash found, marked where it is; the joint drawn's ends, the
    /// centre of mass and the move handles' dots.
    pub(crate) fn screen_marks(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceMark> {
        let mut marks: Vec<core_document::ScreenSpaceMark> = self
            .clashes_on_screen(ctx)
            .map(|(pos, _)| {
                core_document::ScreenSpaceMark::icon(
                    pos,
                    "warning",
                    18.0,
                    ctx.sketch_palette.conflict,
                )
            })
            .collect();
        for (_, dots, _) in self.joint_drawings(ctx) {
            marks.extend(
                dots.into_iter().map(|p| {
                    core_document::ScreenSpaceMark::dot(p, 4.0, ctx.sketch_palette.selected)
                }),
            );
        }
        if let Some(pos) = self.centre_of_mass_on_screen(ctx) {
            marks.push(core_document::ScreenSpaceMark::crosshair(
                pos,
                14.0,
                ctx.sketch_palette.selected,
            ));
            marks.push(core_document::ScreenSpaceMark::dot(
                pos,
                3.0,
                ctx.sketch_palette.selected,
            ));
        }
        if let Some(body) = self.handled_body(ctx) {
            marks.extend(self.tab.handles.drawing(ctx, body).marks);
        }
        marks
    }

    /// How much each clash shares, beside its mark; the centre of mass,
    /// each near pair's distance, the joint drawn's name and the move
    /// handles' letters and value.
    pub(crate) fn screen_labels(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpaceLabel> {
        let com = self.centre_of_mass_on_screen(ctx).map(|[x, y]| {
            core_document::ScreenSpaceLabel::new(
                [x + 14.0, y - 14.0],
                "Centre of mass".to_string(),
                ctx.sketch_palette.selected,
                11.0,
            )
            .pill()
        });
        let near: Vec<core_document::ScreenSpaceLabel> = self
            .near_on_screen(ctx)
            .into_iter()
            .map(|(a, b, d)| {
                core_document::ScreenSpaceLabel::new(
                    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0 - 14.0],
                    format!("{d:.2} mm"),
                    ctx.sketch_palette.conflict,
                    12.0,
                )
                .pill()
                .mono()
            })
            .collect();
        self.clashes_on_screen(ctx)
            .map(|([x, y], clash)| {
                core_document::ScreenSpaceLabel::new(
                    [x, y + 20.0],
                    format!("{:.1} mm³", clash.volume_mm3),
                    ctx.sketch_palette.conflict,
                    12.0,
                )
                .pill()
                .mono()
            })
            .chain(com)
            .chain(near)
            .chain(
                self.joint_drawings(ctx)
                    .into_iter()
                    .filter_map(|(_, dots, name)| {
                        let [a, b] = dots.as_slice().try_into().ok()?;
                        let [a, b]: [[f32; 2]; 2] = [a, b];
                        Some(
                            core_document::ScreenSpaceLabel::new(
                                [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0 - 14.0],
                                name,
                                ctx.sketch_palette.selected,
                                11.0,
                            )
                            .pill(),
                        )
                    }),
            )
            .chain(
                self.handled_body(ctx)
                    .into_iter()
                    .flat_map(|body| self.tab.handles.drawing(ctx, body).labels),
            )
            .collect()
    }

    /// The move handles' cones, squares and swept angle.
    pub(crate) fn screen_polygons(
        &self,
        ctx: &WorkbenchRuntimeContext,
        _active_feature: Option<FeatureId>,
    ) -> Vec<core_document::ScreenSpacePolygon> {
        self.handled_body(ctx)
            .map(|body| self.tab.handles.drawing(ctx, body).polygons)
            .unwrap_or_default()
    }

    pub(crate) fn viewport_hud(&self, _ctx: &WorkbenchRuntimeContext) -> Option<ViewportHud> {
        let picking = self.tab.picking.as_ref()?;
        Some(ViewportHud {
            tool: Some(ToolHint {
                icon: picking.kind.icon(),
                name: picking.kind.label().to_string(),
                prompt: picking.prompt().to_string(),
                keys: vec![("Esc".to_string(), "cancel")],
            }),
            ..ViewportHud::default()
        })
    }
}
