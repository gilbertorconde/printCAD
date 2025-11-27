use std::{
    collections::HashSet,
    ffi::{CStr, CString},
};

use ash::{
    khr::{surface::Instance as SurfaceLoader, swapchain::Device as SwapchainLoader},
    vk, Entry,
};
use egui::TextureId;
use egui_ash_renderer::{Options as EguiRendererOptions, Renderer as EguiRenderer};
use tracing::{debug, info, warn};
use uuid::Uuid;
use winit::window::Window;

use crate::{
    find_depth_format, get_max_usable_sample_count, identity_matrix, is_srgb_format, map_egui_err,
    mesh::MeshRenderer, msaa_samples_to_vk, picking::PickRenderer, surface, util::find_memory_type,
    FrameSubmission, PickResult, RenderError, RenderSettings, ViewportRect, MAX_FRAMES_IN_FLIGHT,
    VALIDATION_LAYER,
};

pub(crate) struct RendererCore {
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
    pub(crate) fn new(
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

    pub(crate) fn recreate_swapchain(&mut self, extent: vk::Extent2D) -> Result<(), RenderError> {
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

    pub(crate) fn gpu_name(&self) -> &str {
        &self.gpu_name
    }

    pub(crate) fn available_gpus(&self) -> &[String] {
        &self.available_gpus
    }

    pub(crate) fn request_pick(&mut self, x: u32, y: u32) {
        self.pending_pick = Some((x, y));
    }

    pub(crate) fn last_pick_result(&self) -> PickResult {
        self.last_pick_result.clone()
    }

    pub(crate) fn swapchain_extent(&self) -> vk::Extent2D {
        self.swapchain_extent
    }

    pub(crate) fn draw_frame(&mut self, frame: &FrameSubmission) -> Result<(), RenderError> {
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
