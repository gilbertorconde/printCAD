use std::collections::HashMap;
use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use uuid::Uuid;

use crate::{BodySubmission, FrameSubmission, HighlightState};

/// Per-vertex format shared by the mesh and pick pipelines. Albedo lives
/// per-vertex (`color`); [`BodySubmission::color`] scales it for bodies
/// without mesh vertex colours (sketches use white × tint).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct MeshVertex {
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
    pub(crate) color: [f32; 3],
}

/// One edge segment, drawn as an instanced quad: its two ends.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct EdgeSegment {
    pub(crate) a: [f32; 3],
    pub(crate) b: [f32; 3],
}

pub(crate) fn apply_highlight_color(base: [f32; 3], highlight: HighlightState) -> [f32; 3] {
    match highlight {
        HighlightState::None => base,
        HighlightState::Hovered => [
            (base[0] * 1.2 + 0.1).min(1.0),
            (base[1] * 1.2 + 0.15).min(1.0),
            (base[2] * 1.2 + 0.2).min(1.0),
        ],
        HighlightState::PeerSelected => [
            (base[0] * 0.6 + 0.05).min(1.0),
            (base[1] * 0.7 + 0.25).min(1.0),
            (base[2] * 0.7 + 0.35).min(1.0),
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
#[derive(Clone, Copy, Default, Pod, Zeroable)]
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
    /// RGB of the face-boundary edges (not shaded).
    pub edge_line_color: [f32; 3],
    /// Requested width in pixels; clamped to [`EDGE_WIDTH_RANGE`] when drawing.
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

/// The widths an edge may be drawn at, in pixels. Edges are quads, so any
/// width draws; the range keeps a stray setting sensible.
pub(crate) const EDGE_WIDTH_RANGE: [f32; 2] = [1.0, 16.0];

/// What every scene shader reads, written once per frame; matches `Frame`
/// in `scene.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    light_main: GpuLight,
    light_back: GpuLight,
    light_fill: GpuLight,
    ambient: [f32; 4],
    /// x = shininess exponent, y = specular intensity.
    shading: [f32; 4],
    /// The clipping plane, or [`NO_CLIP`].
    clip_plane: [f32; 4],
    /// xy = the viewport in pixels, z = the edge width in pixels.
    viewport: [f32; 4],
}

/// A clip plane that keeps everything: its distance is 1 at every point.
pub(crate) const NO_CLIP: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

impl FrameUniform {
    pub(crate) fn new(frame: &FrameSubmission, viewport: [f32; 2]) -> Self {
        let lights = &frame.lighting;
        let p = frame.camera_pos;
        Self {
            view_proj: frame.view_proj,
            camera_pos: [p[0], p[1], p[2], 1.0],
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
            clip_plane: frame.clip_plane.unwrap_or(NO_CLIP),
            viewport: [
                viewport[0].max(1.0),
                viewport[1].max(1.0),
                lights
                    .edge_line_width
                    .clamp(EDGE_WIDTH_RANGE[0], EDGE_WIDTH_RANGE[1]),
                0.0,
            ],
        }
    }
}

/// One body's draw data, at a dynamic offset; matches `Draw` in
/// `scene.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct DrawUniform {
    face_color: [f32; 4],
    edge_color: [f32; 4],
    object_id: [u32; 4],
}

impl DrawUniform {
    fn new(body: &BodySubmission, lighting: &LightingData, is_line_body: bool) -> Self {
        let face = apply_highlight_color(body.color, body.highlight);
        // Face-boundary edges take the body's edge colour, else the
        // scene's; a line body has nothing else to show its own colour with.
        let edge = if is_line_body {
            face
        } else {
            body.edge_color.unwrap_or(lighting.edge_line_color)
        };
        Self {
            face_color: [face[0], face[1], face[2], body.opacity.clamp(0.0, 1.0)],
            edge_color: [edge[0], edge[1], edge[2], 0.0],
            object_id: uuid_to_u32s(body.id),
        }
    }
}

pub(crate) fn uuid_to_u32s(uuid: Uuid) -> [u32; 4] {
    let b = uuid.as_bytes();
    [
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
        u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
    ]
}

pub(crate) fn u32s_to_uuid(values: [u32; 4]) -> Uuid {
    let mut bytes = [0u8; 16];
    for (i, v) in values.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    Uuid::from_bytes(bytes)
}

/// Vertex-count threshold above which the parallel CPU pack path wins.
/// Below it the rayon dispatch overhead dominates the actual copy.
#[cfg(not(target_arch = "wasm32"))]
const PARALLEL_PACK_THRESHOLD: usize = 16_384;

/// The six clip planes of a column-vector `view_proj`, Gribb–Hartmann form:
/// each plane is `[a, b, c, d]` with `a·x + b·y + c·z + d >= 0` inside.
/// Works for any convention baked into the matrix (including our Y-flip),
/// because the planes are extracted from the very matrix the vertex shader
/// applies.
pub(crate) fn frustum_planes(m: &[[f32; 4]; 4]) -> [[f32; 4]; 6] {
    // m is column-major (as handed to the GPU): m[col][row].
    let row = |r: usize| [m[0][r], m[1][r], m[2][r], m[3][r]];
    let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
    let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
    let sub = |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
    [
        add(r3, r0), // left
        sub(r3, r0), // right
        add(r3, r1), // bottom
        sub(r3, r1), // top
        r2,          // near (depth 0..1)
        sub(r3, r2), // far
    ]
}

/// Approximate on-screen extent of an AABB, in pixels: the NDC spread of its
/// corners scaled by the viewport. Corners behind the camera make the answer
/// conservative (large), never small: a body near the eye keeps its edges.
pub(crate) fn aabb_screen_px(
    m: &[[f32; 4]; 4],
    lo: [f32; 3],
    hi: [f32; 3],
    vp_width: f32,
    vp_height: f32,
) -> f32 {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for i in 0..8 {
        let p = [
            if i & 1 == 0 { lo[0] } else { hi[0] },
            if i & 2 == 0 { lo[1] } else { hi[1] },
            if i & 4 == 0 { lo[2] } else { hi[2] },
        ];
        let clip: [f32; 4] =
            core::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]);
        if clip[3] <= 1e-6 {
            return f32::INFINITY;
        }
        let ndc = [clip[0] / clip[3], clip[1] / clip[3]];
        for a in 0..2 {
            min[a] = min[a].min(ndc[a]);
            max[a] = max[a].max(ndc[a]);
        }
    }
    (((max[0] - min[0]) * 0.5 * vp_width).abs()).max(((max[1] - min[1]) * 0.5 * vp_height).abs())
}

/// Conservative AABB-vs-frustum test: true when the box is entirely outside
/// at least one plane (definitely invisible); false means "maybe visible".
pub(crate) fn aabb_outside_frustum(planes: &[[f32; 4]; 6], lo: [f32; 3], hi: [f32; 3]) -> bool {
    planes.iter().any(|p| {
        let x = if p[0] >= 0.0 { hi[0] } else { lo[0] };
        let y = if p[1] >= 0.0 { hi[1] } else { lo[1] };
        let z = if p[2] >= 0.0 { hi[2] } else { lo[2] };
        p[0] * x + p[1] * y + p[2] * z + p[3] < 0.0
    })
}

/// What the last `draw` actually submitted, for the frame log.
#[derive(Debug, Default, Clone, Copy)]
pub struct DrawStats {
    pub bodies_drawn: u32,
    pub bodies_culled: u32,
    pub triangle_indices: u64,
    pub edge_indices: u64,
}

/// GPU buffers for a single body, kept alive across frames so a static mesh
/// only ever uploads once. Keyed by `BodySubmission::id` and invalidated when
/// `BodySubmission::revision` advances. wgpu keeps a buffer alive until the
/// GPU is done with it, so replacing or dropping one needs no deferral.
pub(crate) struct CachedMesh {
    /// Object-space AABB of the uploaded positions, for frustum culling.
    /// `None` for an empty mesh.
    pub(crate) bounds: Option<([f32; 3], [f32; 3])>,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    pub(crate) index_count: u32,
    edge_buffer: Option<wgpu::Buffer>,
    /// Segments in `edge_buffer`.
    edge_segments: u32,
    /// The mesh's edge indices, two per segment, for the frame log.
    pub(crate) edge_index_count: u32,
    revision: u64,
}

/// Per-body GPU buffer cache shared by the scene and pick passes. A body's
/// `CachedMesh` only re-uploads when its `BodySubmission::revision` advances;
/// pan, orbit and hover all hit the cache.
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

    /// Make sure `body` has up-to-date GPU buffers. Re-uploads on revision
    /// mismatch; a single hashmap probe on a hit.
    pub(crate) fn ensure_uploaded(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        body: &BodySubmission,
    ) {
        let fresh = self
            .entries
            .get(&body.id)
            .is_some_and(|c| c.revision == body.revision);
        if !fresh {
            self.upload_body(device, queue, body);
        }
    }

    /// Upload (or refresh) a body's GPU buffers, reusing the existing
    /// allocations when their capacity permits.
    fn upload_body(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, body: &BodySubmission) {
        let mesh = body.mesh.as_ref();
        let vertex_count = mesh.positions.len();
        let vertex_at = |i: usize| MeshVertex {
            position: mesh.positions[i],
            normal: mesh.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
            color: mesh.colors.get(i).copied().unwrap_or([1.0, 1.0, 1.0]),
        };
        #[cfg(not(target_arch = "wasm32"))]
        let vertices: Vec<MeshVertex> = if vertex_count >= PARALLEL_PACK_THRESHOLD {
            use rayon::prelude::*;
            (0..vertex_count).into_par_iter().map(vertex_at).collect()
        } else {
            (0..vertex_count).map(vertex_at).collect()
        };
        // A browser page has one thread.
        #[cfg(target_arch = "wasm32")]
        let vertices: Vec<MeshVertex> = (0..vertex_count).map(vertex_at).collect();
        // No triangles and no edges means an implicit triangle list over
        // the positions; edges alone mean a line body with nothing solid.
        let indices: Vec<u32> = if mesh.indices.is_empty() && mesh.edges.is_empty() {
            (0..vertex_count as u32).collect()
        } else {
            mesh.indices.clone()
        };
        let segments: Vec<EdgeSegment> = mesh
            .edges
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|pair| {
                Some(EdgeSegment {
                    a: *mesh.positions.get(pair[0] as usize)?,
                    b: *mesh.positions.get(pair[1] as usize)?,
                })
            })
            .collect();

        let entry = self.entries.entry(body.id).or_insert_with(|| CachedMesh {
            bounds: None,
            vertex_buffer: None,
            index_buffer: None,
            index_count: 0,
            edge_buffer: None,
            edge_segments: 0,
            edge_index_count: 0,
            revision: u64::MAX,
        });
        write_into(
            device,
            queue,
            &mut entry.vertex_buffer,
            bytemuck::cast_slice(&vertices),
            wgpu::BufferUsages::VERTEX,
            "mesh vertices",
        );
        write_into(
            device,
            queue,
            &mut entry.index_buffer,
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::INDEX,
            "mesh indices",
        );
        write_into(
            device,
            queue,
            &mut entry.edge_buffer,
            bytemuck::cast_slice(&segments),
            wgpu::BufferUsages::VERTEX,
            "mesh edges",
        );
        entry.index_count = indices.len() as u32;
        entry.edge_segments = segments.len() as u32;
        entry.edge_index_count = mesh.edges.len() as u32;
        entry.bounds = mesh.positions.iter().fold(None, |acc, p| {
            let (mut lo, mut hi) = acc.unwrap_or((*p, *p));
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
            Some((lo, hi))
        });
        entry.revision = body.revision;
    }

    /// True if at least one cache entry is no longer in the alive set, so a
    /// stable scene skips the sweep.
    pub(crate) fn has_dead_entries(&self, alive: &[Uuid]) -> bool {
        if self.entries.len() > alive.len() {
            return true;
        }
        let alive: std::collections::HashSet<&Uuid> = alive.iter().collect();
        self.entries.keys().any(|id| !alive.contains(id))
    }

    /// Drop every entry whose id is not in `alive`.
    pub(crate) fn retain_only(&mut self, alive: &[Uuid]) {
        let alive: std::collections::HashSet<&Uuid> = alive.iter().collect();
        self.entries.retain(|id, _| alive.contains(id));
    }
}

/// Write `data` into `slot`, growing the buffer to the next power of two
/// when it is too small. Empty data leaves the slot as it is.
fn write_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    slot: &mut Option<wgpu::Buffer>,
    data: &[u8],
    usage: wgpu::BufferUsages,
    label: &str,
) {
    if data.is_empty() {
        return;
    }
    let needed = data.len() as u64;
    if slot.as_ref().is_none_or(|b| b.size() < needed) {
        *slot = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: needed.next_power_of_two().max(1024),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    if let Some(buffer) = slot {
        queue.write_buffer(buffer, 0, data);
    }
}

/// The pipeline variants of the scene pass. They share the vertex format,
/// the bind groups and the layout; they differ in shaders, topology,
/// polygon mode, culling, blending and depth state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeshPipelineMode {
    /// Standard solid mesh pass (triangle list, fill, no culling, depth
    /// write on).
    Solid,
    /// Triangle wireframe overlay drawn on top of the solid pass with depth
    /// bias.
    WireframeTriangles,
    /// The solid pass again, but blended over what is already there and
    /// never writing depth: for bodies drawn with an opacity under 1.0,
    /// after every opaque body and its edges.
    Translucent,
    /// As `Translucent`, its back faces culled: a see-through solid whose
    /// winding is its own (a feature's preview), one layer deep rather than
    /// every face behind it.
    TranslucentFront,
    /// As `Translucent` with no depth test: drawn over everything, for
    /// what the scene would hide and must show.
    OnTop,
    /// Face-boundary outlines as instanced quads. Two-sided, depth-tested
    /// (`LessEqual`, depth write off); a small bias and the clip-space pull
    /// in `vs_edge` limit coplanar z-fight.
    Edges,
}

/// Builds and draws the scene: the shaded solids, their edges, the overlays
/// and the pick pass, out of the shared [`MeshCache`].
pub(crate) struct MeshRenderer {
    solid: wgpu::RenderPipeline,
    wireframe: wgpu::RenderPipeline,
    edges: wgpu::RenderPipeline,
    translucent: wgpu::RenderPipeline,
    translucent_front: wgpu::RenderPipeline,
    on_top: wgpu::RenderPipeline,
    pick: wgpu::RenderPipeline,
    /// Whether `pick` also writes the triangle drawn.
    picks_primitives: bool,
    frame_buffer: wgpu::Buffer,
    frame_bind_group: wgpu::BindGroup,
    draw_layout: wgpu::BindGroupLayout,
    draw_buffer: wgpu::Buffer,
    draw_bind_group: wgpu::BindGroup,
    /// Bytes between two bodies' draw data: the device's uniform offset
    /// alignment.
    draw_stride: u64,
}

/// The pick pass's attachments: the id, and the depth's bits as a colour.
/// Both integer formats, which every backend renders to.
pub(crate) const PICK_ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
pub(crate) const PICK_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
/// The pick pass's third attachment, where the device numbers primitives:
/// the triangle under each texel, plus one.
pub(crate) const PICK_PRIMITIVE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// The scene's shaders with the pick stage that writes the triangle drawn,
/// which needs the device's primitive index.
pub(crate) const PRIMITIVE_PICK_SOURCE: &str = concat!(
    "enable primitive_index;\n",
    include_str!("../shaders/scene.wgsl"),
    include_str!("../shaders/pick_primitive.wgsl"),
);

impl MeshRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        msaa_samples: u32,
        polygon_line: bool,
        picks_primitives: bool,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene shaders"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/scene.wgsl").into()),
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<FrameUniform>() as u64),
                },
                count: None,
            }],
        });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draw uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(size_of::<DrawUniform>() as u64),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&frame_layout), Some(&draw_layout)],
            immediate_size: 0,
        });
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame uniform"),
            size: size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame uniform"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let draw_stride = (size_of::<DrawUniform>() as u64).next_multiple_of(align);
        let (draw_buffer, draw_bind_group) = draw_slots(device, &draw_layout, draw_stride, 64);

        let pipeline = |mode| {
            mesh_pipeline(
                device,
                &layout,
                &shader,
                color_format,
                msaa_samples,
                mode,
                polygon_line,
            )
        };
        let pick = if picks_primitives {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pick shaders"),
                source: wgpu::ShaderSource::Wgsl(PRIMITIVE_PICK_SOURCE.into()),
            });
            pick_pipeline(device, &layout, &shader, true)
        } else {
            pick_pipeline(device, &layout, &shader, false)
        };
        Self {
            solid: pipeline(MeshPipelineMode::Solid),
            wireframe: pipeline(MeshPipelineMode::WireframeTriangles),
            edges: pipeline(MeshPipelineMode::Edges),
            translucent: pipeline(MeshPipelineMode::Translucent),
            translucent_front: pipeline(MeshPipelineMode::TranslucentFront),
            on_top: pipeline(MeshPipelineMode::OnTop),
            pick,
            picks_primitives,
            frame_buffer,
            frame_bind_group,
            draw_layout,
            draw_buffer,
            draw_bind_group,
            draw_stride,
        }
    }

    /// Write the frame's uniform and every body's draw data, one slot per
    /// body in submission order. Both passes read them.
    pub(crate) fn write_uniforms(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cache: &MeshCache,
        frame: &FrameSubmission,
        viewport: [f32; 2],
    ) {
        queue.write_buffer(
            &self.frame_buffer,
            0,
            bytemuck::bytes_of(&FrameUniform::new(frame, viewport)),
        );
        let count = frame.bodies.len().max(1) as u64;
        if self.draw_buffer.size() < count * self.draw_stride {
            let slots = count.next_power_of_two();
            (self.draw_buffer, self.draw_bind_group) =
                draw_slots(device, &self.draw_layout, self.draw_stride, slots);
        }
        let stride = self.draw_stride as usize;
        let mut bytes = vec![0u8; frame.bodies.len() * stride];
        for (i, body) in frame.bodies.iter().enumerate() {
            let line_body = cache.get(&body.id).is_some_and(|c| c.index_count == 0);
            let draw = DrawUniform::new(body, &frame.lighting, line_body);
            bytes[i * stride..i * stride + size_of::<DrawUniform>()]
                .copy_from_slice(bytemuck::bytes_of(&draw));
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.draw_buffer, 0, &bytes);
        }
    }

    fn bind_body(&self, pass: &mut wgpu::RenderPass<'_>, index: usize) {
        let offset = (index as u64 * self.draw_stride) as u32;
        pass.set_bind_group(1, &self.draw_bind_group, &[offset]);
    }

    fn draw_body(&self, pass: &mut wgpu::RenderPass<'_>, cached: &CachedMesh, index: usize) {
        let (Some(vertices), Some(indices)) = (&cached.vertex_buffer, &cached.index_buffer) else {
            return;
        };
        self.bind_body(pass, index);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..cached.index_count, 0, 0..1);
    }

    fn draw_body_edges(&self, pass: &mut wgpu::RenderPass<'_>, cached: &CachedMesh, index: usize) {
        let Some(segments) = &cached.edge_buffer else {
            return;
        };
        self.bind_body(pass, index);
        pass.set_vertex_buffer(0, segments.slice(..));
        pass.draw(0..6, 0..cached.edge_segments);
    }

    /// Draw the scene into `pass`, its viewport already set. Every pass
    /// shares one frustum-culling verdict per body.
    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        cache: &MeshCache,
        frame: &FrameSubmission,
        viewport: [f32; 2],
        grids: &crate::grid::GridPipeline,
    ) -> DrawStats {
        let bodies = &frame.bodies;
        if bodies.is_empty() {
            grids.draw(pass);
            return DrawStats::default();
        }
        let view_proj = frame.view_proj;
        let planes = frustum_planes(&view_proj);
        let mut stats = DrawStats::default();
        // Below this on-screen size a body's edge hairlines are subpixel
        // noise. Override via PRINTCAD_EDGE_MIN_PX (0 disables it).
        let edge_min_px = std::env::var("PRINTCAD_EDGE_MIN_PX")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(24.0);
        let visible: Vec<bool> = bodies
            .iter()
            .map(|body| {
                let outside = cache
                    .get(&body.id)
                    .and_then(|c| c.bounds)
                    .is_some_and(|(lo, hi)| aabb_outside_frustum(&planes, lo, hi));
                if outside {
                    stats.bodies_culled += 1;
                }
                !outside
            })
            .collect();
        let edges_eligible: Vec<bool> = bodies
            .iter()
            .zip(&visible)
            .map(|(body, v)| {
                // Line bodies (sketches) are all edges: never skipped for size.
                *v && cache.get(&body.id).is_none_or(|c| {
                    c.index_count == 0
                        || c.bounds.is_none_or(|(lo, hi)| {
                            edge_min_px <= 0.0
                                || aabb_screen_px(&view_proj, lo, hi, viewport[0], viewport[1])
                                    >= edge_min_px
                        })
                })
            })
            .collect();
        pass.set_bind_group(0, &self.frame_bind_group, &[]);

        // A set of bodies drawn as faces in one pipeline.
        let faces = |pass: &mut wgpu::RenderPass<'_>,
                     stats: &mut DrawStats,
                     pipeline: &wgpu::RenderPipeline,
                     pick: &dyn Fn(&BodySubmission) -> bool| {
            if !bodies.iter().zip(&visible).any(|(b, v)| *v && pick(b)) {
                return;
            }
            pass.set_pipeline(pipeline);
            for (i, body) in bodies.iter().enumerate() {
                if !visible[i] || !pick(body) {
                    continue;
                }
                let Some(cached) = cache.get(&body.id).filter(|c| c.index_count > 0) else {
                    continue;
                };
                stats.bodies_drawn += 1;
                stats.triangle_indices += u64::from(cached.index_count);
                self.draw_body(pass, cached, i);
            }
        };

        // Solid pass first; edges, wireframes, translucent and on-top
        // bodies follow, each in its own pipeline.
        let opaque = |b: &BodySubmission| !b.is_wireframe && !b.on_top && b.opacity >= 1.0;
        faces(pass, &mut stats, &self.solid, &opaque);

        // The frame says whether edges draw (the draw style); the
        // experiment hook PRINTCAD_NO_EDGES=1 forces them off.
        let force_off = !frame.draw_edges || std::env::var_os("PRINTCAD_NO_EDGES").is_some();
        // A line body (a sketch, a guide) is nothing but its edges and draws
        // whatever the style; a solid's boundary edges follow it.
        let draws_edges = |cached: &CachedMesh| {
            cached.edge_segments > 0 && (cached.index_count == 0 || !force_off)
        };
        // A see-through body drawn one layer deep has its edges drawn after
        // its faces, over them.
        let edges_later = |b: &BodySubmission| b.front_only && b.opacity < 1.0;
        let edges_now = |b: &BodySubmission| !b.is_wireframe && !b.on_top && !edges_later(b);
        if bodies.iter().enumerate().any(|(i, b)| {
            edges_eligible[i] && edges_now(b) && cache.get(&b.id).is_some_and(draws_edges)
        }) {
            pass.set_pipeline(&self.edges);
            // Solids' boundary edges first, then line bodies: a sketch drawn
            // along a solid's own edges ties with them in depth, and the
            // later draw wins the tie, so the sketch shows.
            for lines in [false, true] {
                for (i, body) in bodies.iter().enumerate() {
                    if !edges_eligible[i] || !edges_now(body) {
                        continue;
                    }
                    let Some(cached) = cache
                        .get(&body.id)
                        .filter(|c| draws_edges(c) && (c.index_count == 0) == lines)
                    else {
                        continue;
                    };
                    stats.edge_indices += u64::from(cached.edge_index_count);
                    self.draw_body_edges(pass, cached, i);
                }
            }
        }

        faces(pass, &mut stats, &self.wireframe, &|b| b.is_wireframe);

        // Grids show through translucent bodies while staying behind their
        // tint. Their pipeline binds its own uniforms at group zero.
        grids.draw(pass);
        pass.set_bind_group(0, &self.frame_bind_group, &[]);

        // Translucent bodies last, over everything opaque and its edges:
        // blended, depth-tested, never writing depth. Those drawn one layer
        // deep come after, in their own pipeline, then their edges.
        let translucent = |b: &BodySubmission| !b.is_wireframe && !b.on_top && b.opacity < 1.0;
        faces(pass, &mut stats, &self.translucent, &|b| {
            translucent(b) && !b.front_only
        });
        faces(pass, &mut stats, &self.translucent_front, &|b| {
            translucent(b) && b.front_only
        });
        let late_edges = |b: &BodySubmission| translucent(b) && edges_later(b);
        if bodies.iter().enumerate().any(|(i, b)| {
            edges_eligible[i] && late_edges(b) && cache.get(&b.id).is_some_and(draws_edges)
        }) {
            pass.set_pipeline(&self.edges);
            for (i, body) in bodies.iter().enumerate() {
                if !edges_eligible[i] || !late_edges(body) {
                    continue;
                }
                let Some(cached) = cache.get(&body.id).filter(|c| draws_edges(c)) else {
                    continue;
                };
                stats.edge_indices += u64::from(cached.edge_index_count);
                self.draw_body_edges(pass, cached, i);
            }
        }

        // Over everything, depth or not.
        faces(pass, &mut stats, &self.on_top, &|b| b.on_top);
        stats
    }

    /// Whether the pick pass writes the triangle drawn, into a third
    /// attachment of [`PICK_PRIMITIVE_FORMAT`].
    pub(crate) fn picks_primitives(&self) -> bool {
        self.picks_primitives
    }

    /// Draw every pickable body's id into the pick pass. Only what the host
    /// marked pickable: a selection overlay sits on or above the face it
    /// marks and would take the pick from it.
    pub(crate) fn draw_pick(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        cache: &MeshCache,
        frame: &FrameSubmission,
    ) {
        pass.set_pipeline(&self.pick);
        pass.set_bind_group(0, &self.frame_bind_group, &[]);
        for (i, body) in frame.bodies.iter().enumerate() {
            if !body.pickable {
                continue;
            }
            if let Some(cached) = cache.get(&body.id).filter(|c| c.index_count > 0) {
                self.draw_body(pass, cached, i);
            }
        }
    }
}

/// A uniform buffer of `slots` draw slots and its bind group.
fn draw_slots(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    stride: u64,
    slots: u64,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("draw uniforms"),
        size: stride * slots,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("draw uniforms"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: wgpu::BufferSize::new(size_of::<DrawUniform>() as u64),
            }),
        }],
    });
    (buffer, bind_group)
}

const MESH_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
const EDGE_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
const PICK_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];

fn mesh_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    color_format: wgpu::TextureFormat,
    msaa_samples: u32,
    mode: MeshPipelineMode,
    polygon_line: bool,
) -> wgpu::RenderPipeline {
    use MeshPipelineMode as M;
    let edges = mode == M::Edges;
    let vertex_layout = if edges {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<EdgeSegment>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &EDGE_ATTRIBUTES,
        }
    } else {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &MESH_ATTRIBUTES,
        }
    };
    // Wireframes and edges pull slightly toward the camera so `LessEqual`
    // passes against coplanar solid depth. Too much bias and edges show
    // through what should hide them.
    let bias = match mode {
        M::WireframeTriangles => wgpu::DepthBiasState {
            constant: 1,
            slope_scale: 1.0,
            clamp: 0.0,
        },
        M::Edges => wgpu::DepthBiasState {
            constant: -1,
            slope_scale: 0.0,
            clamp: 0.0,
        },
        _ => wgpu::DepthBiasState::default(),
    };
    // Back-face culling is off for solids: STEP files from mainstream CAD
    // exporters regularly contain faces wound inward, and culling them
    // makes holes. The fragment shader shades both sides. A see-through
    // solid of consistent winding shows one layer, and a wireframe its
    // front.
    let cull_mode = match mode {
        M::WireframeTriangles | M::TranslucentFront => Some(wgpu::Face::Back),
        _ => None,
    };
    let polygon_mode = if mode == M::WireframeTriangles && polygon_line {
        wgpu::PolygonMode::Line
    } else {
        wgpu::PolygonMode::Fill
    };
    let (depth_write, depth_compare) = match mode {
        M::Solid => (true, wgpu::CompareFunction::Less),
        M::OnTop => (false, wgpu::CompareFunction::Always),
        M::Translucent | M::TranslucentFront | M::WireframeTriangles | M::Edges => {
            (false, wgpu::CompareFunction::LessEqual)
        }
    };
    let blend = matches!(mode, M::Translucent | M::TranslucentFront | M::OnTop).then_some(
        wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::Zero,
                operation: wgpu::BlendOperation::Add,
            },
        },
    );
    let (vs, fs) = if edges {
        ("vs_edge", "fs_edge")
    } else {
        ("vs_mesh", "fs_mesh")
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&format!("scene {mode:?}")),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers: &[Some(vertex_layout)],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode,
            unclipped_depth: false,
            polygon_mode,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(depth_compare),
            stencil: wgpu::StencilState::default(),
            bias,
        }),
        multisample: wgpu::MultisampleState {
            count: msaa_samples,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn pick_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    primitives: bool,
) -> wgpu::RenderPipeline {
    let target = |format| {
        Some(wgpu::ColorTargetState {
            format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    let targets = [
        target(PICK_ID_FORMAT),
        target(PICK_DEPTH_FORMAT),
        target(PICK_PRIMITIVE_FORMAT),
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pick"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_pick"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<MeshVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &PICK_ATTRIBUTES,
            })],
        },
        // Both sides rasterize, as in the scene pass: a face wound inward
        // is drawn all the same, so it must pick all the same.
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if primitives {
                "fs_pick_primitive"
            } else {
                "fs_pick"
            }),
            compilation_options: Default::default(),
            targets: if primitives { &targets } else { &targets[..2] },
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_are_laid_out_as_the_shader_reads_them() {
        // `Frame` in scene.wgsl: a mat4, five vec4s and three lights of two.
        assert_eq!(size_of::<FrameUniform>(), 64 + 16 * 5 + 32 * 3);
        assert_eq!(size_of::<FrameUniform>() % 16, 0);
        assert_eq!(size_of::<DrawUniform>(), 48);
        assert_eq!(size_of::<MeshVertex>(), 36);
        assert_eq!(size_of::<EdgeSegment>(), 24);
    }

    #[test]
    fn a_uuid_survives_the_round_trip_through_the_id_target() {
        let id = Uuid::new_v4();
        assert_eq!(u32s_to_uuid(uuid_to_u32s(id)), id);
    }

    #[test]
    fn a_box_behind_the_camera_is_culled_and_one_ahead_is_not() {
        // Identity: the visible box is x, y in -1..1 and depth 0..1.
        let m = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let planes = frustum_planes(&m);
        assert!(!aabb_outside_frustum(&planes, [-0.5; 3], [0.5; 3]));
        assert!(aabb_outside_frustum(
            &planes,
            [2.0, 2.0, 0.2],
            [3.0, 3.0, 0.4]
        ));
        assert!(aabb_outside_frustum(
            &planes,
            [-0.5, -0.5, -2.0],
            [0.5, 0.5, -1.0]
        ));
        assert!((aabb_screen_px(&m, [-0.5; 3], [0.5; 3], 200.0, 100.0) - 100.0).abs() < 1e-4);
    }

    #[test]
    fn a_line_body_draws_its_edges_in_its_own_colour() {
        let body = BodySubmission {
            id: Uuid::new_v4(),
            revision: 0,
            mesh: std::sync::Arc::new(kernel_api::TriMesh::default()),
            color: [0.2, 0.4, 0.6],
            opacity: 1.0,
            highlight: HighlightState::None,
            is_wireframe: false,
            pickable: true,
            on_top: false,
            edge_color: None,
            front_only: false,
        };
        let lighting = LightingData::default();
        let line = DrawUniform::new(&body, &lighting, true);
        assert_eq!(&line.edge_color[..3], &[0.2, 0.4, 0.6]);
        let solid = DrawUniform::new(&body, &lighting, false);
        assert_eq!(&solid.edge_color[..3], &lighting.edge_line_color);
        assert_eq!(solid.object_id, uuid_to_u32s(body.id));
    }
}
