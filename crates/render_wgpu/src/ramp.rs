use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui_wgpu::wgpu;

pub(crate) const WIDTH: u32 = 256;
const ROWS: u32 = 16;

pub(crate) fn resample(stops: &[[f32; 3]]) -> [[u8; 4]; WIDTH as usize] {
    std::array::from_fn(|i| {
        let at = i as f32 / (WIDTH - 1) as f32 * stops.len().saturating_sub(1) as f32;
        let low = at.floor() as usize;
        let a = stops.get(low).copied().unwrap_or([0.0; 3]);
        let b = stops.get(low + 1).copied().unwrap_or(a);
        let t = at - low as f32;
        let rgb = std::array::from_fn::<_, 3, _>(|c| {
            ((a[c] * (1.0 - t) + b[c] * t).clamp(0.0, 1.0) * 255.0).round() as u8
        });
        [rgb[0], rgb[1], rgb[2], 255]
    })
}

pub(crate) struct RampAtlas {
    texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) sampler: wgpu::Sampler,
    rows: HashMap<u64, u32>,
}

impl RampAtlas {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scalar ramps"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: ROWS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scalar ramp sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            texture,
            view,
            sampler,
            rows: HashMap::new(),
        }
    }

    /// A full atlas asks its caller for a fresh one. Bind groups keep any
    /// rows referenced earlier in the frame alive until their draws finish.
    pub(crate) fn row(&mut self, queue: &wgpu::Queue, stops: &[[f32; 3]]) -> Option<u32> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        stops.len().hash(&mut hash);
        for stop in stops {
            stop.map(f32::to_bits).hash(&mut hash);
        }
        let key = hash.finish();
        if let Some(row) = self.rows.get(&key) {
            return Some(*row);
        }
        let row = self.rows.len() as u32;
        if row == ROWS {
            return None;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: row, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&resample(stops)),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.rows.insert(key, row);
        Some(row)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn resampling_keeps_the_ends_and_interpolates_the_stops() {
        let texels = super::resample(&[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(texels.len(), 256);
        assert_eq!(texels[0], [255, 0, 0, 255]);
        assert_eq!(texels[255], [0, 0, 255, 255]);
        assert_eq!(texels[85], [85, 170, 0, 255]);
    }
}
