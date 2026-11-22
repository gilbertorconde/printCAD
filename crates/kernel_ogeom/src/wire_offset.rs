//! A closed 2D wire of lines and arcs offset by a distance, each piece of
//! the offset named by the segment or corner it comes from.

use kernel_api::{
    KernelError, KernelResult, OffsetFrom, OffsetPiece, ProfilePlane, ProfileSegment, ProfileWire,
};
use ogeom::geom::{Curve, Curve3d as _};
use ogeom::offset::{Join, offset_wire};
use ogeom::topo::{Filter, Model, Orientation, Shape, ShapeType, explore};

use crate::tess;

fn failed(e: impl std::fmt::Display) -> KernelError {
    KernelError::InvalidInput(e.to_string())
}

pub(crate) fn offset_wire_2d(
    wire: &ProfileWire,
    distance: f64,
    round: bool,
) -> KernelResult<Vec<Vec<OffsetPiece>>> {
    if wire
        .segments
        .iter()
        .any(|s| !matches!(s, ProfileSegment::Line { .. } | ProfileSegment::Arc { .. }))
    {
        return Err(KernelError::InvalidInput(
            "a wire offset takes lines and arcs".into(),
        ));
    }
    let tol = tess::tolerances();
    let mut model = Model::with_tolerances(tol);
    let built = crate::profile::build_wire_edges(&mut model, &ProfilePlane::default(), wire, true)
        .map_err(KernelError::InvalidInput)?;
    let join = if round { Join::Arc } else { Join::Intersection };
    let offset = offset_wire(&mut model, &built.wire, distance, join, tol).map_err(failed)?;
    let history = &offset.history;
    // The corner after segment `i` is the vertex it ends at.
    let corners: Vec<Shape> = built
        .edges
        .iter()
        .map(|e| {
            ogeom::algo::edge_vertices(&model, e)
                .map_err(failed)?
                .map(|(_, end)| end)
                .ok_or_else(|| failed("a segment has no ends"))
        })
        .collect::<KernelResult<_>>()?;
    let from_of = |edge: &Shape| -> KernelResult<OffsetFrom> {
        let image = |images: &[Shape]| images.iter().any(|m| m.is_same(edge));
        if let Some(i) = built.edges.iter().position(|e| image(history.modified(e))) {
            return Ok(OffsetFrom::Segment(i));
        }
        corners
            .iter()
            .position(|v| image(history.generated(v)))
            .map(OffsetFrom::Corner)
            .ok_or_else(|| failed("an offset piece comes from nothing in the wire"))
    };
    let mut loops = Vec::new();
    for ring in explore(&model, &offset.shape, Filter::OfType(ShapeType::Wire)).map_err(failed)? {
        let mut pieces = Vec::new();
        for edge in model.children_of(&ring).map_err(failed)? {
            let Some((curve, range)) =
                ogeom::algo::placed_edge_curve(&model, &edge, tol).map_err(failed)?
            else {
                return Err(failed("an offset piece has no curve"));
            };
            let at = |t: f64| curve.point_at(t, tol).map(|p| [p.x, p.y]).map_err(failed);
            let (mut start, mut end) = (at(range.0)?, at(range.1)?);
            if edge.orientation() == Orientation::Reversed {
                std::mem::swap(&mut start, &mut end);
            }
            let segment = match curve {
                Curve::Line(_) => ProfileSegment::Line { start, end },
                Curve::Circle(_) => ProfileSegment::Arc {
                    start,
                    mid: at(f64::midpoint(range.0, range.1))?,
                    end,
                },
                _ => return Err(failed("an offset piece is neither a line nor an arc")),
            };
            pieces.push(OffsetPiece {
                segment,
                from: from_of(&edge)?,
            });
        }
        loops.push(pieces);
    }
    Ok(loops)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(w: f64, h: f64) -> ProfileWire {
        let c = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
        ProfileWire::new(
            (0..4)
                .map(|k| ProfileSegment::Line {
                    start: c[k],
                    end: c[(k + 1) % 4],
                })
                .collect(),
        )
    }

    fn ends(s: &ProfileSegment) -> ([f64; 2], [f64; 2]) {
        match s {
            ProfileSegment::Line { start, end } | ProfileSegment::Arc { start, end, .. } => {
                (*start, *end)
            }
            _ => unreachable!(),
        }
    }

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-9
    }

    /// Each loop runs end to start, and back round to its first piece.
    fn assert_chained(ring: &[OffsetPiece]) {
        for k in 0..ring.len() {
            let (_, end) = ends(&ring[k].segment);
            let (next, _) = ends(&ring[(k + 1) % ring.len()].segment);
            assert!(
                close(end, next),
                "piece {k} ends at {end:?}, next starts {next:?}"
            );
        }
    }

    #[test]
    fn a_rectangle_grows_with_rounded_corners_named_by_their_corner() {
        let loops = offset_wire_2d(&rectangle(10.0, 5.0), 1.0, true).unwrap();
        assert_eq!(loops.len(), 1);
        let ring = &loops[0];
        assert_chained(ring);
        let mut sides: Vec<usize> = ring
            .iter()
            .filter_map(|p| match p.from {
                OffsetFrom::Segment(i) => Some(i),
                OffsetFrom::Corner(_) => None,
            })
            .collect();
        sides.sort_unstable();
        assert_eq!(sides, [0, 1, 2, 3]);
        // The corner after the bottom side (0, ending at (10, 0)) rounds
        // about that corner, radius 1.
        let corner = ring
            .iter()
            .find(|p| p.from == OffsetFrom::Corner(0))
            .expect("the corner after the bottom side");
        let ProfileSegment::Arc { start, end, .. } = corner.segment else {
            panic!("a corner rounds with an arc");
        };
        assert!(
            close(start, [10.0, -1.0]) && close(end, [11.0, 0.0]),
            "{start:?} {end:?}"
        );
    }

    #[test]
    fn a_rectangle_shrinks_with_sharp_corners() {
        let loops = offset_wire_2d(&rectangle(10.0, 5.0), -1.0, false).unwrap();
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 4);
        assert_chained(&loops[0]);
        let (start, _) = ends(&loops[0][0].segment);
        assert!(close(start, [1.0, 1.0]), "{start:?}");
    }

    /// Two blocks joined by a narrow neck, shrunk past half the neck: the
    /// neck closes and two loops stay.
    #[test]
    fn a_shrink_that_closes_a_neck_leaves_two_loops() {
        let c = [
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 4.0],
            [14.0, 4.0],
            [14.0, 0.0],
            [24.0, 0.0],
            [24.0, 10.0],
            [13.0, 10.0],
            [13.0, 6.0],
            [11.0, 6.0],
            [11.0, 10.0],
            [0.0, 10.0],
        ];
        let wire = ProfileWire::new(
            (0..c.len())
                .map(|k| ProfileSegment::Line {
                    start: c[k],
                    end: c[(k + 1) % c.len()],
                })
                .collect(),
        );
        let loops = offset_wire_2d(&wire, -1.5, false).unwrap();
        assert_eq!(loops.len(), 2, "{loops:?}");
        for ring in &loops {
            assert_chained(ring);
        }
    }
}
