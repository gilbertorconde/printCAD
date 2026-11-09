//! Rectangle parameters and local-frame geometry, shared by preview and application.
use glam::DVec2;
use sketch_geometry::types::Angle;
use sketch_geometry::world::{WorldLength, WorldOffset, WorldPoint, WorldVec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthSide {
    Top,
    Bottom,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightSide {
    Right,
    Left,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectDimPlacement {
    pub width_side: WidthSide,
    pub height_side: HeightSide,
    pub offset: WorldLength,
}
impl RectDimPlacement {
    pub fn width_offset(&self) -> WorldOffset {
        match self.width_side {
            WidthSide::Top => self.offset.as_offset(),
            WidthSide::Bottom => -self.offset.as_offset(),
        }
    }
    pub fn height_offset(&self) -> WorldOffset {
        match self.height_side {
            HeightSide::Right => self.offset.as_offset(),
            HeightSide::Left => -self.offset.as_offset(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Corner {
    BottomLeft,
    BottomRight,
    TopRight,
    TopLeft,
}
impl Corner {
    /// Counterclockwise contour order in the rectangle's local frame.
    pub const ALL: [Self; 4] = [
        Self::BottomLeft,
        Self::BottomRight,
        Self::TopRight,
        Self::TopLeft,
    ];
    pub fn index(self) -> usize {
        match self {
            Self::BottomLeft => 0,
            Self::BottomRight => 1,
            Self::TopRight => 2,
            Self::TopLeft => 3,
        }
    }
    pub fn opposite(self) -> Self {
        match self {
            Self::BottomLeft => Self::TopRight,
            Self::BottomRight => Self::TopLeft,
            Self::TopRight => Self::BottomLeft,
            Self::TopLeft => Self::BottomRight,
        }
    }
    pub fn across_width(self) -> Self {
        match self {
            Self::BottomLeft => Self::BottomRight,
            Self::BottomRight => Self::BottomLeft,
            Self::TopRight => Self::TopLeft,
            Self::TopLeft => Self::TopRight,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RectCornerMode {
    #[default]
    None,
    Chamfer,
    Fillet,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RectFrame {
    AxisAligned,
    Rotated(Angle),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Bottom,
    Right,
    Top,
    Left,
}
impl Side {
    /// Contour order matching [`Side::index`]: side `i` runs from corner `i` to `i + 1`.
    pub const ALL: [Self; 4] = [Self::Bottom, Self::Right, Self::Top, Self::Left];
    /// The two corners bounding the side, in counterclockwise contour order.
    pub fn corners(self) -> [Corner; 2] {
        match self {
            Self::Bottom => [Corner::BottomLeft, Corner::BottomRight],
            Self::Right => [Corner::BottomRight, Corner::TopRight],
            Self::Top => [Corner::TopRight, Corner::TopLeft],
            Self::Left => [Corner::TopLeft, Corner::BottomLeft],
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Bottom => 0,
            Self::Right => 1,
            Self::Top => 2,
            Self::Left => 3,
        }
    }
    pub fn between(a: Corner, b: Corner) -> Self {
        match (a, b) {
            (Corner::BottomLeft, Corner::BottomRight)
            | (Corner::BottomRight, Corner::BottomLeft) => Self::Bottom,
            (Corner::BottomRight, Corner::TopRight) | (Corner::TopRight, Corner::BottomRight) => {
                Self::Right
            }
            (Corner::TopRight, Corner::TopLeft) | (Corner::TopLeft, Corner::TopRight) => Self::Top,
            (Corner::TopLeft, Corner::BottomLeft) | (Corner::BottomLeft, Corner::TopLeft) => {
                Self::Left
            }
            _ => panic!("a rectangle side needs adjacent logical corners"),
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub enum SideRelation {
    Horizontal(Side),
    Vertical(Side),
    Parallel(Side, Side),
    Perpendicular(Side, Side),
}
pub fn side_relations(frame: RectFrame) -> Vec<SideRelation> {
    use Side::{Bottom, Left, Right, Top};
    match frame {
        RectFrame::AxisAligned => {
            vec![
                SideRelation::Horizontal(Bottom),
                SideRelation::Horizontal(Top),
                SideRelation::Vertical(Left),
                SideRelation::Vertical(Right),
            ]
        }
        RectFrame::Rotated(_) => {
            vec![
                SideRelation::Parallel(Bottom, Top),
                SideRelation::Parallel(Left, Right),
                SideRelation::Perpendicular(Bottom, Left),
            ]
        }
    }
}
impl RectFrame {
    pub fn axes(self) -> (WorldVec, WorldVec) {
        let (u, v) = self.axes_raw();
        (WorldVec(u), WorldVec(v))
    }
    fn axes_raw(self) -> (DVec2, DVec2) {
        match self {
            Self::AxisAligned => (DVec2::X, DVec2::Y),
            Self::Rotated(angle) => {
                let u = DVec2::from_angle(angle.radians());
                (u, u.perp())
            }
        }
    }
    pub fn to_local(self, world: WorldVec) -> WorldVec {
        WorldVec(self.local_of(world.0))
    }
    fn local_of(self, world: DVec2) -> DVec2 {
        let (u, v) = self.axes_raw();
        DVec2::new(world.dot(u), world.dot(v))
    }
    fn world_of(self, local: DVec2) -> DVec2 {
        let (u, v) = self.axes_raw();
        u * local.x + v * local.y
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectDraft {
    pub center: WorldPoint,
    pub half_w: WorldLength,
    pub half_h: WorldLength,
    pub frame: RectFrame,
    pub corner_mode: RectCornerMode,
    pub corner_size: WorldLength,
    pub dims: RectDimPlacement,
    pub wall: Option<WorldLength>,
}
impl RectDraft {
    pub fn corners(&self) -> [WorldPoint; 4] {
        let at = |sx: f64, sy: f64| {
            let local = DVec2::new(sx * self.half_w.0, sy * self.half_h.0);
            WorldPoint::from_dvec2(self.center.0 + self.frame.world_of(local))
        };
        [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)]
    }
    pub fn corner(&self, corner: Corner) -> WorldPoint {
        self.corners()[corner.index()]
    }
    /// The frame stays fixed while a handle crosses its opposite corner.
    pub fn from_diagonal(
        a: WorldPoint,
        b: WorldPoint,
        frame: RectFrame,
        corner_mode: RectCornerMode,
        corner_size: WorldLength,
        dims: RectDimPlacement,
    ) -> Self {
        let local_a = frame.local_of(a.0);
        let local_b = frame.local_of(b.0);
        let min = local_a.min(local_b);
        let max = local_a.max(local_b);
        Self {
            center: WorldPoint::from_dvec2(frame.world_of((min + max) * 0.5)),
            half_w: WorldLength::new((max.x - min.x) * 0.5),
            half_h: WorldLength::new((max.y - min.y) * 0.5),
            frame,
            corner_mode,
            corner_size,
            dims,
            wall: None,
        }
    }
    /// The third click measures the perpendicular projection onto the first side.
    pub fn from_three_corners(
        p0: WorldPoint,
        p1: WorldPoint,
        p2: WorldPoint,
        corner_mode: RectCornerMode,
        corner_size: WorldLength,
        dims: RectDimPlacement,
    ) -> Self {
        let edge = p1 - p0;
        assert!(
            edge.length().0 > 0.0,
            "rotated rectangle needs a nondegenerate first edge"
        );
        let frame = RectFrame::Rotated(Angle::from_vec(edge));
        let (_, v) = frame.axes();
        let far = p1 + v * (p2 - p1).0.dot(v.0);
        Self::from_diagonal(p0, far, frame, corner_mode, corner_size, dims)
    }
    pub fn width(&self) -> WorldLength {
        self.half_w * 2.0
    }
    pub fn height(&self) -> WorldLength {
        self.half_h * 2.0
    }
    pub fn corners_treated(&self) -> bool {
        self.corner_mode != RectCornerMode::None && self.corner_size.0 > 0.0
    }
    pub fn inner_corners(&self) -> Result<Option<[WorldPoint; 4]>, RectRejection> {
        let Some(wall) = self.wall else {
            return Ok(None);
        };
        if !wall.0.is_finite()
            || wall <= WorldLength::DEGENERATE
            || self.half_w.0 - wall.0 <= WorldLength::DEGENERATE.0
            || self.half_h.0 - wall.0 <= WorldLength::DEGENERATE.0
        {
            return Err(RectRejection::WallDoesNotFit);
        }
        if self.corners_treated() {
            return Err(RectRejection::FrameNeedsSharpCorners);
        }
        Ok(Some(
            Self {
                half_w: self.half_w - wall,
                half_h: self.half_h - wall,
                wall: None,
                ..*self
            }
            .corners(),
        ))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RectRejection {
    WallDoesNotFit,
    FrameNeedsSharpCorners,
}
