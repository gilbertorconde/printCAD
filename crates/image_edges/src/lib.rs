//! Subpixel edge chains in a grey image.
//!
//! 1. The image is smoothed by a Gaussian (`Settings::sigma`).
//! 2. Sobel gradients give each pixel a direction and a strength.
//! 3. A pixel stays an edge where its strength peaks across the edge: no
//!    weaker than the strength one pixel either way along the gradient,
//!    read between pixels.
//! 4. Thresholds come from the image: the noise is read from the median
//!    gradient, the high threshold set above it (and above a share of the
//!    strongest edge), the low at a share of the high. Ridges above the
//!    high threshold are kept with every ridge above the low one they
//!    touch.
//! 5. Each kept pixel moves to the top of a parabola through the strength
//!    one pixel either way along the gradient: the edge between pixels.
//! 6. Neighbouring pixels link into chains, each walking on to the
//!    neighbour that keeps its course; a chain whose ends touch closes.
//!
//! Coordinates are pixels: the centre of pixel `(i, j)` is `(i, j)`, `x`
//! to the right and `y` down.

pub mod objects;
pub use objects::{Object, objects, objects_stretched};

/// A grey image, row after row.
#[derive(Debug, Clone, Copy)]
pub struct Gray<'a> {
    pub width: usize,
    pub height: usize,
    pub pixels: &'a [f32],
}

/// How edges are found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// The smoothing's standard deviation, pixels.
    pub sigma: f64,
    /// From 0 (only the clearest edges) to 1 (faint ones too); 0.5 by
    /// default.
    pub sensitivity: f64,
    /// Chains of fewer points are dropped.
    pub min_points: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sigma: 1.0,
            sensitivity: 0.5,
            min_points: 8,
        }
    }
}

/// One chain of edge points.
#[derive(Debug, Clone, PartialEq)]
pub struct Chain {
    pub points: Vec<[f64; 2]>,
    /// The last point touches the first.
    pub closed: bool,
    /// Walking along the chain, the brighter side is on the left (seen
    /// with `y` down, as the image is).
    pub brighter_left: bool,
}

/// Every edge chain of `image`.
pub fn detect(image: &Gray<'_>, settings: &Settings) -> Vec<Chain> {
    let (w, h) = (image.width, image.height);
    if w < 3 || h < 3 || image.pixels.len() < w * h {
        return Vec::new();
    }
    let smooth = blur(image, settings.sigma.max(0.0));
    let (gx, gy) = sobel(&smooth, w, h);
    let mag: Vec<f64> = gx.iter().zip(&gy).map(|(x, y)| x.hypot(*y)).collect();
    let at = |x: f64, y: f64| bilinear(&mag, w, h, x, y);

    // Ridges: strength peaking across the edge.
    let mut ridge = vec![false; w * h];
    let mut strongest: f64 = 0.0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let m = mag[i];
            if m <= 0.0 {
                continue;
            }
            let (dx, dy) = (gx[i] / m, gy[i] / m);
            let (fx, fy) = (x as f64, y as f64);
            let before = at(fx - dx, fy - dy);
            let after = at(fx + dx, fy + dy);
            if m >= before && m > after {
                ridge[i] = true;
                strongest = strongest.max(m);
            }
        }
    }
    if strongest <= 0.0 {
        return Vec::new();
    }

    // Thresholds from the noise and the strongest edge.
    let mut sorted: Vec<f64> = mag.iter().copied().filter(|m| *m > 0.0).collect();
    sorted.sort_by(f64::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let s = settings.sensitivity.clamp(0.0, 1.0);
    let high = (6.0 * median).max(0.08 * strongest) * (1.5 - s);
    let low = 0.4 * high;

    // Hysteresis: strong ridges and the weak ones they reach.
    let mut kept = vec![false; w * h];
    let mut stack: Vec<usize> = (0..w * h)
        .filter(|i| ridge[*i] && mag[*i] >= high)
        .collect();
    for i in &stack {
        kept[*i] = true;
    }
    while let Some(i) = stack.pop() {
        for j in neighbours(i, w, h) {
            if !kept[j] && ridge[j] && mag[j] >= low {
                kept[j] = true;
                stack.push(j);
            }
        }
    }

    // Each kept pixel at the top of its parabola across the edge.
    let subpixel = |i: usize| -> [f64; 2] {
        let (x, y) = ((i % w) as f64, (i / w) as f64);
        let m = mag[i];
        let (dx, dy) = (gx[i] / m, gy[i] / m);
        let (a, c) = (at(x - dx, y - dy), at(x + dx, y + dy));
        let bend = a - 2.0 * m + c;
        let t = if bend < 0.0 {
            (0.5 * (a - c) / bend).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        [x + t * dx, y + t * dy]
    };

    let chains = link(&kept, w, h, &gx, &gy);
    chains
        .into_iter()
        .filter(|c| c.len() >= settings.min_points.max(2))
        .map(|pixels| {
            let closed = pixels.len() > 2 && touch(pixels[0], pixels[pixels.len() - 1], w);
            let points: Vec<[f64; 2]> = pixels.iter().map(|i| subpixel(*i)).collect();
            // The gradient points to the brighter side.
            let mut left = 0.0;
            for (k, i) in pixels.iter().enumerate() {
                let next = points[(k + 1).min(points.len() - 1)];
                let prev = points[k.saturating_sub(1)];
                let (tx, ty) = (next[0] - prev[0], next[1] - prev[1]);
                // Left of the direction of travel, with y down.
                left += ty * gx[*i] - tx * gy[*i];
            }
            Chain {
                points,
                closed,
                brighter_left: left > 0.0,
            }
        })
        .collect()
}

/// The pixels kept, linked into chains of neighbours: from loose ends
/// first, then round what is left (loops).
fn link(kept: &[bool], w: usize, h: usize, gx: &[f64], gy: &[f64]) -> Vec<Vec<usize>> {
    let mut used = vec![false; w * h];
    let degree = |i: usize| neighbours(i, w, h).filter(|j| kept[*j]).count();
    let mut chains = Vec::new();
    let mut starts: Vec<usize> = (0..w * h).filter(|i| kept[*i] && degree(*i) == 1).collect();
    starts.extend((0..w * h).filter(|i| kept[*i] && degree(*i) != 1));
    for start in starts {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut chain = vec![start];
        loop {
            let here = *chain.last().unwrap_or(&start);
            // Along the edge: square to the gradient, the way it is going.
            let (tx, ty) = (-gy[here], gx[here]);
            let heading = if chain.len() > 1 {
                let prev = chain[chain.len() - 2];
                let (px, py) = step(prev, here, w);
                if px * tx + py * ty < 0.0 {
                    (-tx, -ty)
                } else {
                    (tx, ty)
                }
            } else {
                (tx, ty)
            };
            let next = neighbours(here, w, h)
                .filter(|j| kept[*j] && !used[*j])
                .max_by(|a, b| {
                    let score = |j: usize| {
                        let (sx, sy) = step(here, j, w);
                        (sx * heading.0 + sy * heading.1) / sx.hypot(sy)
                    };
                    score(*a).total_cmp(&score(*b))
                });
            let Some(next) = next else {
                break;
            };
            used[next] = true;
            chain.push(next);
        }
        // A chain started mid-way runs on the other way too.
        if degree(start) != 1 {
            let mut back = vec![start];
            loop {
                let here = *back.last().unwrap_or(&start);
                let next = neighbours(here, w, h).find(|j| kept[*j] && !used[*j]);
                let Some(next) = next else {
                    break;
                };
                used[next] = true;
                back.push(next);
            }
            if back.len() > 1 {
                back.reverse();
                back.pop();
                back.extend(chain);
                chain = back;
            }
        }
        chains.push(chain);
    }
    chains
}

fn step(from: usize, to: usize, w: usize) -> (f64, f64) {
    (
        (to % w) as f64 - (from % w) as f64,
        (to / w) as f64 - (from / w) as f64,
    )
}

fn touch(a: usize, b: usize, w: usize) -> bool {
    let (dx, dy) = step(a, b, w);
    dx.abs() <= 1.0 && dy.abs() <= 1.0
}

fn neighbours(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = ((i % w) as isize, (i / w) as isize);
    (-1..=1)
        .flat_map(move |dy| (-1..=1).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| *dx != 0 || *dy != 0)
        .map(move |(dx, dy)| (x + dx, y + dy))
        .filter(move |(x, y)| *x >= 0 && *y >= 0 && (*x as usize) < w && (*y as usize) < h)
        .map(move |(x, y)| y as usize * w + x as usize)
}

/// The image smoothed by a Gaussian of `sigma`, its border repeated.
fn blur(image: &Gray<'_>, sigma: f64) -> Vec<f64> {
    let (w, h) = (image.width, image.height);
    let src: Vec<f64> = image.pixels[..w * h]
        .iter()
        .map(|p| f64::from(*p))
        .collect();
    if sigma < 0.1 {
        return src;
    }
    let r = (3.0 * sigma).ceil() as isize;
    let mut kernel: Vec<f64> = (-r..=r)
        .map(|k| (-((k * k) as f64) / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f64 = kernel.iter().sum();
    kernel.iter_mut().for_each(|k| *k /= sum);
    let pass = |src: &[f64], along_x: bool| -> Vec<f64> {
        let mut out = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                out[y * w + x] = (-r..=r)
                    .zip(&kernel)
                    .map(|(k, wt)| {
                        let (sx, sy) = if along_x {
                            ((x as isize + k).clamp(0, w as isize - 1) as usize, y)
                        } else {
                            (x, (y as isize + k).clamp(0, h as isize - 1) as usize)
                        };
                        src[sy * w + sx] * wt
                    })
                    .sum();
            }
        }
        out
    };
    pass(&pass(&src, true), false)
}

/// Sobel gradients, in grey units per pixel; zero on the border.
fn sobel(v: &[f64], w: usize, h: usize) -> (Vec<f64>, Vec<f64>) {
    let (mut gx, mut gy) = (vec![0.0; w * h], vec![0.0; w * h]);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: isize, dy: isize| {
                v[(y as isize + dy) as usize * w + (x as isize + dx) as usize]
            };
            let i = y * w + x;
            gx[i] =
                (p(1, -1) + 2.0 * p(1, 0) + p(1, 1) - p(-1, -1) - 2.0 * p(-1, 0) - p(-1, 1)) / 8.0;
            gy[i] =
                (p(-1, 1) + 2.0 * p(0, 1) + p(1, 1) - p(-1, -1) - 2.0 * p(0, -1) - p(1, -1)) / 8.0;
        }
    }
    (gx, gy)
}

fn bilinear(v: &[f64], w: usize, h: usize, x: f64, y: f64) -> f64 {
    let x = x.clamp(0.0, (w - 1) as f64);
    let y = y.clamp(0.0, (h - 1) as f64);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let row = |y: usize| v[y * w + x0] * (1.0 - fx) + v[y * w + x1] * fx;
    row(y0) * (1.0 - fy) + row(y1) * fy
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image of `inside` within `shape` and `outside` beyond it, each
    /// pixel averaged over an 8 × 8 grid of samples, with a little
    /// repeatable noise.
    fn render(w: usize, h: usize, noise: f32, shape: impl Fn(f64, f64) -> bool) -> Vec<f32> {
        let mut seed = 12345u64;
        let mut rand = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 10_000) as f32 / 10_000.0 - 0.5
        };
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f64, (i / w) as f64);
                let mut inside = 0;
                for a in 0..8 {
                    for b in 0..8 {
                        let (sx, sy) = (
                            x - 0.5 + (a as f64 + 0.5) / 8.0,
                            y - 0.5 + (b as f64 + 0.5) / 8.0,
                        );
                        inside += usize::from(shape(sx, sy));
                    }
                }
                let t = inside as f32 / 64.0;
                0.8 - 0.6 * t + noise * rand()
            })
            .collect()
    }

    /// The least-squares circle through `points`: centre and radius.
    fn circle(points: &[[f64; 2]]) -> ([f64; 2], f64) {
        let n = points.len() as f64;
        let (mx, my) = points
            .iter()
            .fold((0.0, 0.0), |(a, b), p| (a + p[0] / n, b + p[1] / n));
        // Iterate the centre to the mean of the points pulled to the mean radius.
        let (mut cx, mut cy) = (mx, my);
        for _ in 0..200 {
            let r = points
                .iter()
                .map(|p| (p[0] - cx).hypot(p[1] - cy))
                .sum::<f64>()
                / n;
            let (mut sx, mut sy) = (0.0, 0.0);
            for p in points {
                let d = (p[0] - cx).hypot(p[1] - cy);
                sx += p[0] - r * (p[0] - cx) / d;
                sy += p[1] - r * (p[1] - cy) / d;
            }
            (cx, cy) = (sx / n, sy / n);
        }
        let r = points
            .iter()
            .map(|p| (p[0] - cx).hypot(p[1] - cy))
            .sum::<f64>()
            / n;
        ([cx, cy], r)
    }

    /// A dark disc of radius 40.3 on a light ground comes back as one
    /// closed chain whose circle is within a tenth of a pixel of it, its
    /// brighter side outside; with noise of 2 % of the contrast as well.
    #[test]
    fn a_disc_reads_within_a_tenth_of_a_pixel() {
        let (cx, cy, r) = (100.2, 90.7, 40.3);
        for noise in [0.0, 0.012] {
            let pixels = render(200, 190, noise, |x, y| (x - cx).hypot(y - cy) <= r);
            let gray = Gray {
                width: 200,
                height: 190,
                pixels: &pixels,
            };
            let chains = detect(&gray, &Settings::default());
            let rim = chains.iter().max_by_key(|c| c.points.len()).unwrap();
            assert!(
                rim.closed && rim.points.len() > 200,
                "{noise}: {} points",
                rim.points.len()
            );
            let (c, radius) = circle(&rim.points);
            assert!(
                (c[0] - cx).abs() < 0.1 && (c[1] - cy).abs() < 0.1,
                "{noise}: {c:?}"
            );
            assert!((radius - r).abs() < 0.1, "{noise}: {radius}");
            let worst = rim
                .points
                .iter()
                .map(|p| ((p[0] - cx).hypot(p[1] - cy) - r).abs())
                .fold(0.0, f64::max);
            assert!(worst < 0.25, "{noise}: {worst}");
            // Walking round, the light ground lies on one side throughout.
            let p = rim.points[0];
            let q = rim.points[3];
            let (tx, ty) = (q[0] - p[0], q[1] - p[1]);
            // Left of travel seen with y down is (ty, -tx).
            let outward = ((p[0] - cx) * ty - (p[1] - cy) * tx) > 0.0;
            assert_eq!(rim.brighter_left, outward, "{noise}");
        }
    }

    /// A straight edge at 17° reads as points within a twentieth of a
    /// pixel of its line.
    #[test]
    fn a_straight_edge_reads_within_a_twentieth_of_a_pixel() {
        let (px, py, a) = (60.3, 50.1, 17f64.to_radians());
        let (nx, ny) = (-a.sin(), a.cos());
        let pixels = render(120, 100, 0.0, |x, y| (x - px) * nx + (y - py) * ny > 0.0);
        let gray = Gray {
            width: 120,
            height: 100,
            pixels: &pixels,
        };
        let chains = detect(&gray, &Settings::default());
        let edge = chains.iter().max_by_key(|c| c.points.len()).unwrap();
        assert!(!edge.closed && edge.points.len() > 90);
        // Away from the image's border, where the smoothing reads the edge
        // bent.
        let inner: Vec<_> = edge
            .points
            .iter()
            .filter(|p| p[0] > 6.0 && p[0] < 114.0 && p[1] > 6.0 && p[1] < 94.0)
            .collect();
        let worst = inner
            .iter()
            .map(|p| ((p[0] - px) * nx + (p[1] - py) * ny).abs())
            .fold(0.0, f64::max);
        assert!(worst < 0.05, "{worst}");
    }

    /// A flat image has no edges, and a tiny one is refused quietly.
    #[test]
    fn nothing_to_find_finds_nothing() {
        let flat = vec![0.5f32; 400];
        assert!(
            detect(
                &Gray {
                    width: 20,
                    height: 20,
                    pixels: &flat
                },
                &Settings::default()
            )
            .is_empty()
        );
        assert!(
            detect(
                &Gray {
                    width: 2,
                    height: 2,
                    pixels: &flat
                },
                &Settings::default()
            )
            .is_empty()
        );
    }
}
