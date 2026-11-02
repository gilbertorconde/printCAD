//! The edges of a sketch's reference pictures: found once per picture on
//! a thread of its own (the `image_edges` crate, at up to [`LONGEST_PX`]),
//! kept in the picture's pixels so a recalibrated picture moves them with
//! it, drawn over the picture as guides, and fitted with a line or a
//! circle where the user clicks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::sketch::{ReferenceImage, Vec2D};

/// The longest side edges are found at, in pixels.
const LONGEST_PX: u32 = 4096;

/// A picture's edges, in its pixels.
#[derive(Debug)]
pub struct EdgeSet {
    pub width: usize,
    pub height: usize,
    pub chains: Vec<image_edges::Chain>,
}

enum State {
    Running,
    Done(Option<Arc<EdgeSet>>),
}

/// Every picture's edges, by asset.
#[derive(Default)]
pub struct Edges(Arc<Mutex<HashMap<Uuid, State>>>);

impl Edges {
    /// The asset's edges once found; the first ask starts finding them.
    pub fn get(
        &self,
        asset: Uuid,
        bytes: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Option<Arc<EdgeSet>> {
        let mut all = self.0.lock().ok()?;
        match all.get(&asset) {
            Some(State::Done(found)) => return found.clone(),
            Some(State::Running) => return None,
            None => {}
        }
        let Some(bytes) = bytes() else {
            all.insert(asset, State::Done(None));
            return None;
        };
        all.insert(asset, State::Running);
        let shared = Arc::clone(&self.0);
        let spawned = std::thread::Builder::new()
            .name("printcad-picture-edges".into())
            .spawn(move || {
                let found = find(&bytes).map(Arc::new);
                if let Ok(mut all) = shared.lock() {
                    all.insert(asset, State::Done(found));
                }
            });
        if spawned.is_err() {
            all.insert(asset, State::Done(None));
        }
        None
    }

    /// The asset's edges, found now on this thread when they are not yet.
    pub fn now(
        &self,
        asset: Uuid,
        bytes: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Option<Arc<EdgeSet>> {
        if let Some(State::Done(found)) = self.0.lock().ok()?.get(&asset) {
            return found.clone();
        }
        let found = bytes().and_then(|b| find(&b)).map(Arc::new);
        if let Ok(mut all) = self.0.lock() {
            all.insert(asset, State::Done(found.clone()));
        }
        found
    }

    /// Whether edges are still being found.
    pub fn running(&self) -> bool {
        self.0
            .lock()
            .is_ok_and(|all| all.values().any(|s| matches!(s, State::Running)))
    }
}

fn find(bytes: &[u8]) -> Option<EdgeSet> {
    let picture = image::load_from_memory(bytes).ok()?;
    let picture = if picture.width().max(picture.height()) > LONGEST_PX {
        picture.resize(
            LONGEST_PX,
            LONGEST_PX,
            image::imageops::FilterType::Triangle,
        )
    } else {
        picture
    };
    let gray = picture.to_luma32f();
    let (width, height) = (gray.width() as usize, gray.height() as usize);
    let chains = image_edges::detect(
        &image_edges::Gray {
            width,
            height,
            pixels: gray.as_raw(),
        },
        &image_edges::Settings::default(),
    );
    Some(EdgeSet {
        width,
        height,
        chains,
    })
}

/// A pixel of `set` where `image` lies on the sketch, in sketch units.
pub fn to_sketch(image: &ReferenceImage, set: &EdgeSet, p: [f64; 2]) -> Vec2D {
    let (w, h) = (set.width.max(1) as f64, set.height.max(1) as f64);
    let scale = image.width / w;
    // Pixel centres sit half a pixel in from the picture's corner.
    let x = (p[0] + 0.5 - w / 2.0) * scale;
    let y = (h / 2.0 - (p[1] + 0.5)) * scale;
    let (sin, cos) = image.angle_deg.to_radians().sin_cos();
    Vec2D::new(
        image.center.x + x * cos - y * sin,
        image.center.y + x * sin + y * cos,
    )
}

/// What a fit to an edge came to, in sketch units.
#[derive(Debug, Clone, PartialEq)]
pub enum Fit {
    Line {
        from: Vec2D,
        to: Vec2D,
        sigma: f64,
        points: usize,
    },
    Circle {
        centre: Vec2D,
        radius: f64,
        sigma: f64,
        points: usize,
    },
}

/// The chain and point of `set` nearest `at`, within `near` of it.
fn nearest(image: &ReferenceImage, set: &EdgeSet, at: Vec2D, near: f64) -> Option<(usize, usize)> {
    let mut best: Option<(f64, usize, usize)> = None;
    for (c, chain) in set.chains.iter().enumerate() {
        for (k, p) in chain.points.iter().enumerate() {
            let q = to_sketch(image, set, *p);
            let d = (q.x - at.x).hypot(q.y - at.y);
            if best.is_none_or(|(b, _, _)| d < b) {
                best = Some((d, c, k));
            }
        }
    }
    best.filter(|(d, _, _)| *d <= near).map(|(_, c, k)| (c, k))
}

/// The points either side of point `k` of `chain` that keep to the shape
/// `circle` says: a dozen round it to start, then on both ways while the
/// next lies within the run's own noise of the shape fitted so far.
fn grow(image: &ReferenceImage, set: &EdgeSet, chain: usize, k: usize, circle: bool) -> Vec<Vec2D> {
    let chain = &set.chains[chain];
    let n = chain.points.len();
    let at = |i: usize| to_sketch(image, set, chain.points[i]);
    let pixel = image.width / set.width.max(1) as f64;
    let step = |i: usize, fwd: bool| -> Option<usize> {
        match (fwd, chain.closed) {
            (true, true) => Some((i + 1) % n),
            (false, true) => Some((i + n - 1) % n),
            (true, false) => (i + 1 < n).then_some(i + 1),
            (false, false) => i.checked_sub(1),
        }
    };
    let shape = |run: &std::collections::VecDeque<Vec2D>| {
        let pts: Vec<Vec2D> = run.iter().copied().collect();
        if circle {
            circle_fit(&pts)
        } else {
            Some(line_fit(&pts))
        }
    };
    let mut run = std::collections::VecDeque::from([at(k)]);
    let (mut lo, mut hi, mut taken) = (k, k, 1);
    // A dozen to start, whatever their shape.
    while taken < n.min(12) {
        let mut grew = false;
        if let Some(i) = step(hi, true).filter(|_| taken < n) {
            run.push_back(at(i));
            (hi, taken, grew) = (i, taken + 1, true);
        }
        if let Some(i) = step(lo, false).filter(|_| taken < n) {
            run.push_front(at(i));
            (lo, taken, grew) = (i, taken + 1, true);
        }
        if !grew {
            break;
        }
    }
    let mut fitted = shape(&run);
    let (mut fwd_open, mut back_open) = (true, true);
    let mut since = 0;
    while (fwd_open || back_open) && taken < n {
        let Some(f) = fitted.clone() else {
            break;
        };
        let band = (4.0 * f.sigma()).max(0.3 * pixel);
        for fwd in [true, false] {
            let open = if fwd { &mut fwd_open } else { &mut back_open };
            if !*open || taken >= n {
                continue;
            }
            let end = if fwd { hi } else { lo };
            match step(end, fwd) {
                Some(i) if f.distance(at(i)) <= band => {
                    if fwd {
                        run.push_back(at(i));
                        hi = i;
                    } else {
                        run.push_front(at(i));
                        lo = i;
                    }
                    taken += 1;
                    since += 1;
                }
                _ => *open = false,
            }
        }
        if since >= 8 {
            fitted = shape(&run);
            since = 0;
        }
    }
    run.into()
}

/// A line or a circle fitted to the edge where the user clicked (within
/// `near` of it): `want` says which, or `None` the one that runs further.
/// A line is refused on a curve and a circle on less than 8° of round.
pub fn fit(
    image: &ReferenceImage,
    set: &EdgeSet,
    at: Vec2D,
    near: f64,
    want: Option<bool>,
) -> Result<Fit, String> {
    let (chain, k) = nearest(image, set, at, near).ok_or("no edge there")?;
    let straight = grow(image, set, chain, k, false);
    let round = grow(image, set, chain, k, true);
    let as_circle = match want {
        Some(circle) => circle,
        None => round.len() as f64 > 1.5 * straight.len() as f64,
    };
    if as_circle {
        let c = circle_fit(&round).ok_or("the edge there does not bend")?;
        if let Fit::Circle { centre, .. } = &c {
            let angle = |p: &Vec2D| (p.y - centre.y).atan2(p.x - centre.x);
            let mut turned = 0.0;
            for w in round.windows(2) {
                let d = angle(&w[1]) - angle(&w[0]);
                turned += (d + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI;
            }
            if turned.abs() < 8f64.to_radians() {
                return Err("the edge there covers too little of a circle to fit one".into());
            }
        }
        return Ok(c);
    }
    // Curved: the edge runs much further as a circle than as a line.
    if straight.len() < 12 || round.len() as f64 > 3.0 * straight.len() as f64 {
        return Err("the edge there is not straight".into());
    }
    Ok(line_fit(&straight))
}

impl Fit {
    /// How far `p` lies from the line or the circle.
    pub fn distance(&self, p: Vec2D) -> f64 {
        match self {
            Fit::Line { from, to, .. } => {
                let (dx, dy) = (to.x - from.x, to.y - from.y);
                let len = dx.hypot(dy).max(1e-12);
                ((p.x - from.x) * dy - (p.y - from.y) * dx).abs() / len
            }
            Fit::Circle { centre, radius, .. } => {
                ((p.x - centre.x).hypot(p.y - centre.y) - radius).abs()
            }
        }
    }

    /// How far the edge's points stray from it, root mean square.
    pub fn sigma(&self) -> f64 {
        match self {
            Fit::Line { sigma, .. } | Fit::Circle { sigma, .. } => *sigma,
        }
    }
}

fn line_fit(points: &[Vec2D]) -> Fit {
    let n = points.len() as f64;
    let (mx, my) = points
        .iter()
        .fold((0.0, 0.0), |(a, b), p| (a + p.x / n, b + p.y / n));
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for p in points {
        let (dx, dy) = (p.x - mx, p.y - my);
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
    }
    let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (dx, dy) = (angle.cos(), angle.sin());
    let along = |p: &Vec2D| (p.x - mx) * dx + (p.y - my) * dy;
    let off = |p: &Vec2D| -(p.x - mx) * dy + (p.y - my) * dx;
    let (lo, hi) = points
        .iter()
        .map(along)
        .fold((f64::MAX, f64::MIN), |(a, b), t| (a.min(t), b.max(t)));
    let sigma = (points.iter().map(|p| off(p).powi(2)).sum::<f64>() / n).sqrt();
    Fit::Line {
        from: Vec2D::new(mx + lo * dx, my + lo * dy),
        to: Vec2D::new(mx + hi * dx, my + hi * dy),
        sigma,
        points: points.len(),
    }
}

fn circle_fit(points: &[Vec2D]) -> Option<Fit> {
    let n = points.len() as f64;
    let (mx, my) = points
        .iter()
        .fold((0.0, 0.0), |(a, b), p| (a + p.x / n, b + p.y / n));
    // Kasa's algebraic fit about the mean, then refined geometrically.
    let (mut suu, mut suv, mut svv, mut suuu, mut svvv, mut suvv, mut svuu) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for p in points {
        let (u, v) = (p.x - mx, p.y - my);
        suu += u * u;
        suv += u * v;
        svv += v * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }
    let det = suu * svv - suv * suv;
    if det.abs() < 1e-18 {
        return None;
    }
    let (a, b) = (0.5 * (suuu + suvv), 0.5 * (svvv + svuu));
    let (mut cx, mut cy) = (
        mx + (a * svv - b * suv) / det,
        my + (b * suu - a * suv) / det,
    );
    // Gauss-Newton on each point's distance from the circle.
    let mut radius = points
        .iter()
        .map(|p| (p.x - cx).hypot(p.y - cy))
        .sum::<f64>()
        / n;
    for _ in 0..30 {
        let (mut m, mut g) = ([[0.0; 3]; 3], [0.0; 3]);
        for p in points {
            let d = (p.x - cx).hypot(p.y - cy).max(1e-12);
            let row = [-(p.x - cx) / d, -(p.y - cy) / d, -1.0];
            let r = d - radius;
            for i in 0..3 {
                g[i] += row[i] * r;
                for k in 0..3 {
                    m[i][k] += row[i] * row[k];
                }
            }
        }
        let Some(step) = solve3(m, g.map(|v| -v)) else {
            break;
        };
        (cx, cy, radius) = (cx + step[0], cy + step[1], radius + step[2]);
        if step.iter().all(|s| s.abs() < 1e-12) {
            break;
        }
    }
    let sigma = (points
        .iter()
        .map(|p| ((p.x - cx).hypot(p.y - cy) - radius).powi(2))
        .sum::<f64>()
        / n)
        .sqrt();
    radius.is_finite().then_some(Fit::Circle {
        centre: Vec2D::new(cx, cy),
        radius,
        sigma,
        points: points.len(),
    })
}

/// `m x = b` for a 3 × 3 system, by Cramer's rule.
fn solve3(m: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(m);
    if d.abs() < 1e-24 {
        return None;
    }
    Some(std::array::from_fn(|k| {
        let mut mk = m;
        for i in 0..3 {
            mk[i][k] = b[i];
        }
        det(mk) / d
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture 100 mm wide of a dark disc: a click on its rim fits its
    /// circle, a line there is refused, and a click far from any edge
    /// finds none.
    #[test]
    fn a_click_on_a_rim_fits_its_circle() {
        let (w, h) = (200usize, 160usize);
        let (cx, cy, r) = (90.3, 75.6, 40.0);
        let pixels: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f64, (i / w) as f64);
                let mut inside = 0;
                for a in 0..8 {
                    for b in 0..8 {
                        let (sx, sy) =
                            (x - 0.4375 + a as f64 * 0.125, y - 0.4375 + b as f64 * 0.125);
                        inside += usize::from((sx - cx).hypot(sy - cy) <= r);
                    }
                }
                0.9 - 0.7 * inside as f32 / 64.0
            })
            .collect();
        let set = EdgeSet {
            width: w,
            height: h,
            chains: image_edges::detect(
                &image_edges::Gray {
                    width: w,
                    height: h,
                    pixels: &pixels,
                },
                &image_edges::Settings::default(),
            ),
        };
        let image = ReferenceImage {
            id: Uuid::nil(),
            asset: Uuid::nil(),
            center: Vec2D::new(0.0, 0.0),
            width: 100.0,
            angle_deg: 0.0,
            opacity: 0.5,
        };
        let mm = 100.0 / w as f64;
        let centre = to_sketch(&image, &set, [cx, cy]);
        let on_rim = Vec2D::new(centre.x + r * mm, centre.y);
        // The fit runs round the whole rim: its circle within a tenth of a
        // pixel.
        match fit(&image, &set, on_rim, 2.0, None).unwrap() {
            Fit::Circle {
                centre: c,
                radius,
                points,
                ..
            } => {
                assert!(points > 200, "{points}");
                assert!((radius - r * mm).abs() < 0.1 * mm, "{radius}");
                assert!((c.x - centre.x).hypot(c.y - centre.y) < 0.1 * mm);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            fit(&image, &set, on_rim, 2.0, Some(false)).is_err(),
            "not straight"
        );
        assert!(
            fit(&image, &set, centre, 2.0, None).is_err(),
            "no edge at the middle"
        );
    }
}
