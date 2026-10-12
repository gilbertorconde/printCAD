//! printCAD's renderer, on wgpu: one renderer for Vulkan, Metal and
//! DirectX 12, and a path to the web's WebGPU. The app hands it a
//! [`FrameSubmission`] each frame through [`RenderBackend`] and gets pixels:
//! the shaded scene, its edges and overlays, GPU picking with an
//! asynchronous readback, a picture of the scene on request, and egui on
//! top.

mod core;
mod grid;
mod mesh;
mod readback;

pub use egui_wgpu::wgpu;
pub use mesh::{DrawStats, GpuLight, LightingData};

use egui::{ClippedPrimitive, TexturesDelta};
use kernel_api::TriMesh;
use std::fmt;
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, info};
use uuid::Uuid;
use winit::{dpi::PhysicalSize, event_loop::OwnedDisplayHandle, window::Window};

use core::RendererCore;

fn identity_matrix() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// The world point drawn at window pixel `(screen_x, screen_y)` at `depth`.
///
/// Window coordinates span the whole window; `viewport` is where the 3D view
/// sits in it. The camera's `view_proj` maps to a framebuffer whose Y runs
/// down, so NDC runs Y-down here too: the top of the viewport is NDC -1.
pub(crate) fn unproject(
    screen_x: f32,
    screen_y: f32,
    depth: f32,
    viewport: &ViewportRect,
    view_proj: [[f32; 4]; 4],
) -> [f32; 3] {
    let ndc_x = ((screen_x - viewport.x as f32) / viewport.width as f32) * 2.0 - 1.0;
    let ndc_y = ((screen_y - viewport.y as f32) / viewport.height as f32) * 2.0 - 1.0;
    let inverse = glam::Mat4::from_cols_array_2d(&view_proj).inverse();
    let world = inverse * glam::Vec4::new(ndc_x, ndc_y, depth, 1.0);
    let world = world / world.w;
    [world.x, world.y, world.z]
}

/// The depths the pick pass drew in a small window around the cursor, and
/// the camera it drew them with: enough to ask whether something near the
/// cursor, an edge a few pixels off, is in front or hidden.
#[derive(Debug, Clone, Default)]
pub struct DepthWindow {
    /// Top-left texel, in window pixels.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Row-major depth per texel; 1.0 where nothing was drawn.
    pub depths: Vec<f32>,
    /// The camera the pick pass drew with.
    pub view_proj: [[f32; 4]; 4],
    pub viewport: ViewportRect,
}

impl DepthWindow {
    /// The world point drawn at window pixel `(x, y)`, at the pixel's
    /// centre; `None` outside the window or where nothing was drawn.
    pub fn world_at(&self, x: i64, y: i64) -> Option<[f32; 3]> {
        let (col, row) = (x - i64::from(self.x), y - i64::from(self.y));
        if col < 0 || row < 0 || col >= i64::from(self.width) || row >= i64::from(self.height) {
            return None;
        }
        let depth = *self
            .depths
            .get(row as usize * self.width as usize + col as usize)?;
        if depth >= 1.0 {
            return None;
        }
        Some(unproject(
            x as f32 + 0.5,
            y as f32 + 0.5,
            depth,
            &self.viewport,
            self.view_proj,
        ))
    }

    /// Whether window pixel `(x, y)` lies in the window at all.
    pub fn covers(&self, x: i64, y: i64) -> bool {
        let (col, row) = (x - i64::from(self.x), y - i64::from(self.y));
        col >= 0 && row >= 0 && col < i64::from(self.width) && row < i64::from(self.height)
    }
}

/// Result of a picking query at a screen position
#[derive(Debug, Clone, Default)]
pub struct PickResult {
    /// The UUID of the picked body, if any
    pub body_id: Option<Uuid>,
    /// The 3D world position under the cursor (if geometry was hit)
    pub world_position: Option<[f32; 3]>,
    /// Depth value (0.0 = near, 1.0 = far)
    pub depth: f32,
    /// The depths around the cursor, whether or not anything is under it.
    pub depth_window: Option<DepthWindow>,
    /// The triangle under the cursor, where the device numbers primitives;
    /// `None` on one that does not, and where nothing was hit.
    pub triangle: Option<PickedTriangle>,
}

/// The triangle the pick pass drew under the cursor, in the picked body's
/// mesh as it was submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickedTriangle {
    /// Its place in the mesh: indices `3 * index .. 3 * index + 3`.
    pub index: u32,
    /// The [`BodySubmission::revision`] the body was drawn with: the index
    /// counts that mesh's triangles, which a later revision may not share.
    pub revision: u64,
}

/// Trait used by the app shell to talk to any renderer implementation.
pub trait RenderBackend {
    /// `display` is the connection to the window system the event loop
    /// holds; the GL backend draws through it and refuses a window's
    /// surface without it (on Wayland especially). A browser page, which
    /// may not block, awaits [`Renderer::initialize_async`] instead.
    #[cfg(not(target_arch = "wasm32"))]
    fn initialize(
        &mut self,
        window: &Window,
        display: OwnedDisplayHandle,
    ) -> Result<(), RenderError>;
    /// Draws the frame. The egui texture deltas are taken out of the
    /// submission here and held until a frame actually applies them, so a
    /// frame skipped for an out-of-date surface loses no upload.
    fn render(&mut self, frame: &mut FrameSubmission) -> Result<(), RenderError>;
    fn resize(&mut self, new_size: PhysicalSize<u32>);
    /// Most recent GPU pick readback. Picks are requested via
    /// [`Renderer::request_pick`] and resolved during `render` once the
    /// frame that recorded them is done, so the result trails the request
    /// by a frame or two.
    fn latest_pick_result(&self) -> PickResult;
}

/// Basic configuration knobs for the renderer.
#[derive(Debug, Clone)]
pub struct RenderSettings {
    /// The graphics API's validation, where it has one.
    pub prefer_validation_layers: bool,
    /// Preferred GPU name substring; None = automatic choice
    pub preferred_gpu: Option<String>,
    /// MSAA sample count (1, 2, 4, or 8)
    pub msaa_samples: u8,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            // Validation is a development tool: on in debug builds, off in
            // release, where it taxes every frame. PRINTCAD_GPU_VALIDATION=1
            // (PRINTCAD_VULKAN_VALIDATION=1 is read too) turns it on.
            prefer_validation_layers: cfg!(debug_assertions)
                || std::env::var_os("PRINTCAD_GPU_VALIDATION").is_some()
                || std::env::var_os("PRINTCAD_VULKAN_VALIDATION").is_some(),
            preferred_gpu: None,
            msaa_samples: 4,
        }
    }
}

/// Highlight state for a body
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HighlightState {
    #[default]
    None,
    Hovered,
    Selected,
    HoveredAndSelected,
    /// Another editor has this body selected: a cool tint, visually
    /// subordinate to the local selection.
    PeerSelected,
}

/// Render-ready body. The mesh is shared via `Arc<TriMesh>` so the renderer
/// can keep a per-body GPU buffer alive across frames keyed by `id` and
/// invalidated when `revision` advances; cloning a `BodySubmission` is
/// effectively a refcount bump regardless of the underlying triangle count.
#[derive(Clone)]
pub struct BodySubmission {
    pub id: Uuid,
    /// Monotonic counter that lets the renderer detect mesh changes without
    /// inspecting triangle data. Bump it whenever `mesh` is reassigned to
    /// different geometry; leave it untouched on hover/select transitions.
    pub revision: u64,
    pub mesh: Arc<TriMesh>,
    pub color: [f32; 3],
    /// 1.0 draws opaque in the solid pass; anything under it draws after
    /// everything opaque, blended over it, without writing depth: a
    /// selection highlight that lets the face show through.
    pub opacity: f32,
    pub highlight: HighlightState,
    /// If true, render as wireframe/line with depth bias to appear on top of solid geometry
    pub is_wireframe: bool,
    /// Whether the pick pass sees it. Paint over a surface (a selection
    /// overlay) and guides are not; a body is, however see-through.
    pub pickable: bool,
    /// Drawn last, blended at `opacity`, over everything whatever its
    /// depth: what the scene would hide and must show.
    pub on_top: bool,
    /// Its face-boundary edges in this colour rather than the scene's.
    pub edge_color: Option<[f32; 3]>,
    /// See-through and one layer deep: its back faces are culled, which
    /// takes winding it made itself, and its edges draw over its faces.
    pub front_only: bool,
}

impl fmt::Debug for BodySubmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BodySubmission")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .field("vertex_count", &self.mesh.positions.len())
            .field("color", &self.color)
            .finish()
    }
}

/// Rectangle defining the 3D viewport area (in physical pixels)
#[derive(Debug, Clone, Copy, Default)]
pub struct ViewportRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A picture of the scene, row by row from the top, RGBA.
#[derive(Debug, Clone)]
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Minimal scene data required to emit a frame.
pub struct FrameSubmission {
    pub bodies: Vec<BodySubmission>,
    pub view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 3],
    pub lighting: LightingData,
    pub egui: Option<EguiSubmission>,
    /// The 3D viewport rect (area where mesh should be rendered)
    pub viewport_rect: Option<ViewportRect>,
    /// Whether face-boundary edges draw over the solids.
    pub draw_edges: bool,
    /// A plane cutting the scene: `[a, b, c, d]` keeps the points where
    /// `a·x + b·y + c·z + d >= 0`, in every pass including picking.
    pub clip_plane: Option<[f32; 4]>,
    /// Grids drawn into the scene, each on its own plane.
    pub grids: Vec<GridSubmission>,
}

/// Lines on a plane, drawn into the scene after the opaque bodies and
/// their edges: depth-tested against them, never writing depth, blended,
/// unseen by picking. Three decades of lines draw at once (`step`, ten and
/// a hundred times it), each as strong as its spacing on screen allows, so
/// the grid thins out with distance and zoom on its own; the host only
/// moves `step` by a decade as the view's scale passes one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSubmission {
    /// Where the plane's axes cross: the lines numbered zero.
    pub origin: [f32; 3],
    /// Unit directions in the plane, square to each other; lines run
    /// along both.
    pub u: [f32; 3],
    pub v: [f32; 3],
    /// The spacing of the finest lines, in model units.
    pub step: f32,
    /// The point of the plane the drawn patch is centred on, and how far
    /// it reaches; it fades out over its outer half.
    pub center: [f32; 3],
    pub radius: f32,
    /// The view's depth range, along `forward` from the frame's camera
    /// position: the grid fades out before the far end would cut it off.
    pub forward: [f32; 3],
    pub depth_range: [f32; 2],
    pub color: [f32; 3],
    /// How opaque the finest lines are once they stand apart on screen,
    /// and lines far apart.
    pub minor_alpha: f32,
    pub major_alpha: f32,
    /// The line along `u` through `origin`, and the one along `v`, drawn
    /// in their own colours over the rest at `axis_alpha` (0 draws none).
    pub u_axis_color: [f32; 3],
    pub v_axis_color: [f32; 3],
    pub axis_alpha: f32,
}

impl Default for FrameSubmission {
    fn default() -> Self {
        Self {
            bodies: Vec::new(),
            view_proj: identity_matrix(),
            camera_pos: [0.0, 0.0, 5.0],
            lighting: LightingData::default(),
            egui: None,
            viewport_rect: None,
            draw_edges: true,
            clip_plane: None,
            grids: Vec::new(),
        }
    }
}

impl fmt::Debug for FrameSubmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameSubmission")
            .field("body_count", &self.bodies.len())
            .field("view_proj", &self.view_proj)
            .field("camera_pos", &self.camera_pos)
            .field(
                "egui",
                if self.egui.is_some() {
                    &"Some"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct EguiSubmission {
    pub pixels_per_point: f32,
    pub textures_delta: TexturesDelta,
    pub primitives: Vec<ClippedPrimitive>,
}

impl fmt::Debug for EguiSubmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EguiSubmission")
            .field("pixels_per_point", &self.pixels_per_point)
            .field("primitive_count", &self.primitives.len())
            .field("textures_set", &self.textures_delta.set.len())
            .field("textures_free", &self.textures_delta.free.len())
            .finish()
    }
}

/// The renderer: owns the GPU, the window's surface and everything drawn.
pub struct Renderer {
    settings: RenderSettings,
    core: Option<RendererCore>,
    /// A size the surface must take before the next frame.
    pending_size: Option<(u32, u32)>,
}

impl Renderer {
    /// Creates the device and the window's surface: what
    /// [`RenderBackend::initialize`] waits for on a desktop, and what a
    /// browser page awaits, since it may not block.
    pub async fn initialize_async(
        &mut self,
        window: &Window,
        display: OwnedDisplayHandle,
    ) -> Result<(), RenderError> {
        if self.core.is_some() {
            return Ok(());
        }
        let size = window.inner_size();
        // A page's canvas may not be laid out yet: it starts at a texel and
        // takes its size from the resize that follows.
        #[cfg(target_arch = "wasm32")]
        let size = PhysicalSize::new(size.width.max(1), size.height.max(1));
        if size.width == 0 || size.height == 0 {
            return Err(RenderError::SurfaceTooSmall);
        }
        info!(
            "Initializing the wgpu renderer (validation={})",
            self.settings.prefer_validation_layers
        );
        let core = RendererCore::new(
            window,
            display,
            (size.width, size.height),
            self.settings.clone(),
        )
        .await?;
        self.core = Some(core);
        Ok(())
    }

    pub fn new(settings: RenderSettings) -> Self {
        Self {
            settings,
            core: None,
            pending_size: None,
        }
    }

    pub fn gpu_name(&self) -> Option<&str> {
        self.core.as_ref().map(|c| c.gpu_name())
    }

    /// The graphics API in use: Vulkan, Metal, DirectX 12 or OpenGL.
    pub fn graphics_api(&self) -> Option<&'static str> {
        self.core.as_ref().map(|c| c.graphics_api())
    }

    pub fn available_gpus(&self) -> Option<&[String]> {
        self.core.as_ref().map(|c| c.available_gpus())
    }

    fn apply_pending_size(&mut self) -> Result<(), RenderError> {
        let core = self.core.as_mut().ok_or(RenderError::NotReady)?;
        if let Some((width, height)) = self.pending_size {
            if width == 0 || height == 0 {
                // Wayland can report zero-sized surfaces when minimized.
                return Ok(());
            }
            core.resize((width, height));
            self.pending_size = None;
        }
        Ok(())
    }
}

impl RenderBackend for Renderer {
    #[cfg(not(target_arch = "wasm32"))]
    fn initialize(
        &mut self,
        window: &Window,
        display: OwnedDisplayHandle,
    ) -> Result<(), RenderError> {
        pollster::block_on(self.initialize_async(window, display))
    }

    fn render(&mut self, frame: &mut FrameSubmission) -> Result<(), RenderError> {
        if let Some(ui) = &frame.egui {
            debug!(
                "egui output: {} primitives, {} texture ops",
                ui.primitives.len(),
                ui.textures_delta.set.len() + ui.textures_delta.free.len()
            );
        }
        self.apply_pending_size()?;
        let core = self.core.as_mut().ok_or(RenderError::NotReady)?;
        if let Some(ui) = frame.egui.as_mut() {
            core.take_textures(std::mem::take(&mut ui.textures_delta));
        }
        match core.draw_frame(frame) {
            Err(RenderError::SwapchainOutOfDate) => {
                self.pending_size = Some(core.surface_size());
                Ok(())
            }
            Err(RenderError::SurfaceTooSmall) => Ok(()),
            other => other,
        }
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        self.pending_size = Some((new_size.width, new_size.height));
    }

    fn latest_pick_result(&self) -> PickResult {
        self.core
            .as_ref()
            .map(|c| c.last_pick_result())
            .unwrap_or_default()
    }
}

impl Renderer {
    /// Whether the last frame re-rendered the 3D scene or reused the cached
    /// scene image under fresh UI.
    pub fn scene_redrawn_last_frame(&self) -> bool {
        self.core
            .as_ref()
            .is_some_and(|c| c.scene_redrawn_last_frame())
    }

    /// What the mesh renderer actually submitted last frame.
    pub fn last_draw_stats(&self) -> DrawStats {
        self.core
            .as_ref()
            .map(|c| c.last_draw_stats())
            .unwrap_or_default()
    }

    /// Request a pick at the given window coordinates, recorded into the
    /// next frame; the answer arrives through `latest_pick_result`.
    pub fn request_pick(&mut self, x: u32, y: u32) {
        if let Some(core) = self.core.as_mut() {
            core.request_pick(x, y);
        }
    }

    /// Ask for a picture of the scene as drawn, the viewport alone: it
    /// arrives through [`Self::take_capture`] a few frames on.
    pub fn request_capture(&mut self) {
        if let Some(core) = self.core.as_mut() {
            core.request_capture();
        }
    }

    /// The picture asked for, once it has arrived.
    pub fn take_capture(&mut self) -> Option<CapturedImage> {
        self.core.as_mut().and_then(|c| c.take_capture())
    }

    /// Whether a picture is asked for or on its way: frames must keep
    /// coming until it arrives.
    pub fn capture_pending(&self) -> bool {
        self.core.as_ref().is_some_and(|c| c.capture_pending())
    }
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("renderer has not been initialized")]
    NotReady,
    #[error("surface is too small to draw on")]
    SurfaceTooSmall,
    #[error("surface out of date")]
    SwapchainOutOfDate,
    #[error("surface creation not supported: {0}")]
    UnsupportedPlatform(String),
    #[error("initialization failed: {0}")]
    Initialization(String),
    #[error("surface error: {0}")]
    Surface(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(name: &str, source: &str, capabilities: naga::valid::Capabilities) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities)
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
    }

    #[test]
    fn the_shaders_parse_and_validate_for_every_backend() {
        let none = naga::valid::Capabilities::empty();
        validate("scene.wgsl", include_str!("../shaders/scene.wgsl"), none);
        validate("blit.wgsl", include_str!("../shaders/blit.wgsl"), none);
    }

    #[test]
    fn the_primitive_pick_validates_where_the_device_numbers_primitives() {
        validate(
            "scene.wgsl + pick_primitive.wgsl",
            mesh::PRIMITIVE_PICK_SOURCE,
            naga::valid::Capabilities::PRIMITIVE_INDEX,
        );
        let module = naga::front::wgsl::parse_str(mesh::PRIMITIVE_PICK_SOURCE).unwrap();
        assert!(
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .is_err(),
            "a device without the primitive index must not be handed it"
        );
    }

    #[test]
    fn a_point_unprojects_where_the_camera_put_it() {
        let viewport = ViewportRect {
            x: 100,
            y: 50,
            width: 200,
            height: 100,
        };
        // A camera that scales x by 2, so the viewport spans x in -0.5..0.5.
        let m = [
            [2.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let p = unproject(300.0, 50.0, 0.5, &viewport, m);
        assert!(
            (p[0] - 0.5).abs() < 1e-6 && (p[1] + 1.0).abs() < 1e-6,
            "{p:?}"
        );
        assert!((p[2] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_depth_window_answers_only_inside_itself() {
        let window = DepthWindow {
            x: 10,
            y: 10,
            width: 2,
            height: 1,
            depths: vec![0.5, 1.0],
            view_proj: identity_matrix(),
            viewport: ViewportRect {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
            },
        };
        assert!(window.covers(11, 10) && !window.covers(12, 10));
        assert!(window.world_at(10, 10).is_some());
        assert!(window.world_at(11, 10).is_none(), "nothing drawn there");
        assert!(window.world_at(9, 10).is_none());
    }
}
