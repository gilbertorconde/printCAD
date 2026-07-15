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
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(self.vp);
        ctx.active_document_object = self.active_object;
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
            }
        }
        (p, l, c, a)
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
fn editing_a_dimension_constraint_re_solves_the_sketch() {
    use wb_sketch::sketch::Constraint;

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
    feature.sketch.constraints.push(Constraint::FixedPoint {
        point: line.start,
        position: anchor,
    });
    feature.sketch.constraints.push(Constraint::Length {
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
        Constraint::Length {
            line: line.id,
            length: 20.0,
        },
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
            sketch.constraints[constraint_idx],
            Constraint::Length { length, .. } if (length - 20.0).abs() < 1e-6
        ),
        "constraint value stored"
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
