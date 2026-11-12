//! The wall thickness check: how thin the sketch's closed profile gets,
//! for a part that has to print.
//!
//! The kernel reads each region of the profile into its medial axis, the
//! centres of the largest discs that fit inside, with each disc's radius
//! (the clearance) along it. Twice the clearance is the wall there. The
//! axis draws over the sketch, in the warning colour where the wall is
//! under the minimum, and the thinnest place is marked with its width.

use std::collections::HashSet;

use core_document::{
    FeatureId, InputResult, ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay, SketchPalette,
    WorkbenchRuntimeContext,
};
use kernel_api::{MedialPath, MedialRegion, Profile, ProfilePlane, ProfileWire};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::overlay::SketchProjector;
use crate::profile;
use crate::sketch::{GeometryElement, Sketch, Vec2D};
use crate::tools::ToolState;
use crate::{SketchWorkbench, commands, sketch, stored_sketch};

/// The minimum wall a printer is taken to manage when nothing else is
/// said: two lines of a 0.4 mm nozzle.
pub const DEFAULT_MINIMUM_MM: f64 = 0.8;

/// What the medial axis is held to, in millimetres.
const AXIS_TOLERANCE_MM: f64 = 1e-3;

/// A check made on one sketch, kept for drawing until the sketch changes.
#[derive(Debug, Clone)]
pub(crate) struct WallCheck {
    pub sketch: FeatureId,
    /// The profile it was made on: a sketch whose profile differs has
    /// changed since.
    pub wires: Vec<ProfileWire>,
    pub regions: Vec<MedialRegion>,
    /// The wall under which a place counts as thin, in millimetres.
    pub minimum: f64,
    /// The document's edit count when the profile was last compared.
    pub seen_seq: u64,
}

impl WallCheck {
    /// The thinnest wall of all the regions and where it is.
    pub fn thinnest(&self) -> Option<(f64, [f64; 2])> {
        self.regions
            .iter()
            .filter_map(|r| r.narrowest)
            .min_by(|a, b| a.clearance.total_cmp(&b.clearance))
            .map(|n| (2.0 * n.clearance, n.at))
    }

    /// One line for the log and the status bar.
    pub fn summary(&self) -> String {
        match self.thinnest() {
            Some((wall, at)) => {
                let verdict = if wall < self.minimum {
                    format!("under the {} minimum", mm(self.minimum))
                } else {
                    format!("at least the {} minimum", mm(self.minimum))
                };
                format!(
                    "Thinnest wall {} at ({:.2}, {:.2}), {verdict}",
                    mm(wall),
                    at[0],
                    at[1]
                )
            }
            None => "The profile has no wall to measure".to_string(),
        }
    }

    /// The command's answer.
    pub fn report(&self) -> Value {
        let place = |at: [f64; 2]| json!({"x": at[0], "y": at[1]});
        let regions: Vec<Value> = self
            .regions
            .iter()
            .map(|r| match r.narrowest {
                Some(n) => json!({"thinnest": 2.0 * n.clearance, "where": place(n.at)}),
                None => json!({"thinnest": Value::Null, "where": Value::Null}),
            })
            .collect();
        let (thinnest, at) = match self.thinnest() {
            Some((wall, at)) => (json!(wall), place(at)),
            None => (Value::Null, Value::Null),
        };
        json!({
            "thinnest": thinnest,
            "where": at,
            "minimum": self.minimum,
            "thin": self.thinnest().is_some_and(|(wall, _)| wall < self.minimum),
            "regions": regions,
        })
    }
}

fn mm(value: f64) -> String {
    format!("{value:.2} mm")
}

/// Check `sketch`'s closed profile against a `minimum` wall.
pub(crate) fn check(
    ctx: &WorkbenchRuntimeContext,
    sketch: FeatureId,
    minimum: f64,
) -> Result<WallCheck, String> {
    let feature = crate::stored_sketch(ctx.document, sketch).ok_or("that is not a sketch")?;
    let wires = profile::extract_wires(&feature.sketch).map_err(|e| e.to_string())?;
    let kernel = ctx.kernel.ok_or("no kernel to measure with")?;
    // The sketch's own coordinates are the plane's.
    let profile = Profile {
        plane: ProfilePlane {
            origin: [0.0; 3],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        },
        wires: wires.clone(),
    };
    let regions = kernel
        .medial_axis(&profile, AXIS_TOLERANCE_MM)
        .map_err(|e| e.to_string())?;
    Ok(WallCheck {
        sketch,
        wires,
        regions,
        minimum,
        seen_seq: ctx.document.mutation_seq(),
    })
}

/// The clearance each point of a path stands for. Towards an end on the
/// boundary the clearance falls to a corner without the region getting
/// thinner, so that run takes the clearance where it starts to fall.
fn effective_clearance(path: &MedialPath) -> Vec<f64> {
    let mut out = path.clearance.clone();
    let n = out.len();
    if path.boundary_ends[0] {
        hold_rise(&mut out, &(0..n).collect::<Vec<_>>());
    }
    if path.boundary_ends[1] {
        hold_rise(&mut out, &(0..n).rev().collect::<Vec<_>>());
    }
    out
}

/// Along `order`, from its first index while the clearance keeps rising,
/// every value takes the one the rise ends at.
fn hold_rise(clearance: &mut [f64], order: &[usize]) {
    let mut top = 0;
    while top + 1 < order.len() && clearance[order[top + 1]] >= clearance[order[top]] {
        top += 1;
    }
    let Some(&peak_at) = order.get(top) else {
        return;
    };
    let peak = clearance[peak_at];
    for &i in &order[..top] {
        clearance[i] = peak;
    }
}

/// The axis, each stretch in the colour of the wall it stands for.
pub(crate) fn overlays(
    check: &WallCheck,
    proj: &SketchProjector,
    pal: &SketchPalette,
) -> Vec<ScreenSpaceOverlay> {
    let mut out = Vec::new();
    for path in check.regions.iter().flat_map(|r| &r.paths) {
        let clearance = effective_clearance(path);
        let px: Vec<Option<[f32; 2]>> = path
            .points
            .iter()
            .map(|p| proj.to_px(Vec2D::new(p[0], p[1])))
            .collect();
        for i in 1..px.len() {
            let (Some(a), Some(b)) = (px[i - 1], px[i]) else {
                continue;
            };
            let wall = 2.0 * clearance[i - 1].min(clearance[i]);
            let color = if wall < check.minimum {
                pal.wall_thin
            } else {
                pal.wall
            };
            out.push(ScreenSpaceOverlay::new(a, b, color, 2.0));
        }
    }
    out
}

/// A dot on each region's narrowest place.
pub(crate) fn marks(
    check: &WallCheck,
    proj: &SketchProjector,
    pal: &SketchPalette,
) -> Vec<ScreenSpaceMark> {
    check
        .regions
        .iter()
        .filter_map(|r| r.narrowest)
        .filter_map(|n| {
            let color = if 2.0 * n.clearance < check.minimum {
                pal.wall_thin
            } else {
                pal.wall
            };
            let pos = proj.to_px(Vec2D::new(n.at[0], n.at[1]))?;
            Some(ScreenSpaceMark::dot(pos, 4.0, color))
        })
        .collect()
}

/// The thinnest wall's width beside its dot.
pub(crate) fn labels(
    check: &WallCheck,
    proj: &SketchProjector,
    pal: &SketchPalette,
) -> Vec<ScreenSpaceLabel> {
    let Some((wall, at)) = check.thinnest() else {
        return Vec::new();
    };
    let Some([x, y]) = proj.to_px(Vec2D::new(at[0], at[1])) else {
        return Vec::new();
    };
    let color = if wall < check.minimum {
        pal.wall_thin
    } else {
        pal.wall
    };
    vec![
        ScreenSpaceLabel::new([x, y - 16.0], mm(wall), color, 12.0)
            .pill()
            .mono(),
    ]
}

impl SketchWorkbench {
    /// Stray points, constraints that reference missing geometry, and
    /// whether the sketch closes into profiles: reported and selected.
    pub(crate) fn validate(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(feature) = self.get_active_sketch(ctx) else {
            return InputResult::ignored();
        };
        let sketch = &feature.sketch;
        let ids: HashSet<Uuid> = sketch.geometry.iter().map(GeometryElement::id).collect();
        let referenced: HashSet<Uuid> = sketch
            .geometry
            .iter()
            .flat_map(|g| match g {
                GeometryElement::Point(_) => Vec::new(),
                GeometryElement::Line(l) => vec![l.start, l.end],
                GeometryElement::Arc(a) => vec![a.center, a.start, a.end],
                GeometryElement::Circle(c) => vec![c.center],
                curve @ (GeometryElement::Ellipse(_)
                | GeometryElement::BSpline(_)
                | GeometryElement::Conic(_)) => Sketch::curve_point_ids(curve),
            })
            .collect();
        self.tab.selected.clear();
        self.tab.selected_constraints.clear();
        let mut problems = Vec::new();
        let stray: Vec<Uuid> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Point(p) if !referenced.contains(&p.id) => Some(p.id),
                _ => None,
            })
            .collect();
        if !stray.is_empty() {
            problems.push(format!("{} stray point(s)", stray.len()));
            self.tab.selected.extend(stray);
        }
        let malformed: Vec<Uuid> = sketch
            .constraints
            .iter()
            .filter(|c| {
                sketch::constraint_refs(&c.kind)
                    .iter()
                    .any(|r| !ids.contains(r))
            })
            .map(|c| c.id)
            .collect();
        if !malformed.is_empty() {
            problems.push(format!(
                "{} constraint(s) referencing missing geometry",
                malformed.len()
            ));
            self.tab.selected_constraints.extend(malformed);
        }
        match profile::extract_wires(sketch) {
            Ok(wires) => {
                problems.push(format!("{} closed profile(s)", wires.len()));
                let loose = profile::loose_ends(sketch);
                if !loose.is_empty() {
                    problems.push(format!(
                        "{} loose end(s) left out of the profile",
                        loose.len()
                    ));
                    self.tab.selected.extend(loose);
                }
            }
            Err(profile::ProfileError::Empty) => problems.push("no closed profile".to_string()),
            Err(profile::ProfileError::OpenAt(id)) => {
                problems.push("an open profile".to_string());
                self.tab.selected.insert(id);
            }
            Err(profile::ProfileError::BranchingAt(id)) => {
                problems.push("a branching profile".to_string());
                self.tab.selected.insert(id);
            }
            Err(profile::ProfileError::MissingPoint(id)) => {
                problems.push("a curve missing its point".to_string());
                self.tab.selected.insert(id);
            }
            Err(profile::ProfileError::Unsolved) => {
                problems.push("conflicting constraints, so no profile".to_string());
            }
            Err(profile::ProfileError::MissingRegion(_)) => {
                problems.push("a chosen region is gone".to_string());
            }
        }
        ctx.log_info(format!("Sketch check: {}", problems.join(", ")));
        InputResult::consumed()
    }

    /// Measure how thin the edited sketch's profile gets: logged, drawn
    /// over the sketch and recorded as the command it is.
    pub(crate) fn check_walls(&mut self, ctx: &mut WorkbenchRuntimeContext) -> InputResult {
        let Some(id) = self.tab.active_sketch_id else {
            return InputResult::ignored();
        };
        // The host drops the action once it is handled, leaving no tool:
        // the one before is put away now, so the check outlives the next
        // event.
        if self.tab.activated_tool.take().is_some() {
            self.tab.tool_state = ToolState::Idle;
            self.tab.dim_capture.clear_buffers();
            let tab = &mut *self.tab;
            tab.dim_capture.sync(&tab.tool_state, None);
            self.flush_draw_record(ctx);
        }
        let minimum = self.options.min_wall;
        match check(ctx, id, minimum) {
            Ok(check) => {
                let summary = check.summary();
                if check.thinnest().is_some_and(|(wall, _)| wall < minimum) {
                    ctx.log_warn(summary);
                } else {
                    ctx.log_info(summary);
                }
                ctx.record(
                    "sketch.wall_thickness",
                    commands::object(serde_json::json!({
                        "sketch": id.0.to_string(),
                        "minimum": minimum,
                    })),
                    check.report(),
                );
                self.tab.wall_check = Some(check);
            }
            Err(err) => {
                self.tab.wall_check = None;
                ctx.log_warn(format!("Cannot check the wall thickness: {err}"));
            }
        }
        InputResult::consumed()
    }

    /// `sketch.wall_thickness`: the check's report, and on the sketch being
    /// edited its drawing too.
    pub(crate) fn wall_thickness_command(
        &mut self,
        args: &core_document::CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> core_document::CommandResult {
        let a = core_document::Args(args);
        let sketch = FeatureId(a.id("sketch")?);
        let minimum = match a.opt_number("minimum")? {
            Some(m) if m > 0.0 => m,
            Some(_) => {
                return Err(core_document::CommandError::bad(
                    "minimum",
                    "must be more than zero",
                ));
            }
            None => self.options.min_wall,
        };
        if stored_sketch(ctx.document, sketch).is_none() {
            return Err(core_document::CommandError::bad(
                "sketch",
                "is not a sketch of this document",
            ));
        }
        let check = check(ctx, sketch, minimum).map_err(core_document::CommandError::failed)?;
        let report = check.report();
        if self.tab.active_sketch_id == Some(sketch) {
            self.tab.wall_check = Some(check);
        }
        Ok(report)
    }

    /// The check on the sketch being edited.
    pub(crate) fn shown_wall_check(&self) -> Option<&WallCheck> {
        self.tab
            .wall_check
            .as_ref()
            .filter(|check| Some(check.sketch) == self.tab.active_sketch_id)
    }

    /// Put the check away once its sketch is no longer edited, or once an
    /// edit has changed its profile.
    pub(crate) fn drop_stale_wall_check(&mut self, ctx: &WorkbenchRuntimeContext) {
        let tab = &mut *self.tab;
        let Some(check) = &mut tab.wall_check else {
            return;
        };
        if Some(check.sketch) != tab.active_sketch_id {
            tab.wall_check = None;
            return;
        }
        let seq = ctx.document.mutation_seq();
        if check.seen_seq == seq {
            return;
        }
        check.seen_seq = seq;
        let same = stored_sketch(ctx.document, check.sketch)
            .is_some_and(|f| profile::extract_wires(&f.sketch).is_ok_and(|w| w == check.wires));
        if !same {
            tab.wall_check = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(clearance: &[f64], boundary_ends: [bool; 2]) -> MedialPath {
        MedialPath {
            points: vec![[0.0, 0.0]; clearance.len()],
            clearance: clearance.to_vec(),
            boundary_ends,
        }
    }

    #[test]
    fn a_branch_into_a_corner_stands_for_the_wall_it_leaves() {
        let got = effective_clearance(&path(&[0.0, 0.3, 0.6, 1.0], [true, false]));
        assert_eq!(got, vec![1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn a_neck_between_branch_points_stays_thin() {
        let got = effective_clearance(&path(&[1.0, 0.4, 1.0], [false, false]));
        assert_eq!(got, vec![1.0, 0.4, 1.0]);
    }

    #[test]
    fn a_corner_run_stops_where_the_clearance_turns() {
        let got = effective_clearance(&path(&[0.0, 0.5, 0.8, 0.6, 0.9], [true, false]));
        assert_eq!(got, vec![0.8, 0.8, 0.8, 0.6, 0.9]);
    }
}

#[cfg(test)]
mod wall_thickness {
    use super::*;
    use crate::*;
    use core_document::{Document, HookOutcome, LogLevel};
    use kernel_api::{
        KernelQueries, KernelResult, MedialPath, MedialRegion, Narrowest, Profile, ProfilePlane,
        ProjectedEdge,
    };

    /// A kernel that finds every profile 0.6 mm thin along y = 1.
    struct Thin;

    impl KernelQueries for Thin {
        fn project_edge(
            &self,
            _brep: &[u8],
            _near: [f64; 3],
            _plane: &ProfilePlane,
        ) -> KernelResult<ProjectedEdge> {
            unreachable!()
        }

        fn medial_axis(&self, profile: &Profile, _: f64) -> KernelResult<Vec<MedialRegion>> {
            Ok(vec![MedialRegion {
                wires: (0..profile.wires.len()).collect(),
                paths: vec![MedialPath {
                    points: vec![[1.0, 1.0], [10.0, 1.0], [19.0, 1.0]],
                    clearance: vec![0.3; 3],
                    boundary_ends: [false; 2],
                }],
                narrowest: Some(Narrowest {
                    at: [10.0, 1.0],
                    clearance: 0.3,
                }),
            }])
        }
    }

    static THIN: Thin = Thin;

    fn view_proj() -> [[f32; 4]; 4] {
        let proj = glam::camera::rh::proj::directx::perspective(
            60f32.to_radians(),
            800.0 / 600.0,
            0.1,
            1000.0,
        );
        let flip_y = glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, 1.0));
        let view = glam::camera::rh::view::look_at_mat4(
            glam::Vec3::new(10.0, 1.0, 50.0),
            glam::Vec3::new(10.0, 1.0, 0.0),
            glam::Vec3::Y,
        );
        (flip_y * proj * view).to_cols_array_2d()
    }

    fn ctx(doc: &mut Document, sketch: FeatureId) -> WorkbenchRuntimeContext<'_> {
        let mut ctx = WorkbenchRuntimeContext::new(
            doc,
            [10.0, 1.0, 50.0],
            [10.0, 1.0, 0.0],
            (0, 0, 800, 600),
        );
        ctx.view_proj = Some(view_proj());
        ctx.kernel = Some(&THIN);
        ctx.active_document_object = Some(sketch);
        ctx
    }

    fn call(ctx: &mut WorkbenchRuntimeContext, id: &str, args: serde_json::Value) {
        commands::run(id, &commands::object(args), ctx).unwrap();
    }

    fn scene() -> (Document, SketchWorkbench, FeatureId) {
        let mut doc = Document::new("t");
        let sketch = doc
            .add_feature(
                SketchFeature::new(Sketch::new("s"), SketchPlane::xy()),
                "s".into(),
            )
            .unwrap();
        let s = sketch.0.to_string();
        call(
            &mut ctx(&mut doc, sketch),
            "sketch.rect",
            serde_json::json!({"sketch": s, "x": 0, "y": 0, "width": 20, "height": 2}),
        );
        let wb = SketchWorkbench {
            tab: crate::Tab {
                active_sketch_id: Some(sketch),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        (doc, wb, sketch)
    }

    fn label_texts(wb: &SketchWorkbench, ctx: &WorkbenchRuntimeContext) -> Vec<String> {
        wb.screen_labels(ctx, None)
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    #[test]
    fn the_tool_reports_draws_and_records_the_thinnest_wall() {
        let (mut doc, mut wb, sketch) = scene();
        let mut ctx = ctx(&mut doc, sketch);
        let move_to = |x: f32| WorkbenchInputEvent::MouseMove {
            viewport_pos: (x, 300.0),
        };
        // A drawing tool was out before the check.
        wb.on_input(&move_to(100.0), Some("sketch.line"), &mut ctx);
        wb.on_input(
            &WorkbenchInputEvent::ToolActivated,
            Some("sketch.wall_thickness"),
            &mut ctx,
        );
        // The host leaves no tool once the action is handled.
        wb.on_input(&move_to(120.0), None, &mut ctx);
        let logs = ctx.drain_logs();
        assert!(
            logs.iter()
                .any(|l| l.level == LogLevel::Warn && l.message.contains("0.60 mm")),
            "{logs:?}"
        );
        let pal = ctx.sketch_palette;
        let thin_lines = wb
            .screen_overlays(&ctx, None)
            .into_iter()
            .filter(|o| o.color == pal.wall_thin)
            .count();
        assert_eq!(thin_lines, 2, "the axis draws in the warning colour");
        assert!(label_texts(&wb, &ctx).contains(&"0.60 mm".to_string()));

        let recorded = HookOutcome::take(&mut ctx).recorded;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].id, "sketch.wall_thickness");
        assert_eq!(recorded[0].result["thin"], serde_json::json!(true));

        // Escape puts it away.
        wb.on_input(
            &WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            },
            None,
            &mut ctx,
        );
        assert!(!label_texts(&wb, &ctx).contains(&"0.60 mm".to_string()));
    }

    #[test]
    fn an_edit_to_the_profile_puts_the_check_away() {
        let (mut doc, mut wb, sketch) = scene();
        let mut ctx = ctx(&mut doc, sketch);
        let s = sketch.0.to_string();
        let args = commands::object(serde_json::json!({"sketch": s, "minimum": 0.5}));
        let report = wb
            .run_command("sketch.wall_thickness", &args, &mut ctx)
            .unwrap();
        assert_eq!(report["thin"], serde_json::json!(false), "{report}");
        assert!((report["thinnest"].as_f64().unwrap() - 0.6).abs() < 1e-9);
        wb.on_frame(0.016, &mut ctx);
        assert!(
            wb.shown_wall_check().is_some(),
            "an unchanged sketch keeps it"
        );

        call(
            &mut ctx,
            "sketch.circle",
            serde_json::json!({"sketch": s, "x": 40, "y": 0, "radius": 2}),
        );
        wb.on_frame(0.016, &mut ctx);
        assert!(wb.shown_wall_check().is_none());
    }
}
