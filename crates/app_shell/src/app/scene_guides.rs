//! What the scene draws to read the model by: a grid on an origin plane,
//! or on the active bench's own plane while it hands one, and the
//! origin's three planes. Neither is part of the document, both follow
//! the view, and the pick pass sees neither.

use std::sync::Arc;

use axes::AxisSystem;
use core_document::datum::BasePlane;
use glam::Vec3;
use kernel_api::TriMesh;
use render_wgpu::{BodySubmission, GridSubmission, HighlightState};
use settings::ProjectionMode;
use ui_kit::tokens;
use uuid::Uuid;

use crate::camera::CameraController;

/// The world's axes, each in its colour.
const WORLD_AXES: [(Vec3, egui::Color32); 3] = [
    (Vec3::X, tokens::AXIS_X),
    (Vec3::Y, tokens::AXIS_Y),
    (Vec3::Z, tokens::AXIS_Z),
];

/// How far an origin plane reaches from the origin, in pixels on screen.
const ORIGIN_PLANE_HALF_PX: f32 = 80.0;
/// How much of its colour an origin plane's face takes, and the one
/// under the cursor while they are picked from.
const ORIGIN_PLANE_OPACITY: f32 = 0.035;
const ORIGIN_PLANE_HOVER_OPACITY: f32 = 0.18;
/// How opaque the grid's finest lines are, lines far apart, and the axes.
const GRID_MINOR_ALPHA: f32 = 0.08;
const GRID_MAJOR_ALPHA: f32 = 0.22;
const GRID_AXIS_ALPHA: f32 = 0.7;

fn rgb(color: egui::Color32) -> [f32; 3] {
    [color.r(), color.g(), color.b()].map(|c| f32::from(c) / 255.0)
}

/// What of the view the guides are laid out by.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GuideView {
    pub eye: Vec3,
    /// Unit direction the view looks along.
    pub forward: Vec3,
    /// The point the view looks at and orbits about.
    pub focal: Vec3,
    /// The view's near and far depths along `forward` from `eye`.
    pub depth_range: [f32; 2],
    /// The viewport's width and height, in pixels.
    pub viewport_px: [f32; 2],
    /// A pixel's length at depth `d`: `d · per_depth` in perspective,
    /// `flat` whatever the depth in an orthographic view.
    pub pixel: PixelSize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PixelSize {
    PerDepth(f32),
    Flat(f32),
}

impl GuideView {
    pub(crate) fn of(camera: &CameraController) -> Self {
        let state = &camera.state;
        let (forward, _) = camera.view_basis();
        let height = state.viewport_size.1.max(1) as f32;
        let pixel = match state.projection {
            ProjectionMode::Perspective => {
                PixelSize::PerDepth(2.0 * (state.height_angle_rad as f32 * 0.5).tan() / height)
            }
            ProjectionMode::Orthographic => PixelSize::Flat(state.ortho_height as f32 / height),
        };
        Self {
            eye: Vec3::from_array(camera.position()),
            forward: forward.normalize_or_zero(),
            focal: camera.focal_point_world(),
            depth_range: [state.near_plane as f32, state.far_plane as f32],
            viewport_px: [state.viewport_size.0 as f32, height],
            pixel,
        }
    }

    /// A pixel's length in world units at `point`.
    pub(crate) fn world_per_px_at(&self, point: Vec3) -> f32 {
        match self.pixel {
            PixelSize::Flat(size) => size,
            PixelSize::PerDepth(per_depth) => {
                let depth = (point - self.eye)
                    .dot(self.forward)
                    .max(self.depth_range[0].max(1e-4));
                depth * per_depth
            }
        }
    }
}

/// The world axes lying in the plane square to `up`, in X, Y, Z order.
fn axes_across(up: Vec3) -> [(Vec3, egui::Color32); 2] {
    let mut across = WORLD_AXES
        .into_iter()
        .filter(|(axis, _)| axis.dot(up).abs() < 0.5);
    match (across.next(), across.next()) {
        (Some(u), Some(v)) => [u, v],
        _ => [WORLD_AXES[0], WORLD_AXES[1]],
    }
}

/// A grid plane kept until another faces the view more squarely by 0.15.
#[derive(Default)]
pub(crate) struct GridPlaneState(Option<Vec3>);

impl GridPlaneState {
    pub(crate) fn grid(
        &mut self,
        view: &GuideView,
        axes: &AxisSystem,
        choice: settings::GridPlane,
        look: &settings::RenderingSettings,
    ) -> GridSubmission {
        let mut up = self.0.unwrap_or_else(|| axes.up_vec().normalize());
        if choice == settings::GridPlane::Ground {
            up = axes.up_vec().normalize();
        } else {
            let best = WORLD_AXES
                .iter()
                .map(|(axis, _)| *axis)
                .max_by(|a, b| {
                    view.forward
                        .dot(*a)
                        .abs()
                        .total_cmp(&view.forward.dot(*b).abs())
                })
                .unwrap();
            if view.forward.dot(best).abs() > view.forward.dot(up).abs() + 0.15 {
                up = best;
            }
        }
        self.0 = Some(up);
        scene_grid(view, up, look)
    }

    /// The nearest and farthest points along the view of the grid an
    /// orthographic `view` shows, its plane the one last chosen: the part of
    /// its patch inside the view's box, which the clip planes must take in
    /// or zooming out cuts the grid off where it nears the eye. `None` in
    /// perspective, whose near plane sits by the eye anyway.
    pub(crate) fn orthographic_reach(
        &self,
        view: &GuideView,
        screen_up: Vec3,
        axes: &AxisSystem,
        look: &settings::RenderingSettings,
    ) -> Option<[Vec3; 2]> {
        let PixelSize::Flat(size) = view.pixel else {
            return None;
        };
        let up = self.0.unwrap_or_else(|| axes.up_vec().normalize());
        let grid = scene_grid(view, up, look);
        let facing = view.forward.dot(up);
        let across = (1.0 - facing * facing).max(0.0).sqrt() * grid.radius;
        let middle = (Vec3::from_array(grid.center) - view.eye).dot(view.forward);
        let (mut near, mut far) = (middle - across, middle + across);
        if facing.abs() > 1e-4 {
            let right = view.forward.cross(screen_up).normalize_or_zero();
            let (half_w, half_h) = (
                size * view.viewport_px[0] * 0.5,
                size * view.viewport_px[1] * 0.5,
            );
            let depths = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)].map(|(x, y)| {
                let corner = view.eye + right * (x * half_w) + screen_up * (y * half_h);
                -corner.dot(up) / facing
            });
            near = near.max(depths.iter().copied().fold(f32::INFINITY, f32::min));
            far = far.min(depths.iter().copied().fold(f32::NEG_INFINITY, f32::max));
        }
        (near <= far).then(|| {
            [
                view.eye + view.forward * near,
                view.eye + view.forward * far,
            ]
        })
    }
}

/// How a grid draws: its lines' colour, how opaque the finest are and
/// lines far apart, its axes' colours and opacity, and its dash.
struct GridLook {
    color: [f32; 3],
    minor_alpha: f32,
    major_alpha: f32,
    axis_colors: [[f32; 3]; 2],
    axis_alpha: f32,
    dash: [f32; 2],
}

/// The grid through the origin square to `up`, its lines a unit apart
/// and every power of ten of it.
fn scene_grid(view: &GuideView, up: Vec3, look: &settings::RenderingSettings) -> GridSubmission {
    let [(u, u_color), (v, v_color)] = axes_across(up);
    let opacity = look.grid_opacity.clamp(0.0, 1.0);
    let look = GridLook {
        color: look.grid_color,
        minor_alpha: GRID_MINOR_ALPHA * opacity,
        major_alpha: GRID_MAJOR_ALPHA * opacity,
        axis_colors: [rgb(u_color), rgb(v_color)],
        axis_alpha: GRID_AXIS_ALPHA,
        dash: look.grid_style.dash(),
    };
    plane_grid(view, Vec3::ZERO, u, v, 1.0, look)
}

/// The grid a bench draws on its own plane, with no axes of its own: the
/// bench draws them.
pub(crate) fn bench_grid(view: &GuideView, grid: &core_document::PlaneGrid) -> GridSubmission {
    let opacity = grid.opacity.clamp(0.0, 1.0);
    let look = GridLook {
        color: grid.color,
        minor_alpha: opacity * GRID_MINOR_ALPHA / GRID_MAJOR_ALPHA,
        major_alpha: opacity,
        axis_colors: [grid.color; 2],
        axis_alpha: 0.0,
        dash: grid.dash,
    };
    let [origin, u, v] = [grid.origin, grid.x, grid.y].map(Vec3::from_array);
    plane_grid(view, origin, u, v, grid.base, look)
}

/// The grid on the plane through `origin` along `u` and `v`, its lines
/// whole multiples of `base` times a power of ten apart, centred under
/// the view's focal point and reaching well past the viewport's edges.
fn plane_grid(
    view: &GuideView,
    origin: Vec3,
    u: Vec3,
    v: Vec3,
    base: f32,
    look: GridLook,
) -> GridSubmission {
    let normal = u.cross(v).normalize_or_zero();
    let center = view.focal - normal * (view.focal - origin).dot(normal);
    let world_per_px = view.world_per_px_at(center);
    let reach_px = view.viewport_px[0].max(view.viewport_px[1]) * 4.0;
    let radius = (world_per_px * reach_px).max(view.eye.distance(center) * 4.0);
    GridSubmission {
        origin: origin.to_array(),
        u: u.to_array(),
        v: v.to_array(),
        step: core_document::grid::finest_step(f64::from(base), f64::from(world_per_px)) as f32,
        center: center.to_array(),
        radius,
        forward: view.forward.to_array(),
        depth_range: view.depth_range,
        color: look.color,
        minor_alpha: look.minor_alpha,
        major_alpha: look.major_alpha,
        u_axis_color: look.axis_colors[0],
        v_axis_color: look.axis_colors[1],
        axis_alpha: look.axis_alpha,
        dash: look.dash,
        fade_px: core_document::grid::FADE_PX,
    }
}

/// How far the origin's planes reach from it in `view`: a fixed size on
/// screen, in steps of about 9 % so a slow zoom rebuilds them seldom.
pub(crate) fn origin_plane_half(view: &GuideView) -> f32 {
    let half = view.world_per_px_at(Vec3::ZERO) * ORIGIN_PLANE_HALF_PX;
    ((half.log2() * 8.0).round() / 8.0).exp2()
}

/// One origin plane's square, `half` out from the origin along its axes:
/// its face and its outline.
fn origin_plane_meshes(plane: BasePlane, half: f32) -> (TriMesh, TriMesh) {
    let (origin, normal, x) = plane.frame();
    let (origin, normal, x) = (
        Vec3::from_array(origin),
        Vec3::from_array(normal),
        Vec3::from_array(x),
    );
    let y = normal.cross(x);
    let positions: Vec<[f32; 3]> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .map(|(a, b)| (origin + (x * a + y * b) * half).to_array())
        .to_vec();
    let face = TriMesh {
        normals: vec![normal.to_array(); 4],
        positions: positions.clone(),
        indices: vec![0, 1, 2, 0, 2, 3],
        ..TriMesh::default()
    };
    let outline = TriMesh {
        normals: vec![normal.to_array(); 4],
        positions,
        edges: vec![0, 1, 1, 2, 2, 3, 3, 0],
        ..TriMesh::default()
    };
    (face, outline)
}

/// The origin plane a ray from `origin` along `dir` meets first inside its
/// square, `half` out from the origin, and how far along the ray it does.
pub(crate) fn origin_plane_hit(origin: Vec3, dir: Vec3, half: f32) -> Option<(BasePlane, f32)> {
    BasePlane::ALL
        .into_iter()
        .filter_map(|plane| {
            let (center, normal, x) = plane.frame();
            let (center, normal, x) = (
                Vec3::from_array(center),
                Vec3::from_array(normal),
                Vec3::from_array(x),
            );
            let across = dir.dot(normal);
            if across.abs() < 1e-6 {
                return None;
            }
            let t = (center - origin).dot(normal) / across;
            let at = origin + dir * t - center;
            let inside = at.dot(x).abs() <= half && at.dot(normal.cross(x)).abs() <= half;
            (t > 0.0 && inside).then_some((plane, t))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The origin's planes as drawn, rebuilt only when their size moves.
pub(crate) struct OriginPlanes {
    /// Two ids a plane: its face and its outline.
    ids: [[Uuid; 2]; 3],
    built: Option<(u32, [[Arc<TriMesh>; 2]; 3])>,
}

impl OriginPlanes {
    pub(crate) fn new() -> Self {
        Self {
            ids: std::array::from_fn(|_| [Uuid::new_v4(), Uuid::new_v4()]),
            built: None,
        }
    }

    /// The bodies that draw the planes `half` out from the origin: each a
    /// see-through face in the colour of the axis square to it, and its
    /// outline, `hovered` the more opaque.
    pub(crate) fn bodies(&mut self, half: f32, hovered: Option<BasePlane>) -> Vec<BodySubmission> {
        let key = half.to_bits();
        let meshes = match &self.built {
            Some((built, meshes)) if *built == key => meshes,
            _ => {
                let meshes = BasePlane::ALL.map(|plane| {
                    let (face, outline) = origin_plane_meshes(plane, half);
                    [Arc::new(face), Arc::new(outline)]
                });
                &self.built.insert((key, meshes)).1
            }
        };
        let mut bodies = Vec::with_capacity(6);
        for ((plane, ids), [face, outline]) in BasePlane::ALL.iter().zip(&self.ids).zip(meshes) {
            let normal = Vec3::from_array(plane.frame().1);
            let color = WORLD_AXES
                .iter()
                .find(|(axis, _)| axis.dot(normal).abs() > 0.5)
                .map_or(tokens::TEXT2, |(_, color)| *color);
            let body = |id: Uuid, mesh: &Arc<TriMesh>, opacity: f32| BodySubmission {
                scalar: None,
                id,
                revision: u64::from(key),
                mesh: Arc::clone(mesh),
                color: rgb(color),
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
            };
            let opacity = if hovered == Some(*plane) {
                ORIGIN_PLANE_HOVER_OPACITY
            } else {
                ORIGIN_PLANE_OPACITY
            };
            bodies.push(body(ids[0], face, opacity));
            bodies.push(body(ids[1], outline, 1.0));
        }
        bodies
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top_view(pixel: PixelSize) -> GuideView {
        GuideView {
            eye: Vec3::new(30.0, 40.0, 500.0),
            forward: Vec3::NEG_Z,
            focal: Vec3::new(30.0, 40.0, 0.0),
            depth_range: [0.1, 5000.0],
            viewport_px: [1600.0, 1000.0],
            pixel,
        }
    }

    #[test]
    fn a_zoomed_out_orthographic_view_clips_round_the_grid_it_shows() {
        // Looking down on the ground at 35° with the eye 200 mm from the
        // origin, 2 mm a pixel: the bottom of the view meets the ground far
        // nearer the eye than any body does.
        let forward = Vec3::new(0.0, 1.0, -0.7).normalize();
        let screen_up = Vec3::X.cross(forward).normalize();
        let view = GuideView {
            eye: -forward * 200.0,
            forward,
            focal: Vec3::ZERO,
            depth_range: [190.0, 210.0],
            viewport_px: [1600.0, 1000.0],
            pixel: PixelSize::Flat(2.0),
        };
        let axes = AxisSystem::from_preset(axes::AxisPreset::ZUpRightHanded);
        let look = settings::RenderingSettings::default();
        let [near, far] = GridPlaneState::default()
            .orthographic_reach(&view, screen_up, &axes, &look)
            .unwrap();
        let depth = |p: Vec3| (p - view.eye).dot(forward);
        let half_h = 1000.0;
        for y in [-half_h, half_h] {
            let corner = view.eye + screen_up * y;
            let shown = depth(corner - forward * corner.z / forward.z);
            assert!(depth(near) <= shown + 1e-2 && shown <= depth(far) + 1e-2);
        }
        assert!(depth(near) < 0.0, "the ground reaches behind the eye");
        let flat = GuideView {
            pixel: PixelSize::PerDepth(0.001),
            ..view
        };
        assert!(
            GridPlaneState::default()
                .orthographic_reach(&flat, screen_up, &axes, &look)
                .is_none()
        );
    }

    #[test]
    fn the_ground_grid_lies_square_to_the_presets_up() {
        let view = top_view(PixelSize::PerDepth(0.001));
        for preset in [
            axes::AxisPreset::ZUpRightHanded,
            axes::AxisPreset::RightHandedZForward,
        ] {
            let axes = AxisSystem::from_preset(preset);
            let up = axes.up_vec();
            let grid = GridPlaneState::default().grid(
                &view,
                &axes,
                settings::GridPlane::Ground,
                &Default::default(),
            );
            let (u, v) = (Vec3::from_array(grid.u), Vec3::from_array(grid.v));
            assert!(u.dot(up).abs() < 1e-6 && v.dot(up).abs() < 1e-6);
            assert!(u.dot(v).abs() < 1e-6);
            assert!(Vec3::from_array(grid.center).dot(up).abs() < 1e-4);
        }
    }

    #[test]
    fn a_z_up_grid_runs_along_x_and_y_in_their_colours() {
        let view = top_view(PixelSize::Flat(0.5));
        let grid = GridPlaneState::default().grid(
            &view,
            &AxisSystem::from_preset(axes::AxisPreset::ZUpRightHanded),
            settings::GridPlane::FollowView,
            &Default::default(),
        );
        assert_eq!((grid.u, grid.v), ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]));
        assert_eq!(grid.u_axis_color, rgb(tokens::AXIS_X));
        assert_eq!(grid.v_axis_color, rgb(tokens::AXIS_Y));
        assert_eq!(grid.center, [30.0, 40.0, 0.0]);
        // Half a unit a pixel: lines 1 apart are 2 px, too close; 10 are 20.
        assert_eq!(grid.step, 10.0);
        // The patch reaches past the viewport whichever way it is turned.
        assert!(grid.radius >= 0.5 * 1600.0);
    }

    #[test]
    fn a_benchs_grid_lies_on_its_plane_in_decades_of_its_step() {
        let view = top_view(PixelSize::Flat(0.5));
        let plane = core_document::PlaneGrid {
            origin: [5.0, -3.0, 12.0],
            x: [0.0, 1.0, 0.0],
            y: [-1.0, 0.0, 0.0],
            base: 7.5,
            color: [0.2, 0.3, 0.4],
            opacity: 0.3,
            dash: [5.0, 1.5],
        };
        let grid = bench_grid(&view, &plane);
        assert_eq!(
            (grid.origin, grid.u, grid.v),
            (plane.origin, plane.x, plane.y)
        );
        // Half a unit a pixel: 7.5 apart is 15 px, past the first 3.
        assert_eq!(grid.step, 7.5);
        assert_eq!(grid.center, [30.0, 40.0, 12.0]);
        assert_eq!(
            (grid.color, grid.dash, grid.axis_alpha),
            (plane.color, plane.dash, 0.0)
        );
        assert_eq!(grid.major_alpha, 0.3);
        assert!(grid.minor_alpha < grid.major_alpha);
    }

    #[test]
    fn grid_faces_the_view_and_holds_near_a_diagonal() {
        let axes = AxisSystem::from_preset(axes::AxisPreset::ZUpRightHanded);
        let mut state = GridPlaneState::default();
        let mut view = top_view(PixelSize::Flat(0.5));
        for (direction, normal) in [
            (Vec3::NEG_Z, Vec3::Z),
            (Vec3::new(0.0, 0.72, -0.69), Vec3::Z),
            (Vec3::Y, Vec3::Y),
            (Vec3::new(0.0, 0.69, -0.72), Vec3::Y),
            (Vec3::NEG_X, Vec3::X),
            (Vec3::Z, Vec3::Z),
        ] {
            view.forward = direction.normalize();
            let grid = state.grid(
                &view,
                &axes,
                settings::GridPlane::FollowView,
                &Default::default(),
            );
            let u = Vec3::from_array(grid.u);
            let v = Vec3::from_array(grid.v);
            assert!(u.cross(v).dot(normal).abs() > 0.99);
            assert!(Vec3::from_array(grid.center).dot(normal).abs() < 1e-4);
        }
    }

    #[test]
    fn a_ray_picks_the_origin_plane_it_meets_first_inside_its_square() {
        let down = Vec3::NEG_Z;
        let hit = origin_plane_hit(Vec3::new(10.0, 20.0, 500.0), down, 80.0);
        assert_eq!(hit, Some((BasePlane::XY, 500.0)));
        assert_eq!(
            origin_plane_hit(Vec3::new(200.0, 0.0, 500.0), down, 80.0),
            None
        );
        assert_eq!(
            origin_plane_hit(Vec3::new(10.0, 20.0, -5.0), down, 80.0),
            None
        );
        // From the front, slanting down: the XZ plane comes before the XY.
        let dir = Vec3::new(0.0, 1.0, -0.1).normalize();
        let hit = origin_plane_hit(Vec3::new(30.0, -100.0, 20.0), dir, 80.0);
        assert_eq!(hit.map(|(plane, _)| plane), Some(BasePlane::XZ));
    }

    #[test]
    fn origin_planes_keep_their_size_on_screen() {
        let near = top_view(PixelSize::PerDepth(0.001));
        let mut far = near;
        far.eye.z *= 4.0;
        let ratio = origin_plane_half(&far) / origin_plane_half(&near);
        assert!((ratio - 4.0).abs() < 0.4, "{ratio}");
        let px = origin_plane_half(&near) / near.world_per_px_at(Vec3::ZERO);
        assert!((px / ORIGIN_PLANE_HALF_PX - 1.0).abs() < 0.05, "{px}");
    }

    #[test]
    fn each_origin_plane_takes_the_colour_of_the_axis_square_to_it() {
        let bodies = OriginPlanes::new().bodies(10.0, None);
        let faces: Vec<_> = bodies.iter().step_by(2).map(|b| b.color).collect();
        assert_eq!(
            faces,
            [
                rgb(tokens::AXIS_Z),
                rgb(tokens::AXIS_Y),
                rgb(tokens::AXIS_X)
            ]
        );
        for (face, plane) in bodies.iter().step_by(2).zip(BasePlane::ALL) {
            let normal = Vec3::from_array(plane.frame().1).abs();
            let (lo, hi) = face.mesh.bounds().expect("a face has bounds");
            let span = Vec3::from_array(hi) - Vec3::from_array(lo);
            assert_eq!(span + normal * 20.0, Vec3::splat(20.0), "{plane:?}");
            assert!(face.opacity < 1.0 && !face.pickable);
        }
    }
}
