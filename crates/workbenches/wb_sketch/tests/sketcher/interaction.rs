//! End-to-end interaction tests: drive `SketchWorkbench::on_input` through
//! a real `Document` + `WorkbenchRuntimeContext`, exactly like the app
//! shell does: clicks arrive as viewport pixels and are raycast onto the
//! sketch plane by the workbench itself.

use core_document::{
    Document, FeatureId, KeyCode, MarkKind, MouseButton, ScreenSpaceMark, SketchPalette,
    ViewportPick, Workbench, WorkbenchFeature, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use glam::{Mat4, Vec3};
use wb_sketch::sketch::{GeometryElement, Sketch};
use wb_sketch::{SketchFeature, SketchWorkbench};

const VIEWPORT: (u32, u32, u32, u32) = (0, 0, 800, 600);
const CAM_POS: [f32; 3] = [0.0, 0.0, 50.0];

fn pal() -> SketchPalette {
    SketchPalette::default()
}

/// Colors match within the 8-bit rounding of the palette.
fn same_color(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

fn is_icon(mark: &ScreenSpaceMark, icon: &str) -> bool {
    matches!(mark.kind, MarkKind::Icon { name, .. } if name == icon)
}

/// Vulkan-convention view-projection matching the app camera: perspective
/// with the Y flip baked in, looking straight down +Z at the default XY
/// sketch plane.
fn view_proj() -> [[f32; 4]; 4] {
    let proj = glam::camera::rh::proj::directx::perspective(
        60f32.to_radians(),
        800.0 / 600.0,
        0.1,
        1000.0,
    );
    let flip_y = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0));
    let view = glam::camera::rh::view::look_at_mat4(Vec3::from_array(CAM_POS), Vec3::ZERO, Vec3::Y);
    (flip_y * proj * view).to_cols_array_2d()
}

/// Mimics the app shell: owns the document, carries `active_document_object`
/// between events the way `apply_hook_outcome` does.
struct Harness {
    doc: Document,
    wb: SketchWorkbench,
    active_object: Option<FeatureId>,
    vp: [[f32; 4]; 4],
    /// The tool the workbench last asked the host to make active.
    tool_request: Option<String>,
}

impl Harness {
    fn new() -> Self {
        Self {
            doc: Document::new("test"),
            wb: SketchWorkbench::default(),
            active_object: None,
            vp: view_proj(),
            tool_request: None,
        }
    }

    fn event(&mut self, event: WorkbenchInputEvent, tool: Option<&str>) {
        self.event_with_ctrl(event, tool, false);
    }

    fn event_with_ctrl(&mut self, event: WorkbenchInputEvent, tool: Option<&str>, ctrl: bool) {
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(self.vp);
        ctx.active_document_object = self.active_object;
        ctx.ctrl_down = ctrl;
        // A body is always "selected" so sketch.create is permitted.
        ctx.selected_body_id = Some(uuid::Uuid::new_v4());
        self.wb.on_input(&event, tool, &mut ctx);
        self.active_object = ctx.active_document_object;
        self.tool_request = ctx.take_requests().into_iter().find_map(|r| match r {
            core_document::HostRequest::ActivateTool(tool) => Some(tool),
            _ => None,
        });
        // The host runs this hook once per frame; tool enablement reads
        // the state it refreshes.
        let mut frame_ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        frame_ctx.view_proj = Some(self.vp);
        frame_ctx.active_document_object = self.active_object;
        self.wb.on_frame(0.016, &mut frame_ctx);
    }

    /// Viewport pixel coordinates for a sketch-plane point (the inverse of
    /// what the workbench's raycast will compute).
    fn px_of(&mut self, x: f32, y: f32) -> (f32, f32) {
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(self.vp);
        ctx.world_to_viewport([x, y, 0.0])
            .expect("sketch point projects into the viewport")
    }

    fn click(&mut self, x: f32, y: f32, tool: &str) {
        let viewport_pos = self.px_of(x, y);
        self.event(
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            },
            Some(tool),
        );
    }

    fn click_ctrl(&mut self, x: f32, y: f32, tool: &str) {
        let viewport_pos = self.px_of(x, y);
        self.event_with_ctrl(
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            },
            Some(tool),
            true,
        );
    }

    fn release_ctrl(&mut self, x: f32, y: f32, tool: &str) {
        let viewport_pos = self.px_of(x, y);
        self.event_with_ctrl(
            WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Left,
                viewport_pos,
            },
            Some(tool),
            true,
        );
    }

    fn key(&mut self, key: KeyCode, tool: Option<&str>) {
        self.event(WorkbenchInputEvent::KeyPress { key }, tool);
    }

    /// Create a sketch feature directly (the interactive path goes through
    /// the egui plane picker, which has no headless harness) and make it the
    /// active object, exactly as the picker's create path does.
    fn create_sketch(&mut self) -> FeatureId {
        use wb_sketch::sketch::Sketch;
        let sketch = Sketch::new("test-sketch");
        let plane = sketch.plane;
        let id = self
            .doc
            .add_feature_in_body(SketchFeature::new(sketch, plane), "sketch".into(), None)
            .expect("create sketch feature");
        self.active_object = Some(id);
        // Let the workbench sync (enter editing) off the selection.
        self.event(WorkbenchInputEvent::KeyPress { key: KeyCode::A }, None);
        id
    }

    fn release(&mut self, x: f32, y: f32, tool: &str) {
        let viewport_pos = self.px_of(x, y);
        self.event(
            WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Left,
                viewport_pos,
            },
            Some(tool),
        );
    }

    fn mouse_move(&mut self, x: f32, y: f32, tool: &str) {
        let viewport_pos = self.px_of(x, y);
        self.event(WorkbenchInputEvent::MouseMove { viewport_pos }, Some(tool));
    }

    fn sketch(&self) -> Sketch {
        let id = self.active_object.expect("active sketch");
        let data = self.doc.get_feature_data(id).expect("feature data");
        SketchFeature::from_json(data)
            .expect("valid sketch feature")
            .sketch
    }

    fn counts(&self) -> (usize, usize, usize, usize) {
        let sketch = self.sketch();
        let mut p = 0;
        let mut l = 0;
        let mut c = 0;
        let mut a = 0;
        for g in &sketch.geometry {
            match g {
                GeometryElement::Point(_) => p += 1,
                GeometryElement::Line(_) => l += 1,
                GeometryElement::Circle(_) => c += 1,
                GeometryElement::Arc(_) => a += 1,
                _ => {}
            }
        }
        (p, l, c, a)
    }

    /// Screen-space labels (constraint glyphs + on-view readouts), exactly
    /// as the app shell fetches them each frame.
    fn labels(&mut self) -> Vec<core_document::ScreenSpaceLabel> {
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(self.vp);
        ctx.active_document_object = self.active_object;
        self.wb.get_screen_space_labels(&ctx, self.active_object)
    }

    /// Screen-space marks (point dots, constraint icons), as the app shell
    /// fetches them each frame.
    fn marks(&mut self) -> Vec<ScreenSpaceMark> {
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(self.vp);
        ctx.active_document_object = self.active_object;
        self.wb.get_screen_space_marks(&ctx, self.active_object)
    }

    /// The viewport HUD the workbench wants drawn this frame.
    fn hud(&mut self) -> Option<core_document::ViewportHud> {
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(self.vp);
        ctx.active_document_object = self.active_object;
        self.wb.viewport_hud(&ctx)
    }

    /// Press/release at raw viewport pixels (glyph clicks: labels report
    /// their position in pixels, not sketch coordinates).
    fn press_px(&mut self, pos: (f32, f32)) {
        self.event(
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos: pos,
            },
            Some("sketch.select"),
        );
    }

    fn release_px(&mut self, pos: (f32, f32)) {
        self.event(
            WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Left,
                viewport_pos: pos,
            },
            Some("sketch.select"),
        );
    }

    fn drag(&mut self, from: (f32, f32), to: (f32, f32)) {
        self.click(from.0, from.1, "sketch.select");
        self.mouse_move(to.0, to.1, "sketch.select");
        self.release(to.0, to.1, "sketch.select");
    }

    /// Whether the toolbar would light this tool up right now.
    fn tool_enabled(&mut self, id: &str) -> bool {
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(self.vp);
        ctx.active_document_object = self.active_object;
        ctx.selected_body_id = Some(uuid::Uuid::new_v4());
        self.wb.is_tool_enabled(id, &ctx)
    }

    fn point_at(&self, x: f32, y: f32) -> bool {
        self.sketch().geometry.iter().any(|g| match g {
            GeometryElement::Point(p) => {
                (p.position.x - x).abs() < 0.05 && (p.position.y - y).abs() < 0.05
            }
            _ => false,
        })
    }

    fn right_click(&mut self, x: f32, y: f32, tool: &str) {
        let viewport_pos = self.px_of(x, y);
        self.event(
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Right,
                viewport_pos,
            },
            Some(tool),
        );
        self.event(
            WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Right,
                viewport_pos,
            },
            Some(tool),
        );
    }

    /// Right press, move, release: the camera's pan gesture.
    fn right_drag(&mut self, from: (f32, f32), to: (f32, f32), tool: &str) {
        let start = self.px_of(from.0, from.1);
        let end = self.px_of(to.0, to.1);
        self.event(
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Right,
                viewport_pos: start,
            },
            Some(tool),
        );
        self.event(
            WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Right,
                viewport_pos: end,
            },
            Some(tool),
        );
    }

    /// Select elements by dragging a box over them (select mode).
    fn box_select(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.click(x0, y0, "sketch.select");
        self.mouse_move(x1, y1, "sketch.select");
        self.release(x1, y1, "sketch.select");
    }
}

#[test]
fn create_sketch_registers_feature() {
    let mut h = Harness::new();
    let id = h.create_sketch();
    assert!(h.doc.get_feature_data(id).is_some());
    let (p, l, c, a) = h.counts();
    assert_eq!((p, l, c, a), (0, 0, 0, 0));
}

#[test]
fn create_action_opens_picker_instead_of_creating() {
    let mut h = Harness::new();
    h.event(
        WorkbenchInputEvent::KeyPress { key: KeyCode::A },
        Some("sketch.create"),
    );
    // No feature yet: the plane picker is pending in the panel.
    assert!(h.active_object.is_none());
    assert_eq!(h.doc.feature_tree().all_nodes().count(), 0);
}

#[test]
fn clicking_an_origin_plane_in_the_picker_sketches_on_it() {
    let mut h = Harness::new();
    h.event(
        WorkbenchInputEvent::KeyPress { key: KeyCode::A },
        Some("sketch.create"),
    );
    assert!(h.wb.shows_origin_planes(), "the picker shows the planes");
    let release = |h: &mut Harness, plane| {
        let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(h.vp);
        ctx.hovered_base_plane = plane;
        h.wb.on_input(
            &WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Left,
                viewport_pos: (400.0, 300.0),
            },
            None,
            &mut ctx,
        );
    };
    release(&mut h, None);
    assert_eq!(
        h.doc.feature_tree().all_nodes().count(),
        0,
        "a miss picks nothing"
    );

    release(&mut h, Some(core_document::BasePlane::XZ));
    let sketches: Vec<_> = h
        .doc
        .feature_tree()
        .all_nodes()
        .filter_map(|(_, node)| SketchFeature::from_json(&node.data).ok())
        .collect();
    assert_eq!(sketches.len(), 1);
    assert_eq!(sketches[0].plane.normal, [0.0, -1.0, 0.0]);
    assert!(
        !h.wb.shows_origin_planes(),
        "the planes go once one is chosen"
    );
}

#[test]
fn cross_workbench_sketch_request_is_consumed() {
    let mut h = Harness::new();
    let body = uuid::Uuid::new_v4();
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.attach_request = Some(core_document::SketchAttachRequest {
        body,
        face: None,
        face_origin: core_document::FaceOrigin::Elsewhere,
        generator: None,
    });
    h.wb.on_input(
        &WorkbenchInputEvent::KeyPress { key: KeyCode::A },
        None,
        &mut ctx,
    );
    assert!(
        ctx.attach_request.is_none(),
        "the sketch workbench takes the pending request"
    );
}

#[test]
fn dragging_a_point_moves_it_and_leaves_it_selected() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");

    // Drag the (10,7) endpoint to (14,9): press, move, release.
    h.click(10.0, 7.0, "sketch.select");
    h.mouse_move(14.0, 9.0, "sketch.select");
    h.release(14.0, 9.0, "sketch.select");

    let sketch = h.sketch();
    let moved = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position),
            _ => None,
        })
        .any(|p| (p.x - 14.0).abs() < 0.05 && (p.y - 9.0).abs() < 0.05);
    assert!(moved, "endpoint followed the drag");

    // The dragged point stays selected, and a press+release without
    // movement adds the other endpoint: Delete takes the whole line.
    h.click(0.0, 0.0, "sketch.select");
    h.release(0.0, 0.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (0, 0), "both endpoints deleted with their line");
}

#[test]
fn dragging_a_constrained_point_respects_constraints() {
    let mut h = Harness::new();
    h.create_sketch();
    // Axis-snapped horizontal line gets an auto Horizontal constraint.
    // Off the axes: a point there would pin to the origin or an axis.
    h.click(3.0, 4.0, "sketch.line");
    h.click(18.0, 4.05, "sketch.line");
    let sketch = h.sketch();
    assert_eq!(sketch.constraints.len(), 1);

    // Drag the far endpoint up and sideways: the solver must keep the line
    // horizontal (y's equal) while the x movement sticks.
    h.click(18.0, 4.0, "sketch.select");
    h.mouse_move(23.0, 10.0, "sketch.select");
    h.release(23.0, 10.0, "sketch.select");

    let sketch = h.sketch();
    let ys: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position.y),
            _ => None,
        })
        .collect();
    assert!(
        (ys[0] - ys[1]).abs() < 1e-3,
        "line stayed horizontal under drag: {ys:?}"
    );
    let max_x = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position.x),
            _ => None,
        })
        .fold(f32::MIN, f32::max);
    assert!(max_x > 20.0, "x movement applied: {max_x}");
}

#[test]
fn two_clicks_draw_a_line_through_the_full_stack() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 6.0, "sketch.line");
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (2, 1));

    // Verify the raycast produced accurate sketch coordinates.
    let sketch = h.sketch();
    let mut positions: Vec<(f32, f32)> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(pt) => Some((pt.position.x, pt.position.y)),
            _ => None,
        })
        .collect();
    positions.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert!((positions[0].0).abs() < 0.05 && (positions[0].1).abs() < 0.05);
    assert!((positions[1].0 - 10.0).abs() < 0.05 && (positions[1].1 - 6.0).abs() < 0.05);
}

#[test]
fn chained_lines_share_vertices() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.5, "sketch.line");
    h.click(10.0, 8.0, "sketch.line");
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (3, 2), "chain adds one point per segment");
}

#[test]
fn escape_cancels_pending_segment_without_orphans() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (0, 0), "no orphan point from the cancelled click");
}

#[test]
fn rectangle_tool_produces_constrained_rectangle() {
    let mut h = Harness::new();
    h.create_sketch();
    // Off the axes: a point there would pin to the origin or an axis.
    h.click(3.0, 4.0, "sketch.rect");
    h.click(15.0, 12.0, "sketch.rect");
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (4, 4));
    let sketch = h.sketch();
    assert_eq!(sketch.constraints.len(), 4, "2 horizontal + 2 vertical");
}

#[test]
fn circle_and_arc_tools_work_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.circle");
    h.click(5.0, 0.0, "sketch.circle");
    h.click(15.0, 0.0, "sketch.arc");
    h.click(19.0, 0.0, "sketch.arc");
    h.click(15.0, 6.0, "sketch.arc");
    let (_, _, c, a) = h.counts();
    assert_eq!((c, a), (1, 1));
    let sketch = h.sketch();
    let circle_r = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Circle(c) => Some(c.radius),
            _ => None,
        })
        .unwrap();
    assert!((circle_r - 5.0).abs() < 0.05, "radius {circle_r}");
}

#[test]
fn deleting_a_line_takes_the_points_nothing_else_uses() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    // Select mid-span with the select tool, then delete.
    h.click(5.0, 3.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (0, 0), "line removed with its endpoints");
}

#[test]
fn deleting_a_point_cascades_to_its_line() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    // Click exactly on an endpoint: point wins the hit-test. Point
    // selection resolves on release (press begins a potential drag).
    h.click(10.0, 7.0, "sketch.select");
    h.release(10.0, 7.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!(
        (p, l),
        (0, 0),
        "the line cascaded away, and its now-unused endpoints with it"
    );
}

#[test]
fn horizontal_axis_snap_adds_auto_constraint() {
    let mut h = Harness::new();
    h.create_sketch();
    // Off the axes: a point there would pin to the origin or an axis.
    h.click(3.0, 4.0, "sketch.line");
    // Slightly off-horizontal: within the 8px snap tolerance at this zoom.
    h.click(18.0, 4.05, "sketch.line");
    let sketch = h.sketch();
    assert_eq!(sketch.constraints.len(), 1);
    let ys: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position.y),
            _ => None,
        })
        .collect();
    assert!(
        ys.iter().all(|y| (y - 4.0).abs() < 1e-4),
        "snapped level: {ys:?}"
    );
}

#[test]
fn endpoint_snap_reuses_vertex_for_closed_profiles() {
    let mut h = Harness::new();
    h.create_sketch();
    // Triangle: three chained segments, last click near the start vertex.
    h.click(0.0, 0.0, "sketch.line");
    h.click(20.0, 3.0, "sketch.line");
    h.click(10.0, 15.0, "sketch.line");
    h.click(0.05, 0.05, "sketch.line"); // snaps back to the first vertex
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (3, 3), "closed triangle shares all vertices");
}

#[test]
fn overlays_are_generated_while_editing() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.rect");
    h.click(10.0, 8.0, "sketch.rect");
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.active_document_object = h.active_object;
    let overlays = h.wb.get_screen_space_overlays(&ctx, h.active_object);
    // 2 axis lines + 4 rectangle edges; the 4 corner points are marks.
    assert!(
        overlays.len() >= 2 + 4,
        "expected axes + rectangle overlays, got {}",
        overlays.len()
    );
    let marks = h.wb.get_screen_space_marks(&ctx, h.active_object);
    let dots = marks
        .iter()
        .filter(|m| matches!(m.kind, MarkKind::Dot { .. }))
        .count();
    assert!(dots >= 4, "corner points drawn as dots, got {dots}");
}

/// Pressed on the end the line tool draws from and dragged, the line tool
/// draws an arc tangent to the line before it, then goes on with lines
/// from the arc's end.
#[test]
fn dragging_from_a_line_s_end_draws_a_tangent_arc() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    // Press on the end, drag, let go.
    h.click(10.0, 0.0, "sketch.line");
    for (x, y) in [(12.0, 1.0), (15.0, 5.0)] {
        let viewport_pos = h.px_of(x, y);
        h.event(
            WorkbenchInputEvent::MouseMove { viewport_pos },
            Some("sketch.line"),
        );
    }
    let viewport_pos = h.px_of(15.0, 5.0);
    h.event(
        WorkbenchInputEvent::MouseRelease {
            button: MouseButton::Left,
            viewport_pos,
        },
        Some("sketch.line"),
    );
    let (_, lines, _, arcs) = h.counts();
    assert_eq!((lines, arcs), (1, 1), "the line, then the arc");
    let sketch = h.sketch();
    assert!(
        sketch
            .constraints
            .iter()
            .any(|c| matches!(c.kind, ConstraintKind::Tangent { .. })),
        "held tangent to the line"
    );
    // The line tool draws on from the arc's end.
    h.click(15.0, 15.0, "sketch.line");
    let (points, lines, _, arcs) = h.counts();
    assert_eq!((lines, arcs), (2, 1));
    assert_eq!(points, 5, "the ends shared, plus the arc's centre");

    // A press that does not move is a click there: no arc.
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    let viewport_pos = h.px_of(10.0, 0.0);
    h.event(
        WorkbenchInputEvent::MouseRelease {
            button: MouseButton::Left,
            viewport_pos,
        },
        Some("sketch.line"),
    );
    assert_eq!(h.counts().3, 0);
}

/// Counts the profiles it is asked to mesh, and meshes each as one triangle.
struct CountingRegions(std::sync::atomic::AtomicUsize);

static REGIONS: CountingRegions = CountingRegions(std::sync::atomic::AtomicUsize::new(0));

impl kernel_api::KernelQueries for CountingRegions {
    fn project_edge(
        &self,
        _brep: &[u8],
        _near: [f64; 3],
        _plane: &kernel_api::ProfilePlane,
    ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
        Err(kernel_api::KernelError::Unsupported("projection".into()))
    }

    fn profile_mesh(
        &self,
        _profile: &kernel_api::Profile,
    ) -> kernel_api::KernelResult<kernel_api::TriMesh> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(kernel_api::TriMesh {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        })
    }
}

/// A closed outline is shaded while it is edited, meshed once for as long
/// as it stays the same; an open one is not.
#[test]
fn closed_regions_are_shaded_while_editing() {
    let kernel = &REGIONS;
    let shaded = |h: &mut Harness| {
        let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(h.vp);
        ctx.active_document_object = h.active_object;
        ctx.kernel = Some(kernel);
        h.wb.get_overlay_meshes(&ctx, h.active_object).len()
    };
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    assert_eq!(shaded(&mut h), 0, "nothing closes");
    h.click(0.0, 5.0, "sketch.rect");
    h.click(10.0, 12.0, "sketch.rect");
    assert_eq!(shaded(&mut h), 1);
    assert_eq!(shaded(&mut h), 1);
    let asked = kernel.0.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(asked, 1, "meshed once while the profile stays");
}

/// A loose end is ringed in the conflict colour once no shape is being
/// drawn; a closed outline has none.
#[test]
fn loose_ends_are_ringed_between_shapes() {
    let rings = |h: &mut Harness| {
        let conflict = core_document::SketchPalette::default().conflict;
        let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(h.vp);
        ctx.active_document_object = h.active_object;
        h.wb.get_screen_space_overlays(&ctx, h.active_object)
            .iter()
            .filter(|o| o.color == conflict)
            .count()
            // A ring is drawn in 48 segments.
            / 48
    };
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    assert_eq!(rings(&mut h), 0, "mid-shape, nothing is ringed");
    h.key(KeyCode::Escape, Some("sketch.line"));
    assert_eq!(rings(&mut h), 2, "the lone line's ends are ringed");

    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.rect");
    h.click(10.0, 8.0, "sketch.rect");
    assert_eq!(rings(&mut h), 0, "a closed outline has no loose end");
}

#[test]
fn construction_geometry_renders_dashed() {
    let mut h = Harness::new();
    let id = h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    let dashed_of = |h: &mut Harness| {
        let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(h.vp);
        ctx.active_document_object = h.active_object;
        h.wb.get_screen_space_overlays(&ctx, h.active_object)
            .iter()
            .filter(|o| o.dash.is_some())
            .count()
    };
    let solid_count = dashed_of(&mut h);

    // Flag the line as construction directly on the stored feature (no
    // selection involved, so the element renders in its base style).
    let mut feature = SketchFeature::from_json(h.doc.get_feature_data(id).unwrap()).unwrap();
    let line_id = feature
        .sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) => Some(l.id),
            _ => None,
        })
        .unwrap();
    feature.sketch.set_construction(line_id, true);
    h.doc.update_feature_data(id, feature.to_json()).unwrap();

    let dashed_count = dashed_of(&mut h);
    assert_eq!(solid_count, 0, "normal geometry draws solid");
    assert!(
        dashed_count >= 1,
        "construction line carries a dash pattern: {dashed_count} dashed overlays"
    );
}

#[test]
fn polygon_tool_draws_closed_hexagon_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.polygon");
    h.click(6.0, 0.0, "sketch.polygon");
    let (p, l, c, a) = h.counts();
    // Six vertices and the centre, six sides, the construction circle
    // that holds it regular.
    assert_eq!((p, l, c, a), (7, 6, 1, 0), "default 6 sides");
    // Closed loop through shared vertices: the profile extractor accepts it.
    let wires = wb_sketch::profile::extract_wires(&h.sketch()).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 6);
}

#[test]
fn slot_tool_draws_closed_slot_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.slot");
    h.click(12.0, 0.0, "sketch.slot");
    let (p, l, c, a) = h.counts();
    assert_eq!(
        (p, l, c, a),
        (6, 2, 0, 2),
        "junctions + centers, sides, caps"
    );
    let wires = wb_sketch::profile::extract_wires(&h.sketch()).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 4);
}

#[test]
fn fillet_tool_rounds_rectangle_corner_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    // Off the axes: a point there would pin to the origin or an axis.
    h.click(3.0, 4.0, "sketch.rect");
    h.click(15.0, 12.0, "sketch.rect");
    // Click the shared corner point with the fillet tool (default r=2).
    h.click(15.0, 12.0, "sketch.fillet");
    let (p, l, c, a) = h.counts();
    assert_eq!((p, l, c, a), (6, 4, 0, 1), "corner replaced by arc");
    let sketch = h.sketch();
    assert_eq!(
        sketch.constraints.len(),
        6,
        "rectangle H/V constraints survive the fillet, and the arc is tangent to both sides"
    );
    let tangents = sketch
        .constraints
        .iter()
        .filter(|c| matches!(c.kind, wb_sketch::sketch::ConstraintKind::Tangent { .. }))
        .count();
    assert_eq!(tangents, 2);
    let wires = wb_sketch::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 5, "4 lines + 1 corner arc");
}

#[test]
fn construction_action_toggles_selected_line() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    // Select the line mid-span, then fire the construction action (an
    // Action tool arrives as the active tool for one input event).
    h.click(5.0, 3.5, "sketch.select");
    h.key(KeyCode::A, Some("sketch.construction"));
    let sketch = h.sketch();
    let line_id = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) => Some(l.id),
            _ => None,
        })
        .unwrap();
    assert!(sketch.is_construction(line_id), "line flagged construction");
    // Firing again flips it back.
    h.key(KeyCode::A, Some("sketch.construction"));
    assert!(!h.sketch().is_construction(line_id));
}

/// The toolbar's button, pressed with a line and its end point picked in
/// the view as a user picks them (press and release, no tool in hand),
/// converts what is picked rather than switching the drawing mode.
#[test]
fn the_construction_button_converts_what_is_picked() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    let mid = h.px_of(5.0, 3.5);
    h.event(
        WorkbenchInputEvent::MousePress {
            button: MouseButton::Left,
            viewport_pos: mid,
        },
        None,
    );
    h.event(
        WorkbenchInputEvent::MouseRelease {
            button: MouseButton::Left,
            viewport_pos: mid,
        },
        None,
    );
    h.event(
        WorkbenchInputEvent::ToolActivated,
        Some("sketch.construction"),
    );
    let sketch = h.sketch();
    let line = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) => Some(l.id),
            _ => None,
        })
        .unwrap();
    assert!(sketch.is_construction(line), "the picked line is converted");
    assert!(
        !h.wb.tool_toggled("sketch.construction"),
        "the drawing mode is left alone"
    );
}

/// A drawn line and a projected one picked together convert together:
/// construction while any of them is normal, then all back to normal, the
/// projected one by counting in the profile.
#[test]
fn the_construction_button_converts_projected_geometry_with_the_rest() {
    let mut h = Harness::new();
    let id = h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(0.0, 8.0, "sketch.line");
    h.click(10.0, 8.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    let lines: Vec<uuid::Uuid> = h
        .sketch()
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some(l.id),
            _ => None,
        })
        .collect();
    let (drawn, projected) = (lines[0], lines[1]);
    // The second line stands in for one projected from a datum.
    let mut feature = SketchFeature::from_json(h.doc.get_feature_data(id).unwrap()).unwrap();
    feature.sketch.external.insert(
        projected,
        wb_sketch::sketch::ExternalSource::of_reference(
            wb_sketch::sketch::ExternalReference::Datum {
                datum: uuid::Uuid::new_v4(),
            },
        ),
    );
    h.doc.update_feature_data(id, feature.to_json()).unwrap();
    h.key(KeyCode::A, None);

    h.click(5.0, 0.0, "sketch.select");
    h.click_ctrl(5.0, 8.0, "sketch.select");
    h.event(
        WorkbenchInputEvent::ToolActivated,
        Some("sketch.construction"),
    );
    let sketch = h.sketch();
    assert!(
        sketch.is_construction(drawn),
        "the drawn line is construction"
    );
    assert!(
        !sketch.external[&projected].defining,
        "the projected one still guides"
    );

    h.event(
        WorkbenchInputEvent::ToolActivated,
        Some("sketch.construction"),
    );
    let sketch = h.sketch();
    assert!(!sketch.is_construction(drawn), "back to normal");
    assert!(
        sketch.external[&projected].defining && !sketch.is_construction(projected),
        "the projected line counts in the profile"
    );
    assert!(!h.wb.tool_toggled("sketch.construction"));
}

/// Half a circle projected from outside the sketch, closed by a line drawn
/// on its ends and made to count: one closed profile to extrude.
#[test]
fn a_projected_arc_closed_by_a_line_is_a_profile() {
    use wb_sketch::sketch::{Arc, ExternalReference, ExternalSource, Point, Vec2D};
    let mut h = Harness::new();
    let id = h.create_sketch();
    let mut feature = SketchFeature::from_json(h.doc.get_feature_data(id).unwrap()).unwrap();
    let sketch = &mut feature.sketch;
    let centre = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let start = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
    let end = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(-10.0, 0.0))));
    let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(centre, start, end, 10.0)));
    let source = ExternalSource::of_reference(ExternalReference::Datum {
        datum: uuid::Uuid::new_v4(),
    });
    for part in [centre, start, end, arc] {
        sketch.external.insert(part, source);
    }
    h.doc.update_feature_data(id, feature.to_json()).unwrap();
    h.key(KeyCode::A, None);

    // A line from end to end, clicked on the arc's ends.
    h.click(10.0, 0.0, "sketch.line");
    h.click(-10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    // The arc made normal geometry.
    h.click(0.0, 10.0, "sketch.select");
    h.event(
        WorkbenchInputEvent::ToolActivated,
        Some("sketch.construction"),
    );

    let sketch = h.sketch();
    assert!(sketch.external[&arc].defining, "the arc counts");
    let wires = wb_sketch::profile::extract_wires(&sketch).expect("a profile");
    assert_eq!(wires.len(), 1, "{wires:?}");
    assert_eq!(wires[0].segments.len(), 2, "the arc and the line");
}

/// A line drawn on the ends of a projected arc cannot move: the sketch has
/// no freedom left, and says so.
#[test]
fn a_line_on_projected_ends_leaves_no_freedom() {
    use wb_sketch::sketch::{Arc, ExternalReference, ExternalSource, Point, Vec2D};
    let mut h = Harness::new();
    let id = h.create_sketch();
    let mut feature = SketchFeature::from_json(h.doc.get_feature_data(id).unwrap()).unwrap();
    let sketch = &mut feature.sketch;
    let centre = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
    let start = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(10.0, 0.0))));
    let end = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(-10.0, 0.0))));
    let arc = sketch.add_geometry(GeometryElement::Arc(Arc::new(centre, start, end, 10.0)));
    let source = ExternalSource::of_reference(ExternalReference::Datum {
        datum: uuid::Uuid::new_v4(),
    });
    for part in [centre, start, end, arc] {
        sketch.external.insert(part, source);
    }
    h.doc.update_feature_data(id, feature.to_json()).unwrap();
    h.key(KeyCode::A, None);
    h.click(10.0, 0.0, "sketch.line");
    h.click(-10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    let badge = h.hud().and_then(|hud| hud.badge).map(|(_, text)| text);
    let sketch = h.sketch();
    let lines: Vec<_> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some((l.start, l.end)),
            _ => None,
        })
        .collect();
    assert_eq!(
        badge.as_deref(),
        Some("0 DoF"),
        "lines {lines:?}, arc ends {start} {end}"
    );
}

/// As with an arc, a line drawn on the ends of a projected spline cannot
/// move.
#[test]
fn a_line_on_projected_spline_ends_leaves_no_freedom() {
    use wb_sketch::sketch::{BSpline, ExternalReference, ExternalSource, Point, Vec2D};
    let mut h = Harness::new();
    let id = h.create_sketch();
    let mut feature = SketchFeature::from_json(h.doc.get_feature_data(id).unwrap()).unwrap();
    let sketch = &mut feature.sketch;
    let source = ExternalSource::of_reference(ExternalReference::Datum {
        datum: uuid::Uuid::new_v4(),
    });
    let mut poles = Vec::new();
    for (x, y) in [
        (10.0, 0.0),
        (10.0, 6.0),
        (4.0, 10.0),
        (-4.0, 10.0),
        (-10.0, 6.0),
        (-10.0, 0.0),
    ] {
        let p = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
        sketch.external.insert(p, source);
        poles.push(p);
    }
    let spline = sketch.add_geometry(GeometryElement::BSpline(BSpline::new(poles.clone(), false)));
    sketch.external.insert(spline, source);
    h.doc.update_feature_data(id, feature.to_json()).unwrap();
    h.key(KeyCode::A, None);
    h.click(10.0, 0.0, "sketch.line");
    h.click(-10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    let badge = h.hud().and_then(|hud| hud.badge).map(|(_, text)| text);
    let sketch = h.sketch();
    let lines: Vec<_> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some((l.start, l.end)),
            _ => None,
        })
        .collect();
    let constraints: Vec<_> = sketch
        .constraints
        .iter()
        .map(|c| format!("{:?}", c.kind))
        .collect();
    assert_eq!(
        badge.as_deref(),
        Some("0 DoF"),
        "lines {lines:?}, spline ends {} {}, constraints {constraints:?}",
        poles[0],
        poles[5]
    );
    assert!(
        sketch.is_fully_constrained,
        "drawn in the fully constrained colour"
    );
}

#[test]
fn construction_toggle_with_empty_selection_flips_mode() {
    let mut h = Harness::new();
    h.create_sketch();
    // Nothing selected: the action toggles construction *mode*.
    h.key(KeyCode::A, Some("sketch.construction"));
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    let sketch = h.sketch();
    assert!(
        sketch
            .geometry
            .iter()
            .all(|g| sketch.is_construction(g.id())),
        "all geometry drawn under construction mode is construction"
    );

    // Toggle the mode back off: new geometry is normal again.
    h.key(KeyCode::A, Some("sketch.construction"));
    h.click(30.0, 0.0, "sketch.line");
    h.click(40.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    let sketch = h.sketch();
    let construction = sketch
        .geometry
        .iter()
        .filter(|g| sketch.is_construction(g.id()))
        .count();
    assert_eq!(
        construction, 3,
        "only the first line's elements are flagged"
    );
    assert_eq!(sketch.geometry.len(), 6);
}

/// Construction mode is for what is drawn: a fillet on a normal profile
/// stays normal, or the profile would open.
#[test]
fn construction_mode_leaves_a_fillet_on_a_profile_normal() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.rect");
    h.click(12.0, 8.0, "sketch.rect");
    let before: Vec<uuid::Uuid> = h.sketch().geometry.iter().map(|g| g.id()).collect();

    h.key(KeyCode::A, Some("sketch.construction")); // mode ON, empty selection
    h.click(12.0, 8.0, "sketch.fillet");

    let sketch = h.sketch();
    let (new_flagged, old_flagged): (Vec<bool>, Vec<bool>) = (
        sketch
            .geometry
            .iter()
            .filter(|g| !before.contains(&g.id()))
            .map(|g| sketch.is_construction(g.id()))
            .collect(),
        sketch
            .geometry
            .iter()
            .filter(|g| before.contains(&g.id()))
            .map(|g| sketch.is_construction(g.id()))
            .collect(),
    );
    assert!(!new_flagged.is_empty(), "fillet added geometry");
    assert!(
        new_flagged.iter().all(|&c| !c),
        "the fillet's arc and points stay part of the profile"
    );
    assert!(
        old_flagged.iter().all(|&c| !c),
        "pre-existing rectangle untouched"
    );
}

#[test]
fn construction_toggle_with_selection_leaves_mode_untouched() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    // Select the line, then fire the action: only the line flips, and the
    // mode stays OFF.
    h.click(5.0, 3.5, "sketch.select");
    h.key(KeyCode::A, Some("sketch.construction"));
    let sketch = h.sketch();
    let line_id = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) => Some(l.id),
            _ => None,
        })
        .unwrap();
    assert!(sketch.is_construction(line_id));

    // New geometry stays normal: the selection path never touched the mode.
    h.key(KeyCode::Escape, Some("sketch.select")); // clear selection
    h.click(30.0, 0.0, "sketch.line");
    h.click(40.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    let sketch = h.sketch();
    let construction = sketch
        .geometry
        .iter()
        .filter(|g| sketch.is_construction(g.id()))
        .count();
    assert_eq!(construction, 1, "only the toggled line is construction");
}

#[test]
fn construction_on_a_mixed_selection_converts_all_of_it() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(0.0, 20.0, "sketch.line");
    h.click(10.0, 27.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    let sketch = h.sketch();
    let lines: Vec<uuid::Uuid> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some(l.id),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 2);

    // Pre-flag the first line only, then convert a selection of both: all
    // become construction, and a second press makes all normal.
    h.click(5.0, 3.5, "sketch.select");
    h.key(KeyCode::A, Some("sketch.construction"));
    assert!(h.sketch().is_construction(lines[0]));
    // Line 1 is still selected; add line 2 to the selection.
    h.click_ctrl(5.0, 23.5, "sketch.select");
    h.key(KeyCode::A, Some("sketch.construction"));
    let sketch = h.sketch();
    assert!(
        sketch.is_construction(lines[0]) && sketch.is_construction(lines[1]),
        "both construction"
    );
    h.key(KeyCode::A, Some("sketch.construction"));
    let sketch = h.sketch();
    assert!(
        !sketch.is_construction(lines[0]) && !sketch.is_construction(lines[1]),
        "both normal again"
    );
}

#[test]
fn editing_a_dimension_constraint_re_solves_the_sketch() {
    use wb_sketch::sketch::{Constraint, ConstraintKind};

    let mut h = Harness::new();
    let id = h.create_sketch();
    // Axis-snapped horizontal line: 10 long, auto Horizontal constraint.
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.05, "sketch.line");

    // Add a Length constraint directly on the stored feature (the panel's
    // "Add Constraint" buttons are egui-only) plus a fixed anchor.
    let sketch = h.sketch();
    let line = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) => Some(l.clone()),
            _ => None,
        })
        .unwrap();
    let anchor = sketch.point_position(line.start).unwrap();
    let mut feature = SketchFeature::from_json(h.doc.get_feature_data(id).unwrap()).unwrap();
    feature.sketch.add_constraint(ConstraintKind::FixedPoint {
        point: line.start,
        position: anchor,
    });
    feature.sketch.add_constraint(ConstraintKind::Length {
        line: line.id,
        length: 10.0,
    });
    let constraint_idx = feature.sketch.constraints.len() - 1;
    h.doc.update_feature_data(id, feature.to_json()).unwrap();

    // Edit the dimension through the panel's code path: replace in place,
    // re-solve, store.
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.active_document_object = h.active_object;
    h.wb.update_constraint(
        &mut ctx,
        constraint_idx,
        Constraint::new(ConstraintKind::Length {
            line: line.id,
            length: 20.0,
        }),
    );

    let sketch = h.sketch();
    let start = sketch.point_position(line.start).unwrap().to_glam();
    let end = sketch.point_position(line.end).unwrap().to_glam();
    assert!(
        ((end - start).length() - 20.0).abs() < 1e-3,
        "line re-solved to the edited length, got {}",
        (end - start).length()
    );
    assert!(
        matches!(
            sketch.constraints[constraint_idx].kind,
            ConstraintKind::Length { length, .. } if (length - 20.0).abs() < 1e-6
        ),
        "constraint value stored"
    );
}

/// Two disjoint lines for the selection tests: L1 (0,0)→(10,7),
/// L2 (0,20)→(10,27); mid-spans at (5,3.5) and (5,23.5).
fn two_lines(h: &mut Harness) {
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(0.0, 20.0, "sketch.line");
    h.click(10.0, 27.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    assert_eq!(h.counts(), (4, 2, 0, 0));
}

#[test]
fn clicking_a_second_element_adds_it_to_the_selection() {
    let mut h = Harness::new();
    two_lines(&mut h);
    // Click L1, then L2: selection inside a sketch accumulates with no
    // modifier, so Delete removes both.
    h.click(5.0, 3.5, "sketch.select");
    h.click(5.0, 23.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (0, 0), "both lines deleted with their endpoints");
}

#[test]
fn ctrl_click_still_accumulates() {
    let mut h = Harness::new();
    two_lines(&mut h);
    h.click(5.0, 3.5, "sketch.select");
    h.click_ctrl(5.0, 23.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (0, 0), "both lines deleted with their endpoints");
}

#[test]
fn clicking_a_selected_element_again_drops_it() {
    let mut h = Harness::new();
    two_lines(&mut h);
    h.click(5.0, 3.5, "sketch.select");
    h.click(5.0, 23.5, "sketch.select");
    // Click L1 again and release in place: it leaves the selection (the
    // press keeps it so a drag can carry it), L2 stays.
    h.click(5.0, 3.5, "sketch.select");
    h.release(5.0, 3.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (2, 1), "only L2 was still selected");
}

#[test]
fn an_empty_click_clears_the_selection() {
    let mut h = Harness::new();
    two_lines(&mut h);

    // A click on empty space is the gesture that clears, with or without
    // the modifier: Delete afterwards is a no-op.
    h.click(5.0, 3.5, "sketch.select");
    h.click_ctrl(50.0, 3.5, "sketch.select");
    h.release_ctrl(50.0, 3.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    assert_eq!(h.counts(), (4, 2, 0, 0), "ctrl empty click cleared it too");

    h.click(5.0, 23.5, "sketch.select");
    h.click(50.0, 3.5, "sketch.select");
    h.release(50.0, 3.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    assert_eq!(
        h.counts(),
        (4, 2, 0, 0),
        "empty click cleared the selection"
    );
}

#[test]
fn ctrl_click_accumulates_points_via_release() {
    let mut h = Harness::new();
    two_lines(&mut h);
    // Point selection resolves on release-without-move; ctrl at press time
    // makes it additive. Both endpoints of L1 selected → Delete cascades L1.
    h.click(0.0, 0.0, "sketch.select");
    h.release(0.0, 0.0, "sketch.select");
    h.click_ctrl(10.0, 7.0, "sketch.select");
    h.release_ctrl(10.0, 7.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (2, 1), "both L1 endpoints deleted, cascading L1");
}

#[test]
fn clicking_a_curve_then_a_point_selects_both() {
    let mut h = Harness::new();
    two_lines(&mut h);
    // Select L1 (curve), then click an L2 endpoint: both are selected, so
    // Delete takes L1 and cascades L2 from its endpoint.
    h.click(5.0, 3.5, "sketch.select");
    h.click(0.0, 20.0, "sketch.select");
    h.release(0.0, 20.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (0, 0), "both lines gone, with every endpoint");
}

/// `two_lines` plus a third line off to the right: L3 (20,0)→(28,7).
fn three_lines(h: &mut Harness) {
    two_lines(h);
    h.click(20.0, 0.0, "sketch.line");
    h.click(28.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    assert_eq!(h.counts(), (6, 3, 0, 0));
}

#[test]
fn box_selection_selects_fully_inside_elements() {
    let mut h = Harness::new();
    three_lines(&mut h);
    // Box around L1 and L2 (and their endpoints); L3 stays outside.
    h.click(-2.0, -2.0, "sketch.select");
    h.mouse_move(12.0, 28.5, "sketch.select");
    h.release(12.0, 28.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (2, 1), "L1+L2 and their endpoints deleted, L3 kept");
}

#[test]
fn box_selection_excludes_partially_covered_elements() {
    let mut h = Harness::new();
    three_lines(&mut h);
    // The box covers L1 fully but cuts L2 mid-span: only L1 (with its
    // endpoints) is selected; L2's inside endpoint at (0,20) also counts.
    h.click(-2.0, -2.0, "sketch.select");
    h.mouse_move(12.0, 23.0, "sketch.select");
    h.release(12.0, 23.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    // L1 + endpoints gone; L2 cascaded via its (0,20) endpoint, taking the
    // other one with it; L3 intact.
    assert_eq!((p, l), (2, 1), "straddling line cascades via its endpoint");
}

#[test]
fn a_box_adds_to_the_existing_selection() {
    let mut h = Harness::new();
    three_lines(&mut h);
    // Select L3, then box around L1+L2: everything is selected.
    h.click(24.0, 3.5, "sketch.select");
    h.click(-2.0, -2.0, "sketch.select");
    h.mouse_move(12.0, 28.5, "sketch.select");
    h.release(12.0, 28.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (0, 0), "every line and its endpoints deleted");
}

#[test]
fn tiny_box_drag_behaves_as_click_clear() {
    let mut h = Harness::new();
    two_lines(&mut h);
    h.click(5.0, 3.5, "sketch.select"); // select L1
    // Sub-threshold drag on empty space: plain empty click → clear.
    h.click(15.0, 10.0, "sketch.select");
    h.mouse_move(15.05, 10.05, "sketch.select");
    h.release(15.05, 10.05, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    assert_eq!(
        h.counts(),
        (4, 2, 0, 0),
        "selection was cleared, Delete no-op"
    );
}

#[test]
fn escape_cancels_box_selection_and_keeps_prior_selection() {
    let mut h = Harness::new();
    two_lines(&mut h);
    h.click(5.0, 3.5, "sketch.select"); // select L1
    // Start a box that would engulf L2, but cancel it with Escape.
    h.click(-2.0, 15.0, "sketch.select");
    h.mouse_move(12.0, 28.5, "sketch.select");
    h.key(KeyCode::Escape, Some("sketch.select"));
    h.release(12.0, 28.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!(
        (p, l),
        (2, 1),
        "box cancelled: only pre-selected L1 deleted, with its endpoints"
    );
}

#[test]
fn box_selection_draws_dashed_rectangle_overlay() {
    let mut h = Harness::new();
    h.create_sketch();
    let overlays_of = |h: &mut Harness| {
        let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(h.vp);
        ctx.active_document_object = h.active_object;
        h.wb.get_screen_space_overlays(&ctx, h.active_object).len()
    };
    let idle_count = overlays_of(&mut h); // axes only
    // Start clear of the axes and the origin, which are selectable.
    h.click(4.0, 4.0, "sketch.select");
    h.mouse_move(10.0, 8.0, "sketch.select");
    let box_count = overlays_of(&mut h);
    assert_eq!(
        box_count,
        idle_count + 4,
        "the box adds four dashed edges: {box_count} vs {idle_count}"
    );
    // Releasing removes the box again.
    h.release(10.0, 8.0, "sketch.select");
    assert_eq!(overlays_of(&mut h), idle_count);
}

#[test]
fn box_selection_covers_circles_and_arcs() {
    let mut h = Harness::new();
    h.create_sketch();
    // Circle center (0,0) r=5; arc centered (15,0) from (19,0) CCW to (15,6).
    h.click(0.0, 0.0, "sketch.circle");
    h.click(5.0, 0.0, "sketch.circle");
    h.click(15.0, 0.0, "sketch.arc");
    h.click(19.0, 0.0, "sketch.arc");
    h.click(15.0, 6.0, "sketch.arc");

    // Box around the circle's bbox only: the arc is outside.
    h.click(-6.0, -6.0, "sketch.select");
    h.mouse_move(6.0, 6.0, "sketch.select");
    h.release(6.0, 6.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (_, _, c, a) = h.counts();
    assert_eq!((c, a), (0, 1), "circle deleted, arc kept");

    // A flat box over the arc's center and start point does NOT select the
    // arc itself (its end point and angular midpoint stick out the top).
    // Verified via the construction toggle: only the contained points flip.
    h.click(13.0, -1.0, "sketch.select");
    h.mouse_move(20.0, 1.0, "sketch.select");
    h.release(20.0, 1.0, "sketch.select");
    h.key(KeyCode::A, Some("sketch.construction"));
    let sketch = h.sketch();
    let arc_id = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Arc(arc) => Some(arc.id),
            _ => None,
        })
        .unwrap();
    assert!(
        !sketch.is_construction(arc_id),
        "arc not fully inside the flat box: not selected, not flipped"
    );
    assert!(
        !sketch.construction.is_empty(),
        "the contained points WERE selected and flipped"
    );

    // A generous box around the whole arc selects it.
    h.click(9.0, -7.0, "sketch.select");
    h.mouse_move(21.0, 7.0, "sketch.select");
    h.release(21.0, 7.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, _, _, a) = h.counts();
    assert_eq!((p, a), (0, 0), "arc and its points deleted");
}

#[test]
fn ellipse_tool_extracts_ellipse_profile() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.ellipse"); // center
    h.click(10.0, 0.0, "sketch.ellipse"); // major vertex
    h.click(5.0, 4.0, "sketch.ellipse"); // minor extent → ratio 0.4
    let sketch = h.sketch();
    let ellipse = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Ellipse(e) => Some(e.clone()),
            _ => None,
        })
        .expect("ellipse created");
    assert!(
        (ellipse.ratio - 0.4).abs() < 0.01,
        "ratio {}",
        ellipse.ratio
    );
    let wires = wb_sketch::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
    assert!(matches!(
        &wires[0].segments[0],
        kernel_api::ProfileSegment::Ellipse { major, ratio, .. }
            if (major[0] - 10.0).abs() < 0.05 && (ratio - 0.4).abs() < 0.01
    ));
}

#[test]
fn bspline_draw_closes_profile_with_line() {
    let mut h = Harness::new();
    h.create_sketch();
    // Three control points, finished with a right click.
    h.click(0.0, 0.0, "sketch.bspline");
    h.click(5.0, 6.0, "sketch.bspline");
    h.click(10.0, 0.0, "sketch.bspline");
    h.right_click(10.0, 0.0, "sketch.bspline");
    // Close the open spline with a line snapped to its end control points.
    h.click(10.0, 0.0, "sketch.line");
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    let sketch = h.sketch();
    assert_eq!(sketch.geometry.len(), 5, "3 points + spline + line");
    let wires = wb_sketch::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 2);
    assert!(wires[0].segments.iter().any(|s| matches!(
        s,
        kernel_api::ProfileSegment::BSpline { control_points, periodic: false }
            if control_points.len() == 3
    )));
}

#[test]
fn trim_middle_span_leaves_two_lines_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    // Horizontal target and two vertical cutters.
    h.click(0.0, 0.0, "sketch.line");
    h.click(20.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(5.0, -5.0, "sketch.line");
    h.click(5.0, 5.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(15.0, -5.0, "sketch.line");
    h.click(15.0, 5.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    assert_eq!(h.counts(), (6, 3, 0, 0));

    h.click(10.0, 0.0, "sketch.trim");
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (8, 4), "middle span removed, two halves left");
}

#[test]
fn extend_line_to_intersection_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(5.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(10.0, -5.0, "sketch.line");
    h.click(10.0, 5.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    // Click the end half of the short line.
    h.click(4.0, 0.0, "sketch.extend");
    let sketch = h.sketch();
    let reached = sketch.geometry.iter().any(|g| match g {
        GeometryElement::Point(p) => {
            (p.position.x - 10.0).abs() < 0.05 && p.position.y.abs() < 0.05
        }
        _ => false,
    });
    assert!(reached, "endpoint moved onto the wall");
    assert_eq!(h.counts(), (4, 2, 0, 0), "no new geometry, endpoint moved");
}

#[test]
fn split_line_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    h.click(4.0, 0.0, "sketch.split");
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (3, 2), "split point shared by both halves");
}

#[test]
fn offset_rectangle_end_to_end_produces_closed_loop() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.rect");
    h.click(12.0, 8.0, "sketch.rect");
    h.box_select(-2.0, -2.0, 14.0, 10.0);
    h.wb.tool_params_mut().offset_distance = 2.0;
    // Click inside the rectangle: the offset loop shrinks inward.
    h.click(6.0, 4.0, "sketch.offset");

    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (8, 8));
    let wires = wb_sketch::profile::extract_wires(&h.sketch()).unwrap();
    assert_eq!(wires.len(), 2, "original + offset are both closed loops");
    let inset = h.sketch().geometry.iter().any(|g| match g {
        GeometryElement::Point(p) => {
            (p.position.x - 2.0).abs() < 0.05 && (p.position.y - 2.0).abs() < 0.05
        }
        _ => false,
    });
    assert!(inset, "offset corner 2mm inside the original");
}

#[test]
fn translate_copy_makes_n_copies_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(8.0, 3.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(4.0, 1.5, "sketch.select"); // select the line
    h.wb.tool_params_mut().copies = 2;

    h.click(0.0, 15.0, "sketch.translate"); // base
    h.click(10.0, 15.0, "sketch.translate"); // destination: Δ = (10, 0)
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (6, 3), "original + 2 copies");
    // Second copy endpoint at (28, 3).
    let far = h.sketch().geometry.iter().any(|g| match g {
        GeometryElement::Point(p) => {
            (p.position.x - 28.0).abs() < 0.05 && (p.position.y - 3.0).abs() < 0.05
        }
        _ => false,
    });
    assert!(far, "second copy landed at 2Δ");
}

#[test]
fn mirror_about_line_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    // Mirror axis along the x-axis, subject line above it.
    h.click(-10.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(2.0, 2.0, "sketch.line");
    h.click(8.0, 5.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(5.0, 3.5, "sketch.select"); // select the subject line

    // One click on the axis line (away from any point) mirrors immediately.
    h.click(0.0, 0.0, "sketch.mirror");
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (6, 3), "mirrored copy added");
    let mirrored = h.sketch().geometry.iter().any(|g| match g {
        GeometryElement::Point(p) => {
            (p.position.x - 8.0).abs() < 0.05 && (p.position.y + 5.0).abs() < 0.05
        }
        _ => false,
    });
    assert!(mirrored, "endpoint mirrored to (8, -5)");
}

#[test]
fn arc3_and_circle3_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(5.0, 0.0, "sketch.arc3");
    h.click(0.0, 5.0, "sketch.arc3");
    h.click(3.5355, 3.5355, "sketch.arc3"); // rim point on the CCW side
    h.click(12.0, 0.0, "sketch.circle3");
    h.click(18.0, 0.0, "sketch.circle3");
    h.click(12.0, 8.0, "sketch.circle3");
    let (_, _, c, a) = h.counts();
    assert_eq!((c, a), (1, 1));
    let sketch = h.sketch();
    let arc_r = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Arc(arc) => Some(arc.radius),
            _ => None,
        })
        .unwrap();
    // The circumcircle is sensitive to the pixel-rounded rim click.
    assert!((arc_r - 5.0).abs() < 0.2, "arc radius {arc_r}");
    let circle_r = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Circle(c) => Some(c.radius),
            _ => None,
        })
        .unwrap();
    assert!((circle_r - 5.0).abs() < 0.05, "circle radius {circle_r}");
}

#[test]
fn rect_center_tool_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(5.0, 3.0, "sketch.rect_center");
    h.click(9.0, 5.0, "sketch.rect_center");
    let (p, l, _, _) = h.counts();
    // Four corners and the centre they are symmetric about.
    assert_eq!((p, l), (5, 4));
    assert_eq!(
        h.sketch().constraints.len(),
        5,
        "H/V constraints as rect, and the symmetry"
    );
    let wires = wb_sketch::profile::extract_wires(&h.sketch()).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 4);
}

#[test]
fn arc_slot_tool_end_to_end_closed_profile() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.arc_slot"); // arc center
    h.click(8.0, 0.0, "sketch.arc_slot"); // centerline start (r = 8)
    h.click(0.0, 8.0, "sketch.arc_slot"); // quarter-turn end
    let (p, l, _, a) = h.counts();
    assert_eq!((p, l, a), (7, 0, 4), "rails + caps");
    let wires = wb_sketch::profile::extract_wires(&h.sketch()).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 4);
}

#[test]
fn chamfer_tool_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.rect");
    h.click(12.0, 8.0, "sketch.rect");
    h.click(12.0, 8.0, "sketch.chamfer"); // default length 2
    let (p, l, _, a) = h.counts();
    assert_eq!((p, l, a), (5, 5, 0), "corner replaced by chamfer line");
    let wires = wb_sketch::profile::extract_wires(&h.sketch()).unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 5);
}

#[test]
fn trim_hover_highlights_removable_span() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    // Hover the line with the trim tool active.
    h.mouse_move(5.0, 0.0, "sketch.trim");
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.active_document_object = h.active_object;
    let overlays = h.wb.get_screen_space_overlays(&ctx, h.active_object);
    let highlight = overlays
        .iter()
        .filter(|o| o.thickness > 2.5 && same_color(o.color, pal().trim))
        .count();
    assert!(
        highlight >= 1,
        "trim hover draws the removable span in the trim color"
    );
}

#[test]
fn clicking_near_a_line_attaches_new_point_onto_it() {
    use wb_sketch::sketch::ConstraintKind;

    let mut h = Harness::new();
    h.create_sketch();
    // Base line along X (gets an auto Horizontal from the axis snap).
    // Off the axes: a point there would pin to the origin or an axis.
    h.click(3.0, 4.0, "sketch.line");
    h.click(23.0, 4.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    // Start a new line just off the base line, away from its ends and its
    // middle: the start point is projected ONTO the line and constrained
    // to it.
    h.click(8.0, 4.5, "sketch.line");
    h.click(17.0, 12.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    let sketch = h.sketch();
    let on_line = sketch
        .constraints
        .iter()
        .find_map(|c| match c.kind {
            ConstraintKind::PointOnLine { point, line } => Some((point, line)),
            _ => None,
        })
        .expect("curve snap auto-added a point-on-line constraint");
    let (point, _line) = on_line;
    let p = sketch.point_position(point).unwrap();
    assert!(
        (p.x - 8.0).abs() < 0.1 && (p.y - 4.0).abs() < 1e-3,
        "start point projected onto the base line, got ({}, {})",
        p.x,
        p.y
    );
    // The shared-endpoint path is untouched: no coincident duplicates.
    let (points, lines, _, _) = h.counts();
    assert_eq!((points, lines), (4, 2));
}

#[test]
fn fully_constrained_sketch_renders_green() {
    use wb_sketch::sketch::{Constraint, ConstraintKind, Vec2D};

    let mut h = Harness::new();
    let id = h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    // Fix both endpoints on the stored feature, then trigger the panel's
    // re-solve path so `is_fully_constrained` updates.
    let mut feature = SketchFeature::from_json(h.doc.get_feature_data(id).unwrap()).unwrap();
    let points: Vec<(uuid::Uuid, Vec2D)> = feature
        .sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some((p.id, p.position)),
            _ => None,
        })
        .collect();
    for (point, position) in &points {
        feature.sketch.add_constraint(ConstraintKind::FixedPoint {
            point: *point,
            position: *position,
        });
    }
    h.doc.update_feature_data(id, feature.to_json()).unwrap();
    let first = h.sketch().constraints[0].clone();
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.active_document_object = h.active_object;
    h.wb.update_constraint(&mut ctx, 0, Constraint::new(first.kind.clone()));

    assert!(h.sketch().is_fully_constrained);
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.active_document_object = h.active_object;
    let overlays = h.wb.get_screen_space_overlays(&ctx, h.active_object);
    let green = overlays
        .iter()
        .filter(|o| same_color(o.color, pal().fully_constrained))
        .count();
    assert!(
        green >= 1,
        "fully constrained geometry drawn in the fully-constrained green"
    );
}

#[test]
fn no_geometry_created_without_active_sketch() {
    let mut h = Harness::new();
    // No create_sketch: clicks must be no-ops without a panic.
    let viewport_pos = (400.0, 300.0);
    h.event(
        WorkbenchInputEvent::MousePress {
            button: MouseButton::Left,
            viewport_pos,
        },
        Some("sketch.line"),
    );
    assert!(h.active_object.is_none());
    assert_eq!(h.doc.feature_tree().all_nodes().count(), 0);
}

// ---------------------------------------------------------------------------
// On-view parameters (type-to-constrain)
// ---------------------------------------------------------------------------

use wb_sketch::sketch::ConstraintKind;

fn distance_constraints(sketch: &Sketch) -> Vec<(f32, bool)> {
    sketch
        .constraints
        .iter()
        .filter_map(|c| match c.kind {
            ConstraintKind::Distance { distance, .. } => Some((distance, c.driving)),
            _ => None,
        })
        .collect()
}

#[test]
fn typed_length_enter_creates_line_with_driving_distance() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Key2, Some("sketch.line"));
    h.key(KeyCode::Key5, Some("sketch.line"));
    h.key(KeyCode::Enter, Some("sketch.line"));

    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (2, 1), "Enter committed the pending point");
    let sketch = h.sketch();
    let pts: Vec<_> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(pt) => Some(pt.position),
            _ => None,
        })
        .collect();
    let len = (pts[1] - pts[0]).to_glam().length();
    assert!((len - 25.0).abs() < 1e-3, "line length {len}");
    let dims = distance_constraints(&sketch);
    assert_eq!(dims.len(), 1);
    assert!((dims[0].0 - 25.0).abs() < 1e-5 && dims[0].1, "driving 25");
}

#[test]
fn tab_focuses_angle_field_and_click_keeps_typed_angle_free() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Tab, Some("sketch.line")); // focus: angle
    h.key(KeyCode::Key4, Some("sketch.line"));
    h.key(KeyCode::Key5, Some("sketch.line"));
    h.click(10.0, 0.0, "sketch.line"); // length from cursor, direction typed

    let sketch = h.sketch();
    assert!(distance_constraints(&sketch).is_empty(), "no length typed");
    assert!(
        !sketch
            .constraints
            .iter()
            .any(|c| matches!(c.kind, ConstraintKind::AngleToAxis { .. })),
        "a click shapes the geometry from the typed angle but leaves it free"
    );
    let end = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position),
            _ => None,
        })
        .find(|p| p.to_glam().length() > 1.0)
        .unwrap();
    let want = 10.0 * std::f32::consts::FRAC_1_SQRT_2;
    assert!(
        (end.x - want).abs() < 0.1 && (end.y - want).abs() < 0.1,
        "end followed the typed 45° direction: ({}, {})",
        end.x,
        end.y
    );
}

#[test]
fn typed_length_and_angle_commit_exact_polar() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Key2, Some("sketch.line"));
    h.key(KeyCode::Key0, Some("sketch.line"));
    h.key(KeyCode::Tab, Some("sketch.line"));
    h.key(KeyCode::Key9, Some("sketch.line"));
    h.key(KeyCode::Key0, Some("sketch.line"));
    h.key(KeyCode::Enter, Some("sketch.line"));

    let sketch = h.sketch();
    let end = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position),
            _ => None,
        })
        .find(|p| p.to_glam().length() > 1.0)
        .expect("end point");
    assert!(end.x.abs() < 1e-3 && (end.y - 20.0).abs() < 1e-3);
    assert_eq!(distance_constraints(&sketch), vec![(20.0, true)]);
    assert!(sketch.constraints.iter().any(|c| matches!(
        c.kind,
        ConstraintKind::AngleToAxis { angle_rad, .. }
            if (angle_rad.to_degrees() - 90.0).abs() < 1e-3
    )));
}

#[test]
fn rect_typed_width_height_creates_edge_lengths() {
    let mut h = Harness::new();
    h.create_sketch();
    // Off the axes: a point there would pin to the origin or an axis.
    h.click(3.0, 4.0, "sketch.rect");
    h.key(KeyCode::Key1, Some("sketch.rect"));
    h.key(KeyCode::Key2, Some("sketch.rect"));
    h.key(KeyCode::Tab, Some("sketch.rect"));
    h.key(KeyCode::Key8, Some("sketch.rect"));
    h.key(KeyCode::Enter, Some("sketch.rect"));

    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (4, 4));
    let sketch = h.sketch();
    let corner = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(pt) => Some(pt.position),
            _ => None,
        })
        .find(|p| (p.x - 15.0).abs() < 1e-3 && (p.y - 12.0).abs() < 1e-3);
    assert!(corner.is_some(), "opposite corner 12 across and 8 up");
    let mut lengths: Vec<f32> = sketch
        .constraints
        .iter()
        .filter_map(|c| match c.kind {
            ConstraintKind::Length { length, .. } => Some(length),
            _ => None,
        })
        .collect();
    lengths.sort_by(f32::total_cmp);
    assert_eq!(
        lengths,
        vec![8.0, 12.0],
        "width on the bottom edge, height on the right"
    );
    assert_eq!(sketch.constraints.len(), 6, "4 H/V + two edge lengths");
}

#[test]
fn circle_typed_diameter_creates_diameter_constraint() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.circle");
    h.key(KeyCode::Key1, Some("sketch.circle"));
    h.key(KeyCode::Key0, Some("sketch.circle"));
    h.key(KeyCode::Enter, Some("sketch.circle"));

    let sketch = h.sketch();
    let radius = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Circle(c) => Some(c.radius),
            _ => None,
        })
        .expect("circle committed");
    assert!((radius - 5.0).abs() < 1e-3, "radius {radius}");
    assert!(sketch.constraints.iter().any(|c| matches!(
        c.kind,
        ConstraintKind::Diameter { diameter, .. } if (diameter - 10.0).abs() < 1e-5
    ) && c.driving));
}

#[test]
fn slot_typed_length_creates_center_distance_constraint() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.slot");
    h.key(KeyCode::Key1, Some("sketch.slot"));
    h.key(KeyCode::Key2, Some("sketch.slot"));
    h.key(KeyCode::Enter, Some("sketch.slot"));

    let (p, l, _, a) = h.counts();
    assert_eq!((p, l, a), (6, 2, 2), "slot committed");
    assert_eq!(distance_constraints(&h.sketch()), vec![(12.0, true)]);
}

#[test]
fn arc_typed_radius_defers_constraint_until_arc_commits() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.arc"); // center
    h.key(KeyCode::Key5, Some("sketch.arc"));
    h.key(KeyCode::Enter, Some("sketch.arc")); // start point at typed radius
    assert!(
        h.sketch().constraints.is_empty(),
        "no arc yet, no constraint"
    );
    h.click(0.0, 5.0, "sketch.arc"); // end click materializes the arc

    let sketch = h.sketch();
    let radius = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Arc(a) => Some(a.radius),
            _ => None,
        })
        .expect("arc committed");
    assert!((radius - 5.0).abs() < 1e-3, "radius {radius}");
    assert!(sketch.constraints.iter().any(|c| matches!(
        c.kind,
        ConstraintKind::Radius { radius, .. } if (radius - 5.0).abs() < 1e-5
    )));
}

#[test]
fn polygon_typed_radius_creates_construction_circumcircle() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.polygon");
    h.key(KeyCode::Key6, Some("sketch.polygon"));
    h.key(KeyCode::Enter, Some("sketch.polygon"));

    let sketch = h.sketch();
    let (p, l, c, _) = h.counts();
    assert_eq!((p, l, c), (7, 6, 1), "hexagon + center + circumcircle");
    let circle = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Circle(circ) => Some(circ),
            _ => None,
        })
        .unwrap();
    assert!((circle.radius - 6.0).abs() < 1e-3);
    assert!(
        sketch.is_construction(circle.id),
        "circumcircle is construction"
    );
    assert!(sketch.constraints.iter().any(|c| matches!(
        c.kind,
        ConstraintKind::Radius { radius, .. } if (radius - 6.0).abs() < 1e-5
    )));
    // The vertices sit on the typed circumradius.
    let on_radius = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(pt) => Some(pt.position),
            _ => None,
        })
        .filter(|p| (p.to_glam().length() - 6.0).abs() < 1e-3)
        .count();
    assert_eq!(on_radius, 6);
}

#[test]
fn escape_clears_typed_buffer_then_cancels_tool() {
    let mut h = Harness::new();
    h.create_sketch();
    // Off the axes: a point there would pin to the origin or an axis.
    h.click(3.0, 4.0, "sketch.line");
    h.key(KeyCode::Key9, Some("sketch.line"));
    h.key(KeyCode::Escape, Some("sketch.line")); // clears the buffer only
    h.click(13.0, 10.0, "sketch.line"); // commits at the cursor, unconstrained

    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (2, 1), "tool survived the first Escape");
    assert!(
        h.sketch().constraints.is_empty(),
        "typed value was discarded"
    );

    h.key(KeyCode::Escape, Some("sketch.line")); // empty buffer: cancels chain
    h.key(KeyCode::Escape, Some("sketch.line")); // idempotent
    assert_eq!(h.counts(), (2, 1, 0, 0));
}

#[test]
fn backspace_edits_typed_buffer_before_deleting_geometry() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Key2, Some("sketch.line"));
    h.key(KeyCode::Key9, Some("sketch.line"));
    h.key(KeyCode::Backspace, Some("sketch.line")); // "29" → "2"
    h.key(KeyCode::Key0, Some("sketch.line")); // "20"
    h.key(KeyCode::Enter, Some("sketch.line"));
    assert_eq!(distance_constraints(&h.sketch()), vec![(20.0, true)]);
}

// ---------------------------------------------------------------------------
// Constraint glyphs
// ---------------------------------------------------------------------------

#[test]
fn glyph_click_selects_constraint_and_delete_removes_it() {
    let mut h = Harness::new();
    h.create_sketch();
    // Off the axes: a point there would pin to the origin or an axis.
    h.click(3.0, 4.0, "sketch.line");
    h.click(18.0, 4.05, "sketch.line"); // axis snap → auto Horizontal
    h.key(KeyCode::Escape, Some("sketch.line"));
    assert_eq!(h.sketch().constraints.len(), 1);

    let marks = h.marks();
    let glyph = marks
        .iter()
        .find(|m| is_icon(m, "constraint-horizontal"))
        .expect("H glyph drawn");
    h.press_px((glyph.pos[0], glyph.pos[1]));
    h.key(KeyCode::Delete, Some("sketch.select"));

    assert!(h.sketch().constraints.is_empty(), "constraint deleted");
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (2, 1), "geometry untouched");
}

#[test]
fn geometry_delete_still_works_when_no_constraint_selected() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(15.0, 0.05, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    // Click the line away from its H glyph (which sits near the midpoint).
    h.click(3.0, 0.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (0, 0), "line deleted with its endpoints");
    assert!(
        h.sketch().constraints.is_empty(),
        "constraint cascaded away"
    );
}

/// Escape during a label drag leaves the label where it was.
#[test]
fn escape_puts_a_dragged_label_back() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Key2, Some("sketch.line"));
    h.key(KeyCode::Key5, Some("sketch.line"));
    h.key(KeyCode::Enter, Some("sketch.line"));
    h.key(KeyCode::Escape, Some("sketch.line"));

    let labels = h.labels();
    let dim = labels
        .iter()
        .find(|l| l.background && l.text == "25")
        .expect("dimension label drawn")
        .clone();
    h.press_px((dim.pos[0], dim.pos[1]));
    h.mouse_move(5.0, 8.0, "sketch.select");
    h.key(KeyCode::Escape, Some("sketch.select"));
    h.release(5.0, 8.0, "sketch.select");
    let back = h.labels();
    let label = back.iter().find(|l| l.text == "25").unwrap();
    assert_eq!(label.pos, dim.pos);
    assert!(
        h.sketch()
            .constraints
            .iter()
            .all(|c| c.label_offset.is_none()),
        "nothing was stored"
    );
}

#[test]
fn dimension_label_drag_updates_label_offset() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Key2, Some("sketch.line"));
    h.key(KeyCode::Key5, Some("sketch.line"));
    h.key(KeyCode::Enter, Some("sketch.line"));
    h.key(KeyCode::Escape, Some("sketch.line")); // end the chain

    let labels = h.labels();
    let dim = labels
        .iter()
        .find(|l| l.background && l.text == "25")
        .expect("dimension label drawn");
    h.press_px((dim.pos[0], dim.pos[1]));
    let seq = h.doc.mutation_seq();
    h.mouse_move(5.0, 6.0, "sketch.select");
    h.mouse_move(5.0, 8.0, "sketch.select");
    assert_eq!(
        h.doc.mutation_seq(),
        seq,
        "nothing is written while dragging"
    );
    assert!(
        h.labels()
            .iter()
            .all(|l| l.text != "25" || l.pos != dim.pos),
        "the label follows the drag"
    );
    h.release(5.0, 8.0, "sketch.select");
    assert_eq!(h.doc.mutation_seq(), seq + 1, "one write for the drag");

    let sketch = h.sketch();
    let offset = sketch
        .constraints
        .iter()
        .find(|c| matches!(c.kind, ConstraintKind::Distance { .. }))
        .and_then(|c| c.label_offset)
        .expect("drag stored a label offset");
    // The label lands where the cursor stopped: offset = cursor − anchor,
    // anchor being the line midpoint (12.5, 0).
    assert!(
        (offset.x + 7.5).abs() < 0.2 && (offset.y - 8.0).abs() < 0.2,
        "offset ({}, {})",
        offset.x,
        offset.y
    );
}

#[test]
fn double_click_dimension_glyph_opens_editor_and_commit_applies() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Key2, Some("sketch.line"));
    h.key(KeyCode::Key5, Some("sketch.line"));
    h.key(KeyCode::Enter, Some("sketch.line"));
    h.key(KeyCode::Escape, Some("sketch.line"));

    let labels = h.labels();
    let dim = labels
        .iter()
        .find(|l| l.background && l.text == "25")
        .expect("dimension label drawn");
    let pos = (dim.pos[0], dim.pos[1]);
    h.press_px(pos);
    h.release_px(pos);
    h.press_px(pos);

    let edit =
        h.wb.pending_dim_edit()
            .expect("double-click opened the editor");
    assert_eq!(edit.text, "25");
    assert!(edit.driving);

    // Type a new value and commit through the same path the popup uses.
    h.wb.pending_dim_edit_mut().unwrap().text = "30".to_string();
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.active_document_object = h.active_object;
    h.wb.commit_dim_edit(&mut ctx);

    let sketch = h.sketch();
    assert!(h.wb.pending_dim_edit().is_none(), "editor closed on commit");
    assert_eq!(distance_constraints(&sketch), vec![(30.0, true)]);
    let pts: Vec<_> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position),
            _ => None,
        })
        .collect();
    let len = (pts[1] - pts[0]).to_glam().length();
    assert!(
        (len - 30.0).abs() < 1e-2,
        "re-solved to the edited value: {len}"
    );
}

#[test]
fn dim_edit_cancel_leaves_constraint_untouched() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Key2, Some("sketch.line"));
    h.key(KeyCode::Key5, Some("sketch.line"));
    h.key(KeyCode::Enter, Some("sketch.line"));
    h.key(KeyCode::Escape, Some("sketch.line"));

    let labels = h.labels();
    let dim = labels.iter().find(|l| l.background).unwrap();
    let pos = (dim.pos[0], dim.pos[1]);
    h.press_px(pos);
    h.release_px(pos);
    h.press_px(pos);
    assert!(h.wb.pending_dim_edit().is_some());
    h.wb.pending_dim_edit_mut().unwrap().text = "99".to_string();
    h.wb.cancel_dim_edit();
    assert!(h.wb.pending_dim_edit().is_none());
    assert_eq!(distance_constraints(&h.sketch()), vec![(25.0, true)]);
}

#[test]
fn glyph_click_keeps_geometry_selection() {
    let mut h = Harness::new();
    h.create_sketch();
    // Off the axes: a point there would pin to the origin or an axis.
    h.click(3.0, 4.0, "sketch.line");
    h.click(18.0, 4.05, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(30.0, 30.0, "sketch.line");
    h.click(40.0, 38.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    // Select the first line, then ctrl-click its H glyph: both stay
    // selected, and Delete takes both in one step.
    h.click(6.0, 4.0, "sketch.select");
    let marks = h.marks();
    let glyph = marks
        .iter()
        .find(|m| is_icon(m, "constraint-horizontal"))
        .unwrap();
    let pos = (glyph.pos[0], glyph.pos[1]);
    h.event_with_ctrl(
        WorkbenchInputEvent::MousePress {
            button: MouseButton::Left,
            viewport_pos: pos,
        },
        Some("sketch.select"),
        true,
    );
    h.key(KeyCode::Delete, Some("sketch.select"));
    assert!(
        !h.sketch()
            .constraints
            .iter()
            .any(|c| matches!(c.kind, ConstraintKind::Horizontal { .. })),
        "the constraint went"
    );
    let (p, l, _, _) = h.counts();
    assert_eq!(
        (p, l),
        (2, 1),
        "the selected line went with it, the other stays"
    );
}

#[test]
fn selected_constraint_highlights_glyph_and_geometry() {
    let mut h = Harness::new();
    h.create_sketch();
    // Off the axes: a point there would pin to the origin or an axis.
    h.click(3.0, 4.0, "sketch.line");
    h.click(18.0, 4.05, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    let marks = h.marks();
    let glyph = marks
        .iter()
        .find(|m| is_icon(m, "constraint-horizontal"))
        .unwrap();
    let pos = (glyph.pos[0], glyph.pos[1]);
    h.press_px(pos);

    // Glyph turns selection green.
    let marks = h.marks();
    let glyph = marks
        .iter()
        .find(|m| is_icon(m, "constraint-horizontal"))
        .unwrap();
    assert!(
        same_color(glyph.color, pal().selected),
        "selected glyph tinted, got {:?}",
        glyph.color
    );
    // Referenced line drawn in the constraint colour: shown, not selected.
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.active_document_object = h.active_object;
    let overlays = h.wb.get_screen_space_overlays(&ctx, h.active_object);
    let referenced_lines = overlays
        .iter()
        .filter(|o| same_color(o.color, pal().constraint))
        .count();
    assert!(referenced_lines >= 1, "referenced geometry highlighted");
    assert!(
        !overlays.iter().any(|o| same_color(o.color, pal().selected)),
        "but not as if selected"
    );
}

#[test]
fn live_readouts_follow_the_cursor_while_drawing() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.mouse_move(3.0, 4.0, "sketch.line");
    let hud = h.hud().expect("hud while editing");
    let ovp = hud
        .ovp
        .expect("on-view parameters while a line is in progress");
    let length = ovp
        .rows
        .iter()
        .find(|r| r.label == "L")
        .expect("length readout");
    assert!(
        length.value.contains("5.00"),
        "live length: {}",
        length.value
    );
    assert_eq!(length.unit, "mm");
    assert!(
        ovp.rows.iter().any(|r| r.label == "∠"),
        "angle readout present"
    );
    let hint = hud.tool.expect("tool hint");
    assert_eq!(hint.name, "Line");

    // Typing locks the focused field with the typed buffer.
    h.key(KeyCode::Key7, Some("sketch.line"));
    let ovp = h.hud().unwrap().ovp.unwrap();
    let length = ovp.rows.iter().find(|r| r.label == "L").unwrap();
    assert!(
        length.locked && length.value == "7",
        "typed buffer shown: {length:?}"
    );
}

#[test]
fn dragging_a_line_carries_both_ends_and_the_line_attached_to_it() {
    let mut h = Harness::new();
    h.create_sketch();
    // A chain off the axes: L1 (3,4)→(13,4), L2 (13,4)→(13,14). They share the corner
    // point, so moving L1 has to bring L2's start with it.
    h.click(3.0, 4.0, "sketch.line");
    h.click(13.0, 4.0, "sketch.line");
    h.click(13.0, 14.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    // Grab L1 mid-span and move it 4 up.
    h.drag((8.0, 4.0), (8.0, 8.0));

    assert!(h.point_at(3.0, 8.0), "the free end followed the drag");
    assert!(h.point_at(13.0, 8.0), "the shared corner followed too");
    assert!(
        h.point_at(13.0, 14.0),
        "the far end of the attached line stayed put, so it stretched"
    );
}

#[test]
fn dragging_inside_a_selection_moves_every_selected_element() {
    let mut h = Harness::new();
    // Two free lines off the axes, where nothing pins them.
    h.create_sketch();
    h.click(3.0, 4.0, "sketch.line");
    h.click(13.0, 11.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(3.0, 24.0, "sketch.line");
    h.click(13.0, 31.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    // Select both lines, then drag from a point of one of them.
    h.click(8.0, 7.5, "sketch.select");
    h.click(8.0, 27.5, "sketch.select");
    h.drag((8.0, 7.5), (8.0, 12.5));

    assert!(h.point_at(3.0, 9.0), "L1 moved by the drag delta");
    assert!(h.point_at(3.0, 29.0), "L2 came along with the selection");
}

#[test]
fn a_drag_below_the_snap_tolerance_leaves_the_geometry_alone() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    h.drag((5.0, 0.0), (5.05, 0.05));
    assert!(
        h.point_at(0.0, 0.0) && h.point_at(10.0, 0.0),
        "nothing moved"
    );
}

#[test]
fn right_click_on_an_idle_tool_asks_for_the_select_tool() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    // Mid-chain the right click ends the chain and keeps the tool.
    h.right_click(10.0, 0.0, "sketch.line");
    assert_eq!(h.tool_request, None, "the chain ended, the tool stayed");

    // With nothing in flight it hands the pointer back to Select.
    h.right_click(20.0, 20.0, "sketch.line");
    assert_eq!(h.tool_request.as_deref(), Some("sketch.select"));
}

#[test]
fn a_right_drag_pans_instead_of_changing_the_tool() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    h.right_drag((20.0, 20.0), (30.0, 26.0), "sketch.line");
    assert_eq!(h.tool_request, None, "the pan left the tool alone");
}

#[test]
fn selecting_two_lines_enables_the_constraint_tools_that_fit_them() {
    let mut h = Harness::new();
    two_lines(&mut h);
    assert!(
        !h.tool_enabled("sketch.constrain.parallel"),
        "nothing selected, nothing to constrain"
    );

    h.click(5.0, 3.5, "sketch.select");
    h.click(5.0, 23.5, "sketch.select");

    assert!(
        h.tool_enabled("sketch.constrain.parallel"),
        "two lines fit a parallel constraint"
    );
    assert!(h.tool_enabled("sketch.constrain.equal"));
    assert!(
        !h.tool_enabled("sketch.constrain.radius"),
        "a radius needs a circle"
    );
}

#[test]
fn a_point_can_be_pinned_to_the_origin_from_the_viewport() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(6.0, 4.0, "sketch.point");

    // Select the point, then the crossing of the axes.
    h.click(6.0, 4.0, "sketch.select");
    h.release(6.0, 4.0, "sketch.select");
    h.click(0.0, 0.0, "sketch.select");
    h.release(0.0, 0.0, "sketch.select");
    assert!(
        h.tool_enabled("sketch.constrain.coincident"),
        "a point and the origin fit a coincident constraint"
    );

    h.key(KeyCode::A, Some("sketch.constrain.coincident"));
    let sketch = h.sketch();
    assert!(
        sketch.constraints.iter().any(|c| matches!(
            c.kind,
            ConstraintKind::Coincident { point2, .. } if point2 == wb_sketch::sketch::ORIGIN_ID
        )),
        "the constraint points at the origin"
    );
    assert!(h.point_at(0.0, 0.0), "and the point moved onto it");
}

#[test]
fn a_line_can_be_squared_against_an_axis() {
    let mut h = Harness::new();
    h.create_sketch();
    // A line that is clearly not horizontal, away from the axes.
    h.click(4.0, 4.0, "sketch.line");
    h.click(12.0, 9.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    h.click(8.0, 6.5, "sketch.select");
    h.click(20.0, 0.0, "sketch.select"); // the X axis, clear of the line
    h.release(20.0, 0.0, "sketch.select");
    assert!(
        h.tool_enabled("sketch.constrain.parallel"),
        "a line and an axis fit a parallel constraint"
    );

    h.key(KeyCode::A, Some("sketch.constrain.parallel"));
    let sketch = h.sketch();
    let ys: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position.y),
            _ => None,
        })
        .collect();
    assert!(
        ys.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-3),
        "the line came level with the axis: {ys:?}"
    );
}

#[test]
fn the_axes_and_the_origin_cannot_be_deleted() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(6.0, 4.0, "sketch.point");
    h.click(0.0, 0.0, "sketch.select"); // the origin
    h.release(0.0, 0.0, "sketch.select");
    h.click(20.0, 0.0, "sketch.select"); // the X axis
    h.release(20.0, 0.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    assert_eq!(h.counts().0, 1, "the drawn point is still there");
}

#[test]
fn a_sketch_measures_the_cursor_distance_to_its_curves_in_pixels() {
    let mut h = Harness::new();
    let id = h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    let node = h.doc.get_feature_meta(id).expect("sketch node").clone();
    let on = h.px_of(5.0, 0.0);
    let vp = h.vp;
    let pick = |cursor: (f32, f32)| ViewportPick {
        view_proj: vp,
        viewport: VIEWPORT,
        cursor,
    };

    let d =
        h.wb.pick_feature(&h.doc, id, &node, &pick(on))
            .expect("a sketch with a curve answers");
    assert!(d < 0.5, "on the line: {d}");
    let d =
        h.wb.pick_feature(&h.doc, id, &node, &pick((on.0, on.1 + 3.0)))
            .expect("answers");
    assert!((2.5..3.5).contains(&d), "3 px off the line: {d}");
    let d =
        h.wb.pick_feature(&h.doc, id, &node, &pick((on.0, on.1 + 20.0)))
            .expect("answers");
    assert!((19.0..21.0).contains(&d), "20 px off the line: {d}");
}

#[test]
fn passive_geometry_follows_the_sketch_and_its_revision_moves_with_it() {
    let mut h = Harness::new();
    let id = h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    let node = h.doc.get_feature_meta(id).expect("sketch node").clone();
    let before =
        h.wb.passive_geometry(&h.doc, id, &node)
            .expect("a sketch has geometry");
    assert!(!before.mesh.positions.is_empty());

    h.click(0.0, 5.0, "sketch.line");
    h.click(10.0, 5.0, "sketch.line");
    let node = h.doc.get_feature_meta(id).expect("sketch node").clone();
    let after =
        h.wb.passive_geometry(&h.doc, id, &node)
            .expect("still has geometry");
    assert_ne!(before.revision, after.revision);
    assert!(after.mesh.positions.len() > before.mesh.positions.len());
}

#[test]
fn the_start_card_command_opens_a_blank_sketch_on_the_selected_body() {
    let mut h = Harness::new();
    let body = h.doc.create_body(None);
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.selected_body_id = Some(body.0);
    let items =
        h.wb.menu_items(&core_document::MenuScope::StartPage, ctx.document);
    assert_eq!(items.len(), 1);
    assert!(
        h.wb.on_command(&items[0].id, &core_document::MenuScope::StartPage, &mut ctx),
        "the bench knows its own item"
    );
    assert!(!h.wb.on_command(
        "sketch.nothing",
        &core_document::MenuScope::StartPage,
        &mut ctx
    ));
    let created = ctx
        .active_document_object
        .expect("the new sketch is the active object");
    let node = h.doc.get_feature_meta(created).expect("the sketch exists");
    assert_eq!(node.workbench_id.as_str(), "wb.sketch");
    assert_eq!(node.body, Some(body));
    assert_eq!(h.wb.editing_feature(), Some(created));
}

#[test]
fn with_grid_snapping_on_a_drawn_line_lands_on_grid_points() {
    let mut h = Harness::new();
    h.create_sketch();
    h.wb.options.grid_on = true;
    h.wb.options.grid_snap = true;
    h.wb.options.grid_auto = false;
    h.wb.options.grid_size = 5.0;
    h.click(1.2, 0.8, "sketch.line");
    h.click(10.9, -0.3, "sketch.line");
    let sketch = h.sketch();
    let mut points: Vec<(f32, f32)> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some((p.position.x, p.position.y)),
            _ => None,
        })
        .collect();
    points.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(points.len(), 2);
    assert!(
        (points[0].0 - 0.0).abs() < 0.05 && points[0].1.abs() < 0.05,
        "{points:?}"
    );
    assert!(
        (points[1].0 - 10.0).abs() < 0.05 && points[1].1.abs() < 0.05,
        "{points:?}"
    );
}

/// With grid snapping on, a move's base and target round to the grid as a
/// drawing click does, so the move is a whole number of grid steps.
#[test]
fn with_grid_snapping_on_a_move_goes_by_grid_steps() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(5.0, 0.0, "sketch.select");
    h.wb.options.grid_on = true;
    h.wb.options.grid_snap = true;
    h.wb.options.grid_auto = false;
    h.wb.options.grid_size = 5.0;

    h.click(20.4, 20.9, "sketch.translate"); // base, rounds to (20, 20)
    h.click(31.1, 24.2, "sketch.translate"); // target, rounds to (30, 25)
    let sketch = h.sketch();
    let mut xs: Vec<(f32, f32)> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some((p.position.x, p.position.y)),
            _ => None,
        })
        .collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(
        (xs[0].0 - 10.0).abs() < 0.05 && (xs[0].1 - 5.0).abs() < 0.05,
        "{xs:?}"
    );
    assert!(
        (xs[1].0 - 20.0).abs() < 0.05 && (xs[1].1 - 5.0).abs() < 0.05,
        "{xs:?}"
    );
}

#[test]
fn hidden_constraints_leave_no_glyph_on_the_viewport() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    // The axis snap gave the line a horizontal constraint, drawn as a glyph.
    assert!(!h.marks().is_empty());
    h.event(
        WorkbenchInputEvent::ToolActivated,
        Some("sketch.show_constraints"),
    );
    let geometry_marks = h.marks().len();
    h.event(
        WorkbenchInputEvent::ToolActivated,
        Some("sketch.show_constraints"),
    );
    assert!(h.marks().len() > geometry_marks, "glyphs come back");
}

/// Normal geometry draws over construction geometry until the rendering
/// order switch puts construction on top.
#[test]
fn rendering_order_decides_which_geometry_draws_on_top() {
    let mut h = Harness::new();
    let id = h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(0.0, 5.0, "sketch.line");
    h.click(10.0, 5.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    // The line drawn first becomes construction.
    let mut feature = SketchFeature::from_json(h.doc.get_feature_data(id).unwrap()).unwrap();
    let first_line = feature
        .sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) => Some(l.id),
            _ => None,
        })
        .unwrap();
    feature.sketch.set_construction(first_line, true);
    h.doc.update_feature_data(id, feature.to_json()).unwrap();

    // Where the dashed (construction) segments and the normal ones fall in
    // the draw order: the later-drawn is on top.
    let order = |h: &mut Harness| {
        let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(h.vp);
        ctx.active_document_object = h.active_object;
        let lines = h.wb.get_screen_space_overlays(&ctx, h.active_object);
        let last_dashed = lines.iter().rposition(|o| o.dash.is_some()).unwrap();
        let first_dashed = lines.iter().position(|o| o.dash.is_some()).unwrap();
        let normal: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, o)| o.dash.is_none() && same_color(o.color, pal().geometry))
            .map(|(i, _)| i)
            .collect();
        (
            first_dashed,
            last_dashed,
            normal[0],
            *normal.last().unwrap(),
        )
    };
    let (_, last_dashed, first_normal, _) = order(&mut h);
    assert!(
        last_dashed < first_normal,
        "normal geometry draws last, on top"
    );

    h.key(KeyCode::A, Some("sketch.rendering_order"));
    let (first_dashed, _, _, last_normal) = order(&mut h);
    assert!(
        last_normal < first_dashed,
        "construction draws last once switched"
    );
}

/// The polyline's line/arc switch is a keyboard action the workbench
/// registers with a default key: the action switches it, and the hint names
/// whatever key the application says is bound.
#[test]
fn the_polyline_switch_is_a_registered_action_named_by_its_bound_key() {
    let mut context = core_document::WorkbenchContext::default();
    let mut h = Harness::new();
    h.wb.configure(&mut context);
    let action = context
        .actions()
        .iter()
        .find(|a| a.id == "sketch.polyline_arc")
        .expect("the switch is registered");
    assert_eq!(
        action.shortcuts,
        vec![core_document::Chord::parse("M").unwrap()]
    );
    let line = context
        .tools()
        .iter()
        .find(|t| t.id == "sketch.line")
        .unwrap();
    assert_eq!(line.shortcuts[0].to_string(), "L");

    let keys = std::collections::HashMap::from([(
        "sketch.polyline_arc".to_string(),
        vec![core_document::Chord::parse("Shift+A").unwrap()],
    )]);
    h.wb.shortcuts_changed(&keys);
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.polyline");
    h.click(10.0, 0.0, "sketch.polyline");
    let names = |h: &mut Harness| -> Vec<(String, &'static str)> {
        h.hud()
            .and_then(|hud| hud.tool)
            .map(|t| t.keys)
            .unwrap_or_default()
    };
    assert!(
        names(&mut h).contains(&("Shift+A".to_string(), "tangent arcs")),
        "{:?}",
        names(&mut h)
    );
    h.event(
        WorkbenchInputEvent::Action {
            id: "sketch.polyline_arc".into(),
        },
        Some("sketch.polyline"),
    );
    assert!(
        names(&mut h).contains(&("Shift+A".to_string(), "square arcs")),
        "the key names the next kind of segment: {:?}",
        names(&mut h)
    );
}

/// The origin and the two axes snap like drawn geometry: a line started at
/// the origin is pinned to it, one ending on an axis stays on it, and a
/// circle centred on the Y axis keeps its centre there.
#[test]
fn drawing_snaps_to_the_origin_and_the_axes_and_pins_to_them() {
    use wb_sketch::sketch::{ORIGIN_ID, X_AXIS_ID, Y_AXIS_ID};
    let mut h = Harness::new();
    h.create_sketch();
    // Near the origin, then near the X axis far from it.
    h.click(0.05, -0.04, "sketch.line");
    h.click(12.0, 0.08, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    // A circle centred near the Y axis.
    h.click(-0.07, 6.0, "sketch.circle");
    h.click(2.0, 6.0, "sketch.circle");

    let sketch = h.sketch();
    let pinned = |target: uuid::Uuid| {
        sketch.constraints.iter().any(|c| match c.kind {
            ConstraintKind::Coincident { point2, .. } => point2 == target,
            ConstraintKind::PointOnLine { line, .. } => line == target,
            _ => false,
        })
    };
    assert!(pinned(ORIGIN_ID), "the line's start is on the origin");
    assert!(pinned(X_AXIS_ID), "the line's end is on the X axis");
    assert!(pinned(Y_AXIS_ID), "the circle's centre is on the Y axis");
    let points: Vec<[f32; 2]> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some([p.position.x, p.position.y]),
            _ => None,
        })
        .collect();
    assert!(
        points
            .iter()
            .any(|p| p[0].abs() < 1e-5 && p[1].abs() < 1e-5)
    );
    assert!(
        points
            .iter()
            .any(|p| (p[0] - 12.0).abs() < 1e-3 && p[1].abs() < 1e-5)
    );
    assert!(
        points
            .iter()
            .any(|p| p[0].abs() < 1e-5 && (p[1] - 6.0).abs() < 1e-3)
    );
}

#[test]
fn a_click_by_a_line_s_middle_lands_on_it_and_stays_there() {
    use wb_sketch::sketch::ConstraintKind;

    let mut h = Harness::new();
    h.create_sketch();
    h.click(3.0, 4.0, "sketch.line");
    h.click(23.0, 4.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    // Near the middle (13, 4), not on it.
    h.click(13.2, 4.3, "sketch.line");
    h.click(17.0, 12.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    let sketch = h.sketch();
    let (point, _) = sketch
        .constraints
        .iter()
        .find_map(|c| match c.kind {
            ConstraintKind::Midpoint { point, line } => Some((point, line)),
            _ => None,
        })
        .expect("the midpoint snap holds the point at the middle");
    let p = sketch.point_position(point).unwrap();
    assert!(
        // The clicked ends sit a pixel's rounding off whole numbers.
        (p.x - 13.0).abs() < 1e-3 && (p.y - 4.0).abs() < 1e-3,
        "at the middle, got ({}, {})",
        p.x,
        p.y
    );
}

#[test]
fn a_click_by_two_lines_crossing_lands_on_the_crossing() {
    use wb_sketch::sketch::ConstraintKind;

    let mut h = Harness::new();
    h.create_sketch();
    h.click(3.0, 4.0, "sketch.line");
    h.click(23.0, 14.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(3.0, 14.0, "sketch.line");
    h.click(13.0, 4.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    // The two cross at (9⅔, 7⅓), away from either's middle; a circle
    // centred just off it.
    h.click(9.8, 7.5, "sketch.circle");
    h.click(16.0, 9.0, "sketch.circle");

    let sketch = h.sketch();
    let center = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            wb_sketch::sketch::GeometryElement::Circle(c) => Some(c.center),
            _ => None,
        })
        .expect("a circle");
    let p = sketch.point_position(center).unwrap();
    assert!(
        (p.x - 29.0 / 3.0).abs() < 1e-3 && (p.y - 22.0 / 3.0).abs() < 1e-3,
        "centred on the crossing, got ({}, {})",
        p.x,
        p.y
    );
    let held = sketch
        .constraints
        .iter()
        .filter(|c| matches!(c.kind, ConstraintKind::PointOnLine { point, .. } if point == center))
        .count();
    assert_eq!(held, 2, "held on both lines");
}

/// The cue names what the cursor would snap to, and the click lands there:
/// the same answer for both.
#[test]
fn the_snap_cue_names_where_the_click_lands() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(3.0, 4.0, "sketch.line");
    h.click(23.0, 4.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    let cue = |h: &mut Harness| {
        h.labels().into_iter().map(|l| l.text).find(|t| {
            ["Endpoint", "Midpoint", "On curve", "On axis", "Origin"].contains(&t.as_str())
        })
    };
    h.mouse_move(23.2, 4.2, "sketch.rect");
    assert_eq!(cue(&mut h).as_deref(), Some("Endpoint"));
    h.mouse_move(13.2, 4.2, "sketch.rect");
    assert_eq!(cue(&mut h).as_deref(), Some("Midpoint"));
    h.mouse_move(17.0, 4.2, "sketch.rect");
    assert_eq!(cue(&mut h).as_deref(), Some("On curve"));
    h.mouse_move(0.2, -0.1, "sketch.rect");
    assert_eq!(cue(&mut h).as_deref(), Some("Origin"));
    h.mouse_move(40.0, 30.0, "sketch.rect");
    assert_eq!(cue(&mut h), None, "nothing near, no cue");

    // A rectangle from the origin to the line's far end: its second corner,
    // which the cue marks as the endpoint, is that very point.
    h.click(0.1, 0.1, "sketch.rect");
    h.click(23.2, 4.2, "sketch.rect");
    let sketch = h.sketch();
    let ends: Vec<_> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            wb_sketch::sketch::GeometryElement::Point(p)
                if (p.position.x - 23.0).abs() < 1e-3 && (p.position.y - 4.0).abs() < 1e-3 =>
            {
                Some(p.id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        ends.len(),
        1,
        "the corner is the line's end, not a copy of it"
    );
}

/// With snapping off, no cue promises a snap the click will not make.
#[test]
fn with_snapping_off_there_is_no_cue_and_no_snap() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(3.0, 4.0, "sketch.line");
    h.click(23.0, 4.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.event(WorkbenchInputEvent::ToolActivated, Some("sketch.snap"));
    h.mouse_move(13.2, 4.2, "sketch.line");
    assert!(
        !h.labels().iter().any(|l| l.text == "Midpoint"),
        "no cue with snapping off"
    );
    h.click(13.2, 4.2, "sketch.line");
    h.click(13.2, 10.0, "sketch.line");
    let sketch = h.sketch();
    assert!(
        sketch.geometry.iter().any(|g| matches!(g,
            wb_sketch::sketch::GeometryElement::Point(p)
                if (p.position.x - 13.2).abs() < 1e-2 && (p.position.y - 4.2).abs() < 1e-2)),
        "the point stays where it was clicked"
    );
}

/// Drawing from a point toward a line lands on the foot of the
/// perpendicular, and toward a circle on the point where a line touches
/// it; each segment is held that way.
#[test]
fn a_segment_snaps_square_to_a_line_and_touching_a_circle() {
    use wb_sketch::sketch::{ConstraintKind, GeometryElement};

    let mut h = Harness::new();
    h.create_sketch();
    h.click(3.0, 4.0, "sketch.line");
    h.click(23.0, 4.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(30.0, 10.0, "sketch.circle");
    h.click(35.0, 10.0, "sketch.circle");

    h.click(8.3, 12.0, "sketch.line");
    h.mouse_move(8.6, 4.3, "sketch.line");
    assert!(h.labels().iter().any(|l| l.text == "Perpendicular"));
    h.click(8.6, 4.3, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    h.click(30.0, 30.0, "sketch.line");
    // Touching at about (34.84, 11.25).
    h.mouse_move(35.0, 11.5, "sketch.line");
    assert!(h.labels().iter().any(|l| l.text == "Tangent"));
    h.click(35.0, 11.5, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    let sketch = h.sketch();
    let has = |f: &dyn Fn(&ConstraintKind) -> bool| sketch.constraints.iter().any(|c| f(&c.kind));
    assert!(has(&|k| matches!(k, ConstraintKind::Perpendicular { .. })));
    assert!(has(&|k| matches!(k, ConstraintKind::Tangent { .. })));
    let near = |x: f32, y: f32| {
        sketch.geometry.iter().any(|g| {
            matches!(g, GeometryElement::Point(p)
                if (p.position.x - x).abs() < 0.02 && (p.position.y - y).abs() < 0.02)
        })
    };
    assert!(near(8.3, 4.0), "the foot, straight below the start");
    assert!(near(34.84, 11.25), "the touching point");
}

/// Switching tools puts away what the last one had begun: a rectangle's
/// first corner is not finished by the rounded rectangle's first click.
#[test]
fn a_new_tool_starts_fresh() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.rect");
    h.click(20.0, 20.0, "sketch.rect:rounded");
    assert_eq!(h.counts().1, 0, "no rectangle came of the stale corner");
    // The polygon variant's sides apply as it is picked, not on every
    // move: the panel's value holds afterwards.
    h.mouse_move(5.0, 5.0, "sketch.polygon:6");
    h.wb.tool_params_mut().polygon_sides = 9;
    h.mouse_move(6.0, 6.0, "sketch.polygon:6");
    assert_eq!(h.wb.tool_params_mut().polygon_sides, 9);
}

/// Right-click finishes a polyline as it finishes a line chain, and drops
/// any other shape half drawn.
#[test]
fn right_click_finishes_a_polyline_and_drops_a_half_drawn_shape() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(3.0, 3.0, "sketch.polyline");
    h.click(13.0, 3.0, "sketch.polyline");
    h.right_click(13.0, 3.0, "sketch.polyline");
    h.click(30.0, 30.0, "sketch.polyline");
    h.click(40.0, 30.0, "sketch.polyline");
    let lines = h.counts().1;
    assert_eq!(lines, 2, "a fresh polyline, not one joined to the first");

    h.click(50.0, 50.0, "sketch.rect");
    h.right_click(60.0, 60.0, "sketch.rect");
    h.click(70.0, 70.0, "sketch.rect");
    assert_eq!(h.counts().1, lines, "the half-drawn rectangle was dropped");
}

/// Backspace in an empty typed field stays with the field: the selection
/// is not deleted.
#[test]
fn backspace_while_typing_never_deletes_the_selection() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(3.0, 3.0, "sketch.line");
    h.click(13.0, 3.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(8.0, 3.0, "sketch.select");
    h.click(30.0, 30.0, "sketch.line");
    h.key(KeyCode::Key2, Some("sketch.line"));
    for _ in 0..3 {
        h.key(KeyCode::Backspace, Some("sketch.line"));
    }
    assert_eq!(h.counts().1, 1, "the selected line is still there");
}

/// A typed angle of 0 constrains the slant once: the auto horizontal the
/// same click would add is left out, not doubled into a redundancy.
#[test]
fn a_typed_level_angle_is_not_repeated_by_an_auto_horizontal() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(3.0, 3.0, "sketch.line");
    h.key(KeyCode::Key1, Some("sketch.line"));
    h.key(KeyCode::Key0, Some("sketch.line"));
    h.key(KeyCode::Tab, Some("sketch.line"));
    h.key(KeyCode::Key0, Some("sketch.line"));
    h.key(KeyCode::Enter, Some("sketch.line"));

    let sketch = h.sketch();
    let has = |f: fn(&ConstraintKind) -> bool| sketch.constraints.iter().any(|c| f(&c.kind));
    assert!(has(|k| matches!(k, ConstraintKind::AngleToAxis { .. })));
    assert!(
        !has(|k| matches!(k, ConstraintKind::Horizontal { .. })),
        "{:?}",
        sketch.constraints
    );
}

/// A boxed line is a line: its endpoints are not selected with it, so the
/// constraint tools see one line.
#[test]
fn a_boxed_line_is_selected_without_its_endpoints() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(3.0, 4.0, "sketch.line");
    h.click(13.0, 4.1, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.box_select(1.0, 2.0, 15.0, 6.0);
    h.key(KeyCode::A, Some("sketch.constrain.dimension"));
    assert!(
        h.sketch().constraints.iter().any(|c| matches!(
            c.kind,
            ConstraintKind::Length { .. } | ConstraintKind::Distance { .. }
        )),
        "the dimension tool took the boxed line as a line: {:?}",
        h.sketch().constraints
    );
}

/// A 3-point circle takes a typed diameter: through the two rim points
/// clicked, at the size typed.
#[test]
fn a_three_point_circle_takes_a_typed_diameter() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(3.0, 3.0, "sketch.circle:3pt");
    h.click(9.0, 3.0, "sketch.circle:3pt");
    h.key(KeyCode::Key1, Some("sketch.circle:3pt"));
    h.key(KeyCode::Key0, Some("sketch.circle:3pt"));
    h.mouse_move(6.0, 8.0, "sketch.circle:3pt");
    h.key(KeyCode::Enter, Some("sketch.circle:3pt"));
    let sketch = h.sketch();
    let circle = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Circle(c) => Some(c.clone()),
            _ => None,
        })
        .expect("a circle");
    assert!(
        (circle.radius - 5.0).abs() < 1e-3,
        "radius {}",
        circle.radius
    );
    let center = sketch.point_position(circle.center).unwrap();
    assert!(
        center.y > 3.0,
        "on the cursor's side of the chord: {center:?}"
    );
}

/// Typing into the dimension editor: every key lands in the frame it
/// arrives, and only the frame the editor takes focus interrupts the
/// input method. Interrupting it every frame makes keys arrive late and
/// several at once.
#[test]
fn typing_into_the_dimension_editor_lands_every_key_at_once() {
    use core_document::{TaskRequest, Workbench};
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Key2, Some("sketch.line"));
    h.key(KeyCode::Key5, Some("sketch.line"));
    h.key(KeyCode::Enter, Some("sketch.line"));
    h.key(KeyCode::Escape, Some("sketch.line"));
    let labels = h.labels();
    let dim = labels.iter().find(|l| l.background).expect("a dimension");
    let pos = (dim.pos[0], dim.pos[1]);
    h.press_px(pos);
    h.release_px(pos);
    h.press_px(pos);
    h.release_px(pos);
    assert!(h.wb.pending_dim_edit().is_some());

    let ctx = egui::Context::default();
    ui_kit::theme::apply_theme(&ctx);
    let backspace = egui::Event::Key {
        key: egui::Key::Backspace,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    };
    // The editor opens with the value selected: the first key replaces it.
    let frames: Vec<(Vec<egui::Event>, &str)> = vec![
        (vec![], "25"),
        (vec![], "25"),
        (vec![egui::Event::Text("3".into())], "3"),
        (vec![egui::Event::Text("0".into())], "30"),
        (vec![backspace], "3"),
        (vec![], "3"),
        (vec![egui::Event::Text("7".into())], "37"),
    ];
    let mut interrupted = Vec::new();
    for (events, want) in frames {
        let input = egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let mut wbctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0; 3], VIEWPORT);
            wbctx.view_proj = Some(h.vp);
            wbctx.active_document_object = h.active_object;
            let _ = h.wb.ui_task_panel(ui, &mut wbctx, TaskRequest::default());
        });
        out.textures_delta.clear();
        interrupted.push(
            out.platform_output
                .ime
                .is_some_and(|ime| ime.should_interrupt_composition),
        );
        assert_eq!(
            h.wb.pending_dim_edit().map(|e| e.text.as_str()),
            Some(want),
            "the key landed in its own frame"
        );
    }
    assert!(
        interrupted.iter().skip(2).all(|i| !i),
        "focus is taken once, not every frame: {interrupted:?}"
    );
}

/// A rounded rectangle takes its typed width and height as dimensions
/// between opposite edges, which stop short of the rounded corners.
#[test]
fn a_typed_rounded_rectangle_keeps_its_width_and_height() {
    let mut h = Harness::new();
    h.create_sketch();
    let tool = "sketch.rect:rounded";
    h.click(5.0, 5.0, tool);
    h.mouse_move(12.0, 9.0, tool);
    h.key(KeyCode::Key2, Some(tool));
    h.key(KeyCode::Key0, Some(tool));
    h.key(KeyCode::Tab, Some(tool));
    h.key(KeyCode::Key1, Some(tool));
    h.key(KeyCode::Key0, Some(tool));
    h.key(KeyCode::Enter, Some(tool));

    let sketch = h.sketch();
    let has = |f: fn(&ConstraintKind) -> bool| sketch.constraints.iter().any(|c| f(&c.kind));
    assert!(has(
        |k| matches!(k, ConstraintKind::DistanceX { value, .. } if (*value - 20.0).abs() < 1e-4)
    ));
    assert!(has(
        |k| matches!(k, ConstraintKind::DistanceY { value, .. } if (*value - 10.0).abs() < 1e-4)
    ));
    let xs: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) if !sketch.is_construction(p.id) => Some(p.position.x),
            _ => None,
        })
        .collect();
    let width =
        xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min);
    assert!((width - 20.0).abs() < 1e-3, "{width}");
}

/// The dimensions and relations that reach past points and lines, made
/// from the toolbar the way a user makes them.
mod curve_constraints {
    use super::*;
    use wb_sketch::sketch::{ConstraintKind, X_AXIS_ID};

    /// Press and release at a sketch point in select mode.
    fn pick(h: &mut Harness, x: f32, y: f32) {
        h.click(x, y, "sketch.select");
        h.release(x, y, "sketch.select");
    }

    /// Draw an arc about `(cx, cy)` from `radius` right of it, a quarter
    /// turn counter-clockwise.
    fn quarter_arc(h: &mut Harness, cx: f32, cy: f32, radius: f32) {
        h.click(cx, cy, "sketch.arc");
        h.click(cx + radius, cy, "sketch.arc");
        h.click(cx, cy + radius * 1.5, "sketch.arc");
    }

    fn arc_length_of(sketch: &Sketch) -> f32 {
        let arc = sketch
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Arc(a) => Some(a.clone()),
                _ => None,
            })
            .expect("an arc");
        let c = sketch.point_position(arc.center).unwrap().to_glam();
        let s = sketch.point_position(arc.start).unwrap().to_glam() - c;
        let e = sketch.point_position(arc.end).unwrap().to_glam() - c;
        let mut sweep = e.y.atan2(e.x) - s.y.atan2(s.x);
        while sweep <= 0.0 {
            sweep += std::f32::consts::TAU;
        }
        arc.radius * sweep
    }

    /// Open the dimension editor on the label whose text starts `prefix`,
    /// type `text` and commit it.
    fn edit_dimension(h: &mut Harness, prefix: &str, text: &str) {
        let labels = h.labels();
        let label = labels
            .iter()
            .find(|l| l.background && l.text.starts_with(prefix))
            .unwrap_or_else(|| {
                panic!(
                    "a label starting {prefix:?} in {:?}",
                    labels.iter().map(|l| &l.text).collect::<Vec<_>>()
                )
            });
        let pos = (label.pos[0], label.pos[1]);
        h.press_px(pos);
        h.release_px(pos);
        h.press_px(pos);
        h.wb.pending_dim_edit_mut()
            .expect("a double click opens the editor")
            .text = text.to_string();
        let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(h.vp);
        ctx.active_document_object = h.active_object;
        h.wb.commit_dim_edit(&mut ctx);
    }

    #[test]
    fn an_arc_takes_a_length_along_it_that_a_formula_can_set() {
        let mut h = Harness::new();
        h.create_sketch();
        quarter_arc(&mut h, 10.0, 0.0, 4.0);
        pick(
            &mut h,
            10.0 + 4.0 * std::f32::consts::FRAC_1_SQRT_2,
            4.0 * std::f32::consts::FRAC_1_SQRT_2,
        );
        assert!(h.tool_enabled("sketch.constrain.arc_length"));
        h.key(KeyCode::A, Some("sketch.constrain.arc_length"));
        let sketch = h.sketch();
        let quarter = 4.0 * std::f32::consts::FRAC_PI_2;
        assert!(
            sketch.constraints.iter().any(|c| matches!(
                c.kind,
                ConstraintKind::ArcLength { length, .. } if (length - quarter).abs() < 0.05
            )),
            "{:?}",
            sketch.constraints
        );

        // Along the arc, and edited like any dimension, by a formula too.
        edit_dimension(&mut h, "◠", "2 * 4");
        let sketch = h.sketch();
        assert!(
            (arc_length_of(&sketch) - 8.0).abs() < 1e-2,
            "the arc re-solved to its new length: {}",
            arc_length_of(&sketch)
        );
        let id = h.active_object.unwrap();
        let node = h.doc.get_feature_meta(id).unwrap().clone();
        let params = h.wb.parameters(&node);
        assert!(
            params.iter().any(|p| p.label == "Arc length"
                && p.dim == core_document::expr::Dim::LENGTH
                && node
                    .data
                    .pointer(&p.pointer)
                    .and_then(serde_json::Value::as_f64)
                    .is_some_and(|v| (v - 8.0).abs() < 1e-4)),
            "a formula can set the arc length: {params:?}"
        );
    }

    #[test]
    fn the_dimension_tool_sets_the_gap_between_two_circles() {
        let mut h = Harness::new();
        h.create_sketch();
        h.click(0.0, 0.0, "sketch.circle");
        h.click(3.0, 0.0, "sketch.circle");
        h.click(12.0, 0.0, "sketch.circle");
        h.click(14.0, 0.0, "sketch.circle");
        pick(&mut h, 0.0, 3.0);
        pick(&mut h, 12.0, 2.0);
        assert!(h.tool_enabled("sketch.constrain.dimension"));
        h.key(KeyCode::A, Some("sketch.constrain.dimension"));
        let sketch = h.sketch();
        let gap = sketch
            .constraints
            .iter()
            .find_map(|c| match c.kind {
                ConstraintKind::Gap { distance, .. } => Some(distance),
                _ => None,
            })
            .unwrap_or_else(|| panic!("a gap: {:?}", sketch.constraints));
        assert!((gap - 7.0).abs() < 0.05, "{gap}");
        assert!(
            h.labels().iter().any(|l| l.background && l.text == "7"),
            "the gap is labelled"
        );
    }

    #[test]
    fn a_new_dimension_opens_its_editor_on_what_it_measured() {
        let mut h = Harness::new();
        h.create_sketch();
        h.click(0.0, 0.0, "sketch.circle");
        h.click(3.0, 0.0, "sketch.circle");
        h.click(12.0, 0.0, "sketch.circle");
        h.click(14.0, 0.0, "sketch.circle");
        pick(&mut h, 0.0, 3.0);
        pick(&mut h, 12.0, 2.0);
        h.key(KeyCode::A, Some("sketch.constrain.dimension"));
        let edit = h.wb.pending_dim_edit().expect("the editor opened").clone();
        assert!(edit.new && edit.select_all && edit.save_as.is_none());
        assert_eq!(edit.text, "7");
        let label = h
            .labels()
            .into_iter()
            .find(|l| l.background && l.text == "7")
            .expect("the gap is labelled");
        assert_eq!(edit.screen_pos, label.pos, "by its label");

        // A value typed and Enter: the gap takes it.
        h.wb.pending_dim_edit_mut().unwrap().text = "5".to_string();
        let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(h.vp);
        ctx.active_document_object = h.active_object;
        h.wb.commit_dim_edit(&mut ctx);
        let gap = h
            .sketch()
            .constraints
            .iter()
            .find_map(|c| match c.kind {
                ConstraintKind::Gap { distance, .. } => Some(distance),
                _ => None,
            })
            .unwrap();
        assert!((gap - 5.0).abs() < 1e-4, "{gap}");
    }

    #[test]
    fn radius_or_diameter_dimensions_the_first_and_equals_the_rest() {
        let mut h = Harness::new();
        h.create_sketch();
        h.click(0.0, 0.0, "sketch.circle");
        h.click(3.0, 0.0, "sketch.circle");
        quarter_arc(&mut h, 12.0, 0.0, 5.0);
        pick(&mut h, 0.0, 3.0);
        pick(
            &mut h,
            12.0 + 5.0 * std::f32::consts::FRAC_1_SQRT_2,
            5.0 * std::f32::consts::FRAC_1_SQRT_2,
        );
        h.key(KeyCode::A, Some("sketch.constrain.radius:radius_diameter"));
        let sketch = h.sketch();
        assert!(
            sketch.constraints.iter().any(|c| matches!(
                c.kind,
                ConstraintKind::Diameter { diameter, .. } if (diameter - 6.0).abs() < 0.05
            )),
            "the circle, drawn first, takes a diameter: {:?}",
            sketch.constraints
        );
        assert!(
            sketch
                .constraints
                .iter()
                .any(|c| matches!(c.kind, ConstraintKind::EqualRadius { .. }))
        );
        let radii: Vec<f32> = sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                GeometryElement::Circle(c) => Some(c.radius),
                GeometryElement::Arc(a) => Some(a.radius),
                _ => None,
            })
            .collect();
        assert!(
            (radii[0] - 3.0).abs() < 0.05 && (radii[1] - 3.0).abs() < 0.05,
            "the arc came to the circle's size: {radii:?}"
        );
    }

    #[test]
    fn the_angle_tool_measures_where_a_line_leaves_an_arc() {
        let mut h = Harness::new();
        h.create_sketch();
        quarter_arc(&mut h, 0.0, 0.0, 5.0);
        // A line on from the arc's start, snapped onto it.
        h.click(5.0, 0.0, "sketch.line");
        h.click(9.0, 3.0, "sketch.line");
        h.key(KeyCode::Escape, Some("sketch.line"));
        h.key(KeyCode::Escape, Some("sketch.line"));
        pick(&mut h, 5.0, 0.0);
        pick(&mut h, 7.0, 1.5);
        pick(
            &mut h,
            5.0 * std::f32::consts::FRAC_1_SQRT_2,
            5.0 * std::f32::consts::FRAC_1_SQRT_2,
        );
        assert!(h.tool_enabled("sketch.constrain.angle"));
        h.key(KeyCode::A, Some("sketch.constrain.angle"));
        let sketch = h.sketch();
        let angle = sketch
            .constraints
            .iter()
            .find_map(|c| match c.kind {
                ConstraintKind::AngleAtPoint { angle_rad, .. } => Some(angle_rad.to_degrees()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("an angle at the point: {:?}", sketch.constraints));
        // The arc leaves its start straight up; the line at 3-4-5.
        let expected = (90.0 - 3f32.atan2(4.0).to_degrees()).abs();
        assert!(
            (angle.abs() - expected).abs() < 0.5,
            "{angle} against {expected}"
        );
        edit_dimension(&mut h, &format!("{}", angle.round() as i32), "30");
        let sketch = h.sketch();
        assert!(
            sketch.constraints.iter().any(|c| matches!(
                c.kind,
                ConstraintKind::AngleAtPoint { angle_rad, .. }
                    if (angle_rad.to_degrees() - 30.0).abs() < 1e-3
            )),
            "the edited angle is stored"
        );
    }

    #[test]
    fn a_refraction_bends_a_ray_at_an_interface_to_a_ratio_of_indices() {
        let mut h = Harness::new();
        h.create_sketch();
        h.click(-10.0, 2.0, "sketch.line");
        h.click(10.0, 2.0, "sketch.line");
        h.key(KeyCode::Escape, Some("sketch.line"));
        h.key(KeyCode::Escape, Some("sketch.line"));
        // The ray in, met on the interface, and the ray out from there.
        h.click(-4.0, 6.0, "sketch.line");
        h.click(0.0, 2.0, "sketch.line");
        h.click(3.0, -2.0, "sketch.line");
        h.key(KeyCode::Escape, Some("sketch.line"));
        h.key(KeyCode::Escape, Some("sketch.line"));
        pick(&mut h, 0.0, 2.0);
        pick(&mut h, -2.0, 4.0);
        pick(&mut h, 1.5, 0.0);
        pick(&mut h, 7.0, 2.0);
        assert!(h.tool_enabled("sketch.constrain.refraction"));
        h.key(KeyCode::A, Some("sketch.constrain.refraction"));
        let sketch = h.sketch();
        let ratio = sketch
            .constraints
            .iter()
            .find_map(|c| match c.kind {
                ConstraintKind::Refraction { ratio, .. } => Some(ratio),
                _ => None,
            })
            .unwrap_or_else(|| panic!("a refraction: {:?}", sketch.constraints));
        let expected = 45f32.to_radians().sin() / 0.6;
        assert!(
            (ratio - expected).abs() < 0.02,
            "{ratio} against {expected}"
        );

        // Its ratio is a plain number, a formula's as much as a typed one.
        edit_dimension(&mut h, "n ", "1.2 + 0.3");
        let sketch = h.sketch();
        let (ray1, ray2, interface, point) = sketch
            .constraints
            .iter()
            .find_map(|c| match c.kind {
                ConstraintKind::Refraction {
                    ray1,
                    ray2,
                    interface,
                    point,
                    ratio,
                } => {
                    assert!((ratio - 1.5).abs() < 1e-5, "{ratio}");
                    Some((ray1, ray2, interface, point))
                }
                _ => None,
            })
            .unwrap();
        let at = sketch.point_position(point).unwrap().to_glam();
        let dir = |line: uuid::Uuid| match sketch.get_geometry(line) {
            Some(GeometryElement::Line(l)) => {
                let s = sketch.point_position(l.start).unwrap().to_glam();
                let e = sketch.point_position(l.end).unwrap().to_glam();
                if (s - at).length() < (e - at).length() {
                    (e - s).normalize()
                } else {
                    (s - e).normalize()
                }
            }
            _ => panic!("a line"),
        };
        let t = dir(interface);
        let sin_in = (-dir(ray1)).dot(t);
        let sin_out = dir(ray2).dot(t);
        assert!(
            (sin_in / sin_out - 1.5).abs() < 1e-3,
            "the rays bend to the new ratio: {}",
            sin_in / sin_out
        );
    }

    /// A level line and the X axis: the line's horizontal constraint and a
    /// parallel to the axis say the same thing.
    fn level_line_against_the_axis(h: &mut Harness) {
        h.create_sketch();
        h.click(4.0, 4.0, "sketch.line");
        h.click(12.0, 4.02, "sketch.line");
        h.key(KeyCode::Escape, Some("sketch.line"));
        h.key(KeyCode::Escape, Some("sketch.line"));
        assert!(
            h.sketch()
                .constraints
                .iter()
                .any(|c| matches!(c.kind, ConstraintKind::Horizontal { .. })),
            "the line was drawn level"
        );
        pick(h, 8.0, 4.0);
        pick(h, 20.0, 0.0);
    }

    #[test]
    fn with_the_preference_on_a_new_constraint_takes_the_one_it_repeats() {
        let mut h = Harness::new();
        h.wb.options.auto_remove_redundant = true;
        level_line_against_the_axis(&mut h);
        h.key(KeyCode::A, Some("sketch.constrain.parallel"));
        let kinds: Vec<ConstraintKind> =
            h.sketch().constraints.into_iter().map(|c| c.kind).collect();
        assert!(
            kinds.iter().any(|k| matches!(
                k,
                ConstraintKind::Parallel { line2, .. } if *line2 == X_AXIS_ID
            )),
            "the new one stays: {kinds:?}"
        );
        assert!(
            !kinds
                .iter()
                .any(|k| matches!(k, ConstraintKind::Horizontal { .. })),
            "the older one it repeated went: {kinds:?}"
        );
    }

    #[test]
    fn with_the_preference_off_both_stay() {
        let mut h = Harness::new();
        h.wb.options.auto_remove_redundant = false;
        level_line_against_the_axis(&mut h);
        h.key(KeyCode::A, Some("sketch.constrain.parallel"));
        let kinds: Vec<ConstraintKind> =
            h.sketch().constraints.into_iter().map(|c| c.kind).collect();
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, ConstraintKind::Horizontal { .. }))
                && kinds
                    .iter()
                    .any(|k| matches!(k, ConstraintKind::Parallel { .. })),
            "{kinds:?}"
        );
    }
}

#[test]
fn rectangle_variants_draw_turned_rectangles_and_frames() {
    let mut h = Harness::new();
    h.create_sketch();
    for (x, y) in [(1.0, 1.0), (9.0, 7.0), (0.0, 6.5)] {
        h.click(x, y, "sketch.rect:3pt");
    }
    assert_eq!(h.counts(), (4, 4, 0, 0));
    for (x, y) in [(-10.0, 1.0), (-6.0, 4.0), (-16.0, 9.0)] {
        h.click(x, y, "sketch.rect:center3pt");
    }
    assert_eq!(h.counts(), (9, 8, 0, 0), "a construction centre as well");
    for (x, y) in [(2.0, -12.0), (14.0, -3.0)] {
        h.click(x, y, "sketch.rect:frame");
    }
    assert_eq!(h.counts(), (17, 16, 0, 0));
    let wires = wb_sketch::profile::extract_wires(&h.sketch()).unwrap();
    assert_eq!(
        wires.len(),
        4,
        "two turned rectangles and a frame's two outlines"
    );
    // The turned rectangle's first edge runs where it was clicked.
    assert!(h.point_at(1.0, 1.0) && h.point_at(9.0, 7.0));
}

#[test]
fn dragging_the_trim_tool_trims_every_span_it_crosses() {
    let mut h = Harness::new();
    h.create_sketch();
    // A horizontal wall and three posts through it.
    h.click(0.0, 0.0, "sketch.line");
    h.click(20.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    for x in [5.0, 10.0, 15.0] {
        h.click(x, -5.0, "sketch.line");
        h.click(x, 5.0, "sketch.line");
        h.key(KeyCode::Escape, Some("sketch.line"));
    }
    // Pressed on empty space, dragged across the posts above the wall.
    h.click(2.0, 3.0, "sketch.trim");
    h.mouse_move(8.0, 3.0, "sketch.trim");
    h.mouse_move(18.0, 3.0, "sketch.trim");
    h.release(18.0, 3.0, "sketch.trim");
    let sketch = h.sketch();
    let tops: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => {
                let a = sketch.point_position(l.start)?;
                let b = sketch.point_position(l.end)?;
                ((a.x - b.x).abs() < 1e-4).then_some(a.y.max(b.y))
            }
            _ => None,
        })
        .collect();
    assert_eq!(tops.len(), 3, "each post keeps its lower part");
    assert!(tops.iter().all(|y| y.abs() < 1e-3), "{tops:?}");
    // Moving with the button up trims nothing.
    let lines = h.counts().1;
    h.mouse_move(2.0, -3.0, "sketch.trim");
    h.mouse_move(18.0, -3.0, "sketch.trim");
    assert_eq!(h.counts().1, lines);
}

#[test]
fn a_spline_through_points_goes_through_the_clicks_and_bends_with_them() {
    let mut h = Harness::new();
    h.create_sketch();
    let tool = "sketch.bspline:through";
    for (x, y) in [(1.0, 1.0), (5.0, 6.0), (10.0, 4.0), (14.0, 8.0)] {
        h.click(x, y, tool);
    }
    h.key(KeyCode::Enter, Some(tool));
    let sketch = h.sketch();
    let spline = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::BSpline(b) => Some(b.clone()),
            _ => None,
        })
        .expect("a spline");
    assert_eq!(spline.fit_points.len(), 4);
    let near = |sketch: &Sketch, b: &wb_sketch::sketch::BSpline, x: f32, y: f32| {
        b.points(sketch, 2000)
            .unwrap()
            .iter()
            .any(|p| (p.x - x).abs() < 0.03 && (p.y - y).abs() < 0.03)
    };
    assert!(near(&sketch, &spline, 5.0, 6.0) && near(&sketch, &spline, 10.0, 4.0));

    // Dragging a point it passes through, the curve follows.
    h.key(KeyCode::Escape, Some("sketch.select"));
    h.click(10.0, 4.0, "sketch.select");
    h.mouse_move(10.0, 1.0, "sketch.select");
    h.release(10.0, 1.0, "sketch.select");
    let sketch = h.sketch();
    let spline = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::BSpline(b) => Some(b.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        near(&sketch, &spline, 10.0, 1.0),
        "follows the dragged point"
    );
    assert!(near(&sketch, &spline, 5.0, 6.0), "keeps the others");
}

#[test]
fn join_lights_up_for_two_curves_and_leaves_a_kink_alone() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(1.0, 1.0, "sketch.line");
    h.click(9.0, 1.0, "sketch.line");
    h.click(15.0, 3.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.key(KeyCode::Escape, Some("sketch.select"));
    h.click(5.0, 1.0, "sketch.select");
    h.release(5.0, 1.0, "sketch.select");
    assert!(!h.tool_enabled("sketch.join"), "one curve joins nothing");
    h.click(12.0, 2.0, "sketch.select");
    h.release(12.0, 2.0, "sketch.select");
    assert!(h.tool_enabled("sketch.join"));
    // Two lines meeting at a kink: no smooth spline keeps within the
    // tolerance there, and the curves are left as they are.
    h.key(KeyCode::A, Some("sketch.join"));
    let sketch = h.sketch();
    let splines = sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::BSpline(_)))
        .count();
    assert_eq!((h.counts().1, splines), (2, 0));
    assert!(h.point_at(1.0, 1.0) && h.point_at(15.0, 3.0));
}

#[test]
fn arcs_of_parabola_and_hyperbola_draw_from_the_ellipse_menu_and_close_profiles() {
    let mut h = Harness::new();
    h.create_sketch();
    for (x, y) in [(1.0, 1.0), (1.0, 3.0), (-3.0, 2.0), (5.0, 7.0)] {
        h.click(x, y, "sketch.ellipse:parabola");
    }
    // Its ends at (-3, 3) and (5, 3): a line snapped to both closes it.
    h.click(5.0, 3.0, "sketch.line");
    h.click(-3.0, 3.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    for (x, y) in [(10.0, 1.0), (13.0, 1.0), (15.0, 3.0), (18.0, -1.0)] {
        h.click(x, y, "sketch.ellipse:hyperbola");
    }
    let sketch = h.sketch();
    let conics = sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::Conic(_)))
        .count();
    assert_eq!(conics, 2);
    assert!(h.point_at(15.0, 3.0) && h.point_at(15.0, -1.0));
    let wires = wb_sketch::profile::extract_wires(&sketch).unwrap();
    assert_eq!(wires.len(), 1, "the hyperbola's open arc is left out");
    h.click(15.0, 3.0, "sketch.line");
    h.click(15.0, -1.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    let wires = wb_sketch::profile::extract_wires(&h.sketch()).unwrap();
    assert_eq!(wires.len(), 2);

    // Dragging an end slides it along its curve.
    h.click(15.0, 3.0, "sketch.select");
    h.mouse_move(16.0, 5.0, "sketch.select");
    h.release(16.0, 5.0, "sketch.select");
    let sketch = h.sketch();
    for g in &sketch.geometry {
        if let GeometryElement::Conic(c) = g {
            let shape = wb_sketch::conic::Shape::of(c, &sketch).unwrap();
            for id in [c.start, c.end] {
                let p = sketch.point_position(id).unwrap();
                let miss = shape.distance([f64::from(p.x), f64::from(p.y)]).abs();
                assert!(miss < 1e-3, "an end is {miss} off its curve");
            }
        }
    }
}

#[test]
fn internal_geometry_shows_on_a_selected_ellipse_follows_a_drag_and_hides() {
    use wb_sketch::sketch::{ConstraintKind, InternalRole};
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.ellipse");
    h.click(10.0, 0.0, "sketch.ellipse");
    h.click(5.0, 4.0, "sketch.ellipse");
    assert!(!h.tool_enabled("sketch.internal_geometry"));
    // The top of the ellipse selects it.
    h.click(0.0, 4.0, "sketch.select");
    h.release(0.0, 4.0, "sketch.select");
    assert!(h.tool_enabled("sketch.internal_geometry"));
    h.key(KeyCode::A, Some("sketch.internal_geometry"));
    let sketch = h.sketch();
    let internal: Vec<(uuid::Uuid, InternalRole)> = sketch
        .constraints
        .iter()
        .filter_map(|c| match c.kind {
            ConstraintKind::InternalAlignment { element, role, .. } => Some((element, role)),
            _ => None,
        })
        .collect();
    assert_eq!(internal.len(), 4, "two axes and two foci");
    // a = 10, b = 4: the foci sit √84 from the centre.
    let f = 84f32.sqrt();
    assert!(h.point_at(f, 0.0) && h.point_at(-f, 0.0));
    assert!(h.point_at(10.0, 0.0) && h.point_at(0.0, -4.0));
    // No glyph clutters the curve for them.
    assert!(
        !h.marks()
            .iter()
            .any(|m| is_icon(m, "show-hide-internal-geometry"))
    );

    // Dragging the major axis's end stretches the ellipse.
    h.click(10.0, 0.0, "sketch.select");
    h.mouse_move(12.0, 0.0, "sketch.select");
    h.release(12.0, 0.0, "sketch.select");
    let sketch = h.sketch();
    let ellipse = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Ellipse(e) => Some(e.clone()),
            _ => None,
        })
        .unwrap();
    let a = ellipse.major.to_glam().length();
    assert!(a > 10.5, "the major radius grew to {a}");
    let f = (a * a - (a * ellipse.ratio).powi(2)).sqrt();
    let centre = sketch.point_position(ellipse.center).unwrap();
    let u = ellipse.major.to_glam() / a;
    let focus = centre.to_glam() + u * f;
    assert!(h.point_at(focus.x, focus.y), "the focus followed");

    // Selecting the ellipse alone, the action hides it all again.
    h.click(30.0, 30.0, "sketch.select");
    h.release(30.0, 30.0, "sketch.select");
    let t = std::f32::consts::FRAC_PI_4;
    let on = centre.to_glam() + u * a * t.cos() + u.perp() * a * ellipse.ratio * t.sin();
    h.click(on.x, on.y, "sketch.select");
    h.release(on.x, on.y, "sketch.select");
    h.key(KeyCode::A, Some("sketch.internal_geometry"));
    let sketch = h.sketch();
    assert!(
        !sketch
            .constraints
            .iter()
            .any(|c| matches!(c.kind, ConstraintKind::InternalAlignment { .. }))
    );
    assert_eq!(
        sketch.geometry.len(),
        2,
        "the ellipse and its centre are left"
    );
}

/// The minor axis's end dragged past the major radius with the foci shown:
/// the ellipse goes through a circle and comes out taller than wide, the
/// foci on its new major axis.
#[test]
fn an_ellipse_showing_its_foci_drags_through_a_circle() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.ellipse");
    h.click(10.0, 0.0, "sketch.ellipse");
    h.click(5.0, 4.0, "sketch.ellipse");
    h.click(0.0, 4.0, "sketch.select");
    h.release(0.0, 4.0, "sketch.select");
    h.key(KeyCode::A, Some("sketch.internal_geometry"));
    // The centre held, the minor axis's top end dragged from 4 up to 14.
    h.click(30.0, 30.0, "sketch.select");
    h.release(30.0, 30.0, "sketch.select");
    h.click(0.0, 0.0, "sketch.select");
    h.release(0.0, 0.0, "sketch.select");
    h.key(KeyCode::A, Some("sketch.constrain.lock"));
    h.click(0.0, 4.0, "sketch.select");
    for k in 1..=40 {
        h.mouse_move(0.0, 4.0 + k as f32 * 0.25, "sketch.select");
    }
    h.release(0.0, 14.0, "sketch.select");
    let sketch = h.sketch();
    let ellipse = sketch
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Ellipse(e) => Some(e.clone()),
            _ => None,
        })
        .unwrap();
    let a = ellipse.major.to_glam().length();
    assert!(
        ellipse.major.x.abs() < 1e-2 && (a - 14.0).abs() < 1e-2,
        "the major axis stands upright through the dragged end: {:?}",
        ellipse.major
    );
    assert!(
        (a * ellipse.ratio - 10.0).abs() < 1e-2,
        "the old major radius is the minor"
    );
    // a = 14, b = 10: the foci sit √96 above and below the centre.
    let f = 96f32.sqrt();
    assert!(
        h.point_at(0.0, f) && h.point_at(0.0, -f),
        "the foci followed"
    );
}

#[test]
fn a_parked_constraint_leaves_the_view_until_its_layer_shows_and_still_holds() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(2.0, 3.0, "sketch.line");
    h.click(12.0, 5.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(7.0, 4.0, "sketch.select");
    h.release(7.0, 4.0, "sketch.select");
    h.key(KeyCode::A, Some("sketch.constrain.horizontal"));
    h.key(KeyCode::Escape, Some("sketch.select"));
    let glyph = h
        .marks()
        .into_iter()
        .find(|m| is_icon(m, "constraint-horizontal"))
        .expect("the horizontal glyph shows");
    h.press_px((glyph.pos[0], glyph.pos[1]));
    h.release_px((glyph.pos[0], glyph.pos[1]));
    assert!(h.tool_enabled("sketch.park_constraints"));
    h.key(KeyCode::A, Some("sketch.park_constraints"));
    assert!(h.sketch().constraints.iter().all(|c| c.parked));
    assert!(
        !h.marks()
            .iter()
            .any(|m| is_icon(m, "constraint-horizontal")),
        "parked, the glyph is off the normal layer"
    );
    // Still solved: a dragged end keeps the line level.
    h.click(12.0, 4.0, "sketch.select");
    h.mouse_move(12.0, 9.0, "sketch.select");
    h.release(12.0, 9.0, "sketch.select");
    let sketch = h.sketch();
    let ys: Vec<f32> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some(p.position.y),
            _ => None,
        })
        .collect();
    assert!((ys[0] - ys[1]).abs() < 1e-3, "{ys:?}");
    // The parked layer shows it, and only it.
    h.key(KeyCode::A, Some("sketch.parked_layer"));
    assert!(
        h.marks()
            .iter()
            .any(|m| is_icon(m, "constraint-horizontal"))
    );
}

#[test]
fn the_constraint_list_filters_by_kind_and_by_the_selection() {
    use wb_sketch::ConstraintFilter;
    use wb_sketch::sketch::ConstraintKind;
    let mut h = Harness::new();
    h.create_sketch();
    // A rectangle: four axis constraints. A free line beside it, sized.
    h.click(2.0, 2.0, "sketch.rect");
    h.click(12.0, 8.0, "sketch.rect");
    h.click(20.0, 2.0, "sketch.line");
    h.click(24.0, 9.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    h.click(22.0, 5.5, "sketch.select");
    h.release(22.0, 5.5, "sketch.select");
    h.key(KeyCode::A, Some("sketch.constrain.dimension"));
    // The new dimension's editor opened; Escape keeps what it measured.
    assert!(h.wb.pending_dim_edit().is_some());
    h.key(KeyCode::Escape, Some("sketch.select"));
    assert!(h.wb.pending_dim_edit().is_none());
    let sketch = h.sketch();
    let length = sketch
        .constraints
        .iter()
        .find(|c| matches!(c.kind, ConstraintKind::Length { .. }))
        .map(|c| c.id)
        .expect("the line is sized");
    let listed = |h: &mut Harness, filter: ConstraintFilter| {
        *h.wb.constraint_filter_mut() = filter;
        h.wb.listed_constraints(&h.sketch())
    };
    assert_eq!(listed(&mut h, ConstraintFilter::All).len(), 5);
    assert_eq!(listed(&mut h, ConstraintFilter::Dimensional), [length]);
    assert_eq!(listed(&mut h, ConstraintFilter::Geometric).len(), 4);
    assert!(listed(&mut h, ConstraintFilter::Reference).is_empty());
    // The line is still selected: only its own constraint relates to it.
    assert_eq!(listed(&mut h, ConstraintFilter::Related), [length]);
    // The rectangle's bottom edge: its own axis constraint, not its
    // neighbours'.
    h.key(KeyCode::Escape, Some("sketch.select"));
    h.click(7.0, 2.0, "sketch.select");
    h.release(7.0, 2.0, "sketch.select");
    let related = listed(&mut h, ConstraintFilter::Related);
    let sketch = h.sketch();
    assert_eq!(related.len(), 1);
    assert!(
        sketch
            .constraints
            .iter()
            .any(|c| c.id == related[0] && matches!(c.kind, ConstraintKind::Horizontal { .. }))
    );
    // Its corner: the constraints of both edges meeting there.
    h.key(KeyCode::Escape, Some("sketch.select"));
    h.click(2.0, 2.0, "sketch.select");
    h.release(2.0, 2.0, "sketch.select");
    let related = listed(&mut h, ConstraintFilter::Related);
    let sketch = h.sketch();
    let kinds: Vec<_> = related
        .iter()
        .filter_map(|id| sketch.constraints.iter().find(|c| c.id == *id))
        .map(|c| std::mem::discriminant(&c.kind))
        .collect();
    assert_eq!(related.len(), 2, "the bottom and left edges' constraints");
    assert_ne!(kinds[0], kinds[1], "one level, one upright");
    // Selected picks out the selected constraints alone.
    assert!(listed(&mut h, ConstraintFilter::Selected).is_empty());
}

#[test]
fn section_view_cuts_the_scene_at_the_sketch_plane_and_stays_with_the_sketch() {
    let mut h = Harness::new();
    let id = h.create_sketch();
    let clip = |h: &mut Harness| {
        let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(h.vp);
        ctx.active_document_object = h.active_object;
        h.wb.clip_plane(&ctx)
    };
    // A sketch opens uncut.
    assert_eq!(clip(&mut h), None);
    assert!(!h.wb.tool_toggled("sketch.section_view"));
    h.doc.clear_feature_dirty(id);
    h.key(KeyCode::A, Some("sketch.section_view"));
    assert!(h.wb.tool_toggled("sketch.section_view"));
    // The camera looks down from +Z: what is above the XY plane goes.
    let [a, b, c, d] = clip(&mut h).expect("the sketch cuts the scene");
    let keeps = |z: f32| a * 2.0 + b * 3.0 + c * z + d >= 0.0;
    assert!(keeps(-4.0) && keeps(0.0) && !keeps(4.0));
    let stored = SketchFeature::from_json(h.doc.get_feature_data(id).unwrap()).unwrap();
    assert!(stored.shows_section(false), "kept with the sketch");
    assert!(
        !h.doc.dirty_features().contains(&id),
        "nothing built from it changes"
    );
    h.key(KeyCode::A, Some("sketch.section_view"));
    assert_eq!(clip(&mut h), None);
}

/// Cancel puts the sketch back as the editing session found it, and says
/// so in the recording, so a replay ends where the live session did.
#[test]
fn cancelling_a_sketch_session_puts_the_sketch_back() {
    use core_document::{TaskRequest, Workbench};
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));
    assert_eq!(h.counts().1, 1, "a line drawn");
    let ctx = egui::Context::default();
    ui_kit::theme::apply_theme(&ctx);
    let mut recorded = Vec::new();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut wbctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0; 3], VIEWPORT);
        wbctx.view_proj = Some(h.vp);
        wbctx.active_document_object = h.active_object;
        let _ = h.wb.ui_task_panel(
            ui,
            &mut wbctx,
            TaskRequest {
                accept: false,
                cancel: true,
            },
        );
        recorded = core_document::HookOutcome::take(&mut wbctx).recorded;
    });
    out.textures_delta.clear();
    assert_eq!(h.counts().1, 0, "the line is gone again");
    assert!(
        recorded.iter().any(|r| r.id == "sketch.restore"),
        "{recorded:?}"
    );
}
