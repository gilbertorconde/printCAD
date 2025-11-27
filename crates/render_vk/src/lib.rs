mod surface;

use ash::{
    khr::{surface::Instance as SurfaceLoader, swapchain::Device as SwapchainLoader},
    vk, Entry,
};
use egui::{ClippedPrimitive, TextureId, TexturesDelta};
use egui_ash_renderer::{Options as EguiRendererOptions, Renderer as EguiRenderer};
use kernel_api::TriMesh;
use std::{
    collections::HashSet,
    ffi::{CStr, CString},
    fmt,
    mem::size_of,
};
use thiserror::Error;
use tracing::{debug, info, warn};
use uuid::Uuid;
use winit::{dpi::PhysicalSize, window::Window};

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
        self.core.as_ref().map(|c| c.gpu_name.as_str())
    }

    pub fn available_gpus(&self) -> Option<&[String]> {
        self.core.as_ref().map(|c| c.available_gpus.as_slice())
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
                self.pending_extent = Some(core.swapchain_extent);
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
            .map(|c| c.last_pick_result.clone())
            .unwrap_or_default()
    }
}

impl VulkanRenderer {
    /// Request a pick at the given screen coordinates (will be processed next frame)
    pub fn request_pick(&mut self, x: u32, y: u32) {
        if let Some(core) = self.core.as_mut() {
            core.pending_pick = Some((x, y));
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

struct RendererCore {
    instance: ash::Instance,
    surface_loader: SurfaceLoader,
    surface: vk::SurfaceKHR,
    device: ash::Device,
    physical_device: vk::PhysicalDevice,
    queue_family_indices: QueueFamilyIndices,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    swapchain_loader: SwapchainLoader,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    swapchain_format: vk::Format,
    swapchain_extent: vk::Extent2D,
    swapchain_image_views: Vec<vk::ImageView>,
    render_pass: vk::RenderPass,
    framebuffers: Vec<vk::Framebuffer>,
    // Separate render pass for UI (no MSAA, renders on top of resolved image)
    ui_render_pass: vk::RenderPass,
    ui_framebuffers: Vec<vk::Framebuffer>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    image_available_semaphores: Vec<vk::Semaphore>,
    render_finished_semaphores: Vec<vk::Semaphore>,
    in_flight_fences: Vec<vk::Fence>,
    images_in_flight: Vec<vk::Fence>,
    current_frame: usize,
    egui_renderer: Option<EguiRenderer>,
    textures_to_free: Vec<Vec<TextureId>>,
    mesh_renderer: Option<MeshRenderer>,
    gpu_name: String,
    available_gpus: Vec<String>,
    // Depth buffer resources
    depth_image: vk::Image,
    depth_image_memory: vk::DeviceMemory,
    depth_image_view: vk::ImageView,
    depth_format: vk::Format,
    // MSAA resources
    msaa_samples: vk::SampleCountFlags,
    color_image: vk::Image,
    color_image_memory: vk::DeviceMemory,
    color_image_view: vk::ImageView,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    // GPU Picking resources
    pick_renderer: Option<PickRenderer>,
    // Last frame's body list for picking (we need UUIDs to decode pick results)
    last_frame_bodies: Vec<Uuid>,
    // Cached pick result (updated after each frame)
    pending_pick: Option<(u32, u32)>,
    last_pick_result: PickResult,
    // View-projection and viewport used for the last picking pass that was submitted
    // (used for unprojection when reading back the pick result)
    pending_pick_view_proj: [[f32; 4]; 4],
    pending_pick_viewport_rect: ViewportRect,
}

impl RendererCore {
    fn new(
        window: &Window,
        extent: vk::Extent2D,
        settings: RenderSettings,
    ) -> Result<Self, RenderError> {
        let entry = Entry::linked();
        let available_layers: HashSet<String> = unsafe {
            entry
                .enumerate_instance_layer_properties()
                .map_err(|e| RenderError::Initialization(e.to_string()))?
                .into_iter()
                .map(|layer| {
                    let cstr = CStr::from_ptr(layer.layer_name.as_ptr());
                    cstr.to_string_lossy().into_owned()
                })
                .collect()
        };

        let validation_enabled =
            settings.prefer_validation_layers && available_layers.contains(VALIDATION_LAYER);
        if validation_enabled {
            info!("Enabling Vulkan validation layers");
        }

        let instance = create_instance(&entry, window, validation_enabled)?;

        let surface = surface::create_surface(&entry, &instance, window)?;

        // Construct extension loaders
        let surface_loader = SurfaceLoader::new(&entry, &instance);

        let candidates = enumerate_suitable_devices(&instance, &surface_loader, surface)?;
        if candidates.is_empty() {
            return Err(RenderError::Initialization(
                "No suitable Vulkan physical device found".into(),
            ));
        }

        let mut chosen_index = 0usize;
        if let Some(pref) = settings.preferred_gpu.as_deref() {
            if let Some(idx) = candidates
                .iter()
                .position(|c| c.name.to_lowercase().contains(&pref.to_lowercase()))
            {
                chosen_index = idx;
            } else {
                warn!(
                    "Preferred GPU '{}' not found, falling back to '{}'",
                    pref, candidates[0].name
                );
            }
        }

        let chosen = &candidates[chosen_index];
        let physical_device = chosen.device;
        let queue_family_indices = chosen.indices;
        let gpu_name = chosen.name.clone();
        let available_gpus: Vec<String> = candidates.iter().map(|c| c.name.clone()).collect();
        let (device, graphics_queue, present_queue) =
            create_logical_device(&instance, physical_device, &queue_family_indices)?;
        let swapchain_loader = SwapchainLoader::new(&instance, &device);

        // Determine MSAA sample count (clamp to device max)
        let requested_samples = msaa_samples_to_vk(settings.msaa_samples);
        let max_samples = get_max_usable_sample_count(&instance, physical_device);
        let msaa_samples = if requested_samples.as_raw() <= max_samples.as_raw() {
            requested_samples
        } else {
            info!(
                "Requested MSAA {}x not supported, falling back to {}x",
                settings.msaa_samples,
                max_samples.as_raw().trailing_zeros() + 1
            );
            max_samples
        };
        info!(
            "Using MSAA: {}x",
            msaa_samples.as_raw().trailing_zeros() + 1
        );

        // Find depth format
        let depth_format = find_depth_format(&instance, physical_device)
            .ok_or_else(|| RenderError::Initialization("No suitable depth format found".into()))?;
        info!("Using depth format: {}", depth_format.as_raw());

        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };

        let mut core = RendererCore {
            instance,
            surface_loader,
            surface,
            device,
            physical_device,
            queue_family_indices,
            graphics_queue,
            present_queue,
            swapchain_loader,
            swapchain: vk::SwapchainKHR::null(),
            swapchain_images: Vec::new(),
            swapchain_format: vk::Format::UNDEFINED,
            swapchain_extent: extent,
            swapchain_image_views: Vec::new(),
            render_pass: vk::RenderPass::null(),
            framebuffers: Vec::new(),
            ui_render_pass: vk::RenderPass::null(),
            ui_framebuffers: Vec::new(),
            command_pool: vk::CommandPool::null(),
            command_buffers: Vec::new(),
            image_available_semaphores: Vec::new(),
            render_finished_semaphores: Vec::new(),
            in_flight_fences: Vec::new(),
            images_in_flight: Vec::new(),
            current_frame: 0,
            egui_renderer: None,
            textures_to_free: vec![Vec::new(); MAX_FRAMES_IN_FLIGHT],
            mesh_renderer: None,
            gpu_name,
            available_gpus,
            depth_image: vk::Image::null(),
            depth_image_memory: vk::DeviceMemory::null(),
            depth_image_view: vk::ImageView::null(),
            depth_format,
            msaa_samples,
            color_image: vk::Image::null(),
            color_image_memory: vk::DeviceMemory::null(),
            color_image_view: vk::ImageView::null(),
            memory_properties,
            pick_renderer: None,
            last_frame_bodies: Vec::new(),
            pending_pick: None,
            last_pick_result: PickResult::default(),
            pending_pick_view_proj: identity_matrix(),
            pending_pick_viewport_rect: ViewportRect::default(),
        };

        core.create_swapchain(extent)?;
        core.create_depth_resources()?;
        core.create_color_resources()?;
        core.create_render_pass()?;
        core.create_ui_render_pass()?;
        core.create_framebuffers()?;
        core.create_ui_framebuffers()?;
        core.create_command_pool()?;
        core.create_command_buffers()?;
        core.create_sync_objects()?;

        // egui uses the UI render pass (no MSAA, loads existing content)
        let egui_options = EguiRendererOptions {
            in_flight_frames: MAX_FRAMES_IN_FLIGHT,
            srgb_framebuffer: is_srgb_format(core.swapchain_format),
            ..Default::default()
        };
        let egui_renderer = EguiRenderer::with_default_allocator(
            &core.instance,
            core.physical_device,
            core.device.clone(),
            core.ui_render_pass,
            egui_options,
        )
        .map_err(map_egui_err)?;
        core.egui_renderer = Some(egui_renderer);

        core.mesh_renderer = Some(MeshRenderer::new(
            &core.instance,
            core.physical_device,
            &core.device,
            core.render_pass,
            core.msaa_samples,
        )?);

        // Initialize picking renderer
        core.pick_renderer = Some(PickRenderer::new(
            &core.device,
            core.swapchain_extent,
            core.depth_format,
            &core.memory_properties,
        )?);

        Ok(core)
    }

    fn recreate_swapchain(&mut self, extent: vk::Extent2D) -> Result<(), RenderError> {
        unsafe {
            self.device.device_wait_idle().map_err(RenderError::from)?;
        }
        self.cleanup_swapchain();
        self.create_swapchain(extent)?;
        self.create_depth_resources()?;
        self.create_color_resources()?;
        self.create_render_pass()?;
        self.create_ui_render_pass()?;
        self.create_framebuffers()?;
        self.create_ui_framebuffers()?;
        self.create_command_buffers()?;
        if let Some(renderer) = self.egui_renderer.as_mut() {
            renderer
                .set_render_pass(self.ui_render_pass)
                .map_err(map_egui_err)?;
        }
        if let Some(renderer) = self.mesh_renderer.as_mut() {
            renderer.set_render_pass(self.render_pass, self.msaa_samples)?;
        }
        // Recreate picking renderer with new extent
        if let Some(pick_renderer) = self.pick_renderer.take() {
            pick_renderer.destroy(&self.device);
        }
        self.pick_renderer = Some(PickRenderer::new(
            &self.device,
            self.swapchain_extent,
            self.depth_format,
            &self.memory_properties,
        )?);
        Ok(())
    }

    fn draw_frame(&mut self, frame: &FrameSubmission) -> Result<(), RenderError> {
        unsafe {
            self.device
                .wait_for_fences(&[self.in_flight_fences[self.current_frame]], true, u64::MAX)
                .map_err(RenderError::from)?;
        }

        if let Some(renderer) = self.egui_renderer.as_mut() {
            let pending = &mut self.textures_to_free[self.current_frame];
            if !pending.is_empty() {
                renderer
                    .free_textures(pending.as_slice())
                    .map_err(map_egui_err)?;
                pending.clear();
            }
        }

        let (image_index, suboptimal) = unsafe {
            match self.swapchain_loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                self.image_available_semaphores[self.current_frame],
                vk::Fence::null(),
            ) {
                Ok(result) => result,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    return Err(RenderError::SwapchainOutOfDate)
                }
                Err(err) => return Err(RenderError::from(err)),
            }
        };

        if suboptimal {
            return Err(RenderError::SwapchainOutOfDate);
        }

        let fence = self.images_in_flight[image_index as usize];
        if fence != vk::Fence::null() {
            unsafe {
                self.device
                    .wait_for_fences(&[fence], true, u64::MAX)
                    .map_err(RenderError::from)?;
            }
        }
        self.images_in_flight[image_index as usize] = self.in_flight_fences[self.current_frame];

        if let (Some(ui), Some(renderer)) = (&frame.egui, self.egui_renderer.as_mut()) {
            renderer
                .set_textures(
                    self.graphics_queue,
                    self.command_pool,
                    ui.textures_delta.set.as_slice(),
                )
                .map_err(map_egui_err)?;
        }

        self.record_command_buffer(self.command_buffers[self.current_frame], image_index, frame)?;

        let signal_semaphores = [self.render_finished_semaphores[self.current_frame]];
        let wait_semaphores = [self.image_available_semaphores[self.current_frame]];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];

        let command_buffers = [self.command_buffers[self.current_frame]];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores);

        unsafe {
            self.device
                .reset_fences(&[self.in_flight_fences[self.current_frame]])
                .map_err(RenderError::from)?;
            self.device
                .queue_submit(
                    self.graphics_queue,
                    &[submit_info],
                    self.in_flight_fences[self.current_frame],
                )
                .map_err(RenderError::from)?;
        }

        let swapchains = [self.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        match unsafe {
            self.swapchain_loader
                .queue_present(self.present_queue, &present_info)
        } {
            Ok(true) => return Err(RenderError::SwapchainOutOfDate),
            Ok(false) => {}
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Err(RenderError::SwapchainOutOfDate),
            Err(vk::Result::SUBOPTIMAL_KHR) => return Err(RenderError::SwapchainOutOfDate),
            Err(err) => return Err(RenderError::from(err)),
        }

        if let Some(ui) = &frame.egui {
            self.textures_to_free[self.current_frame] = ui.textures_delta.free.clone();
        } else {
            self.textures_to_free[self.current_frame].clear();
        }

        // Store body IDs for picking
        self.last_frame_bodies = frame.bodies.iter().map(|b| b.id).collect();

        // Process pending pick request - we need to wait for the previous frame to complete first
        // The pick result we read was rendered with pick_view_proj/pick_viewport_rect
        if let Some((x, y)) = self.pending_pick.take() {
            if let Some(pick_renderer) = &self.pick_renderer {
                // Wait for the current frame's fence to ensure the previous picking pass is complete
                unsafe {
                    let _ = self.device.wait_for_fences(
                        &[self.in_flight_fences[self.current_frame]],
                        true,
                        u64::MAX,
                    );
                }

                // The picking pass we're reading from was rendered with pending_pick_* matrices
                // (set during the frame that just completed)
                // Use those matrices for unprojection
                match pick_renderer.read_pick_result(
                    &self.device,
                    self.command_pool,
                    self.graphics_queue,
                    x,
                    y,
                    self.pending_pick_view_proj,
                    &self.pending_pick_viewport_rect,
                ) {
                    Ok(result) => {
                        if result.body_id.is_some() {
                            debug!("GPU pick hit: {:?} at ({}, {})", result.body_id, x, y);
                        }
                        self.last_pick_result = result;
                    }
                    Err(e) => {
                        warn!("GPU pick failed: {:?}", e);
                    }
                }
            }
        }

        self.current_frame = (self.current_frame + 1) % MAX_FRAMES_IN_FLIGHT;
        Ok(())
    }

    fn create_swapchain(&mut self, target_extent: vk::Extent2D) -> Result<(), RenderError> {
        let support =
            query_swapchain_support(self.physical_device, &self.surface_loader, self.surface)?;
        let surface_format = choose_surface_format(&support.formats);
        let present_mode = choose_present_mode(&support.present_modes);
        let extent = choose_extent(&support.capabilities, target_extent);

        let mut image_count = support.capabilities.min_image_count + 1;
        if support.capabilities.max_image_count > 0
            && image_count > support.capabilities.max_image_count
        {
            image_count = support.capabilities.max_image_count;
        }

        let indices = self.queue_family_indices;
        let queue_family_indices = [indices.graphics_family, indices.present_family];
        let (image_sharing_mode, p_queue_family_indices): (vk::SharingMode, &[u32]) =
            if indices.graphics_family != indices.present_family {
                (vk::SharingMode::CONCURRENT, &queue_family_indices)
            } else {
                (vk::SharingMode::EXCLUSIVE, &queue_family_indices[..1])
            };

        let swapchain_info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(image_sharing_mode)
            .queue_family_indices(p_queue_family_indices)
            .pre_transform(support.capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(self.swapchain);

        unsafe {
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_loader
                    .destroy_swapchain(self.swapchain, None);
            }

            self.swapchain = self
                .swapchain_loader
                .create_swapchain(&swapchain_info, None)
                .map_err(RenderError::from)?;
            self.swapchain_images = self
                .swapchain_loader
                .get_swapchain_images(self.swapchain)
                .map_err(RenderError::from)?;
        }

        self.swapchain_format = surface_format.format;
        self.swapchain_extent = extent;
        self.create_image_views()?;
        self.images_in_flight = vec![vk::Fence::null(); self.swapchain_images.len()];
        Ok(())
    }

    fn create_image_views(&mut self) -> Result<(), RenderError> {
        self.cleanup_image_views();
        let mut views = Vec::with_capacity(self.swapchain_images.len());
        for &image in &self.swapchain_images {
            let subresource_range = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            };

            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(self.swapchain_format)
                .subresource_range(subresource_range);
            let view = unsafe { self.device.create_image_view(&view_info, None) }
                .map_err(RenderError::from)?;
            views.push(view);
        }
        self.swapchain_image_views = views;
        Ok(())
    }

    fn create_depth_resources(&mut self) -> Result<(), RenderError> {
        self.cleanup_depth_resources();

        let (image, memory) = self.create_image(
            self.swapchain_extent.width,
            self.swapchain_extent.height,
            self.depth_format,
            vk::ImageTiling::OPTIMAL,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            self.msaa_samples,
        )?;

        let subresource_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::DEPTH,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(self.depth_format)
            .subresource_range(subresource_range);

        let view = unsafe { self.device.create_image_view(&view_info, None) }
            .map_err(RenderError::from)?;

        self.depth_image = image;
        self.depth_image_memory = memory;
        self.depth_image_view = view;
        Ok(())
    }

    fn create_color_resources(&mut self) -> Result<(), RenderError> {
        self.cleanup_color_resources();

        // Only create MSAA color buffer if samples > 1
        if self.msaa_samples == vk::SampleCountFlags::TYPE_1 {
            return Ok(());
        }

        let (image, memory) = self.create_image(
            self.swapchain_extent.width,
            self.swapchain_extent.height,
            self.swapchain_format,
            vk::ImageTiling::OPTIMAL,
            vk::ImageUsageFlags::TRANSIENT_ATTACHMENT | vk::ImageUsageFlags::COLOR_ATTACHMENT,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            self.msaa_samples,
        )?;

        let subresource_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(self.swapchain_format)
            .subresource_range(subresource_range);

        let view = unsafe { self.device.create_image_view(&view_info, None) }
            .map_err(RenderError::from)?;

        self.color_image = image;
        self.color_image_memory = memory;
        self.color_image_view = view;
        Ok(())
    }

    fn create_image(
        &self,
        width: u32,
        height: u32,
        format: vk::Format,
        tiling: vk::ImageTiling,
        usage: vk::ImageUsageFlags,
        properties: vk::MemoryPropertyFlags,
        samples: vk::SampleCountFlags,
    ) -> Result<(vk::Image, vk::DeviceMemory), RenderError> {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .format(format)
            .tiling(tiling)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .samples(samples);

        let image =
            unsafe { self.device.create_image(&image_info, None) }.map_err(RenderError::from)?;

        let mem_requirements = unsafe { self.device.get_image_memory_requirements(image) };

        let memory_type = find_memory_type(
            mem_requirements.memory_type_bits,
            properties,
            &self.memory_properties,
        )
        .ok_or_else(|| RenderError::Initialization("Failed to find suitable memory type".into()))?;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_requirements.size)
            .memory_type_index(memory_type);

        let memory =
            unsafe { self.device.allocate_memory(&alloc_info, None) }.map_err(RenderError::from)?;

        unsafe {
            self.device
                .bind_image_memory(image, memory, 0)
                .map_err(RenderError::from)?;
        }

        Ok((image, memory))
    }

    fn create_render_pass(&mut self) -> Result<(), RenderError> {
        self.cleanup_render_pass();

        let using_msaa = self.msaa_samples != vk::SampleCountFlags::TYPE_1;

        if using_msaa {
            // MSAA render pass with resolve
            // Attachment 0: MSAA color (multisampled)
            let color_attachment = vk::AttachmentDescription::default()
                .format(self.swapchain_format)
                .samples(self.msaa_samples)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

            // Attachment 1: Depth (multisampled)
            let depth_attachment = vk::AttachmentDescription::default()
                .format(self.depth_format)
                .samples(self.msaa_samples)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::DONT_CARE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

            // Attachment 2: Resolve target (swapchain image)
            // Final layout is COLOR_ATTACHMENT_OPTIMAL so UI pass can render on top
            let color_resolve_attachment = vk::AttachmentDescription::default()
                .format(self.swapchain_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

            let attachments = [color_attachment, depth_attachment, color_resolve_attachment];

            let color_attachment_ref = vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
            let color_attachment_refs = [color_attachment_ref];

            let depth_attachment_ref = vk::AttachmentReference::default()
                .attachment(1)
                .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

            let resolve_attachment_ref = vk::AttachmentReference::default()
                .attachment(2)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
            let resolve_attachment_refs = [resolve_attachment_ref];

            let subpass = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color_attachment_refs)
                .depth_stencil_attachment(&depth_attachment_ref)
                .resolve_attachments(&resolve_attachment_refs);
            let subpasses = [subpass];

            let dependency = vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                )
                .dst_stage_mask(
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                )
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                );
            let dependencies = [dependency];

            let render_pass_info = vk::RenderPassCreateInfo::default()
                .attachments(&attachments)
                .subpasses(&subpasses)
                .dependencies(&dependencies);

            self.render_pass = unsafe { self.device.create_render_pass(&render_pass_info, None) }
                .map_err(RenderError::from)?;
        } else {
            // Non-MSAA render pass with depth
            // Final layout is COLOR_ATTACHMENT_OPTIMAL so UI pass can render on top
            let color_attachment = vk::AttachmentDescription::default()
                .format(self.swapchain_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

            let depth_attachment = vk::AttachmentDescription::default()
                .format(self.depth_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::DONT_CARE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

            let attachments = [color_attachment, depth_attachment];

            let color_attachment_ref = vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
            let color_attachment_refs = [color_attachment_ref];

            let depth_attachment_ref = vk::AttachmentReference::default()
                .attachment(1)
                .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

            let subpass = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color_attachment_refs)
                .depth_stencil_attachment(&depth_attachment_ref);
            let subpasses = [subpass];

            let dependency = vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                )
                .dst_stage_mask(
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                )
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                );
            let dependencies = [dependency];

            let render_pass_info = vk::RenderPassCreateInfo::default()
                .attachments(&attachments)
                .subpasses(&subpasses)
                .dependencies(&dependencies);

            self.render_pass = unsafe { self.device.create_render_pass(&render_pass_info, None) }
                .map_err(RenderError::from)?;
        }

        Ok(())
    }

    /// Create a separate render pass for UI that loads the existing color content
    /// and renders on top without MSAA
    fn create_ui_render_pass(&mut self) -> Result<(), RenderError> {
        self.cleanup_ui_render_pass();

        // Single color attachment that loads existing content
        let color_attachment = vk::AttachmentDescription::default()
            .format(self.swapchain_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::LOAD) // Load existing content from 3D pass
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

        let attachments = [color_attachment];

        let color_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let color_attachment_refs = [color_attachment_ref];

        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment_refs);
        let subpasses = [subpass];

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);
        let dependencies = [dependency];

        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);

        self.ui_render_pass = unsafe { self.device.create_render_pass(&render_pass_info, None) }
            .map_err(RenderError::from)?;

        Ok(())
    }

    fn create_framebuffers(&mut self) -> Result<(), RenderError> {
        self.cleanup_framebuffers();

        let using_msaa = self.msaa_samples != vk::SampleCountFlags::TYPE_1;
        let mut framebuffers = Vec::with_capacity(self.swapchain_image_views.len());

        for &swapchain_view in &self.swapchain_image_views {
            let attachments = if using_msaa {
                // MSAA: [color_msaa, depth, resolve_target]
                vec![self.color_image_view, self.depth_image_view, swapchain_view]
            } else {
                // No MSAA: [color, depth]
                vec![swapchain_view, self.depth_image_view]
            };

            let framebuffer_info = vk::FramebufferCreateInfo::default()
                .render_pass(self.render_pass)
                .attachments(&attachments)
                .width(self.swapchain_extent.width)
                .height(self.swapchain_extent.height)
                .layers(1);
            let framebuffer = unsafe { self.device.create_framebuffer(&framebuffer_info, None) }
                .map_err(RenderError::from)?;
            framebuffers.push(framebuffer);
        }
        self.framebuffers = framebuffers;
        Ok(())
    }

    fn create_ui_framebuffers(&mut self) -> Result<(), RenderError> {
        self.cleanup_ui_framebuffers();

        let mut framebuffers = Vec::with_capacity(self.swapchain_image_views.len());

        for &swapchain_view in &self.swapchain_image_views {
            let attachments = [swapchain_view];

            let framebuffer_info = vk::FramebufferCreateInfo::default()
                .render_pass(self.ui_render_pass)
                .attachments(&attachments)
                .width(self.swapchain_extent.width)
                .height(self.swapchain_extent.height)
                .layers(1);
            let framebuffer = unsafe { self.device.create_framebuffer(&framebuffer_info, None) }
                .map_err(RenderError::from)?;
            framebuffers.push(framebuffer);
        }
        self.ui_framebuffers = framebuffers;
        Ok(())
    }

    fn create_command_pool(&mut self) -> Result<(), RenderError> {
        if self.command_pool != vk::CommandPool::null() {
            unsafe {
                self.device.destroy_command_pool(self.command_pool, None);
            }
        }

        let pool_info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .queue_family_index(self.queue_family_indices.graphics_family);
        let pool = unsafe { self.device.create_command_pool(&pool_info, None) }
            .map_err(RenderError::from)?;
        self.command_pool = pool;
        Ok(())
    }

    fn create_command_buffers(&mut self) -> Result<(), RenderError> {
        if !self.command_buffers.is_empty() {
            unsafe {
                self.device
                    .free_command_buffers(self.command_pool, &self.command_buffers);
            }
            self.command_buffers.clear();
        }

        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(MAX_FRAMES_IN_FLIGHT as u32);
        let buffers = unsafe { self.device.allocate_command_buffers(&alloc_info) }
            .map_err(RenderError::from)?;
        self.command_buffers = buffers;
        Ok(())
    }

    fn create_sync_objects(&mut self) -> Result<(), RenderError> {
        self.cleanup_sync_objects();
        let semaphore_info = vk::SemaphoreCreateInfo::default();
        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            let image_available = unsafe { self.device.create_semaphore(&semaphore_info, None) }
                .map_err(RenderError::from)?;
            let render_finished = unsafe { self.device.create_semaphore(&semaphore_info, None) }
                .map_err(RenderError::from)?;
            let fence = unsafe { self.device.create_fence(&fence_info, None) }
                .map_err(RenderError::from)?;
            self.image_available_semaphores.push(image_available);
            self.render_finished_semaphores.push(render_finished);
            self.in_flight_fences.push(fence);
        }
        self.images_in_flight = vec![vk::Fence::null(); self.swapchain_images.len()];
        self.current_frame = 0;
        Ok(())
    }

    fn record_command_buffer(
        &mut self,
        command_buffer: vk::CommandBuffer,
        image_index: u32,
        frame: &FrameSubmission,
    ) -> Result<(), RenderError> {
        let begin_info = vk::CommandBufferBeginInfo::default();
        unsafe {
            self.device
                .reset_command_buffer(command_buffer, vk::CommandBufferResetFlags::empty())
                .map_err(RenderError::from)?;
            self.device
                .begin_command_buffer(command_buffer, &begin_info)
                .map_err(RenderError::from)?;
        }

        // Record picking pass (renders to offscreen buffer for object ID detection)
        if let Some(pick_renderer) = self.pick_renderer.as_mut() {
            pick_renderer.record_commands(
                &self.device,
                command_buffer,
                &frame.bodies,
                frame.view_proj,
                frame.viewport_rect.as_ref(),
                &self.memory_properties,
            )?;

            // Store the view_proj used for this picking pass
            // When this frame completes, these become the "current" pick matrices
            self.pending_pick_view_proj = frame.view_proj;
            self.pending_pick_viewport_rect = frame.viewport_rect.unwrap_or(ViewportRect {
                x: 0,
                y: 0,
                width: self.swapchain_extent.width,
                height: self.swapchain_extent.height,
            });
        }

        let using_msaa = self.msaa_samples != vk::SampleCountFlags::TYPE_1;
        let clear_values = if using_msaa {
            // MSAA: [color, depth, resolve]
            vec![
                vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [0.05, 0.08, 0.12, 1.0],
                    },
                },
                vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 1.0,
                        stencil: 0,
                    },
                },
                vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [0.05, 0.08, 0.12, 1.0],
                    },
                },
            ]
        } else {
            // No MSAA: [color, depth]
            vec![
                vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [0.05, 0.08, 0.12, 1.0],
                    },
                },
                vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 1.0,
                        stencil: 0,
                    },
                },
            ]
        };

        let render_area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: self.swapchain_extent,
        };
        let render_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.render_pass)
            .framebuffer(self.framebuffers[image_index as usize])
            .render_area(render_area)
            .clear_values(&clear_values);

        unsafe {
            self.device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_info,
                vk::SubpassContents::INLINE,
            );
        }

        if let Some(mesh_renderer) = self.mesh_renderer.as_mut() {
            mesh_renderer.draw(
                command_buffer,
                self.swapchain_extent,
                frame.viewport_rect.as_ref(),
                &frame.bodies,
                frame.view_proj,
                frame.camera_pos,
                &frame.lighting,
            )?;
        }

        unsafe {
            self.device.cmd_end_render_pass(command_buffer);
        }

        // Second render pass for UI (loads existing content, no MSAA)
        if let (Some(ui), Some(renderer)) = (&frame.egui, self.egui_renderer.as_mut()) {
            let ui_render_area = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.swapchain_extent,
            };
            let ui_render_pass_info = vk::RenderPassBeginInfo::default()
                .render_pass(self.ui_render_pass)
                .framebuffer(self.ui_framebuffers[image_index as usize])
                .render_area(ui_render_area);

            unsafe {
                self.device.cmd_begin_render_pass(
                    command_buffer,
                    &ui_render_pass_info,
                    vk::SubpassContents::INLINE,
                );
            }

            renderer
                .cmd_draw(
                    command_buffer,
                    self.swapchain_extent,
                    ui.pixels_per_point,
                    &ui.primitives,
                )
                .map_err(map_egui_err)?;

            unsafe {
                self.device.cmd_end_render_pass(command_buffer);
            }
        }

        unsafe {
            self.device
                .end_command_buffer(command_buffer)
                .map_err(RenderError::from)?;
        }

        Ok(())
    }

    fn cleanup_swapchain(&mut self) {
        self.cleanup_ui_framebuffers();
        self.cleanup_framebuffers();
        self.cleanup_ui_render_pass();
        self.cleanup_render_pass();
        self.cleanup_depth_resources();
        self.cleanup_color_resources();
        self.cleanup_image_views();
        if self.swapchain != vk::SwapchainKHR::null() {
            unsafe {
                self.swapchain_loader
                    .destroy_swapchain(self.swapchain, None);
            }
            self.swapchain = vk::SwapchainKHR::null();
        }
    }

    fn cleanup_framebuffers(&mut self) {
        for framebuffer in self.framebuffers.drain(..) {
            unsafe { self.device.destroy_framebuffer(framebuffer, None) };
        }
    }

    fn cleanup_ui_framebuffers(&mut self) {
        for framebuffer in self.ui_framebuffers.drain(..) {
            unsafe { self.device.destroy_framebuffer(framebuffer, None) };
        }
    }

    fn cleanup_render_pass(&mut self) {
        if self.render_pass != vk::RenderPass::null() {
            unsafe {
                self.device.destroy_render_pass(self.render_pass, None);
            }
            self.render_pass = vk::RenderPass::null();
        }
    }

    fn cleanup_ui_render_pass(&mut self) {
        if self.ui_render_pass != vk::RenderPass::null() {
            unsafe {
                self.device.destroy_render_pass(self.ui_render_pass, None);
            }
            self.ui_render_pass = vk::RenderPass::null();
        }
    }

    fn cleanup_image_views(&mut self) {
        for view in self.swapchain_image_views.drain(..) {
            unsafe {
                self.device.destroy_image_view(view, None);
            }
        }
    }

    fn cleanup_depth_resources(&mut self) {
        if self.depth_image_view != vk::ImageView::null() {
            unsafe {
                self.device.destroy_image_view(self.depth_image_view, None);
            }
            self.depth_image_view = vk::ImageView::null();
        }
        if self.depth_image != vk::Image::null() {
            unsafe {
                self.device.destroy_image(self.depth_image, None);
            }
            self.depth_image = vk::Image::null();
        }
        if self.depth_image_memory != vk::DeviceMemory::null() {
            unsafe {
                self.device.free_memory(self.depth_image_memory, None);
            }
            self.depth_image_memory = vk::DeviceMemory::null();
        }
    }

    fn cleanup_color_resources(&mut self) {
        if self.color_image_view != vk::ImageView::null() {
            unsafe {
                self.device.destroy_image_view(self.color_image_view, None);
            }
            self.color_image_view = vk::ImageView::null();
        }
        if self.color_image != vk::Image::null() {
            unsafe {
                self.device.destroy_image(self.color_image, None);
            }
            self.color_image = vk::Image::null();
        }
        if self.color_image_memory != vk::DeviceMemory::null() {
            unsafe {
                self.device.free_memory(self.color_image_memory, None);
            }
            self.color_image_memory = vk::DeviceMemory::null();
        }
    }

    fn cleanup_sync_objects(&mut self) {
        for semaphore in self.image_available_semaphores.drain(..) {
            unsafe { self.device.destroy_semaphore(semaphore, None) };
        }
        for semaphore in self.render_finished_semaphores.drain(..) {
            unsafe { self.device.destroy_semaphore(semaphore, None) };
        }
        for fence in self.in_flight_fences.drain(..) {
            unsafe { self.device.destroy_fence(fence, None) };
        }
        self.images_in_flight.clear();
    }
}

impl Drop for RendererCore {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();
        }
        self.cleanup_swapchain();
        self.cleanup_sync_objects();
        if self.command_pool != vk::CommandPool::null() {
            unsafe {
                self.device.destroy_command_pool(self.command_pool, None);
            }
        }
        if let Some(renderer) = self.mesh_renderer.take() {
            renderer.destroy();
        }
        unsafe {
            self.device.destroy_device(None);
            self.surface_loader.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}

#[repr(C)]
struct MeshVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
}

impl MeshVertex {
    fn new(position: [f32; 3], normal: [f32; 3], color: [f32; 3]) -> Self {
        Self {
            position,
            normal,
            color,
        }
    }
}

/// Apply highlight tint to a base color
fn apply_highlight_color(base: [f32; 3], highlight: HighlightState) -> [f32; 3] {
    match highlight {
        HighlightState::None => base,
        HighlightState::Hovered => {
            // Brighten and add slight cyan tint for hover
            [
                (base[0] * 1.2 + 0.1).min(1.0),
                (base[1] * 1.2 + 0.15).min(1.0),
                (base[2] * 1.2 + 0.2).min(1.0),
            ]
        }
        HighlightState::Selected => {
            // Add orange/gold tint for selection
            [
                (base[0] * 0.7 + 0.3).min(1.0),
                (base[1] * 0.7 + 0.2).min(1.0),
                (base[2] * 0.5).min(1.0),
            ]
        }
        HighlightState::HoveredAndSelected => {
            // Combine: selected base with brighter hover effect
            [
                (base[0] * 0.6 + 0.4).min(1.0),
                (base[1] * 0.6 + 0.35).min(1.0),
                (base[2] * 0.4 + 0.1).min(1.0),
            ]
        }
    }
}

/// Light data packed for GPU (16 bytes each for alignment)
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GpuLight {
    /// xyz = direction, w = intensity
    pub direction_intensity: [f32; 4],
    /// rgb = color, a = enabled (1.0 or 0.0)
    pub color_enabled: [f32; 4],
}

impl GpuLight {
    pub fn new(direction: [f32; 3], color: [f32; 3], intensity: f32, enabled: bool) -> Self {
        Self {
            direction_intensity: [direction[0], direction[1], direction[2], intensity],
            color_enabled: [
                color[0],
                color[1],
                color[2],
                if enabled { 1.0 } else { 0.0 },
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MeshPushConstants {
    view_proj: [[f32; 4]; 4], // 64 bytes
    camera_pos: [f32; 4],     // 16 bytes - xyz = position, w = unused
    light_main: GpuLight,     // 32 bytes
    light_back: GpuLight,     // 32 bytes
    light_fill: GpuLight,     // 32 bytes
    ambient: [f32; 4],        // 16 bytes - rgb = color * intensity, a = unused
} // Total: 192 bytes

impl MeshPushConstants {
    fn new(view_proj: [[f32; 4]; 4], camera_pos: [f32; 3], lights: &LightingData) -> Self {
        Self {
            view_proj,
            camera_pos: [camera_pos[0], camera_pos[1], camera_pos[2], 1.0],
            light_main: lights.main_light,
            light_back: lights.backlight,
            light_fill: lights.fill_light,
            ambient: [
                lights.ambient_color[0] * lights.ambient_intensity,
                lights.ambient_color[1] * lights.ambient_intensity,
                lights.ambient_color[2] * lights.ambient_intensity,
                1.0,
            ],
        }
    }
}

/// Lighting data to pass to the renderer each frame
#[derive(Clone, Copy, Default)]
pub struct LightingData {
    pub main_light: GpuLight,
    pub backlight: GpuLight,
    pub fill_light: GpuLight,
    pub ambient_color: [f32; 3],
    pub ambient_intensity: f32,
}

struct MeshRenderer {
    device: ash::Device,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    vertex_buffer: vk::Buffer,
    vertex_memory: vk::DeviceMemory,
    vertex_capacity: usize,
    index_buffer: vk::Buffer,
    index_memory: vk::DeviceMemory,
    index_capacity: usize,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    msaa_samples: vk::SampleCountFlags,
}

impl MeshRenderer {
    fn new(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        render_pass: vk::RenderPass,
        msaa_samples: vk::SampleCountFlags,
    ) -> Result<Self, RenderError> {
        let device = device.clone();
        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };

        let pipeline_layout = create_mesh_pipeline_layout(&device)?;
        let pipeline = create_mesh_pipeline(&device, render_pass, pipeline_layout, msaa_samples)?;

        Ok(Self {
            device,
            memory_properties,
            vertex_buffer: vk::Buffer::null(),
            vertex_memory: vk::DeviceMemory::null(),
            vertex_capacity: 0,
            index_buffer: vk::Buffer::null(),
            index_memory: vk::DeviceMemory::null(),
            index_capacity: 0,
            pipeline_layout,
            pipeline,
            msaa_samples,
        })
    }

    fn set_render_pass(
        &mut self,
        render_pass: vk::RenderPass,
        msaa_samples: vk::SampleCountFlags,
    ) -> Result<(), RenderError> {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
        }
        self.msaa_samples = msaa_samples;
        self.pipeline = create_mesh_pipeline(
            &self.device,
            render_pass,
            self.pipeline_layout,
            msaa_samples,
        )?;
        Ok(())
    }

    fn draw(
        &mut self,
        command_buffer: vk::CommandBuffer,
        swapchain_extent: vk::Extent2D,
        viewport_rect: Option<&ViewportRect>,
        bodies: &[BodySubmission],
        view_proj: [[f32; 4]; 4],
        camera_pos: [f32; 3],
        lighting: &LightingData,
    ) -> Result<(), RenderError> {
        let index_count = self.upload_meshes(bodies)?;
        if index_count == 0 {
            return Ok(());
        }

        // Use provided viewport rect or fall back to full swapchain extent
        let (vp_x, vp_y, vp_width, vp_height) = match viewport_rect {
            Some(rect) => (
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            ),
            None => (
                0.0,
                0.0,
                swapchain_extent.width as f32,
                swapchain_extent.height as f32,
            ),
        };

        let viewport = vk::Viewport {
            x: vp_x,
            y: vp_y,
            width: vp_width,
            height: vp_height,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D {
                x: vp_x as i32,
                y: vp_y as i32,
            },
            extent: vk::Extent2D {
                width: vp_width as u32,
                height: vp_height as u32,
            },
        };

        unsafe {
            self.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );
            self.device.cmd_set_viewport(command_buffer, 0, &[viewport]);
            self.device.cmd_set_scissor(command_buffer, 0, &[scissor]);
            self.device
                .cmd_bind_vertex_buffers(command_buffer, 0, &[self.vertex_buffer], &[0]);
            self.device.cmd_bind_index_buffer(
                command_buffer,
                self.index_buffer,
                0,
                vk::IndexType::UINT32,
            );
            let push = MeshPushConstants::new(view_proj, camera_pos, lighting);
            let push_bytes = std::slice::from_raw_parts(
                &push as *const _ as *const u8,
                size_of::<MeshPushConstants>(),
            );
            self.device.cmd_push_constants(
                command_buffer,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                push_bytes,
            );
            self.device
                .cmd_draw_indexed(command_buffer, index_count, 1, 0, 0, 0);
        }

        Ok(())
    }

    fn upload_meshes(&mut self, bodies: &[BodySubmission]) -> Result<u32, RenderError> {
        let vertex_count: usize = bodies.iter().map(|b| b.mesh.positions.len()).sum();
        if vertex_count == 0 {
            return Ok(0);
        }
        let index_count: usize = bodies
            .iter()
            .map(|body| {
                let mesh = &body.mesh;
                if mesh.indices.is_empty() {
                    (mesh.positions.len() / 3) * 3
                } else {
                    mesh.indices.len()
                }
            })
            .sum();

        let vertex_bytes = vertex_count * size_of::<MeshVertex>();
        let index_bytes = index_count * size_of::<u32>();

        self.ensure_vertex_capacity(vertex_bytes)?;
        self.ensure_index_capacity(index_bytes)?;

        unsafe {
            let vertex_ptr = self
                .device
                .map_memory(
                    self.vertex_memory,
                    0,
                    vertex_bytes as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(RenderError::from)? as *mut MeshVertex;
            let vertex_slice = std::slice::from_raw_parts_mut(vertex_ptr, vertex_count);

            let mut v_offset = 0;
            for body in bodies {
                let mesh = &body.mesh;
                // Apply highlight color modification
                let final_color = apply_highlight_color(body.color, body.highlight);
                for (i, position) in mesh.positions.iter().enumerate() {
                    let normal = mesh.normals.get(i).cloned().unwrap_or([0.0, 1.0, 0.0]);
                    vertex_slice[v_offset] = MeshVertex::new(*position, normal, final_color);
                    v_offset += 1;
                }
            }
            self.device.unmap_memory(self.vertex_memory);

            let index_ptr = self
                .device
                .map_memory(
                    self.index_memory,
                    0,
                    index_bytes as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(RenderError::from)? as *mut u32;
            let index_slice = std::slice::from_raw_parts_mut(index_ptr, index_count);

            let mut index_offset = 0;
            let mut vertex_base = 0u32;
            for body in bodies {
                let mesh = &body.mesh;
                if mesh.indices.is_empty() {
                    let tris = mesh.positions.len() / 3;
                    for tri in 0..tris {
                        index_slice[index_offset] = vertex_base + (tri * 3) as u32;
                        index_slice[index_offset + 1] = vertex_base + (tri * 3 + 1) as u32;
                        index_slice[index_offset + 2] = vertex_base + (tri * 3 + 2) as u32;
                        index_offset += 3;
                    }
                } else {
                    for &idx in &mesh.indices {
                        index_slice[index_offset] = vertex_base + idx;
                        index_offset += 1;
                    }
                }
                vertex_base += mesh.positions.len() as u32;
            }
            self.device.unmap_memory(self.index_memory);
        }

        Ok(index_count as u32)
    }

    fn ensure_vertex_capacity(&mut self, required: usize) -> Result<(), RenderError> {
        if required <= self.vertex_capacity {
            return Ok(());
        }
        self.destroy_vertex_buffer();
        let size = required.next_power_of_two() as vk::DeviceSize;
        let (buffer, memory) = self.create_buffer(
            size,
            vk::BufferUsageFlags::VERTEX_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        self.vertex_buffer = buffer;
        self.vertex_memory = memory;
        self.vertex_capacity = size as usize;
        Ok(())
    }

    fn ensure_index_capacity(&mut self, required: usize) -> Result<(), RenderError> {
        if required <= self.index_capacity {
            return Ok(());
        }
        self.destroy_index_buffer();
        let size = required.next_power_of_two() as vk::DeviceSize;
        let (buffer, memory) = self.create_buffer(
            size,
            vk::BufferUsageFlags::INDEX_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        self.index_buffer = buffer;
        self.index_memory = memory;
        self.index_capacity = size as usize;
        Ok(())
    }

    fn create_buffer(
        &self,
        size: vk::DeviceSize,
        usage: vk::BufferUsageFlags,
        properties: vk::MemoryPropertyFlags,
    ) -> Result<(vk::Buffer, vk::DeviceMemory), RenderError> {
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer =
            unsafe { self.device.create_buffer(&buffer_info, None) }.map_err(RenderError::from)?;
        let requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let memory_type = find_memory_type(
            requirements.memory_type_bits,
            properties,
            &self.memory_properties,
        )
        .ok_or_else(|| RenderError::Initialization("Failed to find suitable memory type".into()))?;
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type);
        let memory =
            unsafe { self.device.allocate_memory(&alloc_info, None) }.map_err(RenderError::from)?;
        unsafe {
            self.device.bind_buffer_memory(buffer, memory, 0)?;
        }
        Ok((buffer, memory))
    }

    fn destroy_vertex_buffer(&mut self) {
        if self.vertex_buffer != vk::Buffer::null() {
            unsafe {
                self.device.destroy_buffer(self.vertex_buffer, None);
            }
            self.vertex_buffer = vk::Buffer::null();
        }
        if self.vertex_memory != vk::DeviceMemory::null() {
            unsafe {
                self.device.free_memory(self.vertex_memory, None);
            }
            self.vertex_memory = vk::DeviceMemory::null();
        }
        self.vertex_capacity = 0;
    }

    fn destroy_index_buffer(&mut self) {
        if self.index_buffer != vk::Buffer::null() {
            unsafe {
                self.device.destroy_buffer(self.index_buffer, None);
            }
            self.index_buffer = vk::Buffer::null();
        }
        if self.index_memory != vk::DeviceMemory::null() {
            unsafe {
                self.device.free_memory(self.index_memory, None);
            }
            self.index_memory = vk::DeviceMemory::null();
        }
        self.index_capacity = 0;
    }

    fn destroy(mut self) {
        self.destroy_vertex_buffer();
        self.destroy_index_buffer();
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
        }
    }
}

// Standalone helper functions for GPU resource creation

fn create_image(
    device: &ash::Device,
    width: u32,
    height: u32,
    format: vk::Format,
    tiling: vk::ImageTiling,
    usage: vk::ImageUsageFlags,
    properties: vk::MemoryPropertyFlags,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    samples: vk::SampleCountFlags,
) -> Result<(vk::Image, vk::DeviceMemory), RenderError> {
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .format(format)
        .tiling(tiling)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(samples);

    let image = unsafe { device.create_image(&image_info, None) }.map_err(RenderError::from)?;

    let mem_requirements = unsafe { device.get_image_memory_requirements(image) };

    let memory_type = find_memory_type(
        mem_requirements.memory_type_bits,
        properties,
        memory_properties,
    )
    .ok_or_else(|| RenderError::Initialization("Failed to find suitable memory type".into()))?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_requirements.size)
        .memory_type_index(memory_type);

    let memory = unsafe { device.allocate_memory(&alloc_info, None) }.map_err(RenderError::from)?;

    unsafe {
        device
            .bind_image_memory(image, memory, 0)
            .map_err(RenderError::from)?;
    }

    Ok((image, memory))
}

fn create_image_view(
    device: &ash::Device,
    image: vk::Image,
    format: vk::Format,
    aspect_flags: vk::ImageAspectFlags,
) -> Result<vk::ImageView, RenderError> {
    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: aspect_flags,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    unsafe { device.create_image_view(&view_info, None) }.map_err(RenderError::from)
}

fn create_buffer(
    device: &ash::Device,
    size: vk::DeviceSize,
    usage: vk::BufferUsageFlags,
    properties: vk::MemoryPropertyFlags,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
) -> Result<(vk::Buffer, vk::DeviceMemory), RenderError> {
    let buffer_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = unsafe { device.create_buffer(&buffer_info, None) }.map_err(RenderError::from)?;
    let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
    let memory_type =
        find_memory_type(requirements.memory_type_bits, properties, memory_properties).ok_or_else(
            || RenderError::Initialization("Failed to find suitable memory type".into()),
        )?;
    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type);
    let memory = unsafe { device.allocate_memory(&alloc_info, None) }.map_err(RenderError::from)?;
    unsafe {
        device.bind_buffer_memory(buffer, memory, 0)?;
    }
    Ok((buffer, memory))
}

fn create_mesh_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    msaa_samples: vk::SampleCountFlags,
) -> Result<vk::Pipeline, RenderError> {
    let vert_module = create_shader_module(device, MESH_VERT_SPV)?;
    let frag_module = create_shader_module(device, MESH_FRAG_SPV)?;

    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vert_module)
            .name(CStr::from_bytes_with_nul(b"main\0").unwrap()),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(frag_module)
            .name(CStr::from_bytes_with_nul(b"main\0").unwrap()),
    ];

    let binding = vk::VertexInputBindingDescription::default()
        .binding(0)
        .stride(size_of::<MeshVertex>() as u32)
        .input_rate(vk::VertexInputRate::VERTEX);
    let bindings = [binding];
    let attributes = [
        vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(0)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(0),
        vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(1)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(12),
        vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(2)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(24),
    ];

    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::BACK)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);

    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(msaa_samples)
        .sample_shading_enable(false);

    // Enable depth testing
    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::LESS)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default().color_write_mask(
        vk::ColorComponentFlags::R
            | vk::ColorComponentFlags::G
            | vk::ColorComponentFlags::B
            | vk::ColorComponentFlags::A,
    );
    let color_blend_attachments = [color_blend_attachment];
    let color_blend =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&color_blend_attachments);

    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterization)
        .multisample_state(&multisample)
        .depth_stencil_state(&depth_stencil)
        .color_blend_state(&color_blend)
        .dynamic_state(&dynamic_state)
        .layout(pipeline_layout)
        .render_pass(render_pass);

    let pipeline = unsafe {
        device.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
    }
    .map_err(|(_, err)| RenderError::from(err))?[0];

    unsafe {
        device.destroy_shader_module(vert_module, None);
        device.destroy_shader_module(frag_module, None);
    }

    Ok(pipeline)
}

fn create_mesh_pipeline_layout(device: &ash::Device) -> Result<vk::PipelineLayout, RenderError> {
    let push_constant_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
        .offset(0)
        .size(size_of::<MeshPushConstants>() as u32);
    let push_ranges = [push_constant_range];
    let layout_info = vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&push_ranges);
    let layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(RenderError::from)?;
    Ok(layout)
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

fn find_memory_type(
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
) -> Option<u32> {
    for i in 0..memory_properties.memory_type_count {
        let suitable = (type_filter & (1 << i)) != 0;
        let supported = memory_properties.memory_types[i as usize]
            .property_flags
            .contains(properties);
        if suitable && supported {
            return Some(i);
        }
    }
    None
}

fn create_instance(
    entry: &Entry,
    window: &Window,
    enable_validation: bool,
) -> Result<ash::Instance, RenderError> {
    let app_name = CString::new("printCAD").unwrap();

    let app_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(vk::make_api_version(0, 0, 1, 0))
        .engine_name(&app_name)
        .engine_version(vk::make_api_version(0, 0, 1, 0))
        .api_version(vk::API_VERSION_1_2);

    let extensions_vec = surface::required_extensions(window, enable_validation)?;

    let validation_layers_cstr: Vec<CString> = if enable_validation {
        vec![CString::new(VALIDATION_LAYER).unwrap()]
    } else {
        Vec::new()
    };
    let validation_layers: Vec<*const i8> = validation_layers_cstr
        .iter()
        .map(|layer| layer.as_ptr())
        .collect();

    let create_info = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_extension_names(&extensions_vec)
        .enabled_layer_names(&validation_layers);

    unsafe { entry.create_instance(&create_info, None) }.map_err(RenderError::from)
}

struct GpuCandidate {
    device: vk::PhysicalDevice,
    name: String,
    indices: QueueFamilyIndices,
}

fn enumerate_suitable_devices(
    instance: &ash::Instance,
    surface_loader: &SurfaceLoader,
    surface: vk::SurfaceKHR,
) -> Result<Vec<GpuCandidate>, RenderError> {
    let devices = unsafe { instance.enumerate_physical_devices() }.map_err(RenderError::from)?;
    let mut candidates = Vec::new();
    for device in devices {
        if let Some(indices) = find_queue_families(instance, device, surface_loader, surface)? {
            if check_device_extension_support(instance, device)? {
                let swapchain_support = query_swapchain_support(device, surface_loader, surface)?;
                let format_supported = !swapchain_support.formats.is_empty();
                let present_supported = !swapchain_support.present_modes.is_empty();
                if format_supported && present_supported {
                    let props = unsafe { instance.get_physical_device_properties(device) };
                    let raw_name = &props.device_name;
                    let cstr = unsafe { CStr::from_ptr(raw_name.as_ptr()) };
                    let name = cstr.to_string_lossy().into_owned();
                    candidates.push(GpuCandidate {
                        device,
                        name,
                        indices,
                    });
                }
            }
        }
    }
    Ok(candidates)
}

fn create_logical_device(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    indices: &QueueFamilyIndices,
) -> Result<(ash::Device, vk::Queue, vk::Queue), RenderError> {
    let mut unique_indices = vec![indices.graphics_family];
    if indices.graphics_family != indices.present_family {
        unique_indices.push(indices.present_family);
    }

    unique_indices.sort();
    unique_indices.dedup();

    let queue_priority = [1.0f32];
    let queue_info: Vec<_> = unique_indices
        .iter()
        .map(|&queue_family| {
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(queue_family)
                .queue_priorities(&queue_priority)
        })
        .collect();

    let device_features = vk::PhysicalDeviceFeatures::default();
    let device_extensions = [ash::khr::swapchain::NAME.as_ptr()];

    let create_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_info)
        .enabled_extension_names(&device_extensions)
        .enabled_features(&device_features);

    let device = unsafe { instance.create_device(physical_device, &create_info, None) }
        .map_err(RenderError::from)?;
    let graphics_queue = unsafe { device.get_device_queue(indices.graphics_family, 0) };
    let present_queue = unsafe { device.get_device_queue(indices.present_family, 0) };

    Ok((device, graphics_queue, present_queue))
}

fn find_queue_families(
    instance: &ash::Instance,
    device: vk::PhysicalDevice,
    surface_loader: &SurfaceLoader,
    surface: vk::SurfaceKHR,
) -> Result<Option<QueueFamilyIndices>, RenderError> {
    let queue_families = unsafe { instance.get_physical_device_queue_family_properties(device) };
    let mut indices = QueueFamilyIndices {
        graphics_family: u32::MAX,
        present_family: u32::MAX,
    };

    for (i, queue_family) in queue_families.iter().enumerate() {
        if queue_family.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
            indices.graphics_family = i as u32;
        }

        let present_support = unsafe {
            surface_loader.get_physical_device_surface_support(device, i as u32, surface)
        }
        .map_err(RenderError::from)?;
        if present_support {
            indices.present_family = i as u32;
        }

        if indices.is_complete() {
            break;
        }
    }

    if indices.is_complete() {
        Ok(Some(indices))
    } else {
        Ok(None)
    }
}

fn check_device_extension_support(
    instance: &ash::Instance,
    device: vk::PhysicalDevice,
) -> Result<bool, RenderError> {
    let extensions = unsafe { instance.enumerate_device_extension_properties(device) }
        .map_err(RenderError::from)?;
    let mut required_extensions: HashSet<&'static std::ffi::CStr> = HashSet::new();
    required_extensions.insert(ash::khr::swapchain::NAME);

    for ext in extensions {
        let name = unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) };
        required_extensions.remove(name);
    }

    Ok(required_extensions.is_empty())
}

struct SwapchainSupportDetails {
    capabilities: vk::SurfaceCapabilitiesKHR,
    formats: Vec<vk::SurfaceFormatKHR>,
    present_modes: Vec<vk::PresentModeKHR>,
}

fn query_swapchain_support(
    device: vk::PhysicalDevice,
    surface_loader: &SurfaceLoader,
    surface: vk::SurfaceKHR,
) -> Result<SwapchainSupportDetails, RenderError> {
    let capabilities =
        unsafe { surface_loader.get_physical_device_surface_capabilities(device, surface) }
            .map_err(RenderError::from)?;
    let formats = unsafe { surface_loader.get_physical_device_surface_formats(device, surface) }
        .map_err(RenderError::from)?;
    let present_modes =
        unsafe { surface_loader.get_physical_device_surface_present_modes(device, surface) }
            .map_err(RenderError::from)?;
    Ok(SwapchainSupportDetails {
        capabilities,
        formats,
        present_modes,
    })
}

fn choose_surface_format(available_formats: &[vk::SurfaceFormatKHR]) -> vk::SurfaceFormatKHR {
    *available_formats
        .iter()
        .find(|format| {
            format.format == vk::Format::B8G8R8A8_SRGB
                && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .unwrap_or(&available_formats[0])
}

fn choose_present_mode(available_present_modes: &[vk::PresentModeKHR]) -> vk::PresentModeKHR {
    if available_present_modes
        .iter()
        .any(|&mode| mode == vk::PresentModeKHR::MAILBOX)
    {
        vk::PresentModeKHR::MAILBOX
    } else {
        vk::PresentModeKHR::FIFO
    }
}

fn choose_extent(capabilities: &vk::SurfaceCapabilitiesKHR, target: vk::Extent2D) -> vk::Extent2D {
    if capabilities.current_extent.width != u32::MAX {
        capabilities.current_extent
    } else {
        vk::Extent2D {
            width: target.width.clamp(
                capabilities.min_image_extent.width,
                capabilities.max_image_extent.width,
            ),
            height: target.height.clamp(
                capabilities.min_image_extent.height,
                capabilities.max_image_extent.height,
            ),
        }
    }
}

#[derive(Clone, Copy)]
struct QueueFamilyIndices {
    graphics_family: u32,
    present_family: u32,
}

impl QueueFamilyIndices {
    fn is_complete(&self) -> bool {
        self.graphics_family != u32::MAX && self.present_family != u32::MAX
    }
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
// GPU Picking Renderer
// ============================================================================

/// Push constants for the picking shader
#[repr(C)]
#[derive(Clone, Copy)]
struct PickPushConstants {
    view_proj: [[f32; 4]; 4],
    object_id: [u32; 4], // UUID encoded as 4 u32s
}

/// GPU-based picking renderer that renders object IDs to an offscreen buffer
struct PickRenderer {
    // Offscreen framebuffer resources
    id_image: vk::Image,
    id_image_memory: vk::DeviceMemory,
    id_image_view: vk::ImageView,
    depth_image: vk::Image,
    depth_image_memory: vk::DeviceMemory,
    depth_image_view: vk::ImageView,
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    // Staging buffer for CPU readback
    staging_buffer: vk::Buffer,
    staging_memory: vk::DeviceMemory,
    // Pipeline
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    // Extent
    extent: vk::Extent2D,
    // Vertex/index buffers (shared with mesh renderer, but we need our own for simplicity)
    vertex_buffer: vk::Buffer,
    vertex_memory: vk::DeviceMemory,
    vertex_capacity: usize,
    index_buffer: vk::Buffer,
    index_memory: vk::DeviceMemory,
    index_capacity: usize,
}

impl PickRenderer {
    fn new(
        device: &ash::Device,
        extent: vk::Extent2D,
        depth_format: vk::Format,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
    ) -> Result<Self, RenderError> {
        // Create ID image (R32G32B32A32_UINT for 128-bit UUID)
        let id_format = vk::Format::R32G32B32A32_UINT;
        let (id_image, id_image_memory) = create_image(
            device,
            extent.width,
            extent.height,
            id_format,
            vk::ImageTiling::OPTIMAL,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            memory_properties,
            vk::SampleCountFlags::TYPE_1,
        )?;
        let id_image_view =
            create_image_view(device, id_image, id_format, vk::ImageAspectFlags::COLOR)?;

        // Create depth image for picking
        let (depth_image, depth_image_memory) = create_image(
            device,
            extent.width,
            extent.height,
            depth_format,
            vk::ImageTiling::OPTIMAL,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            memory_properties,
            vk::SampleCountFlags::TYPE_1,
        )?;
        let depth_image_view = create_image_view(
            device,
            depth_image,
            depth_format,
            vk::ImageAspectFlags::DEPTH,
        )?;

        // Create render pass
        let render_pass = Self::create_render_pass(device, id_format, depth_format)?;

        // Create framebuffer
        let attachments = [id_image_view, depth_image_view];
        let framebuffer_info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(&attachments)
            .width(extent.width)
            .height(extent.height)
            .layers(1);
        let framebuffer = unsafe { device.create_framebuffer(&framebuffer_info, None) }
            .map_err(RenderError::from)?;

        // Create staging buffer for readback (16 bytes for ID + padding + 4 bytes for depth)
        let staging_size = 64u64; // 16 bytes for ID + 16 bytes padding + 4 bytes for depth + extra
        let (staging_buffer, staging_memory) = create_buffer(
            device,
            staging_size,
            vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            memory_properties,
        )?;

        // Create pipeline
        let pipeline_layout = Self::create_pipeline_layout(device)?;
        let pipeline = Self::create_pipeline(device, render_pass, pipeline_layout)?;

        Ok(Self {
            id_image,
            id_image_memory,
            id_image_view,
            depth_image,
            depth_image_memory,
            depth_image_view,
            render_pass,
            framebuffer,
            staging_buffer,
            staging_memory,
            pipeline_layout,
            pipeline,
            extent,
            vertex_buffer: vk::Buffer::null(),
            vertex_memory: vk::DeviceMemory::null(),
            vertex_capacity: 0,
            index_buffer: vk::Buffer::null(),
            index_memory: vk::DeviceMemory::null(),
            index_capacity: 0,
        })
    }

    fn create_render_pass(
        device: &ash::Device,
        color_format: vk::Format,
        depth_format: vk::Format,
    ) -> Result<vk::RenderPass, RenderError> {
        let attachments = [
            // ID attachment
            vk::AttachmentDescription::default()
                .format(color_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
            // Depth attachment
            vk::AttachmentDescription::default()
                .format(depth_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
        ];

        let color_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let depth_ref = vk::AttachmentReference::default()
            .attachment(1)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

        let color_refs = [color_ref];
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_refs)
            .depth_stencil_attachment(&depth_ref);

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .src_access_mask(vk::AccessFlags::empty())
            .dst_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            );

        let subpasses = [subpass];
        let dependencies = [dependency];
        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);

        unsafe { device.create_render_pass(&render_pass_info, None) }.map_err(RenderError::from)
    }

    fn create_pipeline_layout(device: &ash::Device) -> Result<vk::PipelineLayout, RenderError> {
        let push_constant_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(size_of::<PickPushConstants>() as u32);

        let push_constant_ranges = [push_constant_range];
        let layout_info =
            vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&push_constant_ranges);

        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(RenderError::from)
    }

    fn create_pipeline(
        device: &ash::Device,
        render_pass: vk::RenderPass,
        layout: vk::PipelineLayout,
    ) -> Result<vk::Pipeline, RenderError> {
        let vert_module = create_shader_module(device, PICK_VERT_SPV)?;
        let frag_module = create_shader_module(device, PICK_FRAG_SPV)?;

        let entry_name = CString::new("main").unwrap();
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert_module)
                .name(&entry_name),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag_module)
                .name(&entry_name),
        ];

        // Same vertex input as mesh shader
        let binding_desc = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(size_of::<MeshVertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);

        let attr_descs = [
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(12),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(24),
        ];

        let binding_descs = [binding_desc];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&binding_descs)
            .vertex_attribute_descriptions(&attr_descs);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS)
            .depth_bounds_test_enable(false)
            .stencil_test_enable(false);

        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false);

        let color_blend_attachments = [color_blend_attachment];
        let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&color_blend_attachments);

        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blending)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .render_pass(render_pass)
            .subpass(0);

        let pipeline = unsafe {
            device.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
        }
        .map_err(|(_, err)| RenderError::from(err))?[0];

        unsafe {
            device.destroy_shader_module(vert_module, None);
            device.destroy_shader_module(frag_module, None);
        }

        Ok(pipeline)
    }

    /// Encode a UUID as 4 u32 values
    fn uuid_to_u32s(uuid: Uuid) -> [u32; 4] {
        let bytes = uuid.as_bytes();
        [
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
        ]
    }

    /// Decode 4 u32 values back to a UUID
    fn u32s_to_uuid(values: [u32; 4]) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&values[0].to_le_bytes());
        bytes[4..8].copy_from_slice(&values[1].to_le_bytes());
        bytes[8..12].copy_from_slice(&values[2].to_le_bytes());
        bytes[12..16].copy_from_slice(&values[3].to_le_bytes());
        Uuid::from_bytes(bytes)
    }

    /// Record commands to render picking pass
    fn record_commands(
        &mut self,
        device: &ash::Device,
        command_buffer: vk::CommandBuffer,
        bodies: &[BodySubmission],
        view_proj: [[f32; 4]; 4],
        viewport_rect: Option<&ViewportRect>,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
    ) -> Result<(), RenderError> {
        // Upload mesh data
        self.upload_meshes(device, bodies, memory_properties)?;

        // Begin render pass
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    uint32: [0, 0, 0, 0], // Zero ID = no object
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        ];

        let render_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.render_pass)
            .framebuffer(self.framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.extent,
            })
            .clear_values(&clear_values);

        unsafe {
            device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_info,
                vk::SubpassContents::INLINE,
            );

            // Set viewport and scissor
            let (vp_x, vp_y, vp_width, vp_height) = match viewport_rect {
                Some(rect) => (
                    rect.x as f32,
                    rect.y as f32,
                    rect.width as f32,
                    rect.height as f32,
                ),
                None => (
                    0.0,
                    0.0,
                    self.extent.width as f32,
                    self.extent.height as f32,
                ),
            };

            let viewport = vk::Viewport {
                x: vp_x,
                y: vp_y,
                width: vp_width,
                height: vp_height,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let scissor = vk::Rect2D {
                offset: vk::Offset2D {
                    x: vp_x as i32,
                    y: vp_y as i32,
                },
                extent: vk::Extent2D {
                    width: vp_width as u32,
                    height: vp_height as u32,
                },
            };

            device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );
            device.cmd_set_viewport(command_buffer, 0, &[viewport]);
            device.cmd_set_scissor(command_buffer, 0, &[scissor]);

            if self.vertex_buffer != vk::Buffer::null() {
                device.cmd_bind_vertex_buffers(command_buffer, 0, &[self.vertex_buffer], &[0]);
                device.cmd_bind_index_buffer(
                    command_buffer,
                    self.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );

                // Draw each body with its unique ID
                let mut index_offset = 0u32;
                for body in bodies {
                    let index_count = if body.mesh.indices.is_empty() {
                        body.mesh.positions.len() as u32
                    } else {
                        body.mesh.indices.len() as u32
                    };

                    let push = PickPushConstants {
                        view_proj,
                        object_id: Self::uuid_to_u32s(body.id),
                    };
                    let push_bytes = std::slice::from_raw_parts(
                        &push as *const _ as *const u8,
                        size_of::<PickPushConstants>(),
                    );
                    device.cmd_push_constants(
                        command_buffer,
                        self.pipeline_layout,
                        vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                        0,
                        push_bytes,
                    );
                    device.cmd_draw_indexed(command_buffer, index_count, 1, index_offset, 0, 0);
                    index_offset += index_count;
                }
            }

            device.cmd_end_render_pass(command_buffer);
        }

        Ok(())
    }

    /// Read back the pick result at a specific pixel
    fn read_pick_result(
        &self,
        device: &ash::Device,
        command_pool: vk::CommandPool,
        queue: vk::Queue,
        x: u32,
        y: u32,
        view_proj: [[f32; 4]; 4],
        viewport: &ViewportRect,
    ) -> Result<PickResult, RenderError> {
        if x >= self.extent.width || y >= self.extent.height {
            return Ok(PickResult::default());
        }

        // Create a one-time command buffer for the copy
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);

        let command_buffer =
            unsafe { device.allocate_command_buffers(&alloc_info) }.map_err(RenderError::from)?[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

        unsafe {
            device
                .begin_command_buffer(command_buffer, &begin_info)
                .map_err(RenderError::from)?;

            // Copy single pixel from ID image to staging buffer (offset 0)
            let id_region = vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_offset(vk::Offset3D {
                    x: x as i32,
                    y: y as i32,
                    z: 0,
                })
                .image_extent(vk::Extent3D {
                    width: 1,
                    height: 1,
                    depth: 1,
                });

            device.cmd_copy_image_to_buffer(
                command_buffer,
                self.id_image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.staging_buffer,
                &[id_region],
            );

            // Copy single pixel from depth image to staging buffer (offset 32 for alignment)
            let depth_region = vk::BufferImageCopy::default()
                .buffer_offset(32)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::DEPTH,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_offset(vk::Offset3D {
                    x: x as i32,
                    y: y as i32,
                    z: 0,
                })
                .image_extent(vk::Extent3D {
                    width: 1,
                    height: 1,
                    depth: 1,
                });

            device.cmd_copy_image_to_buffer(
                command_buffer,
                self.depth_image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.staging_buffer,
                &[depth_region],
            );

            device
                .end_command_buffer(command_buffer)
                .map_err(RenderError::from)?;

            // Submit and wait
            let command_buffers = [command_buffer];
            let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
            device
                .queue_submit(queue, &[submit_info], vk::Fence::null())
                .map_err(RenderError::from)?;
            device.queue_wait_idle(queue).map_err(RenderError::from)?;

            device.free_command_buffers(command_pool, &[command_buffer]);

            // Read back the data (ID at offset 0, depth at offset 32)
            let data_ptr = device
                .map_memory(self.staging_memory, 0, 36, vk::MemoryMapFlags::empty())
                .map_err(RenderError::from)? as *const u32;

            let id_values = [
                *data_ptr,
                *data_ptr.add(1),
                *data_ptr.add(2),
                *data_ptr.add(3),
            ];

            // Read depth at offset 32 (8 u32s from start)
            let depth = *((data_ptr.add(8)) as *const f32);

            device.unmap_memory(self.staging_memory);

            // Check if we hit anything (all zeros = no hit)
            if id_values == [0, 0, 0, 0] {
                return Ok(PickResult::default());
            }

            let uuid = Self::u32s_to_uuid(id_values);

            // Compute world position by unprojecting the screen coordinates with depth
            // The screen coordinates are in window space, we need to convert to viewport-relative
            let world_pos = Self::unproject(x as f32, y as f32, depth, viewport, view_proj);

            Ok(PickResult {
                body_id: Some(uuid),
                world_position: Some(world_pos),
                depth,
            })
        }
    }

    /// Unproject screen coordinates + depth to world position
    ///
    /// screen_x and screen_y are in window coordinates (full window, not viewport-relative).
    /// The viewport defines where the 3D view is rendered within the window.
    fn unproject(
        screen_x: f32,
        screen_y: f32,
        depth: f32,
        viewport: &ViewportRect,
        view_proj: [[f32; 4]; 4],
    ) -> [f32; 3] {
        // Convert window coordinates to viewport-relative coordinates
        let vp_x = screen_x - viewport.x as f32;
        let vp_y = screen_y - viewport.y as f32;
        let vp_width = viewport.width as f32;
        let vp_height = viewport.height as f32;

        // Convert to NDC (-1 to 1) within the viewport
        // Note: Vulkan has Y=0 at top, NDC has Y=+1 at top
        // But glam's perspective_rh already handles this, so we DON'T flip Y here
        let ndc_x = (vp_x / vp_width) * 2.0 - 1.0;
        let ndc_y = (vp_y / vp_height) * 2.0 - 1.0; // No flip - Vulkan convention
        let ndc_z = depth; // Vulkan depth is 0 to 1

        // Build inverse view-projection matrix
        let vp = glam::Mat4::from_cols_array_2d(&view_proj);
        let inv_vp = vp.inverse();

        // Unproject
        let clip = glam::Vec4::new(ndc_x, ndc_y, ndc_z, 1.0);
        let world = inv_vp * clip;
        let world = world / world.w;

        [world.x, world.y, world.z]
    }

    fn upload_meshes(
        &mut self,
        device: &ash::Device,
        bodies: &[BodySubmission],
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
    ) -> Result<(), RenderError> {
        let vertex_count: usize = bodies.iter().map(|b| b.mesh.positions.len()).sum();
        if vertex_count == 0 {
            return Ok(());
        }
        let index_count: usize = bodies
            .iter()
            .map(|body| {
                let mesh = &body.mesh;
                if mesh.indices.is_empty() {
                    mesh.positions.len()
                } else {
                    mesh.indices.len()
                }
            })
            .sum();

        let vertex_bytes = vertex_count * size_of::<MeshVertex>();
        let index_bytes = index_count * size_of::<u32>();

        self.ensure_vertex_capacity(device, vertex_bytes, memory_properties)?;
        self.ensure_index_capacity(device, index_bytes, memory_properties)?;

        unsafe {
            let vertex_ptr = device
                .map_memory(
                    self.vertex_memory,
                    0,
                    vertex_bytes as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(RenderError::from)? as *mut MeshVertex;
            let vertex_slice = std::slice::from_raw_parts_mut(vertex_ptr, vertex_count);

            let mut v_offset = 0;
            for body in bodies {
                let mesh = &body.mesh;
                for (i, position) in mesh.positions.iter().enumerate() {
                    let normal = mesh.normals.get(i).cloned().unwrap_or([0.0, 1.0, 0.0]);
                    vertex_slice[v_offset] = MeshVertex::new(*position, normal, body.color);
                    v_offset += 1;
                }
            }
            device.unmap_memory(self.vertex_memory);

            let index_ptr = device
                .map_memory(
                    self.index_memory,
                    0,
                    index_bytes as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(RenderError::from)? as *mut u32;
            let index_slice = std::slice::from_raw_parts_mut(index_ptr, index_count);

            let mut i_offset = 0usize;
            let mut base_vertex = 0u32;
            for body in bodies {
                let mesh = &body.mesh;
                if mesh.indices.is_empty() {
                    for i in 0..mesh.positions.len() {
                        index_slice[i_offset] = base_vertex + i as u32;
                        i_offset += 1;
                    }
                } else {
                    for idx in &mesh.indices {
                        index_slice[i_offset] = base_vertex + *idx;
                        i_offset += 1;
                    }
                }
                base_vertex += mesh.positions.len() as u32;
            }
            device.unmap_memory(self.index_memory);
        }

        Ok(())
    }

    fn ensure_vertex_capacity(
        &mut self,
        device: &ash::Device,
        required: usize,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
    ) -> Result<(), RenderError> {
        if required <= self.vertex_capacity {
            return Ok(());
        }
        let new_capacity = required.next_power_of_two().max(1024);
        if self.vertex_buffer != vk::Buffer::null() {
            unsafe {
                device.destroy_buffer(self.vertex_buffer, None);
                device.free_memory(self.vertex_memory, None);
            }
        }
        let (buffer, memory) = create_buffer(
            device,
            new_capacity as u64,
            vk::BufferUsageFlags::VERTEX_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            memory_properties,
        )?;
        self.vertex_buffer = buffer;
        self.vertex_memory = memory;
        self.vertex_capacity = new_capacity;
        Ok(())
    }

    fn ensure_index_capacity(
        &mut self,
        device: &ash::Device,
        required: usize,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
    ) -> Result<(), RenderError> {
        if required <= self.index_capacity {
            return Ok(());
        }
        let new_capacity = required.next_power_of_two().max(1024);
        if self.index_buffer != vk::Buffer::null() {
            unsafe {
                device.destroy_buffer(self.index_buffer, None);
                device.free_memory(self.index_memory, None);
            }
        }
        let (buffer, memory) = create_buffer(
            device,
            new_capacity as u64,
            vk::BufferUsageFlags::INDEX_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            memory_properties,
        )?;
        self.index_buffer = buffer;
        self.index_memory = memory;
        self.index_capacity = new_capacity;
        Ok(())
    }

    fn destroy(self, device: &ash::Device) {
        unsafe {
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_pipeline_layout(self.pipeline_layout, None);
            device.destroy_framebuffer(self.framebuffer, None);
            device.destroy_render_pass(self.render_pass, None);
            device.destroy_image_view(self.id_image_view, None);
            device.destroy_image(self.id_image, None);
            device.free_memory(self.id_image_memory, None);
            device.destroy_image_view(self.depth_image_view, None);
            device.destroy_image(self.depth_image, None);
            device.free_memory(self.depth_image_memory, None);
            device.destroy_buffer(self.staging_buffer, None);
            device.free_memory(self.staging_memory, None);
            if self.vertex_buffer != vk::Buffer::null() {
                device.destroy_buffer(self.vertex_buffer, None);
                device.free_memory(self.vertex_memory, None);
            }
            if self.index_buffer != vk::Buffer::null() {
                device.destroy_buffer(self.index_buffer, None);
                device.free_memory(self.index_memory, None);
            }
        }
    }
}
