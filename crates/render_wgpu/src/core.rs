//! The device, the window's surface and the frame: the scene pass into a
//! cached image, the pick pass and the picture copy when asked for, and the
//! window pass that lays the scene under the UI.

use std::sync::Arc;

use egui_wgpu::wgpu;
use tracing::{debug, info, warn};
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

use crate::mesh::{
    DEPTH_FORMAT, DrawStats, MeshCache, MeshRenderer, PICK_DEPTH_FORMAT, PICK_ID_FORMAT,
    PICK_PRIMITIVE_FORMAT, Pane,
};
use crate::readback::{self, CaptureRequest, Done, PickRequest, Polled, Readback};
use crate::{
    CaptureKind, CapturedImage, FrameSubmission, PickResult, RenderError, RenderSettings,
    ViewportRect,
};

/// Clear colour of the 3D scene, RGBA in 0.0 to 1.0.
const VIEWPORT_BACKGROUND: wgpu::Color = wgpu::Color::TRANSPARENT;

/// The textures sized to the window.
struct Targets {
    /// The rendered scene, single-sample, kept between frames. The scene
    /// pass writes it only when the scene changed; every frame lays it
    /// under the UI.
    scene: wgpu::Texture,
    scene_view: wgpu::TextureView,
    /// The multisampled colour the scene pass resolves into `scene`;
    /// `None` without MSAA.
    msaa_view: Option<wgpu::TextureView>,
    depth_view: wgpu::TextureView,
    /// The blit's view of `scene`.
    blit_group: wgpu::BindGroup,
    /// The pick pass's id and depth, single-sample, read back in part; the
    /// depth as a colour, since a depth texture cannot be copied in part.
    pick_ids: wgpu::Texture,
    pick_ids_view: wgpu::TextureView,
    pick_depth: wgpu::Texture,
    pick_depth_view: wgpu::TextureView,
    /// The triangle under each texel, where the device numbers primitives.
    pick_primitives: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The pick pass's depth test.
    pick_depth_test_view: wgpu::TextureView,
}

pub(crate) struct RendererCore {
    egui_renderer: egui_wgpu::Renderer,
    mesh_renderer: MeshRenderer,
    grid_renderer: crate::grid::GridPipeline,
    /// Per-body GPU buffers shared by the scene and pick passes. Bodies only
    /// re-upload when their `BodySubmission::revision` advances.
    mesh_cache: MeshCache,
    targets: Targets,
    blit_pipeline: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    /// egui texture uploads and frees handed over but not yet applied: a
    /// frame that finds the surface out of date leaves them for the next.
    pending_textures: egui::TexturesDelta,
    /// Copies submitted and waiting to map.
    readbacks: Vec<Readback>,
    /// Pick request from the app; consumed by the next drawn frame.
    pending_pick: Option<(u32, u32)>,
    last_pick_result: PickResult,
    /// The pictures wanted: the next frame copies each out.
    pending_captures: Vec<CaptureKind>,
    captured: Vec<CapturedImage>,
    /// Fingerprint of the scene last drawn into the scene image; `None`
    /// forces a redraw (first frame, resize).
    last_scene_fingerprint: Option<u64>,
    scene_redrawn_last_frame: bool,
    last_draw_stats: DrawStats,
    msaa_samples: u32,
    /// The scene image's format: the window's, in sRGB.
    scene_format: wgpu::TextureFormat,
    gpu_name: String,
    /// The graphics API wgpu drives the GPU through.
    graphics_api: &'static str,
    available_gpus: Vec<String>,
    config: wgpu::SurfaceConfiguration,
    surface: wgpu::Surface<'static>,
    queue: wgpu::Queue,
    device: wgpu::Device,
    adapter: wgpu::Adapter,
    _instance: wgpu::Instance,
}

impl RendererCore {
    pub(crate) async fn new(
        window: &Window,
        display: OwnedDisplayHandle,
        (width, height): (u32, u32),
        settings: RenderSettings,
    ) -> Result<Self, RenderError> {
        let flags = if settings.prefer_validation_layers {
            wgpu::InstanceFlags::debugging()
        } else {
            wgpu::InstanceFlags::empty()
        }
        .with_env();
        // WGPU_BACKEND picks one (vulkan, metal, dx12, gl) for a test.
        let backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY);
        let descriptor = wgpu::InstanceDescriptor {
            backends,
            flags,
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            // Only the GL backend reads it, and needs it to draw on the window.
            display: Some(Box::new(display)),
        };
        let instance = wgpu::Instance::new(descriptor);
        // SAFETY: the handles are the live window's, which outlives the
        // surface: the app drops the renderer before the window (`Gfx`).
        let surface = unsafe {
            let target = wgpu::SurfaceTargetUnsafe::from_display_and_window(window, window)
                .map_err(|e| RenderError::UnsupportedPlatform(e.to_string()))?;
            instance.create_surface_unsafe(target)
        }
        .map_err(|e| RenderError::UnsupportedPlatform(e.to_string()))?;

        let adapters: Vec<wgpu::Adapter> = instance
            .enumerate_adapters(backends)
            .await
            .into_iter()
            .filter(|a| a.is_surface_supported(&surface))
            .collect();
        let available_gpus: Vec<String> = adapters.iter().map(|a| a.get_info().name).collect();
        let preferred = settings.preferred_gpu.as_deref().and_then(|pref| {
            let pref = pref.to_lowercase();
            let found = adapters
                .iter()
                .find(|a| a.get_info().name.to_lowercase().contains(&pref));
            if found.is_none() {
                warn!("Preferred GPU '{pref}' not found, choosing automatically");
            }
            found.cloned()
        });
        let adapter = match preferred {
            Some(adapter) => adapter,
            None => instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: Some(&surface),
                    apply_limit_buckets: false,
                })
                .await
                .map_err(|e| RenderError::Initialization(format!("no suitable GPU: {e}")))?,
        };
        let info = adapter.get_info();
        info!(
            "GPU: {} ({:?}, {:?}, driver {} {})",
            info.name, info.backend, info.device_type, info.driver, info.driver_info
        );

        // A wireframe drawn as lines needs this; without it, it draws filled.
        let polygon_line = adapter
            .features()
            .contains(wgpu::Features::POLYGON_MODE_LINE);
        // Sample counts beyond the portable ones (8x MSAA) are the
        // adapter's own and usable only with this feature turned on.
        let adapter_formats = adapter
            .features()
            .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
        // The pick pass writes the triangle under the cursor where the
        // device numbers primitives; elsewhere the host finds it from the
        // picked point.
        let picks_primitives = adapter.features().contains(wgpu::Features::PRIMITIVE_INDEX);
        let mut required_features = wgpu::Features::empty();
        required_features.set(wgpu::Features::POLYGON_MODE_LINE, polygon_line);
        required_features.set(wgpu::Features::PRIMITIVE_INDEX, picks_primitives);
        required_features.set(
            wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            adapter_formats,
        );
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("printCAD"),
                required_features,
                // Everything the GPU offers: a big assembly's buffers outgrow
                // the portable defaults.
                required_limits: adapter.limits(),
                experimental_features: Default::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: Default::default(),
            })
            .await
            .map_err(|e| RenderError::Initialization(format!("GPU device: {e}")))?;
        device.on_uncaptured_error(Arc::new(|error| {
            // The description holds the detail; the summary alone says only
            // "Validation Error".
            let text = match &error {
                wgpu::Error::Validation { description, .. }
                | wgpu::Error::Internal { description, .. } => description.clone(),
                other => format!("{other:?}"),
            };
            tracing::error!(target: "printcad.gpu", "{text}");
        }));

        let caps = surface.get_capabilities(&adapter);
        // The window takes a plain format, as egui blends for; the scene is
        // drawn in sRGB and encoded on its way onto the window.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm))
            .or_else(|| {
                caps.formats
                    .iter()
                    .copied()
                    .find(|f| matches!(f, wgpu::TextureFormat::Rgba8Unorm))
            })
            .or_else(|| caps.formats.iter().copied().find(|f| !f.is_srgb()))
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| RenderError::Initialization("the surface offers no format".into()))?;
        let scene_format = format.add_srgb_suffix();
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: Vec::new(),
        };
        surface.configure(&device, &config);
        info!("Surface: {format:?}, {present_mode:?}");

        // MSAA, clamped to what both the colour and depth formats support
        // on this device: the adapter's counts with its format features on,
        // the portable ones otherwise.
        let supported = |n: u32| {
            [scene_format, DEPTH_FORMAT].iter().all(|f| {
                let features = if adapter_formats {
                    adapter.get_texture_format_features(*f)
                } else {
                    f.guaranteed_format_features(device.features())
                };
                features.flags.sample_count_supported(n)
            })
        };
        let requested = u32::from(settings.msaa_samples.max(1));
        let msaa_samples = [requested, 8, 4, 2, 1]
            .into_iter()
            .filter(|n| *n <= requested)
            .find(|n| supported(*n))
            .unwrap_or(1);
        if msaa_samples != requested {
            info!("Requested MSAA {requested}x not supported, using {msaa_samples}x");
        }
        info!("Using MSAA: {msaa_samples}x");

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/blit.wgsl").into()),
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let blit_pipeline = blit_pipeline(&device, &blit_layout, &blit_shader, format);
        info!(
            "Pick pass: {}",
            if picks_primitives {
                "triangles from the GPU"
            } else {
                "triangles found from the picked point"
            }
        );
        let targets = create_targets(
            &device,
            &blit_layout,
            &config,
            scene_format,
            msaa_samples,
            picks_primitives,
        );
        let mesh_renderer = MeshRenderer::new(
            &device,
            scene_format,
            msaa_samples,
            polygon_line,
            picks_primitives,
        );
        let grid_renderer = crate::grid::GridPipeline::new(&device, scene_format, msaa_samples);
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            format,
            egui_wgpu::RendererOptions {
                msaa_samples: 1,
                ..Default::default()
            },
        );

        Ok(Self {
            egui_renderer,
            mesh_renderer,
            grid_renderer,
            mesh_cache: MeshCache::new(),
            targets,
            blit_pipeline,
            blit_layout,
            pending_textures: egui::TexturesDelta::default(),
            readbacks: Vec::new(),
            pending_pick: None,
            last_pick_result: PickResult::default(),
            pending_captures: Vec::new(),
            captured: Vec::new(),
            last_scene_fingerprint: None,
            scene_redrawn_last_frame: false,
            last_draw_stats: DrawStats::default(),
            msaa_samples,
            scene_format,
            graphics_api: api_name(info.backend),
            gpu_name: info.name,
            available_gpus,
            config,
            surface,
            queue,
            device,
            adapter,
            _instance: instance,
        })
    }

    /// Resize the surface and everything sized to it.
    pub(crate) fn resize(&mut self, (width, height): (u32, u32)) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
        self.targets = create_targets(
            &self.device,
            &self.blit_layout,
            &self.config,
            self.scene_format,
            self.msaa_samples,
            self.mesh_renderer.picks_primitives(),
        );
        self.last_scene_fingerprint = None;
    }

    pub(crate) fn surface_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub(crate) fn gpu_name(&self) -> &str {
        &self.gpu_name
    }

    pub(crate) fn graphics_api(&self) -> &'static str {
        self.graphics_api
    }

    pub(crate) fn available_gpus(&self) -> &[String] {
        &self.available_gpus
    }

    pub(crate) fn last_draw_stats(&self) -> DrawStats {
        self.last_draw_stats
    }

    pub(crate) fn scene_redrawn_last_frame(&self) -> bool {
        self.scene_redrawn_last_frame
    }

    pub(crate) fn request_pick(&mut self, x: u32, y: u32) {
        self.pending_pick = Some((x, y));
    }

    pub(crate) fn request_capture(&mut self, kind: CaptureKind) {
        if !self.pending_captures.contains(&kind) {
            self.pending_captures.push(kind);
        }
    }

    pub(crate) fn take_captures(&mut self) -> Vec<CapturedImage> {
        std::mem::take(&mut self.captured)
    }

    /// Whether a picture is asked for or on its way.
    pub(crate) fn capture_pending(&self) -> bool {
        !self.pending_captures.is_empty() || self.readbacks.iter().any(Readback::is_capture)
    }

    pub(crate) fn last_pick_result(&self) -> PickResult {
        self.last_pick_result.clone()
    }

    /// Queues egui texture changes for the next frame that gets as far as
    /// applying them.
    pub(crate) fn take_textures(&mut self, deltas: egui::TexturesDelta) {
        self.pending_textures.append(deltas);
    }

    /// Collect the readbacks whose buffers have mapped.
    fn collect_readbacks(&mut self) {
        if let Err(e) = self.device.poll(wgpu::PollType::Poll) {
            warn!("GPU poll failed: {e}");
        }
        let mut results = Vec::new();
        self.readbacks.retain(|readback| match readback.poll() {
            Polled::Waiting => true,
            Polled::Failed => {
                warn!("A GPU readback failed to map");
                false
            }
            Polled::Done(done) => {
                results.push(done);
                false
            }
        });
        // In submission order, so the latest pick wins.
        for done in results {
            match done {
                Done::Pick(result) => {
                    if result.body_id.is_some() {
                        debug!("GPU pick hit: {:?}", result.body_id);
                    }
                    self.last_pick_result = result;
                }
                Done::Capture(image) => self.captured.push(image),
            }
        }
    }

    pub(crate) fn draw_frame(&mut self, frame: &FrameSubmission) -> Result<(), RenderError> {
        self.collect_readbacks();

        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(_) | wgpu::CurrentSurfaceTexture::Outdated => {
                return Err(RenderError::SwapchainOutOfDate);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Err(RenderError::SurfaceTooSmall);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err(RenderError::Surface("the window's surface was lost".into()));
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderError::Surface(
                    "acquiring the window's image failed validation".into(),
                ));
            }
        };
        let (width, height) = self.surface_size();
        let full = ViewportRect {
            x: 0,
            y: 0,
            width,
            height,
        };
        let viewport = clamp_rect(frame.viewport_rect.unwrap_or(full), width, height);
        let viewport_px = [viewport.width as f32, viewport.height as f32];

        // Every body has fresh GPU buffers before either pass draws: both
        // draw out of the same buffers, uploaded once per revision.
        for body in &frame.bodies {
            self.mesh_cache
                .ensure_uploaded(&self.device, &self.queue, body);
        }

        // The scene is redrawn only when something in it changed; UI-only
        // frames (hover, panels, typing) reuse the cached scene image.
        let fingerprint = scene_fingerprint(frame);
        let scene_dirty = self.last_scene_fingerprint != Some(fingerprint);
        if scene_dirty {
            self.grid_renderer.prepare(&self.device, frame);
        }
        self.scene_redrawn_last_frame = scene_dirty;
        let pick = self
            .pending_pick
            .take()
            .filter(|(x, y)| *x < width && *y < height);
        if scene_dirty || pick.is_some() {
            self.mesh_renderer.write_uniforms(
                &self.device,
                &self.queue,
                &self.mesh_cache,
                frame,
                viewport_px,
            );
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let mut new_readbacks = Vec::new();

        // The pick pass and its readback, only when the app asked for one.
        if let Some((x, y)) = pick {
            // A pick in the side pane draws its bodies with its camera.
            let pane = Pane::at(frame, x, y);
            let pick_rect = pane.rect(frame, frame.viewport_rect.unwrap_or(full));
            let pane_rect = clamp_rect(pick_rect, width, height);
            {
                let attachment = |view, clear| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                };
                let mut color_attachments = vec![
                    attachment(&self.targets.pick_ids_view, wgpu::Color::TRANSPARENT),
                    // Nothing drawn reads as the far plane: the bits of a
                    // depth of 1.
                    attachment(
                        &self.targets.pick_depth_view,
                        wgpu::Color {
                            r: f64::from(1.0f32.to_bits()),
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        },
                    ),
                ];
                // Zero, the cleared texel, is no triangle: the pass writes
                // the index plus one.
                if let Some((_, view)) = &self.targets.pick_primitives {
                    color_attachments.push(attachment(view, wgpu::Color::TRANSPARENT));
                }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pick"),
                    color_attachments: &color_attachments,
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.targets.pick_depth_test_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                set_viewport(&mut pass, pane_rect);
                self.mesh_renderer
                    .draw_pick(&mut pass, &self.mesh_cache, frame, pane);
            }
            let request = PickRequest {
                x,
                y,
                view_proj: pane.camera(frame).view_proj,
                viewport: pick_rect,
                window: readback::window_around(x, y, width, height),
                drawn: self.targets.pick_primitives.is_some().then(|| {
                    frame
                        .bodies
                        .iter()
                        .filter(|b| b.pickable && pane.draws(frame, b))
                        .map(|b| (b.id, b.revision))
                        .collect()
                }),
            };
            new_readbacks.push(Readback::pick(
                &self.device,
                &mut encoder,
                &self.targets.pick_ids,
                self.targets
                    .pick_primitives
                    .as_ref()
                    .map(|(texture, _)| texture),
                &self.targets.pick_depth,
                request,
            ));
        }

        if scene_dirty {
            let (view, resolve_target) = match &self.targets.msaa_view {
                Some(msaa) => (msaa, Some(&self.targets.scene_view)),
                None => (&self.targets.scene_view, None),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(VIEWPORT_BACKGROUND),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            set_viewport(&mut pass, viewport);
            self.last_draw_stats = self.mesh_renderer.draw(
                &mut pass,
                &self.mesh_cache,
                frame,
                viewport_px,
                &self.grid_renderer,
                Pane::Main,
            );
            // The side pane: its bodies and its camera, in its own rect.
            if let Some(side) = &frame.side_pane {
                set_viewport(&mut pass, side.rect);
                let more = self.mesh_renderer.draw(
                    &mut pass,
                    &self.mesh_cache,
                    frame,
                    viewport_px,
                    &self.grid_renderer,
                    Pane::Side,
                );
                self.last_draw_stats.bodies_drawn += more.bodies_drawn;
                self.last_draw_stats.triangle_indices += more.triangle_indices;
            }
            drop(pass);
            self.last_scene_fingerprint = Some(fingerprint);
        }

        let captures = std::mem::take(&mut self.pending_captures);
        let capture_request = |kind, format| CaptureRequest {
            kind,
            width,
            height,
            bgra: matches!(
                format,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
            ),
            viewport: frame.viewport_rect.unwrap_or(full),
        };
        // A picture of the scene asked for: the scene image into host memory.
        if captures.contains(&CaptureKind::Scene) {
            new_readbacks.push(Readback::capture(
                &self.device,
                &mut encoder,
                &self.targets.scene,
                capture_request(CaptureKind::Scene, self.scene_format),
            ));
        }

        // egui's uploads now; its frees once the frame is submitted.
        let mut textures = std::mem::take(&mut self.pending_textures);
        for (id, deltas) in &textures.set {
            for delta in deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: frame.egui.as_ref().map_or(1.0, |ui| ui.pixels_per_point),
        };
        let mut user_commands = Vec::new();
        if let Some(ui) = &frame.egui {
            user_commands = self.egui_renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &ui.primitives,
                &screen,
            );
        }

        // The window: the scene under, the UI over.
        let target = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.draw_window(&mut encoder, &target, frame, &screen);

        // A picture of the view asked for: the window drawn again into an
        // image of its own, since the window's own may not be copied from.
        for kind in [CaptureKind::View, CaptureKind::Window] {
            if !captures.contains(&kind) {
                continue;
            }
            let image = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("window picture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = image.create_view(&wgpu::TextureViewDescriptor::default());
            self.draw_window(&mut encoder, &view, frame, &screen);
            let mut request = capture_request(kind, self.config.format);
            if kind == CaptureKind::Window {
                request.viewport = full;
            }
            new_readbacks.push(Readback::capture(
                &self.device,
                &mut encoder,
                &image,
                request,
            ));
        }

        self.queue
            .submit(user_commands.into_iter().chain([encoder.finish()]));
        for readback in &new_readbacks {
            readback.map();
        }
        self.readbacks.extend(new_readbacks);
        self.queue.present(surface_texture);

        for id in &textures.free {
            self.egui_renderer.free_texture(id);
        }
        // Every change is applied: the list may go.
        textures.clear();

        // Drop GPU buffers for any mesh no body submits any more.
        let alive: Vec<_> = frame
            .bodies
            .iter()
            .map(|b| (b.mesh_key(), b.scalar.is_some()))
            .collect();
        if self.mesh_cache.has_dead_entries(&alive) {
            self.mesh_cache.retain_only(&alive);
        }
        Ok(())
    }

    /// Record the window's pass into `target`: the scene image blitted
    /// under, egui's frame over it.
    fn draw_window(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: &FrameSubmission,
        screen: &egui_wgpu::ScreenDescriptor,
    ) {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("window"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, &self.targets.blit_group, &[]);
        pass.draw(0..3, 0..1);
        if let Some(ui) = &frame.egui {
            self.egui_renderer.render(&mut pass, &ui.primitives, screen);
        }
    }
}

impl Drop for RendererCore {
    fn drop(&mut self) {
        let started = std::time::Instant::now();
        // Let the GPU finish before its resources go.
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let _ = &self.adapter;
        tracing::debug!(
            target: "printcad.frame",
            ms = started.elapsed().as_millis() as u64,
            "renderer torn down"
        );
    }
}

/// The name people know a backend's graphics API by.
fn api_name(backend: wgpu::Backend) -> &'static str {
    match backend {
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Metal => "Metal",
        wgpu::Backend::Dx12 => "DirectX 12",
        wgpu::Backend::Gl => "OpenGL",
        wgpu::Backend::BrowserWebGpu => "WebGPU",
        wgpu::Backend::Noop => "None",
    }
}

/// A rectangle inside the target, at least a texel each way.
fn clamp_rect(rect: ViewportRect, width: u32, height: u32) -> ViewportRect {
    let x = rect.x.min(width.saturating_sub(1));
    let y = rect.y.min(height.saturating_sub(1));
    ViewportRect {
        x,
        y,
        width: rect.width.clamp(1, width - x),
        height: rect.height.clamp(1, height - y),
    }
}

fn set_viewport(pass: &mut wgpu::RenderPass<'_>, rect: ViewportRect) {
    pass.set_viewport(
        rect.x as f32,
        rect.y as f32,
        rect.width as f32,
        rect.height as f32,
        0.0,
        1.0,
    );
    pass.set_scissor_rect(rect.x, rect.y, rect.width, rect.height);
}

fn create_targets(
    device: &wgpu::Device,
    blit_layout: &wgpu::BindGroupLayout,
    config: &wgpu::SurfaceConfiguration,
    scene_format: wgpu::TextureFormat,
    msaa_samples: u32,
    picks_primitives: bool,
) -> Targets {
    let size = wgpu::Extent3d {
        width: config.width,
        height: config.height,
        depth_or_array_layers: 1,
    };
    let texture = |label, format, samples, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
    let scene = texture(
        "scene",
        scene_format,
        1,
        attachment | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
    );
    let scene_view = scene.create_view(&Default::default());
    let msaa_view = (msaa_samples > 1).then(|| {
        texture("scene msaa", scene_format, msaa_samples, attachment)
            .create_view(&Default::default())
    });
    let depth_view = texture("scene depth", DEPTH_FORMAT, msaa_samples, attachment)
        .create_view(&Default::default());
    let blit_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("blit"),
        layout: blit_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&scene_view),
        }],
    });
    let readable = attachment | wgpu::TextureUsages::COPY_SRC;
    let pick_ids = texture("pick ids", PICK_ID_FORMAT, 1, readable);
    let pick_ids_view = pick_ids.create_view(&Default::default());
    let pick_depth = texture("pick depth", PICK_DEPTH_FORMAT, 1, readable);
    let pick_depth_view = pick_depth.create_view(&Default::default());
    let pick_primitives = picks_primitives.then(|| {
        let texture = texture("pick primitives", PICK_PRIMITIVE_FORMAT, 1, readable);
        let view = texture.create_view(&Default::default());
        (texture, view)
    });
    let pick_depth_test_view =
        texture("pick depth test", DEPTH_FORMAT, 1, attachment).create_view(&Default::default());
    Targets {
        scene,
        scene_view,
        msaa_view,
        depth_view,
        blit_group,
        pick_ids,
        pick_ids_view,
        pick_depth,
        pick_depth_view,
        pick_primitives,
        pick_depth_test_view,
    }
}

fn blit_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("blit"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blit"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_blit"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            // A plain window gets the scene encoded to sRGB by the shader;
            // an sRGB one encodes it itself.
            entry_point: Some(if format.is_srgb() {
                "fs_blit"
            } else {
                "fs_blit_encode"
            }),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Everything that determines what the scene pass would draw. Two frames
/// with equal fingerprints render identical scene images, so the second
/// reuses the first. Completeness is the contract: anything the scene pass
/// reads must be hashed here, or a change to it would show stale.
pub(crate) fn scene_fingerprint(frame: &FrameSubmission) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let f32s = |h: &mut std::collections::hash_map::DefaultHasher, xs: &[f32]| {
        for x in xs {
            x.to_bits().hash(h);
        }
    };
    for col in &frame.view_proj {
        f32s(&mut h, col);
    }
    f32s(&mut h, &frame.camera_pos);
    frame.grids.len().hash(&mut h);
    for grid in &frame.grids {
        for value in [
            grid.origin,
            grid.u,
            grid.v,
            grid.center,
            grid.forward,
            grid.color,
            grid.u_axis_color,
            grid.v_axis_color,
        ] {
            f32s(&mut h, &value);
        }
        f32s(&mut h, &grid.depth_range);
        f32s(
            &mut h,
            &[
                grid.step,
                grid.radius,
                grid.minor_alpha,
                grid.major_alpha,
                grid.axis_alpha,
                grid.dash[0],
                grid.dash[1],
            ],
        );
    }
    frame.draw_edges.hash(&mut h);
    match &frame.clip_plane {
        Some(plane) => f32s(&mut h, plane),
        None => 0u8.hash(&mut h),
    }
    if let Some(r) = &frame.viewport_rect {
        (r.x, r.y, r.width, r.height).hash(&mut h);
    } else {
        0u8.hash(&mut h);
    }
    if let Some(side) = &frame.side_pane {
        let r = &side.rect;
        (r.x, r.y, r.width, r.height, &side.bodies).hash(&mut h);
        if let Some(camera) = &side.camera {
            for col in &camera.view_proj {
                f32s(&mut h, col);
            }
            f32s(&mut h, &camera.camera_pos);
        } else {
            0u8.hash(&mut h);
        }
    } else {
        0u8.hash(&mut h);
    }
    let l = &frame.lighting;
    for light in [&l.main_light, &l.backlight, &l.fill_light] {
        f32s(&mut h, &light.direction_intensity);
        f32s(&mut h, &light.color_enabled);
    }
    f32s(&mut h, &l.ambient_color);
    f32s(
        &mut h,
        &[
            l.ambient_intensity,
            l.specular_shininess,
            l.specular_intensity,
            l.edge_line_width,
        ],
    );
    f32s(&mut h, &l.edge_line_color);
    frame.bodies.len().hash(&mut h);
    for body in &frame.bodies {
        body.id.hash(&mut h);
        body.revision.hash(&mut h);
        // The mesh pointer catches overlays rebuilt without a revision bump.
        (Arc::as_ptr(&body.mesh) as usize).hash(&mut h);
        f32s(&mut h, &body.color);
        body.opacity.to_bits().hash(&mut h);
        (body.highlight as u8).hash(&mut h);
        body.is_wireframe.hash(&mut h);
        body.on_top.hash(&mut h);
        match &body.edge_color {
            Some(color) => f32s(&mut h, color),
            None => 0u8.hash(&mut h),
        }
        body.front_only.hash(&mut h);
        body.vertex_colors.hash(&mut h);
        body.section.open.hash(&mut h);
        body.section.hatch.map(f32::to_bits).hash(&mut h);
        body.section.uncut.hash(&mut h);
        body.scalar.is_some().hash(&mut h);
        if let Some(field) = &body.scalar {
            field.revision.hash(&mut h);
            f32s(&mut h, &field.range);
            field.bands.hash(&mut h);
            field.ramp.len().hash(&mut h);
            for color in field.ramp.iter() {
                f32s(&mut h, color);
            }
            f32s(&mut h, &field.under);
            f32s(&mut h, &field.over);
            f32s(&mut h, &field.none);
        }
        match &body.instance {
            Some(instance) => {
                instance.key.hash(&mut h);
                for col in &instance.transform {
                    f32s(&mut h, col);
                }
            }
            None => 0u8.hash(&mut h),
        }
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_viewport_is_kept_inside_the_target() {
        let r = clamp_rect(
            ViewportRect {
                x: 10,
                y: 20,
                width: 500,
                height: 500,
            },
            300,
            200,
        );
        assert_eq!((r.x, r.y, r.width, r.height), (10, 20, 290, 180));
        let r = clamp_rect(
            ViewportRect {
                x: 400,
                y: 0,
                width: 0,
                height: 10,
            },
            300,
            200,
        );
        assert_eq!((r.x, r.width), (299, 1));
    }

    #[test]
    fn a_changed_camera_or_body_changes_the_fingerprint() {
        let mut frame = FrameSubmission::default();
        let first = scene_fingerprint(&frame);
        assert_eq!(first, scene_fingerprint(&frame));
        frame.camera_pos[0] += 1.0;
        let moved = scene_fingerprint(&frame);
        assert_ne!(first, moved);
        frame.draw_edges = !frame.draw_edges;
        assert_ne!(moved, scene_fingerprint(&frame));
    }

    /// A side pane with a camera of its own draws and picks with it: the
    /// fingerprint follows that camera, and a pixel in its rect unprojects
    /// through it to the point its body has there.
    #[test]
    fn a_side_pane_draws_and_picks_with_its_own_camera() {
        let look = |eye: glam::Vec3| {
            let proj = glam::camera::rh::proj::directx::perspective(1.0, 1.0, 0.1, 100.0);
            (proj * glam::camera::rh::view::look_at_mat4(eye, glam::Vec3::ZERO, glam::Vec3::Z))
                .to_cols_array_2d()
        };
        let body = uuid::Uuid::new_v4();
        let rect = |x| ViewportRect {
            x,
            y: 0,
            width: 200,
            height: 200,
        };
        let mut frame = FrameSubmission {
            view_proj: look(glam::Vec3::new(10.0, 0.0, 0.0)),
            viewport_rect: Some(rect(0)),
            side_pane: Some(crate::SidePane {
                rect: rect(202),
                bodies: vec![body],
                camera: None,
            }),
            ..FrameSubmission::default()
        };
        let linked = scene_fingerprint(&frame);
        let own = crate::PaneCamera {
            view_proj: look(glam::Vec3::new(0.0, -8.0, 3.0)),
            camera_pos: [0.0, -8.0, 3.0],
        };
        frame.side_pane.as_mut().unwrap().camera = Some(own);
        assert_ne!(linked, scene_fingerprint(&frame));
        assert_eq!(Pane::Side.camera(&frame), own);
        assert_eq!(Pane::Main.camera(&frame).view_proj, frame.view_proj);

        // Where the side camera shows a point of the body, a pick lands in
        // the side pane and reads the point back through its camera.
        let point = glam::Vec4::new(0.5, 0.2, 0.3, 1.0);
        let clip = glam::Mat4::from_cols_array_2d(&own.view_proj) * point;
        let ndc = clip / clip.w;
        let side = rect(202);
        let x = side.x as f32 + (ndc.x + 1.0) * 0.5 * side.width as f32;
        let y = side.y as f32 + (ndc.y + 1.0) * 0.5 * side.height as f32;
        let pane = Pane::at(&frame, x as u32, y as u32);
        assert_eq!(pane, Pane::Side);
        assert_eq!(Pane::at(&frame, 50, 50), Pane::Main);
        let rect = pane.rect(&frame, frame.viewport_rect.unwrap());
        let back = crate::unproject(x, y, ndc.z, &rect, pane.camera(&frame).view_proj);
        assert!((glam::Vec3::from(back) - point.truncate()).length() < 1e-3);
        let drawn = crate::BodySubmission {
            scalar: None,
            id: body,
            revision: 0,
            mesh: Arc::new(kernel_api::TriMesh::default()),
            color: [1.0; 3],
            opacity: 1.0,
            highlight: crate::HighlightState::None,
            is_wireframe: false,
            pickable: true,
            on_top: false,
            edge_color: None,
            front_only: false,
            vertex_colors: false,
            instance: None,
            section: Default::default(),
        };
        assert!(pane.draws(&frame, &drawn) && !Pane::Main.draws(&frame, &drawn));
    }

    #[test]
    fn a_shared_mesh_moved_changes_the_fingerprint() {
        let shift = |x: f32| crate::MeshInstance {
            key: uuid::Uuid::nil(),
            transform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [x, 0.0, 0.0, 1.0],
            ],
        };
        let mut frame = FrameSubmission::default();
        frame.bodies.push(crate::BodySubmission {
            scalar: None,
            id: uuid::Uuid::new_v4(),
            revision: 0,
            mesh: Arc::new(kernel_api::TriMesh::default()),
            color: [1.0; 3],
            opacity: 1.0,
            highlight: crate::HighlightState::None,
            is_wireframe: false,
            pickable: true,
            on_top: false,
            edge_color: None,
            front_only: false,
            vertex_colors: false,
            instance: Some(shift(10.0)),
            section: Default::default(),
        });
        let first = scene_fingerprint(&frame);
        frame.bodies[0].instance = Some(shift(20.0));
        let moved = scene_fingerprint(&frame);
        assert_ne!(first, moved);
        assert_eq!(frame.bodies[0].mesh_key(), uuid::Uuid::nil());
        // Its own colours kept see-through redraw the scene too.
        frame.bodies[0].vertex_colors = true;
        assert_ne!(moved, scene_fingerprint(&frame));
        frame.bodies[0].scalar = Some(crate::ScalarField {
            values: Arc::from([0.0]),
            revision: 0,
            range: [-1.0, 1.0],
            bands: 0,
            ramp: Arc::from([[0.0; 3], [1.0; 3]]),
            under: [0.0; 3],
            over: [1.0; 3],
            none: [0.5; 3],
        });
        assert_eq!(frame.bodies[0].mesh_key(), frame.bodies[0].id);
        let changes: [fn(&mut crate::ScalarField); 7] = [
            |s| s.revision += 1,
            |s| s.range[0] -= 1.0,
            |s| s.bands += 1,
            |s| s.ramp = Arc::from([[0.2; 3]]),
            |s| s.under[0] += 0.1,
            |s| s.over[0] -= 0.1,
            |s| s.none[0] += 0.1,
        ];
        for change in changes {
            let before = scene_fingerprint(&frame);
            change(frame.bodies[0].scalar.as_mut().unwrap());
            assert_ne!(before, scene_fingerprint(&frame));
        }
    }
}
