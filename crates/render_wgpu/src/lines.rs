//! Screen-space line segments as one instanced draw inside the UI pass: an
//! egui paint callback ([`lines_shape`]), so the segments stack with the
//! shapes painted before and after them exactly as painted shapes would,
//! while the GPU, not egui's tessellator, makes each segment's quad.

use std::sync::{Arc, OnceLock};

use egui_wgpu::wgpu;

/// One segment, in physical pixels from the corner of the shape's rect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenLine {
    pub start: [f32; 2],
    pub end: [f32; 2],
    /// Colour as egui's colours are (not linear), and opacity.
    pub color: [f32; 4],
    pub thickness: f32,
    /// `(dash, gap)` in pixels, `None` solid.
    pub dash: Option<(f32, f32)>,
    /// How far into the dash pattern the segment starts, in pixels.
    pub dash_offset: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    ends: [f32; 4],
    color: [f32; 4],
    style: [f32; 4],
    area: [f32; 2],
}

const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x2
];

/// The pipeline and the buffer the frame's segments go into, kept in the
/// egui renderer's callback resources.
pub(crate) struct LinePass {
    pipeline: wgpu::RenderPipeline,
    buffer: Arc<wgpu::Buffer>,
    /// Bytes of `buffer` this frame's callbacks have taken.
    used: u64,
}

impl LinePass {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("screen lines"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/lines.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("screen lines"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("screen lines"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_line"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &ATTRIBUTES,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(if format.is_srgb() {
                    "fs_line_linear"
                } else {
                    "fs_line"
                }),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: premultiplied,
                        alpha: premultiplied,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            buffer: Arc::new(buffer(device, 64 * 1024)),
            used: 0,
        }
    }
}

fn buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screen lines"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A shape drawing `lines` over `rect` (in points), clipped as the painter
/// it is added to clips. The segments' pixels count from `rect`'s corner.
pub fn lines_shape(rect: egui::Rect, lines: Vec<ScreenLine>) -> egui::Shape {
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(
        rect,
        LinesCallback {
            rect,
            lines,
            written: OnceLock::new(),
        },
    ))
}

struct LinesCallback {
    rect: egui::Rect,
    lines: Vec<ScreenLine>,
    /// The buffer the segments went into, where, and how many.
    written: OnceLock<(Arc<wgpu::Buffer>, u64, u32)>,
}

impl egui_wgpu::CallbackTrait for LinesCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(pass) = resources.get_mut::<LinePass>() else {
            return Vec::new();
        };
        if self.lines.is_empty() {
            return Vec::new();
        }
        // The area as egui sets the viewport for it, rounded to pixels;
        // the segments' pixels count from the unrounded corner.
        let ppp = screen.pixels_per_point;
        let (min, max) = (self.rect.min * ppp, self.rect.max * ppp);
        let corner = egui::pos2(min.x.round(), min.y.round());
        let area = [max.x.round() - corner.x, max.y.round() - corner.y];
        let shift = [min.x - corner.x, min.y - corner.y];
        let instances: Vec<Instance> = self
            .lines
            .iter()
            .map(|line| {
                let (dash, gap) = line.dash.unwrap_or((0.0, 0.0));
                Instance {
                    ends: [
                        line.start[0] + shift[0],
                        line.start[1] + shift[1],
                        line.end[0] + shift[0],
                        line.end[1] + shift[1],
                    ],
                    color: line.color,
                    style: [line.thickness, dash, gap, line.dash_offset],
                    area,
                }
            })
            .collect();
        let bytes: &[u8] = bytemuck::cast_slice(&instances);
        let size = bytes.len() as u64;
        if pass.used + size > pass.buffer.size() {
            // A larger buffer from its start: what this frame wrote before
            // keeps the old one, held by its callbacks.
            pass.buffer = Arc::new(buffer(device, (size * 2).max(pass.buffer.size() * 2)));
            pass.used = 0;
        }
        queue.write_buffer(&pass.buffer, pass.used, bytes);
        let _ = self
            .written
            .set((pass.buffer.clone(), pass.used, instances.len() as u32));
        pass.used += size;
        Vec::new()
    }

    fn finish_prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Every callback of the frame has written: the next frame's start
        // at the buffer's beginning.
        if let Some(pass) = resources.get_mut::<LinePass>() {
            pass.used = 0;
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let (Some(pass), Some((buffer, offset, count))) =
            (resources.get::<LinePass>(), self.written.get())
        else {
            return;
        };
        let end = offset + u64::from(*count) * size_of::<Instance>() as u64;
        render_pass.set_pipeline(&pass.pipeline);
        render_pass.set_vertex_buffer(0, buffer.slice(*offset..end));
        render_pass.draw(0..6, 0..*count);
    }
}
