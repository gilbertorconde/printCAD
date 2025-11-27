use std::{ffi::CString, mem::size_of};

use ash::vk;
use uuid::Uuid;

use crate::{
    create_shader_module,
    mesh::MeshVertex,
    util::{create_buffer, create_image, create_image_view},
    BodySubmission, PickResult, RenderError, ViewportRect, PICK_FRAG_SPV, PICK_VERT_SPV,
};

/// Push constants for the picking shader
#[repr(C)]
#[derive(Clone, Copy)]
struct PickPushConstants {
    view_proj: [[f32; 4]; 4],
    object_id: [u32; 4], // UUID encoded as 4 u32s
}

/// GPU-based picking renderer that renders object IDs to an offscreen buffer
pub(crate) struct PickRenderer {
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
    pub(crate) fn new(
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
    pub(crate) fn record_commands(
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
    pub(crate) fn read_pick_result(
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

    pub(crate) fn destroy(self, device: &ash::Device) {
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
