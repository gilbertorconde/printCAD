//! End-to-end interaction tests: drive `SketchWorkbench::on_input` through
//! a real `Document` + `WorkbenchRuntimeContext`, exactly like the app
//! shell does — clicks arrive as viewport pixels and are raycast onto the
//! sketch plane by the workbench itself.

use core_document::{
    Document, FeatureId, KeyCode, MouseButton, Workbench, WorkbenchFeature, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};
use glam::{Mat4, Vec3};
use wb_sketch::sketch::{GeometryElement, Sketch};
use wb_sketch::{SketchFeature, SketchWorkbench};

const VIEWPORT: (u32, u32, u32, u32) = (0, 0, 800, 600);
const CAM_POS: [f32; 3] = [0.0, 0.0, 50.0];

/// Vulkan-convention view-projection matching the app camera: perspective
/// with the Y flip baked in, looking straight down +Z at the default XY
/// sketch plane.
fn view_proj() -> [[f32; 4]; 4] {
    let proj = Mat4::perspective_rh(60f32.to_radians(), 800.0 / 600.0, 0.1, 1000.0);
    let flip_y = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0));
    let view = Mat4::look_at_rh(Vec3::from_array(CAM_POS), Vec3::ZERO, Vec3::Y);
    (flip_y * proj * view).to_cols_array_2d()
}

/// Mimics the app shell: owns the document, carries `active_document_object`
/// between events the way `apply_hook_outcome` does.
struct Harness {
    doc: Document,
    wb: SketchWorkbench,
    active_object: Option<FeatureId>,
    vp: [[f32; 4]; 4],
}

impl Harness {
    fn new() -> Self {
        Self {
            doc: Document::new("test"),
            wb: SketchWorkbench::default(),
            active_object: None,
            vp: view_proj(),
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

    fn right_click(&mut self, x: f32, y: f32, tool: &str) {
        let viewport_pos = self.px_of(x, y);
        self.event(
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Right,
                viewport_pos,
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
    // No feature yet — the plane picker is pending in the panel.
    assert!(h.active_object.is_none());
    assert_eq!(h.doc.feature_tree().all_nodes().count(), 0);
}

#[test]
fn cross_workbench_sketch_request_is_consumed() {
    let mut h = Harness::new();
    let body = uuid::Uuid::new_v4();
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.start_sketch_on_body = Some(core_document::SketchAttachRequest { body, face: None });
    h.wb.on_input(
        &WorkbenchInputEvent::KeyPress { key: KeyCode::A },
        None,
        &mut ctx,
    );
    assert!(
        ctx.start_sketch_on_body.is_none(),
        "the sketch workbench takes the pending request"
    );
}

#[test]
fn dragging_a_point_moves_it_and_click_still_selects() {
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

    // Press+release without movement is still a click (selects the point,
    // so Delete cascades the line away).
    h.click(0.0, 0.0, "sketch.select");
    h.release(0.0, 0.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (1, 0), "clicked point deleted with its line");
}

#[test]
fn dragging_a_constrained_point_respects_constraints() {
    let mut h = Harness::new();
    h.create_sketch();
    // Axis-snapped horizontal line gets an auto Horizontal constraint.
    h.click(0.0, 0.0, "sketch.line");
    h.click(15.0, 0.05, "sketch.line");
    let sketch = h.sketch();
    assert_eq!(sketch.constraints.len(), 1);

    // Drag the far endpoint up and sideways: the solver must keep the line
    // horizontal (y's equal) while the x movement sticks.
    h.click(15.0, 0.0, "sketch.select");
    h.mouse_move(20.0, 6.0, "sketch.select");
    h.release(20.0, 6.0, "sketch.select");

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
    assert!(max_x > 17.0, "x movement applied: {max_x}");
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
    h.click(10.0, 0.5, "sketch.line"); // not axis-snapped (0.5 > tolerance at this zoom? verify below)
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
    h.click(0.0, 0.0, "sketch.rect");
    h.click(12.0, 8.0, "sketch.rect");
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
fn select_and_delete_line_keeps_shared_points() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    // Select mid-span with the select tool, then delete.
    h.click(5.0, 3.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (2, 0), "line removed, endpoints kept");
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
    assert_eq!((p, l), (1, 0), "line cascaded away with its endpoint");
}

#[test]
fn horizontal_axis_snap_adds_auto_constraint() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    // Slightly off-horizontal: within the 8px snap tolerance at this zoom.
    h.click(15.0, 0.05, "sketch.line");
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
    assert!(ys.iter().all(|y| y.abs() < 1e-4), "snapped level: {ys:?}");
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
    // 2 axis lines + 4 rectangle edges + 4 point markers (2 segments each).
    assert!(
        overlays.len() >= 2 + 4 + 8,
        "expected axes + rectangle overlays, got {}",
        overlays.len()
    );
}

#[test]
fn construction_geometry_renders_dashed() {
    let mut h = Harness::new();
    let id = h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(10.0, 7.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    let overlays_of = |h: &mut Harness| {
        let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(h.vp);
        ctx.active_document_object = h.active_object;
        h.wb.get_screen_space_overlays(&ctx, h.active_object).len()
    };
    let solid_count = overlays_of(&mut h);

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

    let dashed_count = overlays_of(&mut h);
    assert!(
        dashed_count > solid_count,
        "construction line splits into dashes: {dashed_count} vs {solid_count} overlays"
    );
}

#[test]
fn polygon_tool_draws_closed_hexagon_end_to_end() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.polygon");
    h.click(6.0, 0.0, "sketch.polygon");
    let (p, l, c, a) = h.counts();
    assert_eq!((p, l, c, a), (6, 6, 0, 0), "default 6 sides");
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
    h.click(0.0, 0.0, "sketch.rect");
    h.click(12.0, 8.0, "sketch.rect");
    // Click the shared corner point with the fillet tool (default r=2).
    h.click(12.0, 8.0, "sketch.fillet");
    let (p, l, c, a) = h.counts();
    assert_eq!((p, l, c, a), (6, 4, 0, 1), "corner replaced by arc");
    let sketch = h.sketch();
    assert_eq!(
        sketch.constraints.len(),
        4,
        "rectangle H/V constraints survive the fillet"
    );
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

#[test]
fn construction_mode_flags_fillet_arcs_too() {
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
        new_flagged.iter().all(|&c| c),
        "fillet arc + points are construction"
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
fn construction_toggle_on_mixed_selection_flips_each_individually() {
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

    // Pre-flag the first line only, then toggle a selection of both: each
    // flips individually (mixed → mixed-inverted).
    h.click(5.0, 3.5, "sketch.select");
    h.key(KeyCode::A, Some("sketch.construction"));
    assert!(h.sketch().is_construction(lines[0]));
    // Line 1 is still selected; add line 2 to the selection.
    h.click_ctrl(5.0, 23.5, "sketch.select");
    h.key(KeyCode::A, Some("sketch.construction"));
    let sketch = h.sketch();
    assert!(
        !sketch.is_construction(lines[0]),
        "construction line flipped back to normal"
    );
    assert!(
        sketch.is_construction(lines[1]),
        "normal line flipped to construction"
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
fn plain_click_replaces_selection() {
    let mut h = Harness::new();
    two_lines(&mut h);
    // Click L1, then L2: the second plain click replaces the first, so
    // Delete only removes L2.
    h.click(5.0, 3.5, "sketch.select");
    h.click(5.0, 23.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (4, 1), "only the last-clicked line deleted");
}

#[test]
fn ctrl_click_accumulates_selection() {
    let mut h = Harness::new();
    two_lines(&mut h);
    h.click(5.0, 3.5, "sketch.select");
    h.click_ctrl(5.0, 23.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (4, 0), "both lines deleted, endpoints kept");
}

#[test]
fn ctrl_click_toggles_element_out_of_selection() {
    let mut h = Harness::new();
    two_lines(&mut h);
    h.click(5.0, 3.5, "sketch.select");
    h.click_ctrl(5.0, 23.5, "sketch.select");
    // Ctrl-click L1 again: it leaves the selection, L2 stays.
    h.click_ctrl(5.0, 3.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (4, 1), "only L2 was still selected");
}

#[test]
fn plain_empty_click_clears_selection_but_ctrl_empty_click_keeps_it() {
    let mut h = Harness::new();
    two_lines(&mut h);

    // Ctrl+click empty space: selection untouched.
    h.click(5.0, 3.5, "sketch.select");
    h.click_ctrl(50.0, 3.5, "sketch.select");
    h.release_ctrl(50.0, 3.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (4, 1), "ctrl empty click kept L1 selected");

    // Plain click on empty space: selection cleared, Delete is a no-op.
    h.click(5.0, 23.5, "sketch.select");
    h.click(50.0, 3.5, "sketch.select");
    h.release(50.0, 3.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (4, 1), "plain empty click cleared the selection");
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
fn plain_click_on_point_replaces_selection() {
    let mut h = Harness::new();
    two_lines(&mut h);
    // Select L1 (curve), then plain-click an L2 endpoint: replaces, so
    // Delete only cascades L2.
    h.click(5.0, 3.5, "sketch.select");
    h.click(0.0, 20.0, "sketch.select");
    h.release(0.0, 20.0, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!(
        (p, l),
        (3, 1),
        "L2 cascaded away with its endpoint, L1 kept"
    );
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
    // L1 + endpoints gone; L2 cascaded with its (0,20) endpoint; L3 intact.
    assert_eq!(
        (p, l),
        (3, 1),
        "straddling line only cascades via its endpoint"
    );
}

#[test]
fn ctrl_box_adds_to_existing_selection() {
    let mut h = Harness::new();
    three_lines(&mut h);
    // Select L3, then ctrl-box around L1+L2: everything is selected.
    h.click(24.0, 3.5, "sketch.select");
    h.click_ctrl(-2.0, -2.0, "sketch.select");
    h.mouse_move(12.0, 28.5, "sketch.select");
    h.release(12.0, 28.5, "sketch.select");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!(
        (p, l),
        (2, 0),
        "L1+L2 cascaded, L3 deleted; L3 endpoints kept"
    );
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
        (4, 1),
        "box cancelled: only pre-selected L1 deleted"
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
    h.click(0.0, 0.0, "sketch.select");
    h.mouse_move(10.0, 8.0, "sketch.select");
    let box_count = overlays_of(&mut h);
    assert!(
        box_count > idle_count + 4,
        "dashed box adds more than 4 solid edges: {box_count} vs {idle_count}"
    );
    // Releasing removes the box again.
    h.release(10.0, 8.0, "sketch.select");
    assert_eq!(overlays_of(&mut h), idle_count);
}

#[test]
fn box_selection_covers_circles_and_arcs() {
    let mut h = Harness::new();
    h.create_sketch();
    // Circle center (0,0) r=5; arc centered (15,0) from (19,0) CCW to (15,6)... reuse
    // the shapes from the circle/arc end-to-end test.
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
    assert_eq!((p, l), (4, 4));
    assert_eq!(h.sketch().constraints.len(), 4, "H/V constraints as rect");
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
        .filter(|o| o.thickness > 2.5 && o.color[0] > 0.9 && o.color[1] < 0.5)
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
    h.click(0.0, 0.0, "sketch.line");
    h.click(20.0, 0.0, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    // Start a new line just off the base line's mid-span (no point nearby):
    // the start point is projected ONTO the line and constrained to it.
    h.click(10.0, 0.5, "sketch.line");
    h.click(14.0, 8.0, "sketch.line");
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
        (p.x - 10.0).abs() < 0.1 && p.y.abs() < 1e-3,
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
        .filter(|o| o.color[1] > 0.85 && o.color[0] < 0.3 && o.color[2] < 0.3)
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
fn tab_focuses_angle_field_and_click_commits_typed_angle_only() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Tab, Some("sketch.line")); // focus: angle
    h.key(KeyCode::Key4, Some("sketch.line"));
    h.key(KeyCode::Key5, Some("sketch.line"));
    h.click(10.0, 0.0, "sketch.line"); // length from cursor, direction typed

    let sketch = h.sketch();
    assert!(distance_constraints(&sketch).is_empty(), "no length typed");
    let angle = sketch
        .constraints
        .iter()
        .find_map(|c| match c.kind {
            ConstraintKind::AngleToAxis { angle_rad, .. } => Some(angle_rad.to_degrees()),
            _ => None,
        })
        .expect("typed angle became an AngleToAxis constraint");
    assert!((angle - 45.0).abs() < 1e-3, "angle {angle}");
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
fn rect_typed_width_height_creates_distance_xy_constraints() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.rect");
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
        .find(|p| (p.x - 12.0).abs() < 1e-3 && (p.y - 8.0).abs() < 1e-3);
    assert!(corner.is_some(), "opposite corner at typed (12, 8)");
    let dx = sketch.constraints.iter().find_map(|c| match c.kind {
        ConstraintKind::DistanceX { value, .. } => Some(value),
        _ => None,
    });
    let dy = sketch.constraints.iter().find_map(|c| match c.kind {
        ConstraintKind::DistanceY { value, .. } => Some(value),
        _ => None,
    });
    assert_eq!((dx, dy), (Some(12.0), Some(8.0)));
    assert_eq!(sketch.constraints.len(), 6, "4 H/V + DistanceX + DistanceY");
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
    h.click(0.0, 0.0, "sketch.line");
    h.key(KeyCode::Key9, Some("sketch.line"));
    h.key(KeyCode::Escape, Some("sketch.line")); // clears the buffer only
    h.click(10.0, 6.0, "sketch.line"); // commits at the cursor, unconstrained

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
    h.click(0.0, 0.0, "sketch.line");
    h.click(15.0, 0.05, "sketch.line"); // axis snap → auto Horizontal
    h.key(KeyCode::Escape, Some("sketch.line"));
    assert_eq!(h.sketch().constraints.len(), 1);

    let labels = h.labels();
    let glyph = labels
        .iter()
        .find(|l| l.text == "H")
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
    assert_eq!((p, l), (2, 0), "line deleted");
    assert!(
        h.sketch().constraints.is_empty(),
        "constraint cascaded away"
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
    h.mouse_move(5.0, 8.0, "sketch.select");
    h.release(5.0, 8.0, "sketch.select");

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
fn ctrl_glyph_click_keeps_geometry_selection() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(15.0, 0.05, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    // Select the line, then ctrl-click the H glyph: both stay selected, so
    // Delete removes the constraint (constraints win) but keeps the line.
    h.click(3.0, 0.0, "sketch.select");
    let labels = h.labels();
    let glyph = labels.iter().find(|l| l.text == "H").unwrap();
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
        h.sketch().constraints.is_empty(),
        "constraint deleted first"
    );
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (2, 1), "geometry kept for the next Delete");
    h.key(KeyCode::Delete, Some("sketch.select"));
    let (p, l, _, _) = h.counts();
    assert_eq!((p, l), (2, 0), "geometry Delete still works afterwards");
}

#[test]
fn selected_constraint_highlights_glyph_and_geometry() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.click(15.0, 0.05, "sketch.line");
    h.key(KeyCode::Escape, Some("sketch.line"));

    let labels = h.labels();
    let glyph = labels.iter().find(|l| l.text == "H").unwrap();
    let pos = (glyph.pos[0], glyph.pos[1]);
    h.press_px(pos);

    // Glyph turns selection green.
    let labels = h.labels();
    let glyph = labels.iter().find(|l| l.text == "H").unwrap();
    assert!(
        glyph.color[1] > 0.9 && glyph.color[0] < 0.4,
        "selected glyph tinted, got {:?}",
        glyph.color
    );
    // Referenced line drawn in the selection color.
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
    ctx.view_proj = Some(h.vp);
    ctx.active_document_object = h.active_object;
    let overlays = h.wb.get_screen_space_overlays(&ctx, h.active_object);
    let selected_lines = overlays
        .iter()
        .filter(|o| o.color[1] > 0.9 && o.color[0] < 0.4 && o.color[2] < 0.5)
        .count();
    assert!(selected_lines >= 1, "referenced geometry highlighted");
}

#[test]
fn live_readout_labels_follow_the_cursor_while_drawing() {
    let mut h = Harness::new();
    h.create_sketch();
    h.click(0.0, 0.0, "sketch.line");
    h.mouse_move(3.0, 4.0, "sketch.line");
    let labels = h.labels();
    let length = labels
        .iter()
        .find(|l| l.text.contains("L ") && l.text.contains("mm"))
        .expect("length readout");
    assert!(length.text.contains("5.00"), "live length: {}", length.text);
    assert!(
        labels.iter().any(|l| l.text.contains('°')),
        "angle readout present"
    );

    // Typing highlights the focused field with the typed buffer.
    h.key(KeyCode::Key7, Some("sketch.line"));
    let labels = h.labels();
    assert!(
        labels
            .iter()
            .any(|l| l.text.contains("7 mm") && l.background),
        "typed buffer shown on a pill: {:?}",
        labels.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}
