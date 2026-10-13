use emath::{pos2, vec2};

use super::*;

struct View {
    origin: DVec3,
}
impl GizmoView for View {
    fn project(&self, point: DVec3) -> Option<Pos2> {
        let p = point - self.origin;
        Some(pos2(300.0 + p.x as f32, 300.0 - p.y as f32))
    }
    fn ray(&self, point: Pos2) -> Option<(DVec3, DVec3)> {
        Some((
            self.origin
                + DVec3::new(
                    f64::from(point.x - 300.0),
                    f64::from(300.0 - point.y),
                    1000.0,
                ),
            -DVec3::Z,
        ))
    }
    fn forward(&self) -> DVec3 {
        -DVec3::Z
    }
    fn world_per_pixel(&self, _: DVec3) -> f64 {
        1.0
    }
}

fn placement(origin: DVec3) -> Placement {
    Placement {
        pivot: origin,
        orientation: DQuat::IDENTITY,
    }
}

/// The classic three-axis set at `origin`, world axes.
fn axes(origin: DVec3) -> HandleSet<Axis> {
    HandleSet::placement_axes(placement(origin))
}

/// The top view of the origin.
const TOP: View = View {
    origin: DVec3::ZERO,
};

/// A drag of the world axes at the origin, pressed at `at` in the top view.
fn begin(mode: Mode, handle: Handle<Axis>, at: Pos2) -> Drag<Axis> {
    Drag::begin(&TOP, &axes(DVec3::ZERO), mode, handle, at).unwrap()
}

/// The parts the world axes at the origin lay out in the top view.
fn top_parts(mode: Mode) -> Vec<Part<Axis>> {
    layout(&TOP, &axes(DVec3::ZERO), mode, &Style::default()).parts
}

#[test]
fn translation_is_press_relative_and_preserves_far_origin() {
    for origin in [DVec3::ZERO, DVec3::splat(1e9)] {
        let view = View { origin };
        let mut drag = Drag::begin(
            &view,
            &axes(origin),
            Mode::Translate,
            Handle::Axis(Axis::X),
            pos2(360.0, 300.0),
        )
        .unwrap();
        drag.sample(&view, pos2(400.0, 250.0), false);
        assert_eq!(drag.delta, Delta::Translation(DVec3::new(40.0, 0.0, 0.0)));
        drag.sample(&view, pos2(370.0, 310.0), false);
        assert_eq!(drag.delta, Delta::Translation(DVec3::new(10.0, 0.0, 0.0)));
    }
}

#[test]
fn plane_and_view_translation_use_the_grab_offset() {
    for handle in [Handle::Plane(Axis::X, Axis::Y), Handle::View] {
        let mut drag = begin(Mode::Translate, handle, pos2(330.0, 270.0));
        drag.sample(&TOP, pos2(350.0, 280.0), false);
        assert_eq!(drag.delta, Delta::Translation(DVec3::new(20.0, -10.0, 0.0)));
    }
}

#[test]
fn rotation_unwraps_multiple_turns_and_snaps() {
    let mut drag = begin(Mode::Rotate, Handle::Ring(Axis::Z), pos2(380.0, 300.0));
    for degrees in (10..=450).step_by(10) {
        let angle = f64::from(degrees).to_radians();
        drag.sample(
            &TOP,
            pos2(
                300.0 + 80.0 * angle.cos() as f32,
                300.0 - 80.0 * angle.sin() as f32,
            ),
            true,
        );
    }
    let Delta::Rotation { angle, .. } = drag.delta else {
        panic!()
    };
    assert!((angle.to_degrees() - 450.0).abs() < 1e-8);
}

#[test]
fn scale_is_positive_and_local_axes_apply_about_pivot() {
    let mut drag = begin(Mode::Scale, Handle::Axis(Axis::X), pos2(380.0, 300.0));
    drag.sample(&TOP, pos2(-2000.0, 300.0), false);
    let Delta::Scale(factors) = drag.delta else {
        panic!()
    };
    assert!(factors.x > 0.0);
    assert_eq!(factors.y, 1.0);
    assert_eq!(factors.z, 1.0);
    let placement = Placement {
        pivot: DVec3::new(5.0, 6.0, 7.0),
        orientation: DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
    };
    let result =
        Delta::Scale(DVec3::new(2.0, 1.0, 1.0)).apply(DVec3::new(5.0, 9.0, 7.0), placement);
    assert!((result - DVec3::new(5.0, 12.0, 7.0)).length() < 1e-12);
}

#[test]
fn edge_on_handles_stay_drawn_and_only_rings_are_picked() {
    let rotation = top_parts(Mode::Rotate);
    let rings: Vec<(Handle, Pick)> = rotation
        .iter()
        .filter(|p| matches!(p.handle, Handle::Ring(_)))
        .map(|p| (p.handle, p.pick))
        .collect();
    // X and Y are edge-on in the top view: drawn as segments and still grabbed.
    assert_eq!(
        rings,
        [
            (Handle::Ring(Axis::X), Pick::Fallback),
            (Handle::Ring(Axis::Y), Pick::Fallback),
            (Handle::Ring(Axis::Z), Pick::Handle),
        ]
    );
    let translation = top_parts(Mode::Translate);
    let pick = |handle| -> Vec<Pick> {
        translation
            .iter()
            .filter(|p| p.handle == handle)
            .map(|p| p.pick)
            .collect()
    };
    // End-on Z arrow: shaft and cone drawn, neither picked.
    assert_eq!(pick(Handle::Axis(Axis::Z)), [Pick::Never, Pick::Never]);
    assert_eq!(pick(Handle::Plane(Axis::Y, Axis::Z)), [Pick::Never]);
    assert_eq!(pick(Handle::Plane(Axis::X, Axis::Y)), [Pick::Handle]);
}

#[test]
fn edge_on_ring_turns_its_nearest_point_with_the_pointer() {
    // Top view: the X ring is edge-on, a vertical segment through the centre.
    // Its point nearest to the camera is +Z; turning +θ about X takes it to
    // (0, −sin θ, cos θ), screen down (`View` flips y). Dragging down 60 px is
    // +60/EDGE_ON_RADIAN_PX rad.
    let start = pos2(300.0, 260.0);
    let mut drag = begin(Mode::Rotate, Handle::Ring(Axis::X), start);
    drag.sample(&TOP, start + vec2(25.0, 60.0), false);
    let Delta::Rotation { axis, angle } = drag.delta else {
        panic!("ring drag rotates: {:?}", drag.delta)
    };
    assert_eq!(axis, DVec3::X);
    assert!((angle - 60.0 / EDGE_ON_RADIAN_PX).abs() < 1e-9, "{angle}");
    let near = Delta::Rotation { axis, angle }.apply(DVec3::Z, placement(DVec3::ZERO));
    let (from, to) = (TOP.project(DVec3::Z).unwrap(), TOP.project(near).unwrap());
    assert!(to.y > from.y, "the near point must follow the pointer down");
    // Shift snaps to 15°: 0.5 rad ≈ 28.6° → 30°.
    drag.sample(&TOP, start + vec2(0.0, 60.0), true);
    let Delta::Rotation { angle, .. } = drag.delta else {
        unreachable!()
    };
    assert!(
        (angle - std::f64::consts::FRAC_PI_6).abs() < 1e-12,
        "{angle}"
    );
}

#[test]
fn modes_stay_separate_arrows_or_rings() {
    let translate = top_parts(Mode::Translate);
    assert!(
        !translate
            .iter()
            .any(|p| matches!(p.handle, Handle::Ring(_)))
    );
    let rotate = top_parts(Mode::Rotate);
    assert!(
        rotate
            .iter()
            .all(|p| matches!(p.handle, Handle::Ring(_) | Handle::View) && p.fill == Fill::Line)
    );
    // The ring radius is the arrow length, 120 px by default; a ring facing the
    // camera is near all round, so it is whole.
    let ring = rotate
        .iter()
        .find(|p| p.handle == Handle::Ring(Axis::Z))
        .unwrap();
    assert_eq!(ring.points.len(), RING_SEGMENTS + 1);
    for point in &ring.points {
        assert!((point.distance(pos2(300.0, 300.0)) - 120.0).abs() < 1e-3);
    }
    // Facing the camera, the letter goes to the top of the ring.
    let (at, axis) = ring.letter.unwrap();
    assert_eq!(axis, Axis::Z);
    assert!((at - pos2(300.0, 300.0 - 134.0)).length() < 1e-3, "{at:?}");
    // In Translate the centre is a disc, not a diamond.
    let disc = translate.iter().find(|p| p.handle == Handle::View).unwrap();
    for point in &disc.points {
        assert!((point.distance(pos2(300.0, 300.0)) - VIEW_HALF).abs() < 1e-3);
    }
}

/// Orthographic view turned by `rotation` (camera looks along `rotation · −Z`),
/// one host unit per pixel, centred at (300, 300).
struct Turned {
    rotation: DQuat,
}
impl GizmoView for Turned {
    fn project(&self, point: DVec3) -> Option<Pos2> {
        let local = self.rotation.inverse() * point;
        Some(pos2(300.0 + local.x as f32, 300.0 - local.y as f32))
    }
    fn ray(&self, point: Pos2) -> Option<(DVec3, DVec3)> {
        let local = DVec3::new(
            f64::from(point.x - 300.0),
            f64::from(300.0 - point.y),
            1000.0,
        );
        Some((self.rotation * local, self.forward()))
    }
    fn forward(&self) -> DVec3 {
        self.rotation * -DVec3::Z
    }
    fn world_per_pixel(&self, _: DVec3) -> f64 {
        1.0
    }
}

#[test]
fn view_translation_snaps_in_the_screen_plane() {
    for rotation in [
        DQuat::IDENTITY,
        DQuat::from_rotation_z(0.4) * DQuat::from_rotation_x(1.0),
        DQuat::from_rotation_y(-0.7) * DQuat::from_rotation_z(1.1),
    ] {
        let view = Turned { rotation };
        let set = HandleSet::placement_axes(Placement {
            pivot: DVec3::ZERO,
            orientation: DQuat::from_rotation_x(0.6),
        });
        let mut drag = Drag::begin(
            &view,
            &set,
            Mode::Translate,
            Handle::View,
            pos2(300.0, 300.0),
        )
        .unwrap();
        drag.sample(&view, pos2(337.0, 319.0), true);
        let offset = translation_of(drag.delta);
        let expected = rotation * DVec3::new(40.0, -20.0, 0.0);
        assert!(
            (offset - expected).length() < 1e-4,
            "{offset:?} != {expected:?}"
        );
        assert!(offset.dot(view.forward()).abs() < 1e-9);
        drag.sample(&view, pos2(337.0, 319.0), false);
        let unsnapped = translation_of(drag.delta);
        assert!((unsnapped - rotation * DVec3::new(37.0, -19.0, 0.0)).length() < 1e-4);
    }
}

#[test]
fn far_half_of_a_ring_is_a_dim_hint_that_never_grabs() {
    // A generic oblique view: no ring is edge-on, none faces the camera.
    let view = Turned {
        rotation: DQuat::from_rotation_z(30_f64.to_radians())
            * DQuat::from_rotation_x(60_f64.to_radians()),
    };
    let forward = view.forward();
    let Layout { parts, decor } =
        layout(&view, &axes(DVec3::ZERO), Mode::Rotate, &Style::default());
    let hit_radius = Style::default().hit_radius;
    let grabs = |at: Pos2, handle: Handle| {
        parts
            .iter()
            .any(|p| p.handle == handle && p.distance(at) <= hit_radius)
    };
    for axis in [Axis::X, Axis::Y, Axis::Z] {
        let handle = Handle::Ring(axis);
        // On the ring of normal n, the point deepest along the line of sight is
        // the direction of `forward` within the ring plane; its opposite is nearest.
        let n = axis.unit();
        let deepest = (forward - n * n.dot(forward)).normalize();
        let far = view.project(deepest * 120.0).unwrap();
        let near = view.project(-deepest * 120.0).unwrap();
        assert!(
            grabs(near, handle),
            "{axis:?}: near point {near:?} must grab"
        );
        assert!(
            !grabs(far, handle),
            "{axis:?}: far point {far:?} must not grab"
        );
        // The far half is still drawn, dimmed, in the ring colour.
        assert!(
            decor.iter().any(|d| {
                matches!(d, Decor::Line(points, stroke)
                if stroke.width < 2.0
                    && stroke.paint == Paint::of(Ink::Axis(axis)).faded(0.35)
                    && points.iter().any(|p| p.distance(far) < 4.0))
            }),
            "{axis:?}: no dim far half through {far:?}"
        );
        // The letter marks the nearest point, just outside the ring.
        let part = parts.iter().find(|p| p.handle == handle).unwrap();
        let (at, letter_axis) = part.letter.unwrap();
        assert_eq!(letter_axis, axis);
        assert!(
            // Half a 3.75° segment at 120 px is ≈ 4 px.
            (at.distance(near) - 14.0).abs() < 6.0,
            "{axis:?}: letter {at:?}, near {near:?}"
        );
    }
}

#[test]
fn arrows_end_in_cones_that_carry_the_letter() {
    let parts = top_parts(Mode::Translate);
    let cone = parts
        .iter()
        .find(|p| p.handle == Handle::Axis(Axis::X) && p.fill == Fill::Solid)
        .expect("X arrow has a cone");
    // The hull covers the tip (120 px) and the cone base (0.78 × 120 ± 9 px).
    assert_eq!(cone.distance(pos2(419.0, 300.0)), 0.0);
    assert_eq!(cone.distance(pos2(95.0 + 300.0, 307.0)), 0.0);
    assert!(cone.distance(pos2(395.0, 320.0)) > 9.0);
    let (at, axis) = cone.letter.expect("the cone carries the axis letter");
    assert_eq!(axis, Axis::X);
    assert!((at.x - (300.0 + 1.16 * 120.0)).abs() < 1e-3 && (at.y - 300.0).abs() < 1e-3);
}

#[test]
fn shift_snaps_translation_to_a_zoom_step() {
    // One host unit per pixel: the step is the 1-2-5 value ≥ 10 px, i.e. 10.
    let mut drag = begin(Mode::Translate, Handle::Axis(Axis::X), pos2(360.0, 300.0));
    assert_eq!(drag.step, 10.0);
    drag.sample(&TOP, pos2(403.0, 300.0), true);
    assert_eq!(drag.delta, Delta::Translation(DVec3::new(40.0, 0.0, 0.0)));
    drag.sample(&TOP, pos2(403.0, 300.0), false);
    assert_eq!(drag.delta, Delta::Translation(DVec3::new(43.0, 0.0, 0.0)));
    // A translation carries the handles along.
    assert_eq!(drag.draw_placement().pivot, DVec3::new(43.0, 0.0, 0.0));
    // A plane rounds both of its components.
    let mut drag = begin(
        Mode::Translate,
        Handle::Plane(Axis::X, Axis::Y),
        pos2(345.0, 255.0),
    );
    drag.sample(&TOP, pos2(362.0, 241.0), true);
    assert_eq!(drag.delta, Delta::Translation(DVec3::new(20.0, 10.0, 0.0)));
}

#[test]
fn nice_step_picks_one_two_five() {
    for (raw, step) in [
        (0.3, 0.5),
        (1.0, 1.0),
        (1.2, 2.0),
        (2.0, 2.0),
        (4.9, 5.0),
        (7.0, 10.0),
        (12345.0, 20000.0),
        (0.001, 0.001),
    ] {
        assert!(
            (nice_step(raw) - step).abs() <= step * 1e-12,
            "{raw}: {}",
            nice_step(raw)
        );
    }
}

#[test]
fn feedback_text_names_the_value_along_the_handle_axes() {
    let world = placement(DVec3::ZERO);
    let text = |set: &HandleSet<Axis>, handle, delta| {
        feedback_text(set.grip(handle), delta, set.placement)
    };
    let translation = |x, y, z| Delta::Translation(DVec3::new(x, y, z));
    let world_set = HandleSet::placement_axes(world);
    assert_eq!(
        text(
            &world_set,
            Handle::Axis(Axis::X),
            translation(40.0, 0.0, 0.0)
        ),
        "X +40.00"
    );
    assert_eq!(
        text(
            &world_set,
            Handle::Plane(Axis::X, Axis::Y),
            translation(20.0, -10.0, 0.0)
        ),
        "X +20.00  Y -10.00"
    );
    assert_eq!(
        text(&world_set, Handle::View, translation(3.0, 4.0, 0.0)),
        "Δ +3.00 +4.00 +0.00  |5.00|"
    );
    // Local axes: X of a placement turned 90° about Z points along world Y.
    let turned = HandleSet::placement_axes(Placement {
        pivot: DVec3::ZERO,
        orientation: DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
    });
    assert_eq!(
        text(&turned, Handle::Axis(Axis::X), translation(0.0, 40.0, 0.0)),
        "X +40.00"
    );
    let rotation = Delta::Rotation {
        axis: DVec3::Z,
        angle: -std::f64::consts::FRAC_PI_2,
    };
    assert_eq!(text(&world_set, Handle::Ring(Axis::Z), rotation), "-90.0°");
    assert_eq!(
        text(
            &world_set,
            Handle::Axis(Axis::X),
            Delta::Scale(DVec3::new(1.25, 1.0, 1.0))
        ),
        "X ×1.250"
    );
}

#[test]
fn rotation_sector_sweeps_the_signed_angle_on_the_ring() {
    let style = Style::default();
    for (to, end) in [
        // Counter-clockwise on screen (+Y is up): +90°.
        (pos2(300.0, 150.0), pos2(300.0, 180.0)),
        // Clockwise: −90°.
        (pos2(300.0, 450.0), pos2(300.0, 420.0)),
    ] {
        let mut drag = begin(Mode::Rotate, Handle::Ring(Axis::Z), pos2(420.0, 300.0));
        drag.sample(&TOP, to, false);
        let sector = drag.sector(&TOP, &style).unwrap();
        // A rotation keeps the handles where they are.
        assert_eq!(drag.draw_placement().pivot, DVec3::ZERO);
        assert!(sector[0].distance(pos2(420.0, 300.0)) < 1e-3);
        assert!(
            sector.last().unwrap().distance(end) < 1e-3,
            "{to:?}: {:?}",
            sector.last()
        );
        for point in &sector {
            assert!((point.distance(pos2(300.0, 300.0)) - 120.0).abs() < 1e-3);
        }
    }
}

// ---- Handles from a list ----

type Key = &'static str;

/// A freedom written in world axes: drawn solid.
const WORLD: bool = false;
/// A freedom written in a datum's or the object's own axes: drawn dashed.
const DATUM: bool = true;

fn spec(key: Key, kind: JointKind, axis: Axis, dashed: bool, direction: DVec3) -> HandleSpec<Key> {
    HandleSpec {
        key,
        kind,
        axis,
        dashed,
        direction: direction.normalize(),
    }
}

/// Listed handles at the origin, world-aligned placement, no screen handle.
fn set_of(handles: Vec<HandleSpec<Key>>) -> HandleSet<Key> {
    HandleSet {
        placement: placement(DVec3::ZERO),
        handles,
        screen: false,
    }
}

/// Press at `start` over whatever the hit-test finds, drag to `end`, release,
/// through the gizmo's own input steps over the top TOP.
fn gesture_of(set: &HandleSet<Key>, mode: Mode, start: Pos2, end: Pos2) -> (Handle<Key>, Delta) {
    let mut gizmo = Gizmo::<Key>::default();
    let handle = gizmo
        .layout(&TOP, set, mode)
        .hit(start, gizmo.style.hit_radius)
        .unwrap_or_else(|| panic!("no handle under {start:?}"));
    assert!(matches!(
        gizmo.press(&TOP, set, mode, handle, start),
        Some(Event::Started(h)) if h == handle
    ));
    gizmo.drag(&TOP, Some(end), false);
    let Event::Finished(delta) = gizmo.release() else {
        unreachable!()
    };
    (handle, delta)
}

fn translation_of(delta: Delta) -> DVec3 {
    let Delta::Translation(offset) = delta else {
        panic!("expected a translation, got {delta:?}")
    };
    offset
}

#[test]
fn only_listed_handles_are_drawn() {
    let set = set_of(vec![spec("x", JointKind::Slide, Axis::X, WORLD, DVec3::X)]);
    let translate = layout(&TOP, &set, Mode::Translate, &Style::default());
    assert!(
        translate
            .parts
            .iter()
            .all(|p| p.handle == Handle::Axis("x"))
    );
    assert_eq!(
        translate.parts.len(),
        2,
        "shaft and cone, no plane, no centre disc"
    );
    // Rotate shows turns; there are none, so nothing at all, not even the ball.
    let rotate = layout(&TOP, &set, Mode::Rotate, &Style::default());
    assert!(rotate.parts.is_empty() && rotate.decor.is_empty());
}

#[test]
fn ring_about_an_arbitrary_direction_turns_by_the_joint_angle() {
    // Tilted 45° toward the camera: neither edge-on nor facing it.
    let direction = DVec3::new(0.0, 1.0, 1.0).normalize();
    let set = set_of(vec![spec("t", JointKind::Turn, Axis::Y, DATUM, direction)]);
    let ring = layout(&TOP, &set, Mode::Rotate, &Style::default())
        .parts
        .into_iter()
        .find(|p| p.handle == Handle::Ring("t"))
        .expect("the listed turn has a ring");
    // World +X is in the ring plane; its point is on the near half (depth 0, the
    // edge of the halves, counts as near by `FRONT_SLACK`).
    let start = pos2(420.0, 300.0);
    assert!(ring.distance(start) < 1.0);
    let mut drag = Drag::begin(&TOP, &set, Mode::Rotate, Handle::Ring("t"), start).unwrap();
    let target =
        DQuat::from_axis_angle(direction, 30_f64.to_radians()) * DVec3::new(120.0, 0.0, 0.0);
    drag.sample(&TOP, TOP.project(target).unwrap(), false);
    let Delta::Rotation { axis, angle } = drag.delta else {
        panic!("{:?}", drag.delta)
    };
    assert!((axis - direction).length() < 1e-12);
    assert!(
        (angle.to_degrees() - 30.0).abs() < 1e-4,
        "{}°",
        angle.to_degrees()
    );
}

#[test]
fn skewed_pair_is_a_parallelogram_and_gram_recovers_the_drag() {
    let first = DVec3::X;
    let second = DVec3::new(0.5, 3_f64.sqrt() / 2.0, 0.0);
    let set = set_of(vec![
        spec("a", JointKind::Slide, Axis::X, DATUM, first),
        spec("b", JointKind::Slide, Axis::Y, DATUM, second),
    ]);
    let plane = layout(&TOP, &set, Mode::Translate, &Style::default())
        .parts
        .into_iter()
        .find(|p| p.handle == Handle::Plane("a", "b"))
        .expect("a non-collinear pair has a planar handle");
    // Corners along the two freedoms, not along X and Y.
    for ((x, y), corner) in [
        (PLANE_MIN, PLANE_MIN),
        (PLANE_MAX, PLANE_MIN),
        (PLANE_MAX, PLANE_MAX),
        (PLANE_MIN, PLANE_MAX),
    ]
    .into_iter()
    .zip(&plane.points)
    {
        let expected = TOP.project((first * x + second * y) * 120.0).unwrap();
        assert!(
            corner.distance(expected) < 1e-3,
            "{corner:?} vs {expected:?}"
        );
    }
    // The parallelogram's centre (72, 41.6) lies outside the orthogonal square
    // (x ≤ 62.4): only a parallelogram catches it.
    let centre = pos2(372.0, 258.4);
    let (handle, delta) = gesture_of(&set, Mode::Translate, centre, centre + vec2(10.0, -20.0));
    assert_eq!(handle, Handle::Plane("a", "b"));
    let offset = translation_of(delta);
    assert!(
        (offset - DVec3::new(10.0, 20.0, 0.0)).length() < 1e-3,
        "{offset:?}"
    );
    let pair = SlidePlane::new(first, second).unwrap();
    let (a, b) = pair.split(offset);
    assert!((first * a + second * b - offset).length() < 1e-9);
    // b = 20 / sin 60°, a = 10 − b·cos 60°: the plate shows joint increments.
    assert!(
        (b - 40.0 / 3_f64.sqrt()).abs() < 1e-3 && (a - (10.0 - 20.0 / 3_f64.sqrt())).abs() < 1e-3
    );
    assert_eq!(
        feedback_text(set.grip(handle), delta, set.placement),
        "X -1.55  Y +23.09"
    );
    // Shift rounds the joint increments (step 10): a → 0, b → 20.
    let mut drag = Drag::begin(&TOP, &set, Mode::Translate, handle, centre).unwrap();
    drag.sample(&TOP, centre + vec2(10.0, -20.0), true);
    let snapped = translation_of(drag.delta);
    assert!((snapped - second * 20.0).length() < 1e-9, "{snapped:?}");
}

#[test]
fn collinear_pair_has_no_plane() {
    // World X and a datum axis along −X: the same line, no plane between them.
    let set = set_of(vec![
        spec("wx", JointKind::Slide, Axis::X, WORLD, DVec3::X),
        spec("dx", JointKind::Slide, Axis::X, DATUM, -DVec3::X),
        spec("y", JointKind::Slide, Axis::Y, WORLD, DVec3::Y),
    ]);
    let planes: Vec<Handle<Key>> = layout(&TOP, &set, Mode::Translate, &Style::default())
        .parts
        .iter()
        .map(|p| p.handle)
        .filter(|h| matches!(h, Handle::Plane(..)))
        .collect();
    assert_eq!(planes, [Handle::Plane("wx", "y"), Handle::Plane("dx", "y")]);
}

#[test]
fn entry_and_local_handles_draw_dashed() {
    let set = set_of(vec![
        spec("w", JointKind::Slide, Axis::X, WORLD, DVec3::X),
        spec("e", JointKind::Slide, Axis::Y, DATUM, DVec3::Y),
        spec("t", JointKind::Turn, Axis::Z, DATUM, DVec3::Z),
    ]);
    let translate = layout(&TOP, &set, Mode::Translate, &Style::default()).parts;
    let shaft = |key| {
        translate
            .iter()
            .find(|p| p.handle == Handle::Axis(key) && p.fill == Fill::Line)
            .unwrap()
            .dashed
    };
    assert!(!shaft("w"));
    assert!(shaft("e"));
    // Cones stay solid; a plane with a datum side is dashed.
    assert!(
        translate
            .iter()
            .filter(|p| p.fill == Fill::Solid)
            .all(|p| !p.dashed)
    );
    assert!(
        translate
            .iter()
            .find(|p| p.handle == Handle::Plane("w", "e"))
            .unwrap()
            .dashed
    );
    let rotate = layout(&TOP, &set, Mode::Rotate, &Style::default()).parts;
    assert!(
        rotate
            .iter()
            .find(|p| p.handle == Handle::Ring("t"))
            .unwrap()
            .dashed
    );
}

#[test]
fn edge_on_picking_follows_an_arbitrary_direction() {
    // Top view, the camera looks along −Z.
    let set = set_of(vec![
        // Nearly along the line of sight: the arrow is 6 px long on screen.
        spec(
            "steep",
            JointKind::Slide,
            Axis::Z,
            DATUM,
            DVec3::new(0.05, 0.0, 1.0),
        ),
        spec(
            "flat",
            JointKind::Slide,
            Axis::X,
            DATUM,
            DVec3::new(1.0, 0.0, 0.3),
        ),
        // Ring axis 2° off the screen plane: the ring is edge-on.
        spec(
            "edge",
            JointKind::Turn,
            Axis::X,
            DATUM,
            DVec3::new(1.0, 1.0, 0.05),
        ),
        spec(
            "open",
            JointKind::Turn,
            Axis::Y,
            DATUM,
            DVec3::new(1.0, 1.0, 0.3),
        ),
        // Its plane with `flat` contains the line of sight (normal ⟂ forward).
        spec(
            "up",
            JointKind::Slide,
            Axis::Y,
            DATUM,
            DVec3::new(0.0, 0.05, 1.0),
        ),
    ]);
    // Every handle is drawn; `Pick` says how it can be grabbed.
    let handles = |mode| -> Vec<(Handle<Key>, Pick)> {
        layout(&TOP, &set, mode, &Style::default())
            .parts
            .iter()
            .map(|p| (p.handle, p.pick))
            .collect()
    };
    let translate = handles(Mode::Translate);
    assert!(translate.contains(&(Handle::Axis("steep"), Pick::Never)));
    assert!(translate.contains(&(Handle::Axis("up"), Pick::Never)));
    assert!(translate.contains(&(Handle::Axis("flat"), Pick::Handle)));
    assert!(translate.contains(&(Handle::Plane("flat", "up"), Pick::Never)));
    let rotate = handles(Mode::Rotate);
    assert!(rotate.contains(&(Handle::Ring("edge"), Pick::Fallback)));
    assert!(rotate.contains(&(Handle::Ring("open"), Pick::Handle)));
}
