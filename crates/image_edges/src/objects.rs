//! Separate parts on a picture: the image split by its grey level into
//! part and ground (Otsu's threshold, the part being the side the border
//! is not), each connected part measured: its area, holes, centroid,
//! equivalent diameter, largest and smallest caliper width, long axis and
//! outline. In pixels, as the rest of the crate.

use crate::Gray;

/// One part found on a picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    /// Its material, pixels² (its holes left out).
    pub area: f64,
    pub holes: usize,
    pub centroid: [f64; 2],
    /// The diameter of a disc of its area, pixels.
    pub equivalent_diameter: f64,
    /// Its widest and narrowest extent between two parallel lines either
    /// side of it, pixels.
    pub max_width: f64,
    pub min_width: f64,
    /// The direction of its long axis from `+x`, degrees, counter-clockwise
    /// as seen with `y` down, in (-90, 90].
    pub long_axis_deg: f64,
    /// Its outer boundary, the centres of its edge pixels in order.
    pub outline: Vec<[f64; 2]>,
}

/// Every part of `image` of at least `min_area` pixels.
pub fn objects(image: &Gray<'_>, min_area: f64) -> Vec<Object> {
    let (w, h) = (image.width, image.height);
    if w < 3 || h < 3 || image.pixels.len() < w * h {
        return Vec::new();
    }
    let px = &image.pixels[..w * h];
    let threshold = otsu(px);
    // The ground is the side most of the border is on.
    let border: Vec<usize> = (0..w)
        .flat_map(|x| [x, (h - 1) * w + x])
        .chain((0..h).flat_map(|y| [y * w, y * w + w - 1]))
        .collect();
    let bright_border = border.iter().filter(|i| px[**i] > threshold).count() * 2 > border.len();
    let part: Vec<bool> = px
        .iter()
        .map(|v| (*v > threshold) != bright_border)
        .collect();

    let (labels, count) = components(&part, w, h, true);
    let (ground, ground_count) =
        components(&part.iter().map(|p| !p).collect::<Vec<_>>(), w, h, false);
    // A patch of ground not reaching the border is a hole of the part
    // around it.
    let mut touches = vec![false; ground_count];
    for i in &border {
        if let Some(g) = ground[*i] {
            touches[g] = true;
        }
    }
    let mut holes = vec![std::collections::HashSet::new(); count];
    for y in 0..h {
        for x in 1..w {
            let (a, b) = (y * w + x - 1, y * w + x);
            for (g, p) in [(ground[a], labels[b]), (ground[b], labels[a])] {
                if let (Some(g), Some(p)) = (g, p)
                    && !touches[g]
                {
                    holes[p].insert(g);
                }
            }
        }
    }

    let mut pixels: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (i, l) in labels.iter().enumerate() {
        if let Some(l) = l {
            pixels[*l].push(i);
        }
    }
    pixels
        .into_iter()
        .enumerate()
        .filter(|(_, p)| p.len() as f64 >= min_area.max(1.0))
        .map(|(l, p)| measure(&p, w, &labels, l, holes[l].len()))
        .collect()
}

fn measure(
    pixels: &[usize],
    w: usize,
    labels: &[Option<usize>],
    label: usize,
    holes: usize,
) -> Object {
    let n = pixels.len() as f64;
    let at = |i: usize| [(i % w) as f64, (i / w) as f64];
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in pixels {
        let [x, y] = at(*i);
        cx += x / n;
        cy += y / n;
    }
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for i in pixels {
        let [x, y] = at(*i);
        sxx += (x - cx) * (x - cx);
        sxy += (x - cx) * (y - cy);
        syy += (y - cy) * (y - cy);
    }
    // Angle with y down: counter-clockwise on screen is a negative turn.
    let image_angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let mut long_axis_deg = -image_angle.to_degrees();
    if long_axis_deg <= -90.0 {
        long_axis_deg += 180.0;
    }
    let mine = |i: usize| labels[i] == Some(label);
    // Edge pixels' corners, for the hull the calipers turn round.
    let h = labels.len() / w;
    let edge = |i: usize| {
        let (x, y) = (i % w, i / w);
        x == 0
            || y == 0
            || x + 1 == w
            || y + 1 == h
            || !mine(i - 1)
            || !mine(i + 1)
            || !mine(i - w)
            || !mine(i + w)
    };
    let corners: Vec<[f64; 2]> = pixels
        .iter()
        .filter(|i| edge(**i))
        .flat_map(|i| {
            let [x, y] = at(*i);
            [
                [x - 0.5, y - 0.5],
                [x + 0.5, y - 0.5],
                [x + 0.5, y + 0.5],
                [x - 0.5, y + 0.5],
            ]
        })
        .collect();
    let hull = hull(corners);
    let (max_width, min_width) = calipers(&hull);
    Object {
        area: n,
        holes,
        centroid: [cx, cy],
        equivalent_diameter: 2.0 * (n / std::f64::consts::PI).sqrt(),
        max_width,
        min_width,
        long_axis_deg,
        outline: trace(pixels, w, h, &mine),
    }
}

/// The outer boundary of a part: its edge pixels' centres walked round
/// (Moore's neighbour tracing) from its top-left pixel.
fn trace(pixels: &[usize], w: usize, h: usize, mine: &dyn Fn(usize) -> bool) -> Vec<[f64; 2]> {
    let Some(&start) = pixels.iter().min() else {
        return Vec::new();
    };
    // Clockwise on screen from west.
    const AROUND: [(isize, isize); 8] = [
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
    ];
    let inside = |x: isize, y: isize| {
        x >= 0
            && y >= 0
            && (x as usize) < w
            && (y as usize) < h
            && mine(y as usize * w + x as usize)
    };
    let (mut x, mut y) = ((start % w) as isize, (start / w) as isize);
    let mut out = vec![[x as f64, y as f64]];
    // Entered from the west, which is outside for the top-left pixel.
    let mut from = 0usize;
    for _ in 0..4 * pixels.len() + 8 {
        let mut moved = false;
        for k in 0..8 {
            let d = (from + 1 + k) % 8;
            let (nx, ny) = (x + AROUND[d].0, y + AROUND[d].1);
            if inside(nx, ny) {
                // Look back to where we came from, two steps behind.
                from = (d + 4 + 2) % 8;
                (x, y) = (nx, ny);
                moved = true;
                break;
            }
        }
        if !moved || (x as usize + y as usize * w) == start {
            break;
        }
        out.push([x as f64, y as f64]);
    }
    out
}

/// The convex hull, counter-clockwise (Andrew's monotone chain).
fn hull(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], *p) <= 0.0 {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], *p) <= 0.0 {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The widest and the narrowest extent of a convex polygon between two
/// parallel lines: its diameter, and the least width over its edges'
/// directions.
fn calipers(hull: &[[f64; 2]]) -> (f64, f64) {
    let n = hull.len();
    if n < 2 {
        return (0.0, 0.0);
    }
    let mut widest: f64 = 0.0;
    for i in 0..n {
        for j in i + 1..n {
            widest = widest.max((hull[i][0] - hull[j][0]).hypot(hull[i][1] - hull[j][1]));
        }
    }
    let mut narrowest = f64::MAX;
    for i in 0..n {
        let (a, b) = (hull[i], hull[(i + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy);
        if len <= 0.0 {
            continue;
        }
        let far = hull
            .iter()
            .map(|p| ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len)
            .fold(0.0, f64::max);
        narrowest = narrowest.min(far);
    }
    (
        widest,
        if narrowest == f64::MAX {
            0.0
        } else {
            narrowest
        },
    )
}

/// Connected patches of `on`: each pixel's patch, and how many there are;
/// eight neighbours when `diagonal`, four otherwise.
fn components(on: &[bool], w: usize, h: usize, diagonal: bool) -> (Vec<Option<usize>>, usize) {
    let mut label = vec![None; w * h];
    let mut count = 0;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !on[start] || label[start].is_some() {
            continue;
        }
        label[start] = Some(count);
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            for (dx, dy) in [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
                (1, 1),
            ] {
                if !diagonal && dx != 0 && dy != 0 {
                    continue;
                }
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if on[j] && label[j].is_none() {
                    label[j] = Some(count);
                    stack.push(j);
                }
            }
        }
        count += 1;
    }
    (label, count)
}

/// The grey level that best splits the image in two (Otsu's).
fn otsu(px: &[f32]) -> f32 {
    let (lo, hi) = px
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    if hi <= lo {
        return lo;
    }
    const BINS: usize = 256;
    let mut hist = [0f64; BINS];
    let bin = |v: f32| (((v - lo) / (hi - lo)) * (BINS - 1) as f32).round() as usize;
    for v in px {
        hist[bin(*v)] += 1.0;
    }
    let total: f64 = hist.iter().sum();
    let sum_all: f64 = hist.iter().enumerate().map(|(i, c)| i as f64 * c).sum();
    let (mut weight, mut sum, mut best, mut at) = (0.0, 0.0, -1.0, 0);
    for (i, c) in hist.iter().enumerate() {
        weight += c;
        if weight == 0.0 || weight == total {
            continue;
        }
        sum += i as f64 * c;
        let (m0, m1) = (sum / weight, (sum_all - sum) / (total - weight));
        let between = weight * (total - weight) * (m0 - m1) * (m0 - m1);
        if between > best {
            (best, at) = (between, i);
        }
    }
    lo + (at as f32 + 0.5) / (BINS - 1) as f32 * (hi - lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A washer (radii 30 and 10) and a 60 × 20 bar turned 30° on a light
    /// ground read as two parts: the washer with its hole, its area and
    /// width; the bar with its widths and long axis.
    #[test]
    fn parts_on_a_picture_are_found_and_measured() {
        let (w, h) = (220usize, 120usize);
        let (bx, by, turn) = (160.0, 60.0, 30f64.to_radians());
        let in_bar = |x: f64, y: f64| {
            // Counter-clockwise on screen with y down.
            let (dx, dy) = (x - bx, y - by);
            let u = dx * turn.cos() - dy * turn.sin();
            let v = dx * turn.sin() + dy * turn.cos();
            u.abs() <= 30.0 && v.abs() <= 10.0
        };
        let pixels: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f64, (i / w) as f64);
                let r = (x - 55.0).hypot(y - 60.0);
                if (10.0..=30.0).contains(&r) || in_bar(x, y) {
                    0.2
                } else {
                    0.85
                }
            })
            .collect();
        let found = objects(
            &Gray {
                width: w,
                height: h,
                pixels: &pixels,
            },
            20.0,
        );
        assert_eq!(found.len(), 2, "{found:?}");
        let washer = found.iter().find(|o| o.holes == 1).unwrap();
        let ring = std::f64::consts::PI * (30.0f64.powi(2) - 10.0f64.powi(2));
        assert!((washer.area - ring).abs() / ring < 0.02, "{}", washer.area);
        assert!((washer.centroid[0] - 55.0).abs() < 0.2 && (washer.centroid[1] - 60.0).abs() < 0.2);
        assert!((washer.max_width - 61.0).abs() < 1.5 && (washer.min_width - 61.0).abs() < 2.0);
        let bar = found.iter().find(|o| o.holes == 0).unwrap();
        assert!(
            (bar.long_axis_deg - 30.0).abs() < 1.0,
            "{}",
            bar.long_axis_deg
        );
        assert!((bar.min_width - 21.0).abs() < 1.5, "{}", bar.min_width);
        assert!(
            (bar.max_width - 61.0f64.hypot(21.0)).abs() < 1.5,
            "{}",
            bar.max_width
        );
        assert!(bar.outline.len() > 100);
    }
}
