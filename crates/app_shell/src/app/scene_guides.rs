//! What the scene draws to read the model by: a grid on the ground plane
//! and the origin's three planes. Neither is part of the document, both
//! follow the view, and the pick pass sees neither.

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
const ORIGIN_PLANE_OPACITY: f32 = 0.12;
const ORIGIN_PLANE_HOVER_OPACITY: f32 = 0.35;
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

/// The spacing of a grid's finest lines at `world_per_px`: the power of
/// ten that sets them 3 to 30 pixels apart.
pub(crate) fn grid_step(world_per_px: f32) -> f32 {
    10f32.powf((world_per_px * 3.0).log10().ceil())
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

/// The grid on the ground plane, the plane through the origin square to
/// the axis preset's up, centred under what the view looks at and
/// reaching well past the viewport's edges.
pub(crate) fn ground_grid(view: &GuideView, axes: &AxisSystem) -> GridSubmission {
    let up = axes.up_vec().normalize();
    let [(u, u_color), (v, v_color)] = axes_across(up);
    let center = view.focal - up * view.focal.dot(up);
    let world_per_px = view.world_per_px_at(center);
    let reach_px = view.viewport_px[0].max(view.viewport_px[1]) * 4.0;
    let radius = (world_per_px * reach_px).max(view.eye.distance(center) * 4.0);
    GridSubmission {
        origin: [0.0; 3],
        u: u.to_array(),
        v: v.to_array(),
        step: grid_step(world_per_px),
        center: center.to_array(),
        radius,
        forward: view.forward.to_array(),
        depth_range: view.depth_range,
        color: rgb(tokens::TEXT2),
        minor_alpha: GRID_MINOR_ALPHA,
        major_alpha: GRID_MAJOR_ALPHA,
        u_axis_color: rgb(u_color),
        v_axis_color: rgb(v_color),
        axis_alpha: GRID_AXIS_ALPHA,
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
    fn the_finest_lines_stand_three_to_thirty_pixels_apart() {
        for world_per_px in [0.0007, 0.01, 0.033, 0.34, 2.0, 99.0] {
            let px = grid_step(world_per_px) / world_per_px;
            assert!((3.0..30.0).contains(&px), "{world_per_px}: {px} px");
        }
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
            let grid = ground_grid(&view, &axes);
            let (u, v) = (Vec3::from_array(grid.u), Vec3::from_array(grid.v));
            assert!(u.dot(up).abs() < 1e-6 && v.dot(up).abs() < 1e-6);
            assert!(u.dot(v).abs() < 1e-6);
            assert!(Vec3::from_array(grid.center).dot(up).abs() < 1e-4);
        }
    }

    #[test]
    fn a_z_up_grid_runs_along_x_and_y_in_their_colours() {
        let view = top_view(PixelSize::Flat(0.5));
        let grid = ground_grid(
            &view,
            &AxisSystem::from_preset(axes::AxisPreset::ZUpRightHanded),
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
