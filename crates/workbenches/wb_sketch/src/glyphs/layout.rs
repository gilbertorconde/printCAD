//! Screen-space clearance shared by the glyphs drawn and picked. Manual
//! dimension centers are obstacles; only automatic labels and icons move.

use glam::Vec2;

use super::{Glyph, GlyphVisual, SYMBOL_HIT_RADIUS_PX, Sketch, SketchProjector};

const GAP_PX: f32 = 4.0;

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

fn suffix_bounds(g: &Glyph) -> Option<Bounds> {
    let GlyphVisual::Icon {
        suffix: Some(n), ..
    } = g.visual
    else {
        return None;
    };
    Some(Bounds::centered(
        Vec2::from_array(g.pos) + Vec2::new(9.0, 7.0),
        Vec2::new(2.7 * n.to_string().len() as f32, 0.65 * 9.0),
    ))
}

pub(super) fn suffix_hit(g: &Glyph, p: [f32; 2]) -> bool {
    suffix_bounds(g).is_some_and(|b| {
        let p = Vec2::from_array(p);
        (p.cmpge(b.min) & p.cmple(b.max)).all()
    })
}

fn bounds(g: &Glyph, size: f32) -> Bounds {
    let half = if g.dimensional {
        let (x, y) = super::pill_half_extents(size, g.text().chars().count());
        Vec2::new(x, y)
    } else {
        Vec2::splat(SYMBOL_HIT_RADIUS_PX)
    };
    let mut b = Bounds::centered(Vec2::from_array(g.pos), half);
    if let Some(suffix) = suffix_bounds(g) {
        b.min = b.min.min(suffix.min);
        b.max = b.max.max(suffix.max);
    }
    b
}

/// Advance along one ray to the first clear position. Each intersected box
/// is left behind permanently, so crowded rows need no arbitrary retry cap.
fn clear(g: &mut Glyph, taken: &[Bounds], direction: Vec2, size: f32) {
    loop {
        let b = bounds(g, size);
        let Some(other) = taken.iter().find(|other| b.overlaps(**other)) else {
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
    let mut taken: Vec<_> = glyphs
        .iter()
        .filter(|g| g.dimensional && constraints[&g.constraint].label_offset.is_some())
        .map(|g| bounds(g, size))
        .collect();
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

pub(super) fn arrange_icons(glyphs: &mut [Glyph], taken: &mut Vec<Bounds>, size: f32) {
    for g in glyphs.iter_mut().filter(|g| !g.dimensional) {
        clear(g, taken, Vec2::X, size);
        taken.push(bounds(g, size));
    }
}
