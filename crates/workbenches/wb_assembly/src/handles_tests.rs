use core_document::{Document, KeyCode, MouseButton, Workbench, WorkbenchInputEvent};
use glam::Mat4;
use transform_gizmo::gizmo::Handle;

use super::*;
use crate::{AssemblyWorkbench, Task};

const EYE: Vec3 = Vec3::new(60.0, -80.0, 140.0);
const TARGET: [f32; 3] = [30.0, 0.0, 40.0];

/// The scene's part with its Move task open, seen in perspective from
/// above.
struct Rig {
    doc: Document,
    wb: AssemblyWorkbench,
    part: BodyId,
    view_proj: [[f32; 4]; 4],
    viewport: (u32, u32, u32, u32),
    pixels_per_point: f32,
    shift_down: bool,
}

fn rig() -> Rig {
    rig_at_scale(1.0)
}

fn rig_at_scale(scale: f32) -> Rig {
    let (doc, _, part) = crate::tests::scene();
    let view = glam::camera::rh::view::look_at_mat4(EYE, Vec3::from_array(TARGET), Vec3::Z);
    let proj = glam::camera::rh::proj::directx::perspective(0.8, 800.0 / 600.0, 0.1, 1000.0);
    Rig {
        doc,
        wb: AssemblyWorkbench {
            task: Some(Task::Move {
                body: part,
                placements: Vec::new(),
            }),
            ..AssemblyWorkbench::default()
        },
        part,
        viewport: (0, 0, (800.0 * scale) as u32, (600.0 * scale) as u32),
        pixels_per_point: scale,
        shift_down: false,
        view_proj: (Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)) * proj * view).to_cols_array_2d(),
    }
}

impl Rig {
    /// A context as the view sees the document, and the bench beside it.
    fn split(&mut self) -> (WorkbenchRuntimeContext<'_>, &mut AssemblyWorkbench) {
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, EYE.to_array(), TARGET, self.viewport);
        ctx.view_proj = Some(self.view_proj);
        ctx.pixels_per_point = self.pixels_per_point;
        ctx.shift_down = self.shift_down;
        (ctx, &mut self.wb)
    }

    fn context(&mut self) -> WorkbenchRuntimeContext<'_> {
        self.split().0
    }

    /// Hand `event` to the bench as the host does; whether it took it.
    fn send(&mut self, event: WorkbenchInputEvent) -> bool {
        let (mut ctx, wb) = self.split();
        wb.on_input(&event, None, &mut ctx).consumed
    }

    fn screen(&self, point: Vec3) -> Pos2 {
        let (x, y) = core_document::runtime::world_to_viewport(
            self.view_proj,
            self.viewport,
            point.to_array(),
        )
        .unwrap();
        emath::pos2(x, y)
    }

    /// The handle a press at `at` would take.
    fn under(&mut self, at: Pos2) -> Option<Handle> {
        let part = self.part;
        let (ctx, wb) = self.split();
        let handles = &wb.handles;
        handles
            .gizmo
            .layout(&View(&ctx), &axes(centre(&ctx, part)?), handles.mode)
            .hit(View(&ctx).logical(at), handles.gizmo.style.hit_radius)
    }

    /// The length that shows a pixel long at `point`.
    fn unit(&mut self, point: Vec3) -> f32 {
        View(&self.context()).world_per_pixel(point.as_dvec3()) as f32
    }

    fn press(&mut self, at: Pos2) -> bool {
        self.send(WorkbenchInputEvent::MousePress {
            button: MouseButton::Left,
            viewport_pos: (at.x, at.y),
        })
    }

    fn move_to(&mut self, at: Pos2) -> bool {
        self.send(WorkbenchInputEvent::MouseMove {
            viewport_pos: (at.x, at.y),
        })
    }

    fn release(&mut self) -> bool {
        self.send(WorkbenchInputEvent::MouseRelease {
            button: MouseButton::Left,
            viewport_pos: (0.0, 0.0),
        })
    }

    /// A point of the X arrow's shaft, half the arrow out from `centre`.
    fn x_shaft(&mut self, centre: Vec3) -> Vec3 {
        let point = centre + Vec3::X * self.unit(centre) * 60.0;
        assert_eq!(self.under(self.screen(point)), Some(Handle::Axis(Axis::X)));
        point
    }
}

/// In Move the arrows slide the body along their axis; switched to Turn,
/// the rings turn it about the middle of its box.
#[test]
fn move_handles_slide_and_turn_the_body() {
    let mut rig = rig();
    // The part's box is (0..10, 0..10, 0) placed at (30, 0, 40).
    let start = rig.x_shaft(Vec3::new(35.0, 5.0, 40.0));
    assert!(rig.press(rig.screen(start)));
    assert!(rig.move_to(rig.screen(start + Vec3::X * 20.0)));
    assert!(rig.release());
    let t = rig.doc.body_placement(rig.part).translation;
    assert!(
        (t[0] - 50.0).abs() < 0.05 && t[1].abs() < 0.05 && (t[2] - 40.0).abs() < 0.05,
        "{t:?}"
    );

    assert!(rig.send(WorkbenchInputEvent::Action {
        id: MODE_ACTION.into()
    }));
    assert_eq!(
        rig.wb.handles.mode_index(),
        1,
        "the action switches to Turn"
    );
    // A quarter turn on the Z ring about the middle, now at (55, 5, 40).
    let centre = Vec3::new(55.0, 5.0, 40.0);
    let radius = rig.unit(centre) * 120.0;
    let on_ring = (0..360)
        .step_by(5)
        .map(|d| {
            let a = (d as f32).to_radians();
            centre + Vec3::new(a.cos(), a.sin(), 0.0) * radius
        })
        .find(|p| rig.under(rig.screen(*p)) == Some(Handle::Ring(Axis::Z)))
        .expect("the Z ring takes a press somewhere");
    assert!(rig.press(rig.screen(on_ring)));
    let quarter = centre + Quat::from_rotation_z(std::f32::consts::FRAC_PI_2) * (on_ring - centre);
    assert!(rig.move_to(rig.screen(quarter)));
    assert!(rig.release());
    let placement = rig.doc.body_placement(rig.part);
    let x = placement.direction([1.0, 0.0, 0.0]);
    assert!((x[1].atan2(x[0]).to_degrees() - 90.0).abs() < 0.5, "{x:?}");
    let middle = placement.point([5.0, 5.0, 0.0]);
    assert!(
        (Vec3::from_array(middle) - centre).length() < 0.05,
        "turned about its middle"
    );
}

/// Escape while a handle is held puts the body back where it was taken,
/// and the press after it is the bench's again.
#[test]
fn escape_puts_the_body_back_where_its_handle_took_it() {
    let mut rig = rig();
    let before = rig.doc.body_placement(rig.part);
    let start = rig.x_shaft(Vec3::new(35.0, 5.0, 40.0));
    assert!(rig.press(rig.screen(start)));
    assert!(rig.move_to(rig.screen(start + Vec3::X * 20.0)));
    assert_ne!(rig.doc.body_placement(rig.part), before);
    assert!(rig.send(WorkbenchInputEvent::KeyPress {
        key: KeyCode::Escape
    }));
    assert_eq!(rig.doc.body_placement(rig.part), before);
    assert!(
        !rig.move_to(rig.screen(start + Vec3::X * 40.0)),
        "nothing is held"
    );
    assert_eq!(rig.doc.body_placement(rig.part), before);
}

/// A scaled display draws and grabs the same logical handle, and its ray
/// still moves the body by the same world distance.
#[test]
fn handles_draw_and_pick_in_logical_pixels() {
    let pivot = Vec3::new(35.0, 5.0, 40.0);
    let mut baseline = rig();
    let base_drawing = {
        let part = baseline.part;
        let (ctx, wb) = baseline.split();
        wb.handles.drawing(&ctx, part)
    };
    for scale in [1.0, 1.25, 2.0, 3.0] {
        let mut rig = rig_at_scale(scale);
        let drawing = {
            let part = rig.part;
            let (ctx, wb) = rig.split();
            wb.handles.drawing(&ctx, part)
        };
        assert_eq!(drawing.overlays.len(), base_drawing.overlays.len());
        for (actual, base) in drawing.overlays.iter().zip(&base_drawing.overlays) {
            for (a, b) in actual
                .start
                .into_iter()
                .zip(base.start)
                .chain(actual.end.into_iter().zip(base.end))
            {
                assert!((a / scale - b).abs() < 0.01);
            }
            assert!((actual.thickness / scale - base.thickness).abs() < 1e-5);
        }
        assert_eq!(drawing.polygons.len(), base_drawing.polygons.len());
        for (actual, base) in drawing.polygons.iter().zip(&base_drawing.polygons) {
            assert_eq!(actual.points.len(), base.points.len());
            for (a, b) in actual
                .points
                .iter()
                .flatten()
                .zip(base.points.iter().flatten())
            {
                assert!((a / scale - b).abs() < 0.01);
            }
        }
        assert_eq!(drawing.labels.len(), base_drawing.labels.len());
        for (actual, base) in drawing.labels.iter().zip(&base_drawing.labels) {
            assert!((actual.size / scale - base.size).abs() < 1e-5);
        }

        let start = rig.x_shaft(pivot);
        let at = rig.screen(start);
        // Just inside the logical hit radius, beyond the shaft's thickness.
        assert!(rig.press(at + emath::vec2(0.0, 7.0 * scale)));
        assert!(rig.move_to(rig.screen(start + Vec3::X * 20.0) + emath::vec2(0.0, 7.0 * scale)));
        assert!(rig.release());
        assert!((rig.doc.body_placement(rig.part).translation[0] - 50.0).abs() < 0.1);
    }
}

#[test]
fn shift_disc_keeps_the_body_in_the_screen_plane_at_every_scale() {
    for scale in [1.0, 1.25, 2.0, 3.0] {
        let mut rig = rig_at_scale(scale);
        let pivot = Vec3::new(35.0, 5.0, 40.0);
        let at = rig.screen(pivot);
        assert_eq!(rig.under(at), Some(Handle::View));
        assert!(rig.press(at));
        let before = rig.doc.body_placement(rig.part).offset();
        rig.shift_down = true;
        assert!(rig.move_to(at + emath::vec2(37.0, 19.0) * scale));
        let offset = rig.doc.body_placement(rig.part).offset() - before;
        let forward = (Vec3::from_array(TARGET) - EYE).normalize();
        assert!(offset.length() > 1.0);
        assert!(
            offset.dot(forward).abs() < 0.001,
            "scale {scale}: {offset:?}"
        );
        assert!(rig.release());
    }
}

#[test]
fn focus_loss_restores_the_press_state_and_releases_the_handle() {
    let mut rig = rig_at_scale(2.0);
    let before = rig.doc.body_placement(rig.part);
    let start = rig.x_shaft(Vec3::new(35.0, 5.0, 40.0));
    assert!(rig.press(rig.screen(start)));
    assert!(rig.move_to(rig.screen(start + Vec3::X * 20.0)));
    assert_ne!(rig.doc.body_placement(rig.part), before);
    {
        let (mut ctx, wb) = rig.split();
        wb.cancel_pointer_gesture(&mut ctx);
    }
    assert_eq!(rig.doc.body_placement(rig.part), before);
    assert!(!rig.move_to(rig.screen(start + Vec3::X * 40.0)));
    assert_eq!(rig.doc.body_placement(rig.part), before);
    assert!(rig.press(rig.screen(start)));
    assert!(rig.release());
}

#[test]
fn focus_loss_also_cancels_a_body_drag_without_handles() {
    let mut rig = rig();
    rig.wb.task = None;
    let part = rig.part;
    let before = rig.doc.body_placement(part);
    let pivot = Vec3::new(35.0, 5.0, 40.0);
    let at = rig.screen(pivot);
    let to = rig.screen(pivot + Vec3::X * 20.0);
    let (mut ctx, wb) = rig.split();
    ctx.hovered_body_id = Some(part.0);
    ctx.hovered_world_pos = Some(pivot.to_array());
    wb.on_input(
        &WorkbenchInputEvent::MousePress {
            button: MouseButton::Left,
            viewport_pos: (at.x, at.y),
        },
        None,
        &mut ctx,
    );
    wb.on_input(
        &WorkbenchInputEvent::MouseMove {
            viewport_pos: (to.x, to.y),
        },
        None,
        &mut ctx,
    );
    assert_ne!(ctx.document.body_placement(part), before);
    wb.cancel_pointer_gesture(&mut ctx);
    assert_eq!(ctx.document.body_placement(part), before);
    assert!(
        !wb.on_input(
            &WorkbenchInputEvent::MouseMove {
                viewport_pos: (to.x + 20.0, to.y),
            },
            None,
            &mut ctx
        )
        .redraw
    );
    assert_eq!(ctx.document.body_placement(part), before);
    assert!(
        core_document::HookOutcome::take(&mut ctx)
            .recorded
            .is_empty()
    );
}
