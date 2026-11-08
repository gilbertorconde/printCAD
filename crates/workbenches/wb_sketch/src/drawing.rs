//! A sketch as a 2D drawing at true scale, in the sketch's own
//! millimetres (or inches), construction (and projected guides) apart from
//! what the sketch draws. As DXF ([`sheet`], written by the kernel) each
//! element is itself on its layer, beside its pictures' edges and its
//! notes; as SVG every curve is a polyline within a chord tolerance; as
//! CSV ([`csv`]) a row per element and per dimension. Each carries a
//! traceability line ([`traceability`]): the scale, where it came from,
//! and the frame.

use std::fmt::Write as _;
use std::sync::Arc;

use kernel_api::{Drawing2d, DrawingCurve, DrawingShape, DrawingText};

use crate::picture_edges::EdgeSet;
use crate::sketch::{GeometryElement, ImageScale, ReferenceImage, Sketch, SketchPlane, Vec2D};

/// The DXF layer of what the sketch draws.
pub(crate) const LAYER_ELEMENTS: &str = "ELEMENTS";
/// The DXF layer of construction and guides, dashed.
pub(crate) const LAYER_CONSTRUCTION: &str = "CONSTRUCTION";
/// The DXF layer of the pictures' edges, dashed.
pub(crate) const LAYER_EDGES: &str = "PICTURE_EDGES";
/// The DXF layer of text: the sketch's text blocks and the traceability
/// line.
pub(crate) const LAYER_NOTES: &str = "NOTES";

/// The height of the traceability line's capitals, mm.
const NOTE_MM: f64 = 2.5;
/// Capitals' height against a font's em.
const CAP_PER_EM: f64 = 0.7;
/// Baseline to baseline against a font's em.
const LINE_PER_EM: f64 = 1.2;
const MM_PER_INCH: f64 = 25.4;

/// A reference picture as an export reads it: its file's size in pixels
/// and the edges found on it, each when known.
pub(crate) struct Picture<'a> {
    pub image: &'a ReferenceImage,
    pub pixels: Option<[u32; 2]>,
    pub edges: Option<Arc<EdgeSet>>,
}

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
            GeometryElement::BSpline(b) => b.polyline(sketch, 256),
        };
        let Some(line) = line.filter(|l| l.len() > 1) else {
            continue;
        };
        let line: Vec<[f64; 2]> = line.iter().map(|p| [p.x, p.y]).collect();
        if is_guide(sketch, id) {
            guides.push(line);
        } else {
            drawn.push(line);
        }
    }
    (drawn, guides)
}

/// Construction, or a projection that does not count.
fn is_guide(sketch: &Sketch, id: uuid::Uuid) -> bool {
    sketch.is_construction(id) || sketch.external.get(&id).is_some_and(|s| !s.defining)
}

/// The sketch as a layered drawing, in millimetres or `inches`: lines,
/// circles and arcs as themselves and other curves as polylines within
/// [`CHORD_MM`] on [`LAYER_ELEMENTS`] or [`LAYER_CONSTRUCTION`], every
/// picture's edges thinned to [`CHORD_MM`] on [`LAYER_EDGES`], and on
/// [`LAYER_NOTES`] the text blocks as text and `trace` under the drawing.
pub(crate) fn sheet(sketch: &Sketch, pictures: &[Picture], trace: &str, inches: bool) -> Drawing2d {
    let k = if inches { 1.0 / MM_PER_INCH } else { 1.0 };
    let xy = |p: Vec2D| [p.x * k, p.y * k];
    let point = |id| sketch.point_position(id);
    let mut curves = Vec::new();
    for g in &sketch.geometry {
        let shape = match g {
            GeometryElement::Point(_) => None,
            GeometryElement::Line(l) => {
                point(l.start)
                    .zip(point(l.end))
                    .map(|(a, b)| DrawingShape::Polyline {
                        points: vec![xy(a), xy(b)],
                        bulges: Vec::new(),
                        closed: false,
                    })
            }
            GeometryElement::Circle(c) => point(c.center).map(|o| DrawingShape::Circle {
                centre: xy(o),
                radius: c.radius * k,
            }),
            GeometryElement::Arc(a) => match (point(a.center), point(a.start), point(a.end)) {
                (Some(o), Some(s), Some(e)) => {
                    let (start, sweep) =
                        crate::snap::arc_angles((s - o).to_glam(), (e - o).to_glam());
                    Some(DrawingShape::Arc {
                        centre: xy(o),
                        radius: a.radius * k,
                        start_angle: start,
                        end_angle: start + sweep,
                    })
                }
                _ => None,
            },
            GeometryElement::Ellipse(e) => sampled(e.points(sketch, 256), k),
            GeometryElement::Conic(c) => sampled(c.points(sketch, 256), k),
            GeometryElement::BSpline(b) => sampled(b.polyline(sketch, 256), k),
        };
        if let Some(shape) = shape {
            let hidden = is_guide(sketch, g.id());
            curves.push(DrawingCurve {
                hidden,
                shape,
                layer: if hidden {
                    LAYER_CONSTRUCTION
                } else {
                    LAYER_ELEMENTS
                }
                .to_string(),
            });
        }
    }
    for picture in pictures {
        let Some(set) = &picture.edges else {
            continue;
        };
        for chain in &set.chains {
            let along: Vec<Vec2D> = chain
                .points
                .iter()
                .map(|p| crate::picture_edges::to_sketch(picture.image, set, *p))
                .collect();
            let points: Vec<[f64; 2]> = thin(&along, CHORD_MM).into_iter().map(xy).collect();
            if points.len() > 1 {
                curves.push(DrawingCurve {
                    hidden: true,
                    shape: DrawingShape::Polyline {
                        points,
                        bulges: Vec::new(),
                        closed: chain.closed,
                    },
                    layer: LAYER_EDGES.to_string(),
                });
            }
        }
    }
    let mut texts = Vec::new();
    for block in &sketch.texts {
        let Some(at) = point(block.anchor) else {
            continue;
        };
        let (sin, cos) = block.angle.to_radians().sin_cos();
        for (i, line) in block.text.lines().enumerate() {
            let down = block.size * LINE_PER_EM * i as f64;
            texts.push(DrawingText {
                at: xy(Vec2D::new(at.x + down * sin, at.y - down * cos)),
                height: block.size * CAP_PER_EM * k,
                angle: block.angle.to_radians(),
                text: line.to_string(),
                layer: LAYER_NOTES.to_string(),
            });
        }
    }
    let (lo, _) = bounds(&curves).unwrap_or(([0.0; 2], [0.0; 2]));
    let height = NOTE_MM * k;
    texts.push(DrawingText {
        at: [lo[0], lo[1] - 2.0 * height],
        height,
        angle: 0.0,
        text: trace.to_string(),
        layer: LAYER_NOTES.to_string(),
    });
    Drawing2d {
        unit_mm: Some(if inches { MM_PER_INCH } else { 1.0 }),
        curves,
        texts,
    }
}

fn sampled(points: Option<Vec<Vec2D>>, k: f64) -> Option<DrawingShape> {
    let points = points.filter(|p| p.len() > 1)?;
    Some(DrawingShape::Polyline {
        points: points.iter().map(|p| [p.x * k, p.y * k]).collect(),
        bulges: Vec::new(),
        closed: false,
    })
}

/// The corners of the box round every curve's points (a circle's or an
/// arc's whole round).
fn bounds(curves: &[DrawingCurve]) -> Option<([f64; 2], [f64; 2])> {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    let mut grow = |p: [f64; 2]| {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    };
    for curve in curves {
        match &curve.shape {
            DrawingShape::Polyline { points, .. } | DrawingShape::Spline { points, .. } => {
                points.iter().copied().for_each(&mut grow);
            }
            DrawingShape::Circle { centre, radius } | DrawingShape::Arc { centre, radius, .. } => {
                grow([centre[0] - radius, centre[1] - radius]);
                grow([centre[0] + radius, centre[1] + radius]);
            }
            DrawingShape::Ellipse { centre, major, .. } => {
                let r = major[0].hypot(major[1]);
                grow([centre[0] - r, centre[1] - r]);
                grow([centre[0] + r, centre[1] + r]);
            }
        }
    }
    (lo[0] <= hi[0]).then_some((lo, hi))
}

/// `points` with those left out that lie within `tol` of the line
/// between the points kept either side of them.
fn thin(points: &[Vec2D], tol: f64) -> Vec<Vec2D> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut spans = vec![(0, points.len() - 1)];
    while let Some((a, b)) = spans.pop() {
        let (p, q) = (points[a], points[b]);
        let d = q - p;
        let len = d.x.hypot(d.y);
        let off = |r: Vec2D| {
            let e = r - p;
            if len > 0.0 {
                (d.x * e.y - d.y * e.x).abs() / len
            } else {
                e.x.hypot(e.y)
            }
        };
        let far = (a + 1..b)
            .map(|i| (i, off(points[i])))
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((i, d)) = far
            && d > tol
        {
            keep[i] = true;
            spans.push((a, i));
            spans.push((i, b));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// A number written short: six decimals at most, trailing zeros dropped.
fn num(x: f64) -> String {
    let text = format!("{x:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0" } else { text }.to_string()
}

/// One line saying what a drawing's numbers rest on: the sketch at 1:1 in
/// `unit`, its frame (the plane's origin and axes in the document), and
/// each picture's scale with where that scale came from.
pub(crate) fn traceability(
    sketch: &Sketch,
    plane: &SketchPlane,
    pictures: &[Picture],
    unit: &str,
) -> String {
    let v = |a: [f64; 3]| format!("({} {} {})", num(a[0]), num(a[1]), num(a[2]));
    let mut out = format!(
        "Sketch \"{}\" at 1:1 in {unit}; frame origin {} x {} y {}",
        sketch.name,
        v(plane.origin),
        v(plane.x_axis),
        v(plane.y_axis)
    );
    for (i, picture) in pictures.iter().enumerate() {
        let image = picture.image;
        let source = match &image.scale {
            ImageScale::Given => "width as given".to_string(),
            ImageScale::Nominal { dpi } => format!("nominal {} dpi", num(*dpi)),
            ImageScale::Calibrated { by } => format!("calibrated by {by}"),
        };
        let scale = match picture.pixels {
            Some([w, _]) if w > 0 => format!("{} mm/px", num(image.width / f64::from(w))),
            _ => format!("{} mm wide", num(image.width)),
        };
        let _ = write!(out, "; picture {}: {scale} ({source})", i + 1);
        if image.stretch != 1.0 {
            let _ = write!(out, ", height x{}", num(image.stretch));
        }
        if image.mirrored {
            out.push_str(", mirrored");
        }
    }
    out
}

/// A CSV field, quoted when it must be.
fn field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// The sketch's elements and dimensions as CSV, `trace` first as a `#`
/// line, then a header; lengths in mm, angles in degrees. Answers the text
/// and its number of rows.
pub(crate) fn csv(sketch: &Sketch, trace: &str) -> (String, usize) {
    let mut out = format!("# {}\n", trace.replace(['\n', '\r'], " "));
    out.push_str("type,kind,id,name,role,x,y,x2,y2,radius,length,value,unit\n");
    let mut rows = 0;
    let point = |id| sketch.point_position(id);
    let n = |x: Option<f64>| x.map(num).unwrap_or_default();
    for g in &sketch.geometry {
        let id = g.id();
        let role = if is_guide(sketch, id) {
            "construction"
        } else {
            "drawn"
        };
        let (kind, a, b, radius, length): (&str, Option<Vec2D>, Option<Vec2D>, _, _) = match g {
            GeometryElement::Point(p) => ("point", Some(p.position), None, None, None),
            GeometryElement::Line(l) => {
                let (a, b) = (point(l.start), point(l.end));
                let length = a.zip(b).map(|(a, b)| (b - a).x.hypot((b - a).y));
                ("line", a, b, None, length)
            }
            GeometryElement::Circle(c) => (
                "circle",
                point(c.center),
                None,
                Some(c.radius),
                Some(std::f64::consts::TAU * c.radius),
            ),
            GeometryElement::Arc(a) => {
                let sweep = match (point(a.center), point(a.start), point(a.end)) {
                    (Some(o), Some(s), Some(e)) => {
                        Some(crate::snap::arc_angles((s - o).to_glam(), (e - o).to_glam()).1)
                    }
                    _ => None,
                };
                (
                    "arc",
                    point(a.center),
                    None,
                    Some(a.radius),
                    sweep.map(|w| w.abs() * a.radius),
                )
            }
            GeometryElement::Ellipse(e) => {
                ("ellipse", None, None, None, run(e.points(sketch, 256)))
            }
            GeometryElement::Conic(c) => ("conic", None, None, None, run(c.points(sketch, 256))),
            GeometryElement::BSpline(s) => {
                ("bspline", None, None, None, run(s.polyline(sketch, 256)))
            }
        };
        let _ = writeln!(
            out,
            "element,{kind},{id},,{role},{},{},{},{},{},{},,mm",
            n(a.map(|p| p.x)),
            n(a.map(|p| p.y)),
            n(b.map(|p| p.x)),
            n(b.map(|p| p.y)),
            n(radius),
            n(length),
        );
        rows += 1;
    }
    for c in &sketch.constraints {
        let Some(value) = crate::sketch::dimension_value(&c.kind) else {
            continue;
        };
        let kind = crate::sketch::constraint_label(&c.kind);
        let unit = if kind.to_ascii_lowercase().contains("angle")
            || matches!(c.kind, crate::sketch::ConstraintKind::PolarPitch { .. })
        {
            "deg"
        } else if matches!(c.kind, crate::sketch::ConstraintKind::Refraction { .. }) {
            ""
        } else {
            "mm"
        };
        let _ = writeln!(
            out,
            "dimension,{},{},{},{},,,,,,,{},{unit}",
            field(&kind),
            c.id,
            field(c.name.as_deref().unwrap_or("")),
            if c.driving { "driving" } else { "reference" },
            num(value),
        );
        rows += 1;
    }
    (out, rows)
}

/// The length of a polyline.
fn run(points: Option<Vec<Vec2D>>) -> Option<f64> {
    let points = points?;
    Some(
        points
            .windows(2)
            .map(|w| (w[1] - w[0]).x.hypot((w[1] - w[0]).y))
            .sum(),
    )
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
/// sketch has it, guides dashed, `trace` written under them.
pub(crate) fn svg(drawn: &[Vec<[f64; 2]>], guides: &[Vec<[f64; 2]>], trace: &str) -> String {
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
    let note = 2.0 * NOTE_MM;
    let (w, h) = (
        hi[0] - lo[0] + 2.0 * margin,
        hi[1] - lo[1] + 2.0 * margin + note,
    );
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
    out.push_str("</g>\n");
    let escaped = trace
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    let _ = writeln!(
        out,
        r#"<text x="{:.3}" y="{:.3}" font-family="sans-serif" font-size="{:.3}">{escaped}</text>"#,
        lo[0] - margin,
        -lo[1] + margin + NOTE_MM * 1.5,
        NOTE_MM / CAP_PER_EM,
    );
    out.push_str("</svg>\n");
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
        let text = svg(&drawn, &guides, "trace");
        assert!(text.contains(r#"width="26.000mm""#) && text.contains("stroke-dasharray"));
    }
}
