//! The patterns: each a height from 0 (the surface as it is) to 1 (the
//! full depth) over a tile one unit on a side, repeating seamlessly.

pub use core_document::texture::Pattern;

/// The height of `pattern` at `(u, v)`, in tiles, from 0 to 1. A
/// picture's comes from its height map, which the caller supplies.
pub fn height(pattern: Pattern, u: f32, v: f32, image: Option<&HeightMap>) -> f32 {
    let (fu, fv) = (fract(u), fract(v));
    let h = match pattern {
        Pattern::Knurl => tri(u + v).min(tri(u - v)),
        Pattern::Ribs => smooth(tri(u)),
        Pattern::Dots => {
            let d = ((fu - 0.5).powi(2) + (fv - 0.5).powi(2)).sqrt();
            let r = (1.0 - d / 0.38).max(0.0);
            (r * (2.0 - r)).sqrt()
        }
        Pattern::Hex => hex(u, v),
        Pattern::Bricks => bricks(u, v),
        Pattern::Waves => 0.5 + 0.5 * (std::f32::consts::TAU * u).sin(),
        Pattern::Noise => noise(u, v),
        Pattern::Crosshatch => 1.0 - groove(tri(u + v)).max(groove(tri(u - v))),
        Pattern::Image { .. } => image.map_or(0.0, |map| map.sample(u, v)),
    };
    h.clamp(0.0, 1.0)
}

/// A picture's heights, row by row from the top, each 0..=1.
#[derive(Debug, Clone, PartialEq)]
pub struct HeightMap {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
}

impl HeightMap {
    /// A PNG or JPEG file's grey levels as heights, white high, no wider
    /// or taller than `longest` pixels.
    pub fn from_image(bytes: &[u8], longest: u32) -> Result<Self, String> {
        let picture = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        let picture = if picture.width().max(picture.height()) > longest {
            picture.resize(longest, longest, image::imageops::FilterType::Triangle)
        } else {
            picture
        };
        let grey = picture.to_luma8();
        Ok(Self {
            width: grey.width() as usize,
            height: grey.height() as usize,
            values: grey.pixels().map(|p| f32::from(p.0[0]) / 255.0).collect(),
        })
    }

    /// The height at `(u, v)` in tiles, the picture repeating, read
    /// between its pixels.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        // The picture's top at the tile's top.
        let x = fract(u) * self.width as f32 - 0.5;
        let y = (1.0 - fract(v)) * self.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let at = |x: f32, y: f32| {
            let xi = (x as i64).rem_euclid(self.width as i64) as usize;
            let yi = (y as i64).rem_euclid(self.height as i64) as usize;
            self.values[yi * self.width + xi]
        };
        let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1.0, y0) * tx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - tx) + at(x0 + 1.0, y0 + 1.0) * tx;
        top * (1.0 - ty) + bottom * ty
    }
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// 0 at whole numbers, 1 halfway between, straight in between.
fn tri(x: f32) -> f32 {
    1.0 - (2.0 * fract(x) - 1.0).abs()
}

/// Eased: flat at 0 and 1.
fn smooth(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}

/// A groove where `t` (a [`tri`]) is near 0: 1 in it, 0 away from it.
fn groove(t: f32) -> f32 {
    const WIDTH: f32 = 0.18;
    smooth((1.0 - t / WIDTH).clamp(0.0, 1.0))
}

/// Raised hexagonal cells, grooves between them. The tile holds a whole
/// number of cells across and down, so it repeats.
fn hex(u: f32, v: f32) -> f32 {
    // One cell across, two rows down (offset by half a cell): a tile that
    // repeats.
    let sqrt3 = 3f32.sqrt();
    let (x, y) = (fract(u), fract(v) * sqrt3);
    let centers = [
        (0.0, 0.0),
        (1.0, 0.0),
        (0.5, sqrt3 / 2.0),
        (0.0, sqrt3),
        (1.0, sqrt3),
    ];
    let (mut first, mut second) = (f32::MAX, f32::MAX);
    for (cx, cy) in centers {
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        if d < first {
            second = first;
            first = d;
        } else if d < second {
            second = d;
        }
    }
    // Near the line halfway between two centres is the groove.
    let edge = (second - first) / 0.5;
    1.0 - groove(edge.clamp(0.0, 1.0))
}

/// Bricks two across, two rows to the tile, the second row offset by
/// half a brick.
fn bricks(u: f32, v: f32) -> f32 {
    let row = (fract(v) * 2.0).floor();
    let y = fract(fract(v) * 2.0);
    let x = fract(fract(u) * 2.0 + if row == 1.0 { 0.5 } else { 0.0 });
    let to_edge = (x.min(1.0 - x) * 2.0).min(y.min(1.0 - y) * 4.0);
    1.0 - groove(to_edge.clamp(0.0, 1.0))
}

/// Value noise that repeats every tile: a few octaves on an integer
/// lattice wrapped at the tile.
fn noise(u: f32, v: f32) -> f32 {
    let (mut sum, mut weight, mut total) = (0.0, 1.0, 0.0);
    for octave in 0..4 {
        let cells = 4u32 << octave;
        sum += weight * lattice(fract(u), fract(v), cells);
        total += weight;
        weight *= 0.5;
    }
    sum / total
}

/// Smoothly interpolated random values on a `cells` × `cells` lattice
/// that wraps at the tile.
fn lattice(u: f32, v: f32, cells: u32) -> f32 {
    let (x, y) = (u * cells as f32, v * cells as f32);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (tx, ty) = (smooth(x - x0 as f32), smooth(y - y0 as f32));
    let at = |i: u32, j: u32| hash(i % cells, j % cells, cells);
    let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
    let bottom = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// A fixed pseudo-random value in 0..=1 for a lattice point.
fn hash(i: u32, j: u32, salt: u32) -> f32 {
    let mut h = i
        .wrapping_mul(0x8da6_b343)
        .wrapping_add(j.wrapping_mul(0xd816_3841))
        .wrapping_add(salt.wrapping_mul(0xcb1a_b31f));
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every pattern stays within 0..=1 and repeats: a tile's left edge
    /// meets its right, its bottom its top.
    #[test]
    fn every_pattern_is_in_range_and_repeats() {
        for pattern in Pattern::BUILT_IN {
            for i in 0..=40 {
                let t = i as f32 / 40.0;
                for (u, v) in [(t, 0.3), (0.7, t), (t * 3.0, -t)] {
                    let h = height(pattern, u, v, None);
                    assert!((0.0..=1.0).contains(&h), "{pattern:?} at {u},{v}: {h}");
                }
                let e = 1e-4;
                let across =
                    (height(pattern, e, t, None) - height(pattern, 1.0 - e, t, None)).abs();
                let down = (height(pattern, t, e, None) - height(pattern, t, 1.0 - e, None)).abs();
                assert!(
                    across < 0.05 && down < 0.05,
                    "{pattern:?} seams: {across} {down}"
                );
            }
        }
    }

    /// A picture file's grey levels are its heights.
    #[test]
    fn a_picture_file_becomes_heights() {
        let mut png = Vec::new();
        image::GrayImage::from_fn(4, 2, |_, y| image::Luma([if y == 0 { 255 } else { 0 }]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let map = HeightMap::from_image(&png, 1024).unwrap();
        assert_eq!((map.width, map.height), (4, 2));
        assert_eq!(map.values[0], 1.0);
        assert_eq!(map.values[4], 0.0);
        assert!(HeightMap::from_image(b"no", 1024).is_err());
    }

    /// A picture reads white as high, its top at the tile's top.
    #[test]
    fn a_picture_is_read_the_right_way_up() {
        // Two rows: white above black.
        let map = HeightMap {
            width: 1,
            height: 2,
            values: vec![1.0, 0.0],
        };
        assert!(map.sample(0.5, 0.75) > 0.9, "the top is white");
        assert!(map.sample(0.5, 0.25) < 0.1, "the bottom is black");
    }
}
