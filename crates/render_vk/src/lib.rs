mod core;
mod mesh;
mod picking;
mod surface;
mod util;

pub use mesh::{GpuLight, LightingData};

use ash::vk;
use core_document::ScreenSpaceOverlay;
use egui::{ClippedPrimitive, TexturesDelta};
use kernel_api::TriMesh;
use std::fmt;
use thiserror::Error;
use tracing::{debug, info};
use uuid::Uuid;
use winit::{dpi::PhysicalSize, window::Window};

use core::RendererCore;

const MAX_FRAMES_IN_FLIGHT: usize = 2;
const VALIDATION_LAYER: &str = "VK_LAYER_KHRONOS_validation";
const MESH_VERT_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mesh.vert.spv"));
const MESH_FRAG_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mesh.frag.spv"));
const PICK_VERT_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pick.vert.spv"));
const PICK_FRAG_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pick.frag.spv"));

fn map_egui_err(err: egui_ash_renderer::RendererError) -> RenderError {
    RenderError::Initialization(format!("egui renderer error: {err}"))
}

fn is_srgb_format(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::B8G8R8A8_SRGB
            | vk::Format::R8G8B8A8_SRGB
            | vk::Format::A8B8G8R8_SRGB_PACK32
            | vk::Format::BC1_RGBA_SRGB_BLOCK
            | vk::Format::BC2_SRGB_BLOCK
            | vk::Format::BC3_SRGB_BLOCK
    )
}

fn identity_matrix() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn msaa_samples_to_vk(samples: u8) -> vk::SampleCountFlags {
    match samples {
        1 => vk::SampleCountFlags::TYPE_1,
        2 => vk::SampleCountFlags::TYPE_2,
        4 => vk::SampleCountFlags::TYPE_4,
        8 => vk::SampleCountFlags::TYPE_8,
        _ => vk::SampleCountFlags::TYPE_4, // Default to 4x
    }
}

fn find_supported_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    candidates: &[vk::Format],
    tiling: vk::ImageTiling,
    features: vk::FormatFeatureFlags,
) -> Option<vk::Format> {
    for &format in candidates {
        let props =
            unsafe { instance.get_physical_device_format_properties(physical_device, format) };
        let supported = match tiling {
            vk::ImageTiling::LINEAR => props.linear_tiling_features.contains(features),
            vk::ImageTiling::OPTIMAL => props.optimal_tiling_features.contains(features),
            _ => false,
        };
        if supported {
            return Some(format);
        }
    }
    None
}

fn find_depth_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Option<vk::Format> {
    find_supported_format(
        instance,
        physical_device,
        &[
            vk::Format::D32_SFLOAT,
            vk::Format::D32_SFLOAT_S8_UINT,
            vk::Format::D24_UNORM_S8_UINT,
        ],
        vk::ImageTiling::OPTIMAL,
        vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT,
    )
}

fn get_max_usable_sample_count(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> vk::SampleCountFlags {
    let props = unsafe { instance.get_physical_device_properties(physical_device) };
    let counts =
        props.limits.framebuffer_color_sample_counts & props.limits.framebuffer_depth_sample_counts;

    if counts.contains(vk::SampleCountFlags::TYPE_8) {
        vk::SampleCountFlags::TYPE_8
    } else if counts.contains(vk::SampleCountFlags::TYPE_4) {
        vk::SampleCountFlags::TYPE_4
    } else if counts.contains(vk::SampleCountFlags::TYPE_2) {
        vk::SampleCountFlags::TYPE_2
    } else {
        vk::SampleCountFlags::TYPE_1
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
}

/// Trait used by the app shell to talk to any renderer implementation.
pub trait RenderBackend {
    fn initialize(&mut self, window: &Window) -> Result<(), RenderError>;
    fn render(&mut self, frame: &FrameSubmission) -> Result<(), RenderError>;
    fn resize(&mut self, new_size: PhysicalSize<u32>);
    /// Query what object is at the given screen position (in physical pixels)
    fn pick_at(&self, x: u32, y: u32) -> PickResult;
}

/// Basic configuration knobs for the renderer.
#[derive(Debug, Clone)]
pub struct RenderSettings {
    pub prefer_validation_layers: bool,
    /// Preferred GPU name substring; None = automatic first suitable device
    pub preferred_gpu: Option<String>,
    /// MSAA sample count (1, 2, 4, or 8)
    pub msaa_samples: u8,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            prefer_validation_layers: true,
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
}

/// Render-ready body (mesh + unique identifier for future picking).
#[derive(Clone)]
pub struct BodySubmission {
    pub id: Uuid,
    pub mesh: TriMesh,
    pub color: [f32; 3],
    pub highlight: HighlightState,
}

impl fmt::Debug for BodySubmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BodySubmission")
            .field("id", &self.id)
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

/// Minimal scene data required to emit a frame.
pub struct FrameSubmission {
    pub bodies: Vec<BodySubmission>,
    pub view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 3],
    pub lighting: LightingData,
    pub egui: Option<EguiSubmission>,
    /// The 3D viewport rect (area where mesh should be rendered)
    pub viewport_rect: Option<ViewportRect>,
    /// Screen-space overlays (constant-thickness lines rendered in 2D screen coordinates)
    pub screen_space_overlays: Vec<ScreenSpaceOverlay>,
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
            screen_space_overlays: Vec::new(),
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

/// Vulkan-backed renderer that owns the full GPU stack.
pub struct VulkanRenderer {
    settings: RenderSettings,
    core: Option<RendererCore>,
    pending_extent: Option<vk::Extent2D>,
}

impl VulkanRenderer {
    pub fn new(settings: RenderSettings) -> Self {
        Self {
            settings,
            core: None,
            pending_extent: None,
        }
    }

    pub fn gpu_name(&self) -> Option<&str> {
        self.core.as_ref().map(|c| c.gpu_name())
    }

    pub fn available_gpus(&self) -> Option<&[String]> {
        self.core.as_ref().map(|c| c.available_gpus())
    }

    fn ensure_swapchain(&mut self) -> Result<(), RenderError> {
        let core = self.core.as_mut().ok_or(RenderError::NotReady)?;
        if let Some(extent) = self.pending_extent {
            if extent.width == 0 || extent.height == 0 {
                // Wayland can report zero-sized surfaces when minimized.
                return Ok(());
            }
            core.recreate_swapchain(extent)?;
            self.pending_extent = None;
        }
        Ok(())
    }
}

impl RenderBackend for VulkanRenderer {
    fn initialize(&mut self, window: &Window) -> Result<(), RenderError> {
        if self.core.is_some() {
            return Ok(());
        }

        let extent = to_extent(window.inner_size()).ok_or(RenderError::SurfaceTooSmall)?;
        info!(
            "Initializing Vulkan renderer (validation_layers={})",
            self.settings.prefer_validation_layers
        );
        let core = RendererCore::new(window, extent, self.settings.clone())?;
        self.core = Some(core);
        Ok(())
    }

    fn render(&mut self, frame: &FrameSubmission) -> Result<(), RenderError> {
        if let Some(ui) = &frame.egui {
            let texture_ops = ui.textures_delta.set.len() + ui.textures_delta.free.len();
            debug!(
                "egui output: {} primitives, {} texture ops",
                ui.primitives.len(),
                texture_ops
            );
        }
        self.ensure_swapchain()?;
        let core = self.core.as_mut().ok_or(RenderError::NotReady)?;
        match core.draw_frame(frame) {
            Err(RenderError::SwapchainOutOfDate) => {
                self.pending_extent = Some(core.swapchain_extent());
                Ok(())
            }
            Err(RenderError::SurfaceTooSmall) => Ok(()),
            other => other,
        }
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        self.pending_extent = to_extent(new_size);
    }

    fn pick_at(&self, _x: u32, _y: u32) -> PickResult {
        // Return the last pick result - the actual picking happens in draw_frame
        // after the pending_pick is set
        self.core
            .as_ref()
            .map(|c| c.last_pick_result())
            .unwrap_or_default()
    }
}

impl VulkanRenderer {
    /// Request a pick at the given screen coordinates (will be processed next frame)
    pub fn request_pick(&mut self, x: u32, y: u32) {
        if let Some(core) = self.core.as_mut() {
            core.request_pick(x, y);
        }
    }
}

fn to_extent(size: PhysicalSize<u32>) -> Option<vk::Extent2D> {
    if size.width == 0 || size.height == 0 {
        None
    } else {
        Some(vk::Extent2D {
            width: size.width,
            height: size.height,
        })
    }
}

fn create_shader_module(
    device: &ash::Device,
    bytes: &[u8],
) -> Result<vk::ShaderModule, RenderError> {
    // SPIR-V is a stream of 32-bit words, but our `bytes` may not be
    // properly aligned for a direct bytemuck cast on all platforms.
    // To avoid `cast_slice` panics, manually assemble a Vec<u32>.
    if bytes.len() % 4 != 0 {
        return Err(RenderError::Initialization(
            "SPIR-V bytecode length is not a multiple of 4".into(),
        ));
    }

    let mut words = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        words.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }

    let info = vk::ShaderModuleCreateInfo::default().code(&words);
    let module = unsafe { device.create_shader_module(&info, None) }.map_err(RenderError::from)?;
    Ok(module)
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("renderer has not been initialized")]
    NotReady,
    #[error("surface is too small to create a swapchain")]
    SurfaceTooSmall,
    #[error("swapchain out of date")]
    SwapchainOutOfDate,
    #[error("surface creation not supported: {0}")]
    UnsupportedPlatform(String),
    #[error("initialization failed: {0}")]
    Initialization(String),
    #[error("vulkan error: {0:?}")]
    Vk(vk::Result),
}

impl From<vk::Result> for RenderError {
    fn from(err: vk::Result) -> Self {
        RenderError::Vk(err)
    }
}

// ============================================================================
