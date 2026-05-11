use ash::vk;
use std::collections::HashMap;
use std::mem::size_of;
use uuid::Uuid;

use crate::{
    util::create_buffer, BodySubmission, HighlightState, RenderError, ViewportRect, EDGE_FRAG_SPV,
    EDGE_VERT_SPV, MESH_FRAG_SPV, MESH_VERT_SPV,
};

use crate::create_shader_module;

/// Per-vertex format shared by the mesh, edge, and pick pipelines. Albedo
/// lives per-vertex (`color`); [`BodySubmission::color`] scales it for bodies
/// without mesh vertex colours (sketches use white × tint).
#[repr(C)]
pub(crate) struct MeshVertex {
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
    pub(crate) color: [f32; 3],
}

impl MeshVertex {
    pub(crate) fn new(position: [f32; 3], normal: [f32; 3], color: [f32; 3]) -> Self {
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

#[derive(Clone, Copy)]
pub struct LightingData {
    pub main_light: GpuLight,
    pub backlight: GpuLight,
    pub fill_light: GpuLight,
    pub ambient_color: [f32; 3],
    pub ambient_intensity: f32,
    /// Blinn–Phong exponent (matches `LightingSettings::specular_shininess`).
    pub specular_shininess: f32,
    pub specular_intensity: f32,
    /// RGB for `LINE_LIST` face-boundary edges (not shaded).
    pub edge_line_color: [f32; 3],
    /// Requested width in pixels; clamped to `VkPhysicalDeviceLimits::lineWidthRange` when drawing.
    pub edge_line_width: f32,
}

impl Default for LightingData {
    fn default() -> Self {
        Self {
            main_light: GpuLight::default(),
            backlight: GpuLight::default(),
            fill_light: GpuLight::default(),
            ambient_color: [0.0; 3],
            ambient_intensity: 0.0,
            specular_shininess: 64.0,
            specular_intensity: 0.0,
            edge_line_color: [0.08, 0.08, 0.08],
            edge_line_width: 3.0,
        }
    }
}

/// Push-constant payload split into a frame-wide range (offset 0) and a
/// per-draw range (offset `MESH_DRAW_PUSH_OFFSET`). Vulkan happily addresses
/// both via `cmd_push_constants` with the appropriate offset, so the heavy
/// frame fields are written once per render pass while only 16 bytes change
/// per body.
#[repr(C)]
#[derive(Clone, Copy)]
struct MeshFramePushConstants {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    light_main: GpuLight,
    light_back: GpuLight,
    light_fill: GpuLight,
    ambient: [f32; 4],
    /// x = shininess exponent, y = specular intensity, zw unused (see `mesh.frag`).
    shading: [f32; 4],
}

impl MeshFramePushConstants {
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
            shading: [
                lights.specular_shininess.max(1.0),
                lights.specular_intensity.max(0.0),
                0.0,
                0.0,
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MeshDrawPushConstants {
    /// xyz = base × highlight color (precomputed CPU-side via
    /// `apply_highlight_color`); w is reserved for future highlight flag bits.
    draw_color: [f32; 4],
}

const MESH_FRAME_PUSH_SIZE: u32 = size_of::<MeshFramePushConstants>() as u32;
const MESH_DRAW_PUSH_SIZE: u32 = size_of::<MeshDrawPushConstants>() as u32;
const MESH_DRAW_PUSH_OFFSET: u32 = MESH_FRAME_PUSH_SIZE;
const MESH_TOTAL_PUSH_SIZE: u32 = MESH_FRAME_PUSH_SIZE + MESH_DRAW_PUSH_SIZE;

/// Vertex-count threshold above which the parallel CPU pack path wins.
/// Below it the rayon dispatch overhead dominates the actual memcpy.
const PARALLEL_PACK_THRESHOLD: usize = 16_384;

/// GPU buffers for a single body, kept alive across frames so a static mesh
/// only ever uploads once. Keyed by `BodySubmission::id` and invalidated when
/// `BodySubmission::revision` advances.
pub(crate) struct CachedMesh {
    pub(crate) vertex_buffer: vk::Buffer,
    pub(crate) vertex_memory: vk::DeviceMemory,
    pub(crate) vertex_count: u32,
    pub(crate) vertex_capacity: usize,
    pub(crate) index_buffer: vk::Buffer,
    pub(crate) index_memory: vk::DeviceMemory,
    pub(crate) index_count: u32,
    pub(crate) index_capacity: usize,
    pub(crate) edge_index_buffer: vk::Buffer,
    pub(crate) edge_index_memory: vk::DeviceMemory,
    pub(crate) edge_index_count: u32,
    pub(crate) edge_index_capacity: usize,
    pub(crate) revision: u64,
}

impl CachedMesh {
    fn destroy(self, device: &ash::Device) {
        unsafe {
            if self.vertex_buffer != vk::Buffer::null() {
                device.destroy_buffer(self.vertex_buffer, None);
                device.free_memory(self.vertex_memory, None);
            }
            if self.index_buffer != vk::Buffer::null() {
                device.destroy_buffer(self.index_buffer, None);
                device.free_memory(self.index_memory, None);
            }
            if self.edge_index_buffer != vk::Buffer::null() {
                device.destroy_buffer(self.edge_index_buffer, None);
                device.free_memory(self.edge_index_memory, None);
            }
        }
    }
}

/// Per-body GPU buffer cache shared by the mesh and pick passes. A body's
/// `CachedMesh` only re-uploads when its `BodySubmission::revision` advances;
/// pan/orbit/hover transitions all hit the cache.
pub(crate) struct MeshCache {
    entries: HashMap<Uuid, CachedMesh>,
}

impl MeshCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub(crate) fn get(&self, id: &Uuid) -> Option<&CachedMesh> {
        self.entries.get(id)
    }

    /// Upload (or refresh) a body's GPU buffers. Returns the cached entry's
    /// counts so the caller can issue a draw immediately. Reuses existing
    /// buffer allocations when capacity permits to avoid the allocator hit on
    /// hover/select transitions in degenerate cases.
    fn upload_body(
        &mut self,
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        body: &BodySubmission,
    ) -> Result<(), RenderError> {
        let mesh = body.mesh.as_ref();
        let vertex_count = mesh.positions.len();
        let index_count = if mesh.indices.is_empty() {
            mesh.positions.len()
        } else {
            mesh.indices.len()
        };
        let edge_count = mesh.edges.len();

        let vertex_bytes = vertex_count * size_of::<MeshVertex>();
        let index_bytes = index_count * size_of::<u32>();
        let edge_bytes = edge_count * size_of::<u32>();

        let entry = self.entries.entry(body.id).or_insert_with(|| CachedMesh {
            vertex_buffer: vk::Buffer::null(),
            vertex_memory: vk::DeviceMemory::null(),
            vertex_count: 0,
            vertex_capacity: 0,
            index_buffer: vk::Buffer::null(),
            index_memory: vk::DeviceMemory::null(),
            index_count: 0,
            index_capacity: 0,
            edge_index_buffer: vk::Buffer::null(),
            edge_index_memory: vk::DeviceMemory::null(),
            edge_index_count: 0,
            edge_index_capacity: 0,
            revision: u64::MAX,
        });

        ensure_buffer(
            device,
            memory_properties,
            &mut entry.vertex_buffer,
            &mut entry.vertex_memory,
            &mut entry.vertex_capacity,
            vertex_bytes,
            vk::BufferUsageFlags::VERTEX_BUFFER,
        )?;
        ensure_buffer(
            device,
            memory_properties,
            &mut entry.index_buffer,
            &mut entry.index_memory,
            &mut entry.index_capacity,
            index_bytes,
            vk::BufferUsageFlags::INDEX_BUFFER,
        )?;
        ensure_buffer(
            device,
            memory_properties,
            &mut entry.edge_index_buffer,
            &mut entry.edge_index_memory,
            &mut entry.edge_index_capacity,
            edge_bytes,
            vk::BufferUsageFlags::INDEX_BUFFER,
        )?;

        // Pack the vertex stream in parallel for big bodies. The work is
        // embarrassingly parallel — each output vertex depends only on the
        // matching position/normal — so rayon's `par_chunks_mut` over the
        // mapped HOST_VISIBLE memory wins ~linearly with cores. For small
        // bodies the chunk-by-chunk overhead is wasted, so we keep a
        // sequential fast path under PARALLEL_PACK_THRESHOLD vertices.
        if vertex_count > 0 {
            unsafe {
                let ptr = device
                    .map_memory(
                        entry.vertex_memory,
                        0,
                        vertex_bytes as u64,
                        vk::MemoryMapFlags::empty(),
                    )
                    .map_err(RenderError::from)? as *mut MeshVertex;
                let slice = std::slice::from_raw_parts_mut(ptr, vertex_count);
                if vertex_count >= PARALLEL_PACK_THRESHOLD {
                    use rayon::prelude::*;
                    slice.par_iter_mut().enumerate().for_each(|(i, dst)| {
                        let pos = mesh.positions[i];
                        let normal = mesh.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
                        let color = mesh.colors.get(i).copied().unwrap_or([1.0, 1.0, 1.0]);
                        *dst = MeshVertex::new(pos, normal, color);
                    });
                } else {
                    for i in 0..vertex_count {
                        let pos = mesh.positions[i];
                        let normal = mesh.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
                        let color = mesh.colors.get(i).copied().unwrap_or([1.0, 1.0, 1.0]);
                        slice[i] = MeshVertex::new(pos, normal, color);
                    }
                }
                device.unmap_memory(entry.vertex_memory);
            }
        }

        if index_count > 0 {
            unsafe {
                let ptr = device
                    .map_memory(
                        entry.index_memory,
                        0,
                        index_bytes as u64,
                        vk::MemoryMapFlags::empty(),
                    )
                    .map_err(RenderError::from)? as *mut u32;
                let slice = std::slice::from_raw_parts_mut(ptr, index_count);
                if mesh.indices.is_empty() {
                    if vertex_count >= PARALLEL_PACK_THRESHOLD {
                        use rayon::prelude::*;
                        slice
                            .par_iter_mut()
                            .enumerate()
                            .for_each(|(i, dst)| *dst = i as u32);
                    } else {
                        for i in 0..vertex_count {
                            slice[i] = i as u32;
                        }
                    }
                } else {
                    slice.copy_from_slice(&mesh.indices);
                }
                device.unmap_memory(entry.index_memory);
            }
        }

        if edge_count > 0 {
            unsafe {
                let ptr = device
                    .map_memory(
                        entry.edge_index_memory,
                        0,
                        edge_bytes as u64,
                        vk::MemoryMapFlags::empty(),
                    )
                    .map_err(RenderError::from)? as *mut u32;
                let slice = std::slice::from_raw_parts_mut(ptr, edge_count);
                slice.copy_from_slice(&mesh.edges);
                device.unmap_memory(entry.edge_index_memory);
            }
        }

        entry.vertex_count = vertex_count as u32;
        entry.index_count = index_count as u32;
        entry.edge_index_count = edge_count as u32;
        entry.revision = body.revision;
        Ok(())
    }

    /// Make sure `id` has up-to-date GPU buffers. Re-uploads on revision
    /// mismatch; cheap (single hashmap probe) on cache hits.
    pub(crate) fn ensure_uploaded(
        &mut self,
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        body: &BodySubmission,
    ) -> Result<(), RenderError> {
        let needs_upload = match self.get(&body.id) {
            Some(cached) => cached.revision != body.revision,
            None => true,
        };
        if needs_upload {
            self.upload_body(device, memory_properties, body)?;
        }
        Ok(())
    }

    /// True if at least one cache entry is no longer in the alive set. The
    /// renderer uses this to short-circuit the wait-idle + retain pair on
    /// the common no-deletion path so panning a stable scene stays free.
    pub(crate) fn has_dead_entries(&self, alive: &[Uuid]) -> bool {
        if alive.len() == self.entries.len() {
            // Same count → check whether every cached id is in `alive`.
            let alive_set: std::collections::HashSet<&Uuid> = alive.iter().collect();
            self.entries.keys().any(|id| !alive_set.contains(id))
        } else {
            // Different count → either we have stale entries (more cache
            // than alive) or new bodies; only the former is a GC trigger.
            self.entries.len() > alive.len() || {
                let alive_set: std::collections::HashSet<&Uuid> = alive.iter().collect();
                self.entries.keys().any(|id| !alive_set.contains(id))
            }
        }
    }

    /// Drop any entry whose id was not referenced by the supplied set. Caller
    /// must guarantee no command buffer in flight is still using the buffers
    /// being destroyed (we wait on the device idle in the renderer's draw
    /// path before invoking GC).
    pub(crate) fn retain_only(&mut self, device: &ash::Device, alive: &[Uuid]) {
        let alive_set: std::collections::HashSet<&Uuid> = alive.iter().collect();
        let dead: Vec<Uuid> = self
            .entries
            .keys()
            .filter(|id| !alive_set.contains(id))
            .copied()
            .collect();
        for id in dead {
            if let Some(entry) = self.entries.remove(&id) {
                entry.destroy(device);
            }
        }
    }

    pub(crate) fn destroy(&mut self, device: &ash::Device) {
        for (_, entry) in self.entries.drain() {
            entry.destroy(device);
        }
    }
}

fn ensure_buffer(
    device: &ash::Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    buffer: &mut vk::Buffer,
    memory: &mut vk::DeviceMemory,
    capacity: &mut usize,
    required: usize,
    usage: vk::BufferUsageFlags,
) -> Result<(), RenderError> {
    if required <= *capacity && *buffer != vk::Buffer::null() {
        return Ok(());
    }
    if required == 0 {
        return Ok(());
    }
    let new_capacity = required.next_power_of_two().max(1024);
    if *buffer != vk::Buffer::null() {
        unsafe {
            // Other swapchain frames may still reference this buffer from an
            // in-flight command buffer — only `MAX_FRAMES` fences are tracked,
            // not a per-buffer fence. Idle before reallocating.
            device.device_wait_idle().map_err(RenderError::from)?;
            device.destroy_buffer(*buffer, None);
            device.free_memory(*memory, None);
        }
    }
    let (new_buffer, new_memory) = create_buffer(
        device,
        new_capacity as u64,
        usage,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        memory_properties,
    )?;
    *buffer = new_buffer;
    *memory = new_memory;
    *capacity = new_capacity;
    Ok(())
}

pub(crate) struct MeshRenderer {
    device: ash::Device,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    wireframe_pipeline: vk::Pipeline,
    edge_pipeline: vk::Pipeline,
    msaa_samples: vk::SampleCountFlags,
    solid_line_width: f32,
    line_width_range: [f32; 2],
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
        let line_range = unsafe { instance.get_physical_device_properties(physical_device) }
            .limits
            .line_width_range;
        let solid_line_width = 1.0f32.clamp(line_range[0], line_range[1]);

        let pipeline_layout = create_mesh_pipeline_layout(&device)?;
        let pipeline = create_mesh_pipeline(
            &device,
            render_pass,
            pipeline_layout,
            msaa_samples,
            MeshPipelineMode::Solid,
            solid_line_width,
            false,
        )?;
        let wireframe_pipeline = create_mesh_pipeline(
            &device,
            render_pass,
            pipeline_layout,
            msaa_samples,
            MeshPipelineMode::WireframeTriangles,
            solid_line_width,
            false,
        )?;
        let edge_pipeline = create_mesh_pipeline(
            &device,
            render_pass,
            pipeline_layout,
            msaa_samples,
            MeshPipelineMode::Edges,
            solid_line_width,
            true,
        )?;

        Ok(Self {
            device,
            memory_properties,
            pipeline_layout,
            pipeline,
            wireframe_pipeline,
            edge_pipeline,
            msaa_samples,
            solid_line_width,
            line_width_range: line_range,
        })
    }

    pub fn set_render_pass(
        &mut self,
        render_pass: vk::RenderPass,
        msaa_samples: vk::SampleCountFlags,
    ) -> Result<(), RenderError> {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline(self.wireframe_pipeline, None);
            self.device.destroy_pipeline(self.edge_pipeline, None);
        }
        self.msaa_samples = msaa_samples;
        self.pipeline = create_mesh_pipeline(
            &self.device,
            render_pass,
            self.pipeline_layout,
            msaa_samples,
            MeshPipelineMode::Solid,
            self.solid_line_width,
            false,
        )?;
        self.wireframe_pipeline = create_mesh_pipeline(
            &self.device,
            render_pass,
            self.pipeline_layout,
            msaa_samples,
            MeshPipelineMode::WireframeTriangles,
            self.solid_line_width,
            false,
        )?;
        self.edge_pipeline = create_mesh_pipeline(
            &self.device,
            render_pass,
            self.pipeline_layout,
            msaa_samples,
            MeshPipelineMode::Edges,
            self.solid_line_width,
            true,
        )?;
        Ok(())
    }

    pub fn memory_properties(&self) -> &vk::PhysicalDeviceMemoryProperties {
        &self.memory_properties
    }

    pub fn draw(
        &mut self,
        cache: &mut MeshCache,
        command_buffer: vk::CommandBuffer,
        swapchain_extent: vk::Extent2D,
        viewport_rect: Option<&ViewportRect>,
        bodies: &[BodySubmission],
        view_proj: [[f32; 4]; 4],
        camera_pos: [f32; 3],
        lighting: &LightingData,
    ) -> Result<(), RenderError> {
        // Make sure every body has fresh GPU buffers in the cache.
        for body in bodies {
            cache.ensure_uploaded(&self.device, &self.memory_properties, body)?;
        }

        if bodies.is_empty() {
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

        let frame_pc = MeshFramePushConstants::new(view_proj, camera_pos, lighting);

        // Solid pass: bind solid pipeline once, draw every non-wireframe body
        // sequentially. Wireframes get a second pass with the depth-biased
        // pipeline, edges a third with line-list topology.
        let has_solid = bodies.iter().any(|b| !b.is_wireframe);
        if has_solid {
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline,
                );
                self.device.cmd_set_viewport(command_buffer, 0, &[viewport]);
                self.device.cmd_set_scissor(command_buffer, 0, &[scissor]);
                self.push_frame_constants(command_buffer, &frame_pc);
            }
            for body in bodies.iter().filter(|b| !b.is_wireframe) {
                let cached = match cache.get(&body.id) {
                    Some(c) if c.index_count > 0 => c,
                    _ => continue,
                };
                self.draw_body(command_buffer, cached, body, false);
            }
        }

        // Edges after solids: biased + LEQUAL depth test (see
        // `MeshPipelineMode::Edges`). Each body has its own edge index buffer
        // pointing into its own vertex buffer.
        let has_edges = bodies
            .iter()
            .filter(|b| !b.is_wireframe)
            .any(|b| matches!(cache.get(&b.id), Some(c) if c.edge_index_count > 0));
        if has_edges {
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.edge_pipeline,
                );
                let edge_w = lighting
                    .edge_line_width
                    .clamp(self.line_width_range[0], self.line_width_range[1]);
                self.device.cmd_set_line_width(command_buffer, edge_w);
                self.device.cmd_set_viewport(command_buffer, 0, &[viewport]);
                self.device.cmd_set_scissor(command_buffer, 0, &[scissor]);
                self.push_frame_constants(command_buffer, &frame_pc);
            }
            for body in bodies.iter().filter(|b| !b.is_wireframe) {
                let cached = match cache.get(&body.id) {
                    Some(c) if c.edge_index_count > 0 => c,
                    _ => continue,
                };
                self.draw_body_edges(command_buffer, cached, lighting);
            }
        }

        let has_wireframe = bodies.iter().any(|b| b.is_wireframe);
        if has_wireframe {
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.wireframe_pipeline,
                );
                self.device.cmd_set_viewport(command_buffer, 0, &[viewport]);
                self.device.cmd_set_scissor(command_buffer, 0, &[scissor]);
                self.push_frame_constants(command_buffer, &frame_pc);
            }
            for body in bodies.iter().filter(|b| b.is_wireframe) {
                let cached = match cache.get(&body.id) {
                    Some(c) if c.index_count > 0 => c,
                    _ => continue,
                };
                self.draw_body(command_buffer, cached, body, true);
            }
        }

        Ok(())
    }

    fn push_frame_constants(
        &self,
        command_buffer: vk::CommandBuffer,
        frame_pc: &MeshFramePushConstants,
    ) {
        unsafe {
            let bytes = std::slice::from_raw_parts(
                frame_pc as *const _ as *const u8,
                MESH_FRAME_PUSH_SIZE as usize,
            );
            self.device.cmd_push_constants(
                command_buffer,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytes,
            );
        }
    }

    fn push_draw_constants(
        &self,
        command_buffer: vk::CommandBuffer,
        draw_pc: &MeshDrawPushConstants,
    ) {
        unsafe {
            let bytes = std::slice::from_raw_parts(
                draw_pc as *const _ as *const u8,
                MESH_DRAW_PUSH_SIZE as usize,
            );
            self.device.cmd_push_constants(
                command_buffer,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                MESH_DRAW_PUSH_OFFSET,
                bytes,
            );
        }
    }

    fn draw_body(
        &self,
        command_buffer: vk::CommandBuffer,
        cached: &CachedMesh,
        body: &BodySubmission,
        _is_wireframe: bool,
    ) {
        let final_color = apply_highlight_color(body.color, body.highlight);
        let draw_pc = MeshDrawPushConstants {
            draw_color: [final_color[0], final_color[1], final_color[2], 0.0],
        };
        self.push_draw_constants(command_buffer, &draw_pc);
        unsafe {
            self.device
                .cmd_bind_vertex_buffers(command_buffer, 0, &[cached.vertex_buffer], &[0]);
            self.device.cmd_bind_index_buffer(
                command_buffer,
                cached.index_buffer,
                0,
                vk::IndexType::UINT32,
            );
            self.device
                .cmd_draw_indexed(command_buffer, cached.index_count, 1, 0, 0, 0);
        }
    }

    fn draw_body_edges(
        &self,
        command_buffer: vk::CommandBuffer,
        cached: &CachedMesh,
        lighting: &LightingData,
    ) {
        let c = lighting.edge_line_color;
        let draw_pc = MeshDrawPushConstants {
            draw_color: [c[0], c[1], c[2], 0.0],
        };
        self.push_draw_constants(command_buffer, &draw_pc);
        unsafe {
            self.device
                .cmd_bind_vertex_buffers(command_buffer, 0, &[cached.vertex_buffer], &[0]);
            self.device.cmd_bind_index_buffer(
                command_buffer,
                cached.edge_index_buffer,
                0,
                vk::IndexType::UINT32,
            );
            self.device
                .cmd_draw_indexed(command_buffer, cached.edge_index_count, 1, 0, 0, 0);
        }
    }

    pub fn destroy(self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline(self.wireframe_pipeline, None);
            self.device.destroy_pipeline(self.edge_pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
        }
    }
}

/// Variant selector for `create_mesh_pipeline`. The three pipelines all share
/// the same vertex format, push-constant block, and pipeline layout — they
/// only differ in shaders, topology, polygon mode, culling, and depth state.
#[derive(Clone, Copy)]
pub(crate) enum MeshPipelineMode {
    /// Standard solid mesh pass (triangle list, fill, back-face culling on,
    /// depth write on).
    Solid,
    /// Triangle wireframe overlay drawn on top of the solid pass with depth
    /// bias toward the camera.
    WireframeTriangles,
    /// Face-boundary outlines as a `LINE_LIST`. Two-sided (no culling),
    /// depth-tested (`LESS_OR_EQUAL`, depth write off) for correct occlusion.
    /// Small constant raster bias + clip-space pull in `edge.vert` limit
    /// coplanar z-fight; if bias is too strong, edges “see through” occluders
    /// (tune those constants vs MSAA / line width).
    Edges,
}

fn create_mesh_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    layout: vk::PipelineLayout,
    msaa_samples: vk::SampleCountFlags,
    mode: MeshPipelineMode,
    line_width: f32,
    dynamic_line_width: bool,
) -> Result<vk::Pipeline, RenderError> {
    let (vert_spv, frag_spv) = match mode {
        MeshPipelineMode::Solid | MeshPipelineMode::WireframeTriangles => {
            (MESH_VERT_SPV, MESH_FRAG_SPV)
        }
        MeshPipelineMode::Edges => (EDGE_VERT_SPV, EDGE_FRAG_SPV),
    };
    let vert_module = create_shader_module(device, vert_spv)?;
    let frag_module = create_shader_module(device, frag_spv)?;

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

    let binding_descs = [binding_desc];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&binding_descs)
        .vertex_attribute_descriptions(&attr_descs);

    let topology = match mode {
        MeshPipelineMode::Solid | MeshPipelineMode::WireframeTriangles => {
            vk::PrimitiveTopology::TRIANGLE_LIST
        }
        MeshPipelineMode::Edges => vk::PrimitiveTopology::LINE_LIST,
    };
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(topology)
        .primitive_restart_enable(false);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    // Wireframes and boundary edges pull slightly toward the camera so LEQUAL
    // passes against coplanar solid depth. For `LINE_LIST`, use **constant
    // bias only** (slope is poorly defined and can over-pull on steep spans).
    // Too much bias + `edge.vert` nudge causes ghost edges through occluders.
    let (depth_bias_enable, depth_bias_constant_factor, depth_bias_slope_factor) = match mode {
        MeshPipelineMode::Solid => (false, 0.0, 0.0),
        MeshPipelineMode::WireframeTriangles => (true, 1.0, 1.0),
        MeshPipelineMode::Edges => (true, -0.55, 0.0),
    };

    // Overlays must not stomp on the depth buffer, otherwise downstream
    // overlays (selection highlights, gizmos, …) lose their occlusion cues.
    let depth_write = matches!(mode, MeshPipelineMode::Solid);

    let polygon_mode = match mode {
        MeshPipelineMode::Solid => vk::PolygonMode::FILL,
        MeshPipelineMode::WireframeTriangles => vk::PolygonMode::LINE,
        MeshPipelineMode::Edges => vk::PolygonMode::FILL, // ignored for line list
    };
    // Back-face culling is intentionally disabled on the solid pass.
    //
    // STEP files imported via OCCT's `STEPControl_Reader` regularly contain
    // faces whose triangulation is wound inward instead of outward, and we
    // can't reliably detect that from `face.Orientation()` alone (the
    // parametric → 3D mapping plus shell composition can flip the winding
    // independently). Culling such faces creates the "see-through holes"
    // symptom (back faces poking through the front) that we observe on real
    // STEP files. The fragment shader handles two-sided solids by flipping the
    // shading normal when `gl_FrontFacing` is false, and depth correctly hides the
    // far surface when both sides rasterize.
    //
    // Cost: roughly twice as many fragments enter the depth test, but most
    // of them fail it cheaply and never run the full lighting code.
    let cull_mode = match mode {
        MeshPipelineMode::Solid | MeshPipelineMode::Edges => vk::CullModeFlags::NONE,
        MeshPipelineMode::WireframeTriangles => vk::CullModeFlags::BACK,
    };
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .depth_clamp_enable(false)
        .rasterizer_discard_enable(false)
        .polygon_mode(polygon_mode)
        .line_width(line_width)
        .cull_mode(cull_mode)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .depth_bias_enable(depth_bias_enable)
        .depth_bias_constant_factor(depth_bias_constant_factor)
        .depth_bias_slope_factor(depth_bias_slope_factor);

    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .sample_shading_enable(false)
        .rasterization_samples(msaa_samples);

    // `LESS_OR_EQUAL` + modest bias lets outlines sit on their own surface
    // without winning over clearly nearer geometry (avoid heavy bias + nudge).
    let (depth_test_enable, depth_compare_op) = match mode {
        MeshPipelineMode::Solid => (true, vk::CompareOp::LESS),
        MeshPipelineMode::WireframeTriangles | MeshPipelineMode::Edges => {
            (true, vk::CompareOp::LESS_OR_EQUAL)
        }
    };
    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(depth_test_enable)
        .depth_write_enable(depth_write)
        .depth_compare_op(depth_compare_op)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)
        .blend_enable(false);

    let color_blend_attachments = [color_blend_attachment];
    let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
        .logic_op_enable(false)
        .attachments(&color_blend_attachments);

    let dynamic_states_with_line = [
        vk::DynamicState::VIEWPORT,
        vk::DynamicState::SCISSOR,
        vk::DynamicState::LINE_WIDTH,
    ];
    let dynamic_states_basic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state = if dynamic_line_width {
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states_with_line)
    } else {
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states_basic)
    };

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
    // Single push-constant range that spans both the frame fields (offset 0)
    // and the per-draw fields (offset MESH_DRAW_PUSH_OFFSET). Vulkan happily
    // lets us update either sub-range with `cmd_push_constants` so we only
    // pay 16 bytes per body for highlight/color updates.
    let push_constant_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
        .offset(0)
        .size(MESH_TOTAL_PUSH_SIZE);

    let push_constant_ranges = [push_constant_range];
    let layout_info =
        vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&push_constant_ranges);

    unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(RenderError::from)
}
