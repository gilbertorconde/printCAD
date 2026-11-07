//! Exact parameter windows and arc-length queries on B-spline carriers.

use glam::DVec2;

use crate::bspline::{ArcLengthTable, BSplineCurve, BSplinePiece, SplineParam};
use crate::world::{WorldLength, WorldOffset, WorldPoint};

/// B-spline parameter window `[t0, t1]`, possibly crossing a periodic seam.
/// Its arc length is computed once at construction.
#[derive(Debug, Clone, PartialEq)]
pub struct SplineWindow {
    curve: BSplineCurve,
    /// Normalized window (`ArcLengthTable::window`).
    window: [SplineParam; 2],
    table: ArcLengthTable,
    length: WorldLength,
    /// A full periodic turn has no endpoints.
    closed: bool,
}

/// Contact probes per smooth piece. For a sketch spline's quarter-turn tangent
/// range, sixteen chords separate neighboring intersections.
pub const SPLINE_PROBES: usize = 16;

/// Resolve stored directed parameters into the visible ascending window.
/// Periodic windows retain an unwrapped crossing of the seam; the bool says
/// whether an open carrier's endpoint order was reversed.
pub fn directed_window(
    curve: &BSplineCurve,
    parameters: [SplineParam; 2],
) -> ([SplineParam; 2], bool) {
    let [start, end] = parameters;
    let (a, b) = curve.domain();
    match curve.period() {
        Some(period) => {
            let mut span = (end.0 - start.0).rem_euclid(period);
            if span == 0.0 {
                span = period;
            }
            ([start, SplineParam(start.0 + span)], false)
        }
        None => {
            let clamp = |t: SplineParam| SplineParam(t.0.clamp(a.0, b.0));
            if start <= end {
                ([clamp(start), clamp(end)], false)
            } else {
                ([clamp(end), clamp(start)], true)
            }
        }
    }
}

impl SplineWindow {
    /// Whole curve: the open domain, or one periodic turn starting at the seam.
    pub fn whole(curve: BSplineCurve) -> Self {
        let (a, b) = curve.domain();
        let closed = curve.is_periodic();
        Self::new(curve, [a, b], closed)
    }

    /// Open window on `curve`, including a full periodic turn opened at one point.
    pub fn on_window(curve: BSplineCurve, window: [SplineParam; 2]) -> Self {
        Self::new(curve, window, false)
    }

    /// Exact bounds candidates: endpoints and axis-parallel tangencies.
    pub fn extremes(&self) -> Vec<WorldPoint> {
        let mut out = vec![self.point(self.window[0]), self.point(self.window[1])];
        for axis in 0..2 {
            out.extend(
                self.curve
                    .axis_turns(axis, self.window)
                    .into_iter()
                    .map(|t| self.point(t)),
            );
        }
        out
    }

    /// Window's carrier curve.
    pub fn curve(&self) -> &BSplineCurve {
        &self.curve
    }

    pub fn length(&self) -> WorldLength {
        self.length
    }

    /// Whether the window is a whole periodic curve without endpoints.
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// Polyline in travel order with `per_piece` segments per smooth piece.
    /// A closed window ends at its first point.
    pub fn sample(&self, per_piece: usize) -> Vec<WorldPoint> {
        assert!(
            per_piece > 0,
            "SplineWindow::sample: per_piece must be positive"
        );
        let breaks = self.curve.breaks(self.window);
        let mut out = vec![self.point(self.window[0])];
        for w in breaks.windows(2) {
            out.extend((1..=per_piece).map(|i| {
                self.point(SplineParam(
                    w[0] + (w[1] - w[0]) * i as f64 / per_piece as f64,
                ))
            }));
        }
        if self.closed {
            let first = out[0];
            *out.last_mut().unwrap() = first;
        }
        out
    }

    fn new(curve: BSplineCurve, window: [SplineParam; 2], closed: bool) -> Self {
        let table = curve.arc_table(window);
        Self {
            window: table.window(),
            length: table.length(),
            curve,
            table,
            closed,
        }
    }

    pub fn window(&self) -> [SplineParam; 2] {
        self.window
    }

    /// Parameter at arc length `s` from the start, wrapping on a closed window.
    pub fn param_at(&self, s: WorldOffset) -> SplineParam {
        let length = self.length.0;
        let (s, turns) = if self.closed {
            (s.0.rem_euclid(length), (s.0 / length).floor())
        } else {
            (s.0.clamp(0.0, length), 0.0)
        };
        let t = self.curve.param_at_length(&self.table, WorldLength::new(s));
        let period = self.curve.period().unwrap_or(0.0);
        SplineParam(t.0 + turns * period)
    }

    pub fn point(&self, t: SplineParam) -> WorldPoint {
        self.curve.point(t)
    }

    /// Nearest window parameter and distance.
    pub fn nearest(&self, p: WorldPoint) -> (SplineParam, WorldLength) {
        let (t, _, d) = self.curve.nearest_in(p, self.window);
        (t, d)
    }

    /// Arc length from the window start to its parameter `t`.
    pub fn length_to(&self, t: SplineParam) -> WorldOffset {
        WorldOffset::new(self.curve.length_at(&self.table, t).0)
    }

    /// Window segment for the solid kernel, in edge travel order.
    pub fn piece(&self, reversed: bool) -> BSplinePiece {
        self.curve.piece(self.window, reversed)
    }

    /// Samples with their arc length from the start: [`SPLINE_PROBES`] per smooth piece.
    pub fn samples(&self) -> Vec<(WorldOffset, WorldPoint)> {
        let speed = |t: f64| self.curve.derivative(SplineParam(t)).0.length();
        let breaks = self.curve.breaks(self.window);
        let mut out = vec![(WorldOffset::ZERO, self.point(self.window[0]))];
        let mut s = 0.0;
        for w in breaks.windows(2) {
            let mut prev = w[0];
            for i in 1..=SPLINE_PROBES {
                let t = w[0] + (w[1] - w[0]) * i as f64 / SPLINE_PROBES as f64;
                s += BSplineCurve::gauss(prev, t, speed);
                out.push((WorldOffset::new(s), self.point(SplineParam(t))));
                prev = t;
            }
        }
        // Piece sums and window length use the same quadrature, within rounding;
        // the last probe is exactly the window end by definition.
        out.last_mut().unwrap().0 = WorldOffset::new(self.length.0);
        out
    }

    /// Polyline with sagitta at most `error`: each smooth piece uses parameter
    /// step `√(8·error / max|C''|)`, from chord sagitta `h²·|C''| / 8`.
    pub fn polyline(&self, reversed: bool, error: WorldLength) -> Result<Vec<DVec2>, String> {
        let breaks = self.curve.breaks(self.window);
        let mut out = vec![self.point(self.window[0]).0];
        for w in breaks.windows(2) {
            let (lo, hi) = (w[0], w[1]);
            let bend = (0..=8)
                .map(|i| {
                    let t = lo + (hi - lo) * i as f64 / 8.0;
                    self.curve.second_derivative(SplineParam(t)).0.length()
                })
                .fold(0.0, f64::max);
            let count = if bend > 0.0 {
                ((hi - lo) / (8.0 * error.0 / bend).sqrt()).ceil().max(1.0)
            } else {
                1.0
            };
            if out.len() as f64 + count > 100_000.0 {
                return Err(
                    "Curve tolerance requires more than 100000 segments; increase tolerance".into(),
                );
            }
            let count = count as usize;
            out.extend((1..=count).map(|i| {
                self.point(SplineParam(lo + (hi - lo) * i as f64 / count as f64))
                    .0
            }));
        }
        if self.closed {
            let first = out[0];
            *out.last_mut().unwrap() = first;
        }
        if reversed {
            out.reverse();
        }
        Ok(out)
    }

    /// Signed area contribution `½∮ (C − origin) × dC` by Green's theorem.
    /// On a polynomial smooth piece the integrand has degree `2p − 1 <= 9`,
    /// so Gauss–Legendre quadrature is exact.
    pub fn area(&self, origin: DVec2, reversed: bool) -> f64 {
        let f = |t: f64| {
            let t = SplineParam(t);
            (self.point(t).0 - origin).perp_dot(self.curve.derivative(t).0) * 0.5
        };
        let area: f64 = self
            .curve
            .breaks(self.window)
            .windows(2)
            .map(|w| signed_gauss(w[0], w[1], f))
            .sum();
        if reversed { -area } else { area }
    }

    /// Right-ray crossings from `point`. Split at knots and y-derivative roots
    /// to obtain y-monotone pieces. Half-open intervals count a shared vertex
    /// once and a tangency zero times, as for segments in `profile_geometry`.
    pub fn crossings(&self, point: DVec2) -> usize {
        let mut cuts = self.curve.breaks(self.window);
        cuts.extend(
            self.curve
                .axis_turns(1, self.window)
                .into_iter()
                .map(|t| t.0),
        );
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let y = |t: f64| self.point(SplineParam(t)).0.y;
        cuts.windows(2)
            .filter(|w| {
                let (mut lo, mut hi) = (w[0], w[1]);
                let (ylo, yhi) = (y(lo), y(hi));
                if (ylo > point.y) == (yhi > point.y) {
                    return false;
                }
                let rising = yhi > ylo;
                for _ in 0..80 {
                    let mid = (lo + hi) * 0.5;
                    if (y(mid) > point.y) == rising {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                self.point(SplineParam((lo + hi) * 0.5)).0.x > point.x
            })
            .count()
    }
}

/// Signed polynomial integral by the eight-point Gauss–Legendre rule.
/// Polynomial exactness avoids an adaptive relative-error criterion near zero.
fn signed_gauss(lo: f64, hi: f64, f: impl Fn(f64) -> f64) -> f64 {
    const X: [f64; 4] = [
        0.183_434_642_495_649_8,
        0.525_532_409_916_329,
        0.796_666_477_413_626_7,
        0.960_289_856_497_536_3,
    ];
    const W: [f64; 4] = [
        0.362_683_783_378_362,
        0.313_706_645_877_887_3,
        0.222_381_034_453_374_5,
        0.101_228_536_290_376_3,
    ];
    let (mid, half) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
    X.iter()
        .zip(W)
        .map(|(&x, w)| w * (f(mid - half * x) + f(mid + half * x)))
        .sum::<f64>()
        * half
}
