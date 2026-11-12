//! Screen-space clearance shared by the glyphs drawn and picked. Manual
//! dimension centers are obstacles; only automatic labels and icons move.

use std::collections::HashMap;

use glam::{IVec2, Vec2};

use super::{
    Glyph, GlyphVisual, SUFFIX_SIZE_PX, SYMBOL_HIT_RADIUS_PX, Sketch, SketchProjector,
    suffix_center, symbol_px,
};

const GAP_PX: f32 = 4.0;
/// The side of a square of the grid the placed boxes are filed in, in px.
const CELL_PX: f32 = 64.0;
/// The most squares one box is filed in; a larger box is checked by every
/// query instead.
const MAX_CELLS: i64 = 1024;

#[derive(Clone, Copy)]
pub(super) struct Bounds {
    min: Vec2,
    max: Vec2,
}

impl Bounds {
    fn centered(center: Vec2, half: Vec2) -> Self {
        Self {
            min: center - half,
            max: center + half,
        }
    }

    fn overlaps(self, other: Self) -> bool {
        (self.min.cmplt(other.max + Vec2::splat(GAP_PX))
            & self.max.cmpgt(other.min - Vec2::splat(GAP_PX)))
        .all()
    }
}

/// The boxes placed so far, in the order they were placed, filed by the
/// grid squares they reach (their gap included) so a query reads only the
/// boxes near it.
#[derive(Default)]
pub(super) struct Taken {
    boxes: Vec<Bounds>,
    cells: HashMap<IVec2, Vec<u32>>,
    /// Boxes the grid does not hold (too large, or not finite), checked by
    /// every query.
    everywhere: Vec<u32>,
}

/// The grid squares a box from `lo` to `hi` lies in; `None` when that is
/// more than [`MAX_CELLS`] or the box is not finite. Two boxes that meet
/// share a point, and `floor(x / CELL_PX)` keeps the order of `x`, so they
/// share that point's square.
fn cells(lo: Vec2, hi: Vec2) -> Option<(IVec2, IVec2)> {
    if !(lo.is_finite() && hi.is_finite()) {
        return None;
    }
    let (a, b) = ((lo / CELL_PX).floor(), (hi / CELL_PX).floor());
    let limit = (i32::MAX / 2) as f32;
    if a.min_element() < -limit || b.max_element() > limit {
        return None;
    }
    let (a, b) = (a.as_ivec2(), b.as_ivec2());
    let area = i64::from(b.x - a.x + 1) * i64::from(b.y - a.y + 1);
    (area <= MAX_CELLS).then_some((a, b))
}

impl Taken {
    pub(super) fn push(&mut self, b: Bounds) {
        let index = self.boxes.len() as u32;
        self.boxes.push(b);
        let gap = Vec2::splat(GAP_PX);
        let Some((lo, hi)) = cells(b.min - gap, b.max + gap) else {
            self.everywhere.push(index);
            return;
        };
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                self.cells.entry(IVec2::new(x, y)).or_default().push(index);
            }
        }
    }

    /// The first box, in the order placed, that `b` overlaps.
    fn first_overlap(&self, b: Bounds) -> Option<Bounds> {
        let Some((lo, hi)) = cells(b.min, b.max) else {
            return self.boxes.iter().copied().find(|other| b.overlaps(*other));
        };
        // Each list is in the order placed: its first box `b` overlaps is
        // the list's earliest, and nothing after the earliest so far counts.
        let mut first: Option<u32> = None;
        let mut search = |list: &[u32]| {
            for &index in list {
                if first.is_some_and(|f| index >= f) {
                    break;
                }
                if b.overlaps(self.boxes[index as usize]) {
                    first = Some(index);
                    break;
                }
            }
        };
        search(&self.everywhere);
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                if let Some(near) = self.cells.get(&IVec2::new(x, y)) {
                    search(near);
                }
            }
        }
        first.map(|index| self.boxes[index as usize])
    }
}

fn suffix_bounds(g: &Glyph, size: f32) -> Option<Bounds> {
    let GlyphVisual::Icon {
        suffix: Some(n), ..
    } = g.visual
    else {
        return None;
    };
    let text = symbol_px(SUFFIX_SIZE_PX, size);
    Some(Bounds::centered(
        Vec2::from_array(suffix_center(g.pos, size)),
        Vec2::new(0.3 * text * n.to_string().len() as f32, 0.65 * text),
    ))
}

pub(super) fn suffix_hit(g: &Glyph, p: [f32; 2], size: f32) -> bool {
    suffix_bounds(g, size).is_some_and(|b| {
        let p = Vec2::from_array(p);
        (p.cmpge(b.min) & p.cmple(b.max)).all()
    })
}

fn bounds(g: &Glyph, size: f32) -> Bounds {
    let half = if g.dimensional {
        let (x, y) = super::pill_half_extents(size, g.text().chars().count());
        Vec2::new(x, y)
    } else {
        Vec2::splat(symbol_px(SYMBOL_HIT_RADIUS_PX, size))
    };
    let mut b = Bounds::centered(Vec2::from_array(g.pos), half);
    if let Some(suffix) = suffix_bounds(g, size) {
        b.min = b.min.min(suffix.min);
        b.max = b.max.max(suffix.max);
    }
    b
}

/// Advance along one ray to the first clear position. Each intersected box
/// is left behind permanently, so crowded rows need no arbitrary retry cap.
fn clear(g: &mut Glyph, taken: &Taken, direction: Vec2, size: f32) {
    loop {
        let b = bounds(g, size);
        let Some(other) = taken.first_overlap(b) else {
            break;
        };
        let exit_axis = |d: f32, min: f32, max: f32, lo: f32, hi: f32| {
            if d > 0.0 {
                (hi + GAP_PX - min) / d
            } else if d < 0.0 {
                (lo - GAP_PX - max) / d
            } else {
                f32::INFINITY
            }
        };
        let distance = exit_axis(direction.x, b.min.x, b.max.x, other.min.x, other.max.x).min(
            exit_axis(direction.y, b.min.y, b.max.y, other.min.y, other.max.y),
        );
        g.pos = (Vec2::from_array(g.pos) + direction * (distance + 0.1)).to_array();
    }
}

pub(super) fn arrange(glyphs: &mut [Glyph], sketch: &Sketch, proj: &SketchProjector, size: f32) {
    let constraints: std::collections::HashMap<_, _> =
        sketch.constraints.iter().map(|c| (c.id, c)).collect();
    let mut taken = Taken::default();
    for g in glyphs
        .iter()
        .filter(|g| g.dimensional && constraints[&g.constraint].label_offset.is_some())
    {
        taken.push(bounds(g, size));
    }
    for g in glyphs.iter_mut().filter(|g| g.dimensional) {
        let c = constraints[&g.constraint];
        if c.label_offset.is_some() {
            continue;
        }
        let before = g.pos;
        let direction = (Vec2::from_array(g.pos) - Vec2::from_array(g.anchor))
            .try_normalize()
            .expect("automatic dimension label leaves its anchor");
        clear(g, &taken, direction, size);
        if g.pos != before {
            let label = proj
                .sketch_at(g.pos)
                .expect("visible label projects onto its sketch");
            g.lines = super::curve_dimension_lines(sketch, proj, &c.kind, label, g.pos)
                .unwrap_or_else(|| super::dimension_lines(sketch, proj, &c.kind, g.pos, g.anchor));
        }
        taken.push(bounds(g, size));
    }
    arrange_icons(glyphs, &mut taken, size);
}

pub(super) fn arrange_icons(glyphs: &mut [Glyph], taken: &mut Taken, size: f32) {
    for g in glyphs.iter_mut().filter(|g| !g.dimensional) {
        clear(g, taken, Vec2::X, size);
        taken.push(bounds(g, size));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid finds the box a walk over every placed box in order finds
    /// first, so the layout is the same: boxes from icon to wide pill,
    /// crowded onto one spot, lying on squares' borders and on the gap's
    /// edge, larger than the grid holds, and not finite.
    #[test]
    fn the_grid_finds_the_first_box_in_the_order_placed() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 40) as f32 / (1u64 << 24) as f32
        };
        let mut random_box = |spread: f32| {
            let pick = next();
            let at = if pick < 0.3 {
                // On a square's border, or a gap's width off it.
                let k = (next() * 8.0).floor() - 4.0;
                Vec2::new(k * CELL_PX, (next() * 8.0).floor() * CELL_PX) + GAP_PX * (next() - 0.5)
            } else {
                Vec2::new(next() - 0.5, next() - 0.5) * spread
            };
            let half = if pick > 0.98 {
                Vec2::splat(CELL_PX * 40.0)
            } else {
                Vec2::new(4.0 + 60.0 * next(), 4.0 + 10.0 * next())
            };
            match (next() * 200.0) as u32 {
                0 => Bounds::centered(Vec2::new(f32::NAN, at.y), half),
                1 => Bounds::centered(Vec2::new(f32::INFINITY, at.y), half),
                _ => Bounds::centered(at, half),
            }
        };
        let bits = |b: Option<Bounds>| b.map(|b| (b.min.to_array(), b.max.to_array()));
        for spread in [10.0, 200.0, 2_000.0, 1e6] {
            let mut taken = Taken::default();
            for _ in 0..600 {
                let query = random_box(spread);
                let walked = taken.boxes.iter().copied().find(|b| query.overlaps(*b));
                assert_eq!(
                    format!("{:?}", bits(taken.first_overlap(query))),
                    format!("{:?}", bits(walked)),
                    "a query of {:?}..{:?}",
                    query.min,
                    query.max
                );
                taken.push(query);
            }
        }
    }
}
