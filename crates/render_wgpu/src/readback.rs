//! What the GPU writes for the host to read: a pick's id, the triangle
//! under the cursor where the device numbers them, the depths around the
//! cursor, and a picture of the scene. Each is a copy into a
//! buffer of its own, mapped once the frame that recorded it is done, so
//! nothing ever waits on the GPU: the answer trails the request by a frame
//! or two.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use egui_wgpu::wgpu;
use uuid::Uuid;

use crate::mesh::u32s_to_uuid;
use crate::{CapturedImage, DepthWindow, PickResult, PickedTriangle, ViewportRect};

/// Texels the depth window reaches either side of the cursor: the edge
/// pick's widest reach at twice the display scale, and a pixel over for
/// its neighbourhood.
pub(crate) const PICK_WINDOW_RADIUS: u32 = 13;

/// Rows of a texture copy are padded to this many bytes.
const ROW_ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// Where the triangle texel sits in a pick's buffer, after the id texel.
const PICK_PRIMITIVE_OFFSET: u64 = ROW_ALIGN as u64;

/// Where the depth window starts in a pick's buffer, after the id and
/// triangle texels.
const PICK_WINDOW_OFFSET: u64 = 2 * ROW_ALIGN as u64;

const WAITING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// A row of `width` texels of `bytes` each, padded for a copy.
pub(crate) fn padded_row(width: u32, bytes: u32) -> u32 {
    (width * bytes).next_multiple_of(ROW_ALIGN)
}

/// The window of texels around `(x, y)` inside a `width` × `height`
/// target: `(x, y, width, height)`.
pub(crate) fn window_around(x: u32, y: u32, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let wx = x.saturating_sub(PICK_WINDOW_RADIUS);
    let wy = y.saturating_sub(PICK_WINDOW_RADIUS);
    let ww = (x + PICK_WINDOW_RADIUS + 1).min(width) - wx;
    let wh = (y + PICK_WINDOW_RADIUS + 1).min(height) - wy;
    (wx, wy, ww, wh)
}

/// A pick: the cursor, the camera it was drawn with, and the window copied.
#[derive(Debug, Clone)]
pub(crate) struct PickRequest {
    pub x: u32,
    pub y: u32,
    pub view_proj: [[f32; 4]; 4],
    pub viewport: ViewportRect,
    pub window: (u32, u32, u32, u32),
    /// Where the pass wrote triangles: each pickable body drawn and the
    /// revision of the mesh it was drawn with, which the triangle's index
    /// counts in. `None` where the device does not number primitives.
    pub drawn: Option<Vec<(Uuid, u64)>>,
}

/// A picture: the scene's size and texel order, and the viewport to keep.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CaptureRequest {
    pub width: u32,
    pub height: u32,
    pub bgra: bool,
    pub viewport: ViewportRect,
}

pub(crate) enum Kind {
    Pick(PickRequest),
    Capture(CaptureRequest),
}

/// A copy recorded into a frame, waiting for its buffer to map.
pub(crate) struct Readback {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    pub(crate) kind: Kind,
}

/// Where a readback stands.
pub(crate) enum Polled {
    Waiting,
    Failed,
    Done(Done),
}

/// What a finished readback holds.
pub(crate) enum Done {
    Pick(PickResult),
    Capture(CapturedImage),
}

impl Readback {
    /// Record the pick's copies: the id texel under the cursor, the
    /// triangle texel where the pass wrote `primitives`, and the depth
    /// window around it, from the pick pass's depth colour target.
    pub(crate) fn pick(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        ids: &wgpu::Texture,
        primitives: Option<&wgpu::Texture>,
        depth: &wgpu::Texture,
        request: PickRequest,
    ) -> Self {
        let (wx, wy, ww, wh) = request.window;
        let size = PICK_WINDOW_OFFSET + u64::from(padded_row(ww, 4)) * u64::from(wh);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pick readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texel = |encoder: &mut wgpu::CommandEncoder, texture, offset| {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: request.x,
                        y: request.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(ROW_ALIGN),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        };
        texel(encoder, ids, 0);
        if let Some(primitives) = primitives {
            texel(encoder, primitives, PICK_PRIMITIVE_OFFSET);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: depth,
                mip_level: 0,
                origin: wgpu::Origin3d { x: wx, y: wy, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: PICK_WINDOW_OFFSET,
                    bytes_per_row: Some(padded_row(ww, 4)),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: ww,
                height: wh,
                depth_or_array_layers: 1,
            },
        );
        Self {
            buffer,
            state: Arc::new(AtomicU8::new(WAITING)),
            kind: Kind::Pick(request),
        }
    }

    /// Record a copy of the whole scene image.
    pub(crate) fn capture(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::Texture,
        request: CaptureRequest,
    ) -> Self {
        let row = padded_row(request.width, 4);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene picture"),
            size: u64::from(row) * u64::from(request.height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            scene.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: request.width,
                height: request.height,
                depth_or_array_layers: 1,
            },
        );
        Self {
            buffer,
            state: Arc::new(AtomicU8::new(WAITING)),
            kind: Kind::Capture(request),
        }
    }

    /// Ask for the buffer, once the frame that fills it is submitted.
    pub(crate) fn map(&self) {
        let state = self.state.clone();
        self.buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                state.store(
                    if result.is_ok() { MAPPED } else { FAILED },
                    Ordering::Release,
                );
            });
    }

    /// Where it stands: still waiting, failed, or done with its answer.
    pub(crate) fn poll(&self) -> Polled {
        match self.state.load(Ordering::Acquire) {
            WAITING => Polled::Waiting,
            FAILED => Polled::Failed,
            _ => {
                let done = {
                    let Ok(bytes) = self.buffer.get_mapped_range(..) else {
                        return Polled::Failed;
                    };
                    match &self.kind {
                        Kind::Pick(request) => Done::Pick(decode_pick(&bytes, request)),
                        Kind::Capture(request) => Done::Capture(decode_capture(&bytes, request)),
                    }
                };
                self.buffer.unmap();
                Polled::Done(done)
            }
        }
    }

    pub(crate) fn is_capture(&self) -> bool {
        matches!(self.kind, Kind::Capture(_))
    }
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The pick's buffer, decoded: the body under the cursor, if any, where it
/// lies, the triangle drawn there, and the depths around it.
pub(crate) fn decode_pick(bytes: &[u8], request: &PickRequest) -> PickResult {
    let id = [
        word(bytes, 0),
        word(bytes, 4),
        word(bytes, 8),
        word(bytes, 12),
    ];
    let (wx, wy, ww, wh) = request.window;
    let row = padded_row(ww, 4) as usize;
    let mut depths = Vec::with_capacity((ww * wh) as usize);
    for r in 0..wh as usize {
        let start = PICK_WINDOW_OFFSET as usize + r * row;
        for c in 0..ww as usize {
            depths.push(f32::from_bits(word(bytes, start + c * 4)));
        }
    }
    let (cx, cy) = ((request.x - wx) as usize, (request.y - wy) as usize);
    let depth = depths.get(cy * ww as usize + cx).copied().unwrap_or(1.0);
    let depth_window = Some(DepthWindow {
        x: wx,
        y: wy,
        width: ww,
        height: wh,
        depths,
        view_proj: request.view_proj,
        viewport: request.viewport,
    });
    // All zeros = cleared texel = no body under the cursor.
    if id == [0, 0, 0, 0] {
        return PickResult {
            depth_window,
            ..PickResult::default()
        };
    }
    let body = u32s_to_uuid(id);
    // Zero is the cleared texel; the pass writes the index plus one.
    let triangle = request.drawn.as_ref().and_then(|drawn| {
        let index = word(bytes, PICK_PRIMITIVE_OFFSET as usize).checked_sub(1)?;
        let revision = drawn.iter().find(|(b, _)| *b == body)?.1;
        Some(PickedTriangle { index, revision })
    });
    PickResult {
        body_id: Some(body),
        world_position: Some(crate::unproject(
            request.x as f32,
            request.y as f32,
            depth,
            &request.viewport,
            request.view_proj,
        )),
        depth,
        depth_window,
        triangle,
    }
}

/// The scene's buffer, decoded: the viewport's texels as RGBA, opaque.
pub(crate) fn decode_capture(bytes: &[u8], request: &CaptureRequest) -> CapturedImage {
    let row = padded_row(request.width, 4) as usize;
    let rect = request.viewport;
    let (x0, y0) = (rect.x.min(request.width), rect.y.min(request.height));
    let w = rect.width.min(request.width - x0);
    let h = rect.height.min(request.height - y0);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let i = y as usize * row + x as usize * 4;
            let p = &bytes[i..i + 4];
            if request.bgra {
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
            } else {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
    }
    CapturedImage {
        width: w,
        height: h,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_stays_inside_the_target() {
        assert_eq!(window_around(5, 5, 100, 100), (0, 0, 19, 19));
        assert_eq!(window_around(50, 50, 100, 100), (37, 37, 27, 27));
        assert_eq!(window_around(99, 0, 100, 10), (86, 0, 14, 10));
    }

    #[test]
    fn rows_are_padded_for_the_copy() {
        assert_eq!(padded_row(1, 16), 256);
        assert_eq!(padded_row(64, 4), 256);
        assert_eq!(padded_row(65, 4), 512);
    }

    /// A pick at the middle of a 3 × 3 viewport, with the camera at rest.
    fn pick_at_the_middle(drawn: Option<Vec<(Uuid, u64)>>) -> PickRequest {
        PickRequest {
            x: 1,
            y: 1,
            view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            viewport: ViewportRect {
                x: 0,
                y: 0,
                width: 3,
                height: 3,
            },
            window: (0, 0, 3, 3),
            drawn,
        }
    }

    /// The buffer such a pick fills: `id` under the cursor, `primitive` as
    /// the triangle texel, a depth of 0.25 under the cursor and 1 around.
    fn filled(id: Uuid, primitive: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; PICK_WINDOW_OFFSET as usize + 3 * 256];
        for (i, w) in crate::mesh::uuid_to_u32s(id).iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let at = PICK_PRIMITIVE_OFFSET as usize;
        bytes[at..at + 4].copy_from_slice(&primitive.to_le_bytes());
        for r in 0..3 {
            for c in 0..3 {
                let at = PICK_WINDOW_OFFSET as usize + r * 256 + c * 4;
                let d: f32 = if (r, c) == (1, 1) { 0.25 } else { 1.0 };
                bytes[at..at + 4].copy_from_slice(&d.to_bits().to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn a_pick_decodes_its_body_and_the_depth_under_the_cursor() {
        let id = uuid::Uuid::new_v4();
        let result = decode_pick(&filled(id, 0), &pick_at_the_middle(None));
        assert_eq!(result.body_id, Some(id));
        assert_eq!(result.depth, 0.25);
        assert_eq!(result.triangle, None);
        let window = result.depth_window.unwrap();
        assert_eq!(window.depths.len(), 9);
        assert_eq!(window.depths[4], 0.25);
        // Texel (1, 1) of a 3-texel viewport, unprojected at its corner as
        // the host has always asked: a third left and up of the centre.
        let p = result.world_position.unwrap();
        let third = -1.0 / 3.0;
        assert!(
            (p[0] - third).abs() < 1e-6 && (p[1] - third).abs() < 1e-6,
            "{p:?}"
        );
        assert!((p[2] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_pick_names_the_triangle_with_the_revision_it_was_drawn_at() {
        let (id, other) = (Uuid::new_v4(), Uuid::new_v4());
        let drawn = Some(vec![(other, 3), (id, 7)]);
        let result = decode_pick(&filled(id, 42), &pick_at_the_middle(drawn.clone()));
        assert_eq!(
            result.triangle,
            Some(PickedTriangle {
                index: 41,
                revision: 7
            })
        );
        // The cleared texel names none, nor does a body not drawn.
        assert_eq!(
            decode_pick(&filled(id, 0), &pick_at_the_middle(drawn)).triangle,
            None
        );
        let only_other = Some(vec![(other, 3)]);
        assert_eq!(
            decode_pick(&filled(id, 42), &pick_at_the_middle(only_other)).triangle,
            None
        );
    }

    #[test]
    fn a_picture_keeps_the_viewport_and_turns_bgra_to_rgba() {
        let request = CaptureRequest {
            width: 2,
            height: 2,
            bgra: true,
            viewport: ViewportRect {
                x: 1,
                y: 0,
                width: 1,
                height: 2,
            },
        };
        let mut bytes = vec![0u8; 2 * 256];
        bytes[4..8].copy_from_slice(&[10, 20, 30, 0]);
        bytes[256 + 4..256 + 8].copy_from_slice(&[40, 50, 60, 0]);
        let picture = decode_capture(&bytes, &request);
        assert_eq!((picture.width, picture.height), (1, 2));
        assert_eq!(picture.rgba, vec![30, 20, 10, 255, 60, 50, 40, 255]);
    }
}
