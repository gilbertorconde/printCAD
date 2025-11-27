use ash::vk;
use std::mem::size_of;

use crate::{
    util::create_buffer, BodySubmission, HighlightState, RenderError, ViewportRect, MESH_FRAG_SPV,
    MESH_VERT_SPV,
};

use crate::create_shader_module;

#[repr(C)]
pub(super) struct MeshVertex {
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

fn apply_highlight_color(base: [f32; 3], highlight: HighlightState) -> [f32; 3] {
    match highlight {
        HighlightState::None => base,
        HighlightState::Hovered => [
            (base[0] * 1.2 + 0.1).min(1.0),
            (base[1] * 1.2 + 0.15).min(1.0),
            (base[2] * 1.2 + 0.2).min(1.0),
        ],
        HighlightState::Selected => [
            (base[0] * 0.7 + 0.3).min(1.0),
            (base[1] * 0.7 + 0.2).min(1.0),
            (base[2] * 0.5).min(1.0),
        ],
        HighlightState::HoveredAndSelected => [
            (base[0] * 0.6 + 0.4).min(1.0),
            (base[1] * 0.6 + 0.35).min(1.0),
            (base[2] * 0.4 + 0.1).min(1.0),
        ],
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GpuLight {
    pub direction_intensity: [f32; 4],
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

#[derive(Clone, Copy, Default)]
pub struct LightingData {
    pub main_light: GpuLight,
    pub backlight: GpuLight,
    pub fill_light: GpuLight,
    pub ambient_color: [f32; 3],
    pub ambient_intensity: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MeshPushConstants {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    light_main: GpuLight,
    light_back: GpuLight,
    light_fill: GpuLight,
    ambient: [f32; 4],
}

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

pub(super) struct MeshRenderer {
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
    pub fn new(
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

    pub fn set_render_pass(
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

    pub fn draw(
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
            self.device.unmap_memory(self.index_memory);
        }

        Ok(index_count as u32)
    }

    fn ensure_vertex_capacity(&mut self, required: usize) -> Result<(), RenderError> {
        if required <= self.vertex_capacity {
            return Ok(());
        }
        let new_capacity = required.next_power_of_two().max(1024);
        if self.vertex_buffer != vk::Buffer::null() {
            unsafe {
                self.device.destroy_buffer(self.vertex_buffer, None);
                self.device.free_memory(self.vertex_memory, None);
            }
        }
        let (buffer, memory) = create_buffer(
            &self.device,
            new_capacity as u64,
            vk::BufferUsageFlags::VERTEX_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            &self.memory_properties,
        )?;
        self.vertex_buffer = buffer;
        self.vertex_memory = memory;
        self.vertex_capacity = new_capacity;
        Ok(())
    }

    fn ensure_index_capacity(&mut self, required: usize) -> Result<(), RenderError> {
        if required <= self.index_capacity {
            return Ok(());
        }
        let new_capacity = required.next_power_of_two().max(1024);
        if self.index_buffer != vk::Buffer::null() {
            unsafe {
                self.device.destroy_buffer(self.index_buffer, None);
                self.device.free_memory(self.index_memory, None);
            }
        }
        let (buffer, memory) = create_buffer(
            &self.device,
            new_capacity as u64,
            vk::BufferUsageFlags::INDEX_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            &self.memory_properties,
        )?;
        self.index_buffer = buffer;
        self.index_memory = memory;
        self.index_capacity = new_capacity;
        Ok(())
    }

    pub fn destroy(self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device.destroy_buffer(self.vertex_buffer, None);
            self.device.free_memory(self.vertex_memory, None);
            self.device.destroy_buffer(self.index_buffer, None);
            self.device.free_memory(self.index_memory, None);
        }
    }
}

fn create_mesh_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    layout: vk::PipelineLayout,
    msaa_samples: vk::SampleCountFlags,
) -> Result<vk::Pipeline, RenderError> {
    let vert_module = create_shader_module(device, MESH_VERT_SPV)?;
    let frag_module = create_shader_module(device, MESH_FRAG_SPV)?;

    let entry_name = std::ffi::CString::new("main").unwrap();
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

    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&[binding_desc])
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
        .rasterization_samples(msaa_samples);

    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::LESS)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)
        .blend_enable(false);

    let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
        .logic_op_enable(false)
        .attachments(&[color_blend_attachment]);

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

fn create_mesh_pipeline_layout(device: &ash::Device) -> Result<vk::PipelineLayout, RenderError> {
    let push_constant_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
        .offset(0)
        .size(size_of::<MeshPushConstants>() as u32);

    let push_constant_ranges = [push_constant_range];
    let layout_info =
        vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&push_constant_ranges);

    unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(RenderError::from)
}
