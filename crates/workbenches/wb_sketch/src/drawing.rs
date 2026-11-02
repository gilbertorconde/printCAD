//! A sketch as a 2D drawing at true scale: every curve as a polyline within
//! a chord tolerance, in the sketch's own millimetres, construction (and
//! projected guides) apart from what the sketch draws. Written as DXF by
//! the kernel and as SVG here.

use std::fmt::Write as _;

use crate::sketch::{GeometryElement, Sketch, Vec2D};

/// Polylines in the sketch's millimetres.
pub(crate) type Lines = Vec<Vec<[f64; 2]>>;

/// How far a written polyline may stray from its curve, mm.
pub(crate) const CHORD_MM: f64 = 0.01;

/// The sketch's curves as polylines: what it draws, then its construction
/// and guides.
pub(crate) fn polylines(sketch: &Sketch) -> (Lines, Lines) {
    let point = |id| sketch.point_position(id);
    let (mut drawn, mut guides) = (Vec::new(), Vec::new());
    for g in &sketch.geometry {
        let id = g.id();
        let line: Option<Vec<Vec2D>> = match g {
            GeometryElement::Point(_) => None,
            GeometryElement::Line(l) => point(l.start).zip(point(l.end)).map(|(a, b)| vec![a, b]),
            GeometryElement::Circle(c) => {
                point(c.center).map(|o| round(o, c.radius, 0.0, std::f64::consts::TAU))
            }
            GeometryElement::Arc(a) => match (point(a.center), point(a.start), point(a.end)) {
                (Some(o), Some(s), Some(e)) => {
                    let (start, sweep) =
                        crate::snap::arc_angles((s - o).to_glam(), (e - o).to_glam());
                    Some(round(o, a.radius, start, sweep))
                }
                _ => None,
            },
            GeometryElement::Ellipse(e) => e.points(sketch, 256),
            GeometryElement::Conic(c) => c.points(sketch, 256),
            GeometryElement::BSpline(b) => b.points(sketch, 256),
        };
        let Some(line) = line.filter(|l| l.len() > 1) else {
            continue;
        };
        let line: Vec<[f64; 2]> = line.iter().map(|p| [p.x, p.y]).collect();
        let guide =
            sketch.is_construction(id) || sketch.external.get(&id).is_some_and(|s| !s.defining);
        if guide {
            guides.push(line);
        } else {
            drawn.push(line);
        }
    }
    (drawn, guides)
}

/// An arc of `radius` about `o` from `start` through `sweep` radians, in
/// enough straight pieces to stay within [`CHORD_MM`].
fn round(o: Vec2D, radius: f64, start: f64, sweep: f64) -> Vec<Vec2D> {
    let step = if radius > CHORD_MM {
        2.0 * (1.0 - CHORD_MM / radius).acos()
    } else {
        std::f64::consts::FRAC_PI_2
    };
    let n = ((sweep.abs() / step).ceil() as usize).clamp(4, 4096);
    (0..=n)
        .map(|i| {
            let t = start + sweep * i as f64 / n as f64;
            Vec2D::new(o.x + radius * t.cos(), o.y + radius * t.sin())
        })
        .collect()
}

/// The polylines as SVG, one user unit a millimetre and y up as the
/// sketch has it, guides dashed.
pub(crate) fn svg(drawn: &[Vec<[f64; 2]>], guides: &[Vec<[f64; 2]>]) -> String {
    let all = drawn.iter().chain(guides).flatten();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in all {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if lo[0] > hi[0] {
        (lo, hi) = ([0.0; 2], [1.0; 2]);
    }
    let margin = 1.0;
    let (w, h) = (hi[0] - lo[0] + 2.0 * margin, hi[1] - lo[1] + 2.0 * margin);
    let mut out = String::new();
    let _ = writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.3}mm" height="{h:.3}mm" viewBox="{:.3} {:.3} {w:.3} {h:.3}">"#,
        lo[0] - margin,
        -(hi[1] + margin)
    );
    let _ = writeln!(
        out,
        r#"<g transform="scale(1,-1)" fill="none" stroke="black" stroke-width="0.1">"#
    );
    let mut path = |line: &Vec<[f64; 2]>, dash: bool| {
        let mut d = String::new();
        for (i, p) in line.iter().enumerate() {
            let _ = write!(
                d,
                "{}{:.4},{:.4} ",
                if i == 0 { "M" } else { "L" },
                p[0],
                p[1]
            );
        }
        let dashes = if dash {
            r#" stroke-dasharray="1,0.6""#
        } else {
            ""
        };
        let _ = writeln!(out, r#"<path d="{}"{dashes}/>"#, d.trim_end());
    };
    for line in drawn {
        path(line, false);
    }
    for line in guides {
        path(line, true);
    }
    out.push_str("</g>\n</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Circle, Line, Point};

    /// A circle of radius 10 writes as a polyline within the chord
    /// tolerance of it, a construction line dashed apart, and the SVG is a
    /// millimetre a unit around them.
    #[test]
    fn a_sketch_draws_at_true_scale() {
        let mut s = Sketch::new("S");
        let o = s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(0.0, 0.0))));
        s.add_geometry(GeometryElement::Circle(Circle::new(o, 10.0)));
        let a = s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(-12.0, 0.0))));
        let b = s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(12.0, 0.0))));
        let axis = s.add_geometry(GeometryElement::Line(Line::new(a, b)));
        s.set_construction(axis, true);
        let (drawn, guides) = polylines(&s);
        assert_eq!((drawn.len(), guides.len()), (1, 1));
        for w in drawn[0].windows(2) {
            let mid = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
            assert!(10.0 - mid[0].hypot(mid[1]) <= CHORD_MM + 1e-12);
        }
        let text = svg(&drawn, &guides);
        assert!(text.contains(r#"width="26.000mm""#) && text.contains("stroke-dasharray"));
    }
}
