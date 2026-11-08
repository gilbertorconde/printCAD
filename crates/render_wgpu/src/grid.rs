//! Plane patches with three decades of antialiased lines and origin axes.
use crate::mesh::{DEPTH_FORMAT, NO_CLIP, NO_WEDGE};
use crate::{FrameSubmission, GridSubmission};
use egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GridUniform {
    view_proj: [[f32; 4]; 4],
    camera_step: [f32; 4],
    clip_plane: [f32; 4],
    anchor_radius: [f32; 4],
    u_minor: [f32; 4],
    v_major: [f32; 4],
    center_axis: [f32; 4],
    forward_near: [f32; 4],
    color_far: [f32; 4],
    u_axis_color: [f32; 4],
    v_axis_color: [f32; 4],
    dash: [f32; 4],
    clip_wedge: [f32; 4],
}
impl GridUniform {
    /// Measure the plane from a nearby node of the coarsest lattice so
    /// shader coordinates stay small. Axis offsets keep the true origin.
    fn new(grid: &GridSubmission, frame: &FrameSubmission) -> Self {
        let dot = |axis: [f32; 3]| -> f64 {
            (0..3)
                .map(|i| {
                    (f64::from(grid.center[i]) - f64::from(grid.origin[i])) * f64::from(axis[i])
                })
                .sum()
        };
        let coarsest = f64::from(grid.step) * 100.0;
        let node = [
            (dot(grid.u) / coarsest).round() * coarsest,
            (dot(grid.v) / coarsest).round() * coarsest,
        ];
        let anchor = std::array::from_fn(|i| {
            (f64::from(grid.origin[i])
                + f64::from(grid.u[i]) * node[0]
                + f64::from(grid.v[i]) * node[1]) as f32
        });
        let with = |xyz: [f32; 3], w: f32| [xyz[0], xyz[1], xyz[2], w];
        Self {
            view_proj: frame.view_proj,
            camera_step: with(frame.camera_pos, grid.step),
            clip_plane: frame.clip_plane.unwrap_or(NO_CLIP),
            anchor_radius: with(anchor, grid.radius),
            u_minor: with(grid.u, grid.minor_alpha),
            v_major: with(grid.v, grid.major_alpha),
            center_axis: with(grid.center, grid.axis_alpha),
            forward_near: with(grid.forward, grid.depth_range[0]),
            color_far: with(grid.color, grid.depth_range[1]),
            u_axis_color: with(grid.u_axis_color, -node[0] as f32),
            v_axis_color: with(grid.v_axis_color, -node[1] as f32),
            dash: [grid.dash[0], grid.dash[1], 0.0, 0.0],
            clip_wedge: frame.clip_wedge.unwrap_or(NO_WEDGE),
        }
    }
}
pub(crate) struct GridPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    groups: Vec<wgpu::BindGroup>,
}
impl GridPipeline {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/grid.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_grid"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_grid"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            layout,
            groups: Vec::new(),
        }
    }
    pub(crate) fn prepare(&mut self, device: &wgpu::Device, frame: &FrameSubmission) {
        self.groups = frame
            .grids
            .iter()
            .map(|grid| {
                assert!(
                    grid.step > 0.0 && grid.radius > 0.0,
                    "grid spacing and radius must be positive"
                );
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("grid"),
                    contents: bytemuck::bytes_of(&GridUniform::new(grid, frame)),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("grid"),
                    layout: &self.layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                })
            })
            .collect();
    }
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.groups.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        for group in &self.groups {
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn grid(center: [f32; 3], step: f32) -> GridSubmission {
        GridSubmission {
            origin: [0.0; 3],
            u: [1.0, 0.0, 0.0],
            v: [0.0, 1.0, 0.0],
            step,
            center,
            radius: 100.0,
            forward: [0.0, 0.0, -1.0],
            depth_range: [0.1, 1000.0],
            color: [1.0; 3],
            minor_alpha: 0.1,
            major_alpha: 0.3,
            u_axis_color: [1.0, 0.0, 0.0],
            v_axis_color: [0.0, 1.0, 0.0],
            axis_alpha: 0.8,
            dash: [0.0; 2],
        }
    }

    #[test]
    fn the_plane_is_measured_from_a_node_next_to_the_patch() {
        let uniform = GridUniform::new(
            &grid([12_345.0, -6_789.0, 0.0], 1.0),
            &FrameSubmission::default(),
        );
        assert_eq!(uniform.anchor_radius[..3], [12_300.0, -6_800.0, 0.0]);
        assert_eq!(uniform.u_axis_color[3], -12_300.0);
        assert_eq!(uniform.v_axis_color[3], 6_800.0);
    }

    #[test]
    fn a_grid_in_another_plane_keeps_its_node_in_that_plane() {
        let mut g = grid([0.0, 260.0, -133.0], 0.5);
        g.origin = [0.0, 0.0, 7.0];
        g.u = [0.0, 1.0, 0.0];
        g.v = [0.0, 0.0, 1.0];
        let uniform = GridUniform::new(&g, &FrameSubmission::default());
        assert_eq!(uniform.anchor_radius[..3], [0.0, 250.0, -143.0]);
    }

    #[test]
    fn every_grid_field_invalidates_the_cached_scene() {
        use crate::core::scene_fingerprint;

        let empty = FrameSubmission::default();
        let frame = FrameSubmission {
            grids: vec![grid([0.0; 3], 1.0)],
            ..FrameSubmission::default()
        };
        let fingerprint = scene_fingerprint(&frame);
        assert_ne!(fingerprint, scene_fingerprint(&empty));
        assert_eq!(fingerprint, scene_fingerprint(&frame));
        let vectors: [fn(&mut GridSubmission) -> &mut [f32]; 9] = [
            |g| &mut g.origin,
            |g| &mut g.u,
            |g| &mut g.v,
            |g| &mut g.center,
            |g| &mut g.forward,
            |g| &mut g.color,
            |g| &mut g.u_axis_color,
            |g| &mut g.v_axis_color,
            |g| &mut g.depth_range,
        ];
        let mut checked = 0;
        for vector in vectors {
            let mut sample = frame.grids[0];
            for index in 0..vector(&mut sample).len() {
                let mut changed = FrameSubmission {
                    grids: frame.grids.clone(),
                    ..FrameSubmission::default()
                };
                vector(&mut changed.grids[0])[index] += 1.0;
                assert_ne!(fingerprint, scene_fingerprint(&changed));
                checked += 1;
            }
        }
        let scalars: [fn(&mut GridSubmission) -> &mut f32; 5] = [
            |g| &mut g.step,
            |g| &mut g.radius,
            |g| &mut g.minor_alpha,
            |g| &mut g.major_alpha,
            |g| &mut g.axis_alpha,
        ];
        for scalar in scalars {
            let mut changed = FrameSubmission {
                grids: frame.grids.clone(),
                ..FrameSubmission::default()
            };
            *scalar(&mut changed.grids[0]) += 1.0;
            assert_ne!(fingerprint, scene_fingerprint(&changed));
            checked += 1;
        }
        assert_eq!(checked, 31);
        let mut two = FrameSubmission {
            grids: frame.grids.clone(),
            ..FrameSubmission::default()
        };
        two.grids.push(frame.grids[0]);
        assert_ne!(fingerprint, scene_fingerprint(&two));
    }

    #[test]
    fn grid_shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("../shaders/grid.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
