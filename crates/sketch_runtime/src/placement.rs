//! Drawing placement precedence, independent of document addresses.

use sketch_geometry::world::WorldPoint;

use crate::pick::SourceRef;
use crate::snap::SnapResult;

/// A geometric query result, before a gesture reduces it to selected sources.
#[derive(Debug, Clone)]
pub struct PlacedPick<R = SourceRef> {
    pub cursor: WorldPoint,
    pub pos: WorldPoint,
    pub snap: Option<SnapResult<R>>,
}

impl PlacedPick {
    pub fn selected(&self) -> crate::pick::Pick {
        crate::pick::Pick {
            cursor: self.cursor,
            pos: self.pos,
            snap: self.snap.as_ref().map(|s| crate::pick::Snap {
                kind: s.kind,
                source: s.source,
                source2: s.source2,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Placement {
    /// None uses the cursor and snap policy; Some is an explicit placement.
    pub position: Option<WorldPoint>,
    /// Free drawing bypasses all automatic object/axis/alignment candidates.
    /// Selection and editing picks do not use this drawing policy.
    pub snap_off: bool,
    /// With automatic alignment constraints off, a line link still lands where
    /// H/V, parallel, perpendicular or
    /// tangent snapping put it, but the link gets no Horizontal/Vertical/
    /// Parallel/Perpendicular/TangentAtPoint for it. Point bindings of the snap
    /// (Coincident, PointOn*) stay: they are where the point is, not alignment.
    /// Negative like `snap_off`, so the derived default keeps them on.
    pub auto_constraints_off: bool,
    /// Drawing lands on grid points; `None` means the grid
    /// does not catch the pointer. The host passes the step it shows at the current
    /// zoom, so the click lands on a line the user sees.
    pub grid: Option<GridSnap>,
}

/// Grid nodes of the sketch: every multiple of `step` along both local axes,
/// counted from the sketch origin, as the host draws them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSnap {
    step: sketch_geometry::world::WorldLength,
}

impl GridSnap {
    pub fn new(step: sketch_geometry::world::WorldLength) -> Self {
        assert!(
            step.0.is_finite() && step.0 > 0.0,
            "grid step must be positive and finite, got {}",
            step.0
        );
        Self { step }
    }

    pub fn step(self) -> sketch_geometry::world::WorldLength {
        self.step
    }

    /// The nearest grid node.
    pub fn round(self, at: WorldPoint) -> WorldPoint {
        let step = self.step.0;
        WorldPoint::from_dvec2((at.0 / step).round() * step)
    }
}

impl Placement {
    /// A number is intentional even below one screen pixel. Mouse input keeps
    /// the normal jitter threshold; both paths still reject degenerate geometry.
    pub fn minimum(
        self,
        pointer_minimum: sketch_geometry::world::WorldLength,
    ) -> sketch_geometry::world::WorldLength {
        if self.position.is_some() {
            sketch_geometry::world::WorldLength::DEGENERATE
        } else {
            pointer_minimum
        }
    }
    /// Where a drawing click at `cursor` lands. A typed
    /// position wins over everything; a snap onto a point (end, middle, centre,
    /// quadrant, intersection, tangent point or perpendicular foot) wins over the
    /// grid; anything else is rounded to the
    /// grid first and then snapped, so a curve, an axis or a direction keeps one
    /// coordinate free for the grid to round. Object snap off still lands on the
    /// grid: the two switches are independent.
    ///
    /// `Pick::cursor` stays the raw pointer in every branch.
    /// A projected host can supply stable source ordering or replay an already
    /// selected pick. Placement precedence stays shared with the native path.
    pub fn resolve_with<R>(
        self,
        cursor: WorldPoint,
        resolve: impl Fn(WorldPoint) -> PlacedPick<R>,
    ) -> PlacedPick<R> {
        if let Some(pos) = self.position {
            return PlacedPick {
                cursor,
                pos,
                snap: None,
            };
        }
        let snapped_at = |at: WorldPoint| {
            if self.snap_off {
                PlacedPick {
                    cursor,
                    pos: at,
                    snap: None,
                }
            } else {
                PlacedPick {
                    cursor,
                    ..resolve(at)
                }
            }
        };
        let Some(grid) = self.grid else {
            return snapped_at(cursor);
        };
        if !self.snap_off {
            let pick = resolve(cursor);
            if pick
                .snap
                .as_ref()
                .is_some_and(|snap| snap.kind.is_point() || snap.source2.is_some())
            {
                return pick;
            }
        }
        snapped_at(grid.round(cursor))
    }
}
