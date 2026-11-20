//! A rib: a web grown from an open chain of a sketch's lines and arcs
//! until it meets the solid.
//!
//! Across the plane, the chain closed by its end tangents and a far side
//! makes a large face on each side of it, extruded by the thickness along
//! the plane's normal; along the normal, the chain thickened in the plane
//! is extruded far each way. What of each such slab lies outside the solid
//! and touches the chain is the web on that side, kept where the solid
//! bounds it: a piece that runs out to the slab's far side met nothing,
//! unless its ends ran on past walls shorter than the line, round which it
//! leaked: the slab cut off square where the solid's corners end along
//! the line keeps the web within them.

use kernel_api::{
    ExtrudeTermination, Profile, ProfileSegment, ProfileWire, RibForm, RibSide, SweepKind,
};
use ogeom::math::{Point, Vector};
use ogeom::topo::{Model, Shape, ShapeType};

use super::{combine_solids, wrap_pieces};
use crate::tess;

type V2 = [f64; 2];

fn sub(a: V2, b: V2) -> V2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: V2, b: V2) -> V2 {
    [a[0] + b[0], a[1] + b[1]]
}

fn scale(a: V2, s: f64) -> V2 {
    [a[0] * s, a[1] * s]
}

fn length(a: V2) -> f64 {
    a[0].hypot(a[1])
}

fn unit(a: V2) -> V2 {
    scale(a, 1.0 / length(a).max(1e-300))
}

/// `a` turned a quarter turn to the left.
fn left(a: V2) -> V2 {
    [-a[1], a[0]]
}

fn cross(a: V2, b: V2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot(a: V2, b: V2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn same(a: V2, b: V2) -> bool {
    length(sub(a, b)) < 1e-9
}

/// One piece of the chain: a line, or an arc through three points.
#[derive(Debug, Clone, Copy)]
enum Piece {
    Line { a: V2, b: V2 },
    Arc { a: V2, m: V2, b: V2 },
}

impl Piece {
    fn of(segment: &ProfileSegment) -> Result<Self, String> {
        match *segment {
            ProfileSegment::Line { start, end } => Ok(Piece::Line { a: start, b: end }),
            ProfileSegment::Arc { start, mid, end } => Ok(Piece::Arc {
                a: start,
                m: mid,
                b: end,
            }),
            _ => Err("a rib's sketch may hold only lines and arcs".into()),
        }
    }

    fn start(&self) -> V2 {
        match *self {
            Piece::Line { a, .. } | Piece::Arc { a, .. } => a,
        }
    }

    fn end(&self) -> V2 {
        match *self {
            Piece::Line { b, .. } | Piece::Arc { b, .. } => b,
        }
    }

    fn middle(&self) -> V2 {
        match *self {
            Piece::Line { a, b } => scale(add(a, b), 0.5),
            Piece::Arc { m, .. } => m,
        }
    }

    /// The arc's centre and whether it turns left (anticlockwise).
    fn circle(a: V2, m: V2, b: V2) -> Option<(V2, bool)> {
        let d = 2.0 * (a[0] * (m[1] - b[1]) + m[0] * (b[1] - a[1]) + b[0] * (a[1] - m[1]));
        if d.abs() < 1e-12 {
            return None;
        }
        let sq = |p: V2| p[0] * p[0] + p[1] * p[1];
        let ux = (sq(a) * (m[1] - b[1]) + sq(m) * (b[1] - a[1]) + sq(b) * (a[1] - m[1])) / d;
        let uy = (sq(a) * (b[0] - m[0]) + sq(m) * (a[0] - b[0]) + sq(b) * (m[0] - a[0])) / d;
        Some(([ux, uy], cross(sub(m, a), sub(b, m)) > 0.0))
    }

    /// The way the piece runs at `p`, one of its points, as a unit vector.
    fn tangent_at(&self, p: V2) -> V2 {
        match *self {
            Piece::Line { a, b } => unit(sub(b, a)),
            Piece::Arc { a, m, b } => match Self::circle(a, m, b) {
                Some((c, turns_left)) => {
                    let t = left(unit(sub(p, c)));
                    if turns_left { t } else { scale(t, -1.0) }
                }
                None => unit(sub(b, a)),
            },
        }
    }

    /// The piece moved `h` to its left (right for a negative `h`).
    fn offset(&self, h: f64) -> Result<Self, String> {
        let at = |p: V2| add(p, scale(left(self.tangent_at(p)), h));
        if let Piece::Arc { a, m, b } = *self
            && let Some((c, _)) = Self::circle(a, m, b)
        {
            let r = length(sub(a, c));
            let moved = length(sub(at(a), c));
            if moved < r * 1e-6 || (moved - r).abs() > h.abs() * 1.5 {
                return Err("the rib is thicker than the radius of an arc it follows".into());
            }
        }
        Ok(match *self {
            Piece::Line { a, b } => Piece::Line { a: at(a), b: at(b) },
            Piece::Arc { a, m, b } => Piece::Arc {
                a: at(a),
                m: at(m),
                b: at(b),
            },
        })
    }

    fn reversed(&self) -> Self {
        match *self {
            Piece::Line { a, b } => Piece::Line { a: b, b: a },
            Piece::Arc { a, m, b } => Piece::Arc { a: b, m, b: a },
        }
    }

    fn segment(&self) -> ProfileSegment {
        match *self {
            Piece::Line { a, b } => ProfileSegment::Line { start: a, end: b },
            Piece::Arc { a, m, b } => ProfileSegment::Arc {
                start: a,
                mid: m,
                end: b,
            },
        }
    }
}

/// Where two lines through `p` along `d` and through `q` along `e` meet.
fn meet(p: V2, d: V2, q: V2, e: V2) -> Option<V2> {
    let den = cross(d, e);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = cross(sub(q, p), e) / den;
    Some(add(p, scale(d, t)))
}

/// The chain moved `h` to its left, its pieces joined again: two lines
/// meet where they run on to, any other pair by a straight piece across
/// the gap.
fn offset_chain(chain: &[Piece], h: f64) -> Result<Vec<Piece>, String> {
    if h == 0.0 {
        return Ok(chain.to_vec());
    }
    let moved = chain
        .iter()
        .map(|p| p.offset(h))
        .collect::<Result<Vec<_>, _>>()?;
    let mut out: Vec<Piece> = Vec::with_capacity(moved.len() * 2);
    for next in moved {
        let Some(last) = out.last_mut() else {
            out.push(next);
            continue;
        };
        let (end, start) = (last.end(), next.start());
        if same(end, start) {
            out.push(next);
            continue;
        }
        if let (Piece::Line { a, b }, Piece::Line { a: c, b: d }) = (*last, next)
            && let Some(x) = meet(a, sub(b, a), c, sub(d, c))
        {
            *last = Piece::Line { a, b: x };
            out.push(Piece::Line { a: x, b: d });
            continue;
        }
        out.push(Piece::Line { a: end, b: start });
        out.push(next);
    }
    Ok(out)
}

/// A closed wire of `pieces`, in order.
fn wire(pieces: &[Piece]) -> ProfileWire {
    ProfileWire::new(pieces.iter().map(Piece::segment).collect())
}

/// The tool a [`kernel_api::SolidOp::Rib`] fuses onto `base`: the web on
/// every side of the chain where the solid bounds it.
pub fn tool(
    model: &mut Model,
    base: &Shape,
    profile: &Profile,
    form: &RibForm,
) -> Result<Shape, String> {
    let thickness = form.thickness;
    if !(thickness > 0.0 && thickness.is_finite()) {
        return Err("the rib's thickness must be more than zero".into());
    }
    let [wire_in] = profile.wires.as_slice() else {
        return Err("a rib grows from one chain of lines and arcs".into());
    };
    let chain = wire_in
        .segments
        .iter()
        .map(Piece::of)
        .collect::<Result<Vec<_>, _>>()?;
    let (Some(first), Some(last)) = (chain.first(), chain.last()) else {
        return Err("the rib's sketch has no curves".into());
    };
    let (s0, e1) = (first.start(), last.end());
    if same(s0, e1) {
        return Err("a rib grows from an open line or chain: its sketch closes on itself".into());
    }
    let plane = profile.plane;
    let world = |p: V2| {
        Point::new(
            plane.origin[0] + plane.x_axis[0] * p[0] + plane.y_axis[0] * p[1],
            plane.origin[1] + plane.x_axis[1] * p[0] + plane.y_axis[1] * p[1],
            plane.origin[2] + plane.x_axis[2] * p[0] + plane.y_axis[2] * p[1],
        )
    };
    let normal = Vector::new(plane.normal[0], plane.normal[1], plane.normal[2]);
    let normal = normal / normal.magnitude().max(1e-300);

    // Far enough that a slab's far side lies well clear of the solid.
    let (lo, hi) = tess::robust_bounds(model, base).ok_or("the solid has no bounds")?;
    let centre = Point::new(
        (lo.x + hi.x) * 0.5,
        (lo.y + hi.y) * 0.5,
        (lo.z + hi.z) * 0.5,
    );
    let middle = chain[chain.len() / 2].middle();
    let span = length(sub(e1, s0));
    let reach = 4.0 * ((hi - lo).magnitude() + span + (world(middle) - centre).magnitude()) + 10.0;

    // Where the thickness lies: its signed offsets from the chain.
    let (near_side, far_side) = match form.side {
        RibSide::Both => (thickness * 0.5, -thickness * 0.5),
        RibSide::Front => (thickness, 0.0),
        RibSide::Back => (0.0, -thickness),
    };
    let mid_thickness = (near_side + far_side) * 0.5;

    // How far each end of the chain runs on along its tangent before it
    // passes the solid's corners: where a web whose line runs on past
    // short walls is cut off, square to its line, rather than leaking
    // round them.
    let corners: Vec<V2> = ogeom::topo::explore_unique(model, base, ShapeType::Vertex)
        .map_err(|e| e.to_string())?
        .iter()
        .filter_map(|v| {
            let p = v
                .transform(model.datums())
                .ok()?
                .apply(model.node(v)?.data().as_vertex()?.point);
            let d = [
                p.x - plane.origin[0],
                p.y - plane.origin[1],
                p.z - plane.origin[2],
            ];
            let on = |axis: [f64; 3]| d[0] * axis[0] + d[1] * axis[1] + d[2] * axis[2];
            Some([on(plane.x_axis), on(plane.y_axis)])
        })
        .collect();
    let run_past = |from: V2, way: V2| {
        corners
            .iter()
            .map(|c| dot(sub(*c, from), way))
            .fold(0.0, f64::max)
    };
    let (d0, d1) = (first.tangent_at(s0), last.tangent_at(e1));
    let to_walls = (run_past(s0, scale(d0, -1.0)), run_past(e1, d1));

    // The slab on side `way` of the chain, its ends run on by `runs`, and
    // a point beside the chain inside it.
    let slab = |model: &mut Model, way: f64, runs: (f64, f64)| -> Result<(Shape, Point), String> {
        let (slab_profile, kind, near) = if form.along_normal {
            // The chain thickened in the plane, swept far one way along
            // the normal.
            let left_path = offset_chain(&chain, near_side)?;
            let right_path = offset_chain(&chain, far_side)?;
            let mut pieces = left_path.clone();
            pieces.push(Piece::Line {
                a: left_path.last().map(Piece::end).unwrap_or(e1),
                b: right_path.last().map(Piece::end).unwrap_or(e1),
            });
            pieces.extend(right_path.iter().rev().map(Piece::reversed));
            pieces.push(Piece::Line {
                a: right_path.first().map(Piece::start).unwrap_or(s0),
                b: left_path.first().map(Piece::start).unwrap_or(s0),
            });
            let across = left(chain[chain.len() / 2].tangent_at(middle));
            let at = add(middle, scale(across, mid_thickness));
            (
                vec![wire(&pieces)],
                SweepKind::Extrude {
                    termination: ExtrudeTermination::Blind { distance: reach },
                    second_side: None,
                    symmetric: false,
                    reversed: way < 0.0,
                    taper_deg: 0.0,
                    direction: None,
                },
                world(at) + normal * (way * 1e-4),
            )
        } else {
            // The chain run on along its end tangents and closed far on
            // one side, given the thickness along the normal.
            let side = scale(unit(left(sub(e1, s0))), way);
            let (a, b) = (sub(s0, scale(d0, runs.0)), add(e1, scale(d1, runs.1)));
            let (c, d) = (add(b, scale(side, reach)), add(a, scale(side, reach)));
            let mut pieces = vec![Piece::Line { a, b: s0 }];
            pieces.extend(chain.iter().copied());
            pieces.extend([
                Piece::Line { a: e1, b },
                Piece::Line { a: b, b: c },
                Piece::Line { a: c, b: d },
                Piece::Line { a: d, b: a },
            ]);
            // An end already past the solid's box runs on no further.
            pieces.retain(|p| !matches!(p, Piece::Line { a, b } if same(*a, *b)));
            let at = add(middle, scale(side, 1e-4));
            (
                vec![wire(&pieces)],
                SweepKind::Extrude {
                    termination: ExtrudeTermination::Blind {
                        distance: thickness,
                    },
                    second_side: None,
                    symmetric: form.side == RibSide::Both,
                    reversed: form.side == RibSide::Back,
                    taper_deg: 0.0,
                    direction: None,
                },
                world(at) + normal * mid_thickness,
            )
        };
        let slab = super::sweep::build_tool(
            model,
            None,
            &Profile {
                plane,
                wires: slab_profile,
            },
            &kind,
        )?;
        Ok((slab, near))
    };
    let mut kept = Vec::new();
    for way in [1.0, -1.0] {
        let mut runs = (reach, reach);
        loop {
            let slab = slab(model, way, runs)?;
            let outside = combine_solids(model, &slab.0, base, kernel_api::BoolKind::Cut)?;
            // The piece at the chain: none when the chain lies in the solid.
            let Some((gap, piece)) =
                crate::probe::by_distance(model, &outside, ShapeType::Solid, slab.1)?
                    .into_iter()
                    .next()
            else {
                break;
            };
            let Some((plo, phi)) = tess::robust_bounds(model, &piece) else {
                break;
            };
            if gap >= 1e-3 {
                break;
            }
            if (phi - plo).magnitude() < reach * 0.5 {
                kept.push(piece);
                break;
            }
            // It ran out to the slab's far side: past the ends of short
            // walls, it is cut off where the solid ends along the line.
            if form.along_normal || runs == to_walls {
                break;
            }
            runs = to_walls;
        }
    }
    if kept.is_empty() {
        return Err(
            "the rib meets no solid to run to: draw its line where the web stands between \
             faces of the solid"
                .into(),
        );
    }
    wrap_pieces(model, kept)
}
