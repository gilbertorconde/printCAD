//! Where a [`SolidOp::Pattern`]'s occurrences stand: its steps composed
//! one after another, a path's stations read off its wire, or off an edge
//! of the solid as it stands where the pattern does.

use kernel_api::{PathLayout, PatternPath, PatternStep, SolidOp};
use ogeom::topo::{Model, Shape};

use crate::profile::SegmentCurve;

/// A row-major affine transform.
pub type Mat4 = [[f64; 4]; 4];

/// Samples a path is walked through, beside its stations, to carry an
/// aligned copy's turn along it without twisting.
const TRANSPORT_SAMPLES: u32 = 256;

/// The most occurrences one path step makes.
const MAX_ALONG: u32 = 10_000;

/// A turn, as a row-major 3 × 3 matrix.
type Turn = [[f64; 3]; 3];

/// Where a station stands, and how the path has turned on the way there.
type Place = ([f64; 3], Turn);

const NO_TURN: Turn = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

const IDENTITY: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// `SolidOp::Pattern { steps, originals, skipped }` as the
/// [`SolidOp::Transform`] (or [`SolidOp::TransformCopies`], the original
/// skipped) it stands for where `solid` is the running solid.
pub fn resolve(
    model: &mut Model,
    solid: Option<&Shape>,
    steps: &[PatternStep],
    originals: &[usize],
    skipped: &[u32],
) -> Result<SolidOp, String> {
    let transforms: Vec<Mat4> = transforms(model, solid, steps)?
        .into_iter()
        .zip(0u32..)
        .skip(1)
        .filter(|(_, index)| !skipped.contains(index))
        .map(|(transform, _)| transform)
        .collect();
    let originals = originals.to_vec();
    Ok(if skipped.contains(&0) {
        SolidOp::TransformCopies {
            transforms,
            originals,
        }
    } else {
        SolidOp::Transform {
            transforms,
            originals,
        }
    })
}

/// Every occurrence's transform, the original's (the identity) first:
/// each step applied to every occurrence of the steps before it.
pub fn transforms(
    model: &mut Model,
    solid: Option<&Shape>,
    steps: &[PatternStep],
) -> Result<Vec<Mat4>, String> {
    if steps.is_empty() {
        return Err("a pattern needs at least one step".into());
    }
    let mut occurrences = vec![IDENTITY];
    for step in steps {
        let own = match step {
            PatternStep::Transforms(list) => list.clone(),
            PatternStep::Path(layout) => along(model, solid, layout)?,
        };
        let mut next = Vec::with_capacity(occurrences.len() * (own.len() + 1));
        for base in &occurrences {
            next.push(*base);
            for t in &own {
                next.push(mul(t, base));
            }
        }
        occurrences = next;
    }
    Ok(occurrences)
}

/// Where `count` occurrences stand along a path `length` long, as
/// fractions of it from its start, the first at 0: `spacing` millimetres
/// apart, or spread over the whole of it (round it evenly when `closed`).
pub fn stations(
    length: f64,
    closed: bool,
    count: u32,
    spacing: Option<f64>,
) -> Result<Vec<f64>, String> {
    if !(1..=MAX_ALONG).contains(&count) {
        return Err(format!(
            "a path pattern takes 1 to {MAX_ALONG} occurrences, not {count}"
        ));
    }
    if !(length.is_finite() && length > 0.0) {
        return Err("the path has no length".into());
    }
    let n = f64::from(count);
    let step = match spacing {
        Some(s) if s.is_finite() && s > 0.0 => s / length,
        Some(s) => {
            return Err(format!(
                "the spacing along the path, {s} mm, is not positive"
            ));
        }
        None if closed => 1.0 / n,
        None if count == 1 => 0.0,
        None => 1.0 / (n - 1.0),
    };
    // A closed path's last place is short of its start, which the
    // original holds.
    if step > 0.0 {
        let fits = (1.0 / step + 1e-9).floor() as u32 + u32::from(!closed);
        if count > fits {
            return Err(format!(
                "the path, {length:.3} mm long, holds {fits} occurrences at that spacing, \
                 not {count}"
            ));
        }
    }
    Ok((0..count).map(|k| (step * f64::from(k)).min(1.0)).collect())
}

/// The copies a path step makes, the original's place left out.
fn along(
    model: &mut Model,
    solid: Option<&Shape>,
    layout: &PathLayout,
) -> Result<Vec<Mat4>, String> {
    let (curves, placement): (Vec<SegmentCurve>, Option<ogeom::math::Transform>) = match &layout
        .path
    {
        PatternPath::Wire { plane, wire } => (crate::profile::wire_curves(plane, wire)?, None),
        PatternPath::Edge(probe) => {
            let solid = solid.ok_or("a pattern along an edge needs a solid to find it on")?;
            let edge =
                crate::probe::find_edge(model, solid, probe.point, probe.direction, probe.faces)
                    .map_err(|e| format!("the pattern's path: {e}"))?;
            let (curve, placement) =
                crate::queries::edge_curve(model, &edge).map_err(|e| e.to_string())?;
            (vec![curve], Some(placement))
        }
    };
    let frames = |at: &[f64]| {
        crate::queries::frames_along(&curves, placement.as_ref(), at).map_err(|e| e.to_string())
    };
    let whole = frames(&[])?;
    let walked = stations(whole.length, whole.closed, layout.count, layout.spacing)?;
    // Walked from the start, every place a station or a sample, so a turn
    // carried from one to the next stays small.
    let mut asked: Vec<(f64, Option<usize>)> = walked
        .iter()
        .enumerate()
        .map(|(k, &w)| (w, Some(k)))
        .collect();
    if layout.align {
        asked.extend(
            (0..=TRANSPORT_SAMPLES).map(|i| (f64::from(i) / f64::from(TRANSPORT_SAMPLES), None)),
        );
    }
    asked.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.is_some().cmp(&a.1.is_some())));
    let on_path: Vec<f64> = asked
        .iter()
        .map(|(w, _)| if layout.reversed { 1.0 - w } else { *w })
        .collect();
    let found = frames(&on_path)?.frames;
    let sign = if layout.reversed { -1.0 } else { 1.0 };
    let mut places: Vec<Option<Place>> = vec![None; walked.len()];
    let mut turn = NO_TURN;
    let mut last: Option<[f64; 3]> = None;
    for ((_, station), frame) in asked.iter().zip(&found) {
        let tangent = frame.tangent.map(|c| c * sign);
        if let Some(before) = last {
            turn = mul3(&arc(before, tangent), &turn);
        }
        last = Some(tangent);
        if let Some(k) = station
            && places[*k].is_none()
        {
            places[*k] = Some((frame.point, turn));
        }
    }
    let places: Vec<Place> = places
        .into_iter()
        .map(|p| p.ok_or_else(|| "a station of the path was not reached".to_string()))
        .collect::<Result<_, _>>()?;
    let (start, _) = places[0];
    Ok(places[1..]
        .iter()
        .map(|(point, r)| {
            let r = if layout.align { *r } else { NO_TURN };
            // Moved from the first station to its own, turned about it.
            let mut m = IDENTITY;
            for i in 0..3 {
                m[i][..3].copy_from_slice(&r[i]);
                m[i][3] = point[i] - (0..3).map(|j| r[i][j] * start[j]).sum::<f64>();
            }
            m
        })
        .collect())
}

/// The shortest turn taking unit `a` to unit `b`.
fn arc(a: [f64; 3], b: [f64; 3]) -> Turn {
    let v = cross(a, b);
    let c = dot(a, b);
    if c <= -1.0 + 1e-12 {
        // Turned right round: a half turn about any line square to `a`.
        let helper = if a[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let n = cross(a, helper);
        let len = dot(n, n).sqrt();
        let n = n.map(|x| x / len);
        return std::array::from_fn(|i| {
            std::array::from_fn(|j| 2.0 * n[i] * n[j] - if i == j { 1.0 } else { 0.0 })
        });
    }
    let k = 1.0 / (1.0 + c);
    let skew = [[0.0, -v[2], v[1]], [v[2], 0.0, -v[0]], [-v[1], v[0], 0.0]];
    let skew2 = mul3(&skew, &skew);
    std::array::from_fn(|i| {
        std::array::from_fn(|j| (if i == j { 1.0 } else { 0.0 }) + skew[i][j] + skew2[i][j] * k)
    })
}

fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| a[i][k] * b[k][j]).sum()))
}

fn mul3(a: &Turn, b: &Turn) -> Turn {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_api::{ProfilePlane, ProfileSegment, ProfileWire};

    fn moved(m: &Mat4, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| (0..3).map(|j| m[i][j] * p[j]).sum::<f64>() + m[i][3])
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-6)
    }

    fn circle(radius: f64) -> PatternPath {
        PatternPath::Wire {
            plane: ProfilePlane::default(),
            wire: ProfileWire::new(vec![ProfileSegment::Circle {
                center: [0.0, 0.0],
                radius,
            }]),
        }
    }

    fn layout(path: PatternPath, count: u32, align: bool) -> PathLayout {
        PathLayout {
            path,
            count,
            spacing: None,
            reversed: false,
            align,
        }
    }

    /// Spread over a closed path, the places go evenly round it, the last
    /// short of the start; spaced, as many as the length holds.
    #[test]
    fn stations_spread_over_the_path_or_spaced_along_it() {
        assert_eq!(
            stations(40.0, true, 4, None).unwrap(),
            vec![0.0, 0.25, 0.5, 0.75]
        );
        assert_eq!(stations(40.0, false, 3, None).unwrap(), vec![0.0, 0.5, 1.0]);
        assert_eq!(
            stations(40.0, false, 3, Some(10.0)).unwrap(),
            vec![0.0, 0.25, 0.5]
        );
        assert!(stations(40.0, false, 6, Some(10.0)).is_err(), "five fit");
        assert!(
            stations(40.0, true, 5, Some(10.0)).is_err(),
            "four fit round"
        );
        assert!(stations(40.0, false, 0, None).is_err());
    }

    /// Round a circle, aligned: each copy moved to its station and turned
    /// with the circle, so a point at the circle's start goes round it and
    /// the centre stays put.
    #[test]
    fn copies_round_a_circle_turn_with_it_when_aligned() {
        let mut model = Model::new();
        let ts = along(&mut model, None, &layout(circle(10.0), 4, true)).unwrap();
        assert_eq!(ts.len(), 3);
        let start = [10.0, 0.0, 0.0];
        let places: Vec<[f64; 3]> = ts.iter().map(|t| moved(t, start)).collect();
        assert!(close(places[1], [-10.0, 0.0, 0.0]), "{places:?}");
        for t in &ts {
            assert!(close(moved(t, [0.0; 3]), [0.0; 3]), "the centre stays");
        }
        // Unaligned, the copies only move.
        let ts = along(&mut model, None, &layout(circle(10.0), 4, false)).unwrap();
        assert!(close(moved(&ts[1], [0.0; 3]), [-20.0, 0.0, 0.0]));
    }

    /// Reversed, the path is walked from its end: along a line from 0 to
    /// 30 on X, three copies over it stand at 30, 15 and 0 from the end.
    #[test]
    fn a_reversed_path_is_walked_from_its_end() {
        let line = PatternPath::Wire {
            plane: ProfilePlane::default(),
            wire: ProfileWire::new(vec![ProfileSegment::Line {
                start: [0.0, 0.0],
                end: [30.0, 0.0],
            }]),
        };
        let mut model = Model::new();
        let mut l = layout(line, 3, true);
        l.reversed = true;
        let ts = along(&mut model, None, &l).unwrap();
        assert!(close(moved(&ts[0], [30.0, 0.0, 0.0]), [15.0, 0.0, 0.0]));
        assert!(close(moved(&ts[1], [30.0, 0.0, 0.0]), [0.0, 0.0, 0.0]));
    }

    /// Steps compose as a multi-transform's do: the path's copies of every
    /// occurrence before.
    #[test]
    fn steps_compose_each_on_every_occurrence_before() {
        let mut model = Model::new();
        let shift: Mat4 = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 5.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let ts = transforms(
            &mut model,
            None,
            &[
                PatternStep::Transforms(vec![shift]),
                PatternStep::Path(layout(circle(10.0), 2, false)),
            ],
        )
        .unwrap();
        assert_eq!(ts.len(), 4);
        assert!(close(moved(&ts[3], [10.0, 0.0, 0.0]), [-10.0, 0.0, 5.0]));
    }
}
