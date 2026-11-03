//! DXF drawings read and written by the kernel, as typed 2D curves and
//! texts on layers (`kernel_api::Drawing2d`).

use kernel_api::{Drawing2d, DrawingCurve, DrawingShape, DrawingText, KernelError, KernelResult};
use ogeom::geom::Curve2d as _;
use ogeom::io::dxf::{
    DxfCurve, DxfItem, DxfLayer, DxfOut, DxfShape, DxfText, DxfUnit, read_dxf_entities,
    write_dxf_entities,
};
use ogeom::math::{Point2, Vector2};

use crate::tess;

/// The points a spline is sampled at for each span between its knots.
const SAMPLES_PER_SPAN: usize = 16;

/// The curves of DXF text: what the kernel's reader finds, each marked
/// hidden or not, in the drawing's own coordinates, with its unit.
///
/// # Errors
///
/// `KernelError::Import` when the text is not a DXF.
pub fn read_dxf(text: &str) -> KernelResult<Drawing2d> {
    let read = read_dxf_entities(text)
        .map_err(|e| KernelError::Import(format!("the DXF could not be read: {e}")))?;
    let xy = |p: Point2| [p.x, p.y];
    let curves = read
        .entities
        .into_iter()
        .filter_map(|entity| {
            let shape = match entity.curve {
                DxfCurve::Line { start, end } => DrawingShape::Polyline {
                    points: vec![xy(start), xy(end)],
                    bulges: Vec::new(),
                    closed: false,
                },
                DxfCurve::Arc {
                    centre,
                    radius,
                    start_angle,
                    end_angle,
                } => DrawingShape::Arc {
                    centre: xy(centre),
                    radius,
                    start_angle,
                    end_angle,
                },
                DxfCurve::Circle { centre, radius } => DrawingShape::Circle {
                    centre: xy(centre),
                    radius,
                },
                DxfCurve::Ellipse {
                    centre,
                    major,
                    ratio,
                    start_param,
                    end_param,
                } => DrawingShape::Ellipse {
                    centre: xy(centre),
                    major: [major.x, major.y],
                    ratio,
                    start_param,
                    end_param,
                },
                DxfCurve::Polyline { vertices, closed } => DrawingShape::Polyline {
                    points: vertices.iter().map(|(p, _)| xy(*p)).collect(),
                    bulges: vertices.iter().map(|(_, b)| *b).collect(),
                    closed,
                },
                DxfCurve::Spline {
                    degree,
                    knots,
                    control_points,
                    weights,
                    closed,
                } => DrawingShape::Spline {
                    points: spline_points(degree, knots, control_points, weights)?,
                    closed,
                },
            };
            Some(DrawingCurve {
                hidden: entity.hidden,
                shape,
                layer: entity.layer,
            })
        })
        .collect();
    let texts = read
        .texts
        .into_iter()
        .map(|label| DrawingText {
            at: xy(label.text.at),
            height: label.text.height,
            angle: label.text.angle,
            text: label.text.text,
            layer: label.layer,
        })
        .collect();
    Ok(Drawing2d {
        unit_mm: read.unit_mm,
        curves,
        texts,
    })
}

/// A drawing as an R2000 DXF: each curve and text as itself on its layer,
/// the layers declared in the order they are first drawn on (dashed when
/// their curves are hidden), the unit from `unit_mm`.
///
/// # Errors
///
/// `KernelError::InvalidInput` when the unit is neither millimetres nor inches,
/// or the kernel refuses a layer's name or a curve.
pub fn write_dxf_drawing(drawing: &Drawing2d) -> KernelResult<String> {
    let unit = match drawing.unit_mm {
        None => DxfUnit::Unitless,
        Some(1.0) => DxfUnit::Millimetre,
        Some(25.4) => DxfUnit::Inch,
        Some(mm) => {
            return Err(KernelError::InvalidInput(format!(
                "a DXF is written in millimetres or inches, not {mm} mm units"
            )));
        }
    };
    let mut layers: Vec<DxfLayer> = Vec::new();
    let mut layer = |name: &str, hidden: bool| -> usize {
        let name = if name.is_empty() { "0" } else { name };
        match layers.iter().position(|l| l.name == name) {
            Some(i) => {
                layers[i].dashed |= hidden;
                i
            }
            None => {
                layers.push(DxfLayer {
                    name: name.to_string(),
                    dashed: hidden,
                });
                layers.len() - 1
            }
        }
    };
    let p = |q: [f64; 2]| Point2::new(q[0], q[1]);
    let mut items = Vec::with_capacity(drawing.curves.len() + drawing.texts.len());
    for curve in &drawing.curves {
        let shape = match &curve.shape {
            DrawingShape::Polyline {
                points,
                bulges,
                closed,
            } => DxfCurve::Polyline {
                vertices: points
                    .iter()
                    .enumerate()
                    .map(|(i, q)| (p(*q), bulges.get(i).copied().unwrap_or(0.0)))
                    .collect(),
                closed: *closed,
            },
            DrawingShape::Arc {
                centre,
                radius,
                start_angle,
                end_angle,
            } => DxfCurve::Arc {
                centre: p(*centre),
                radius: *radius,
                start_angle: *start_angle,
                end_angle: *end_angle,
            },
            DrawingShape::Circle { centre, radius } => DxfCurve::Circle {
                centre: p(*centre),
                radius: *radius,
            },
            DrawingShape::Ellipse {
                centre,
                major,
                ratio,
                start_param,
                end_param,
            } => DxfCurve::Ellipse {
                centre: p(*centre),
                major: Vector2::new(major[0], major[1]),
                ratio: *ratio,
                start_param: *start_param,
                end_param: *end_param,
            },
            DrawingShape::Spline { points, closed } => DxfCurve::Spline {
                degree: 3,
                knots: Vec::new(),
                control_points: points.iter().map(|q| p(*q)).collect(),
                weights: None,
                closed: *closed,
            },
        };
        items.push(DxfItem {
            layer: layer(&curve.layer, curve.hidden),
            shape: DxfShape::Curve(shape),
        });
    }
    for text in &drawing.texts {
        items.push(DxfItem {
            layer: layer(&text.layer, false),
            shape: DxfShape::Text(DxfText {
                at: p(text.at),
                height: text.height,
                angle: text.angle,
                text: text.text.clone(),
            }),
        });
    }
    write_dxf_entities(&DxfOut {
        unit,
        layers: &layers,
        items: &items,
    })
    .map_err(|e| KernelError::InvalidInput(format!("the DXF could not be written: {e}")))
}

/// Points along a DXF spline: the exact curve sampled evenly within each
/// knot span, or its fit points as given when it has no knots. None when
/// its knots and points do not describe a curve.
fn spline_points(
    degree: usize,
    knots: Vec<f64>,
    control_points: Vec<Point2>,
    weights: Option<Vec<f64>>,
) -> Option<Vec<[f64; 2]>> {
    if knots.is_empty() {
        return (control_points.len() >= 2)
            .then(|| control_points.iter().map(|p| [p.x, p.y]).collect());
    }
    let tol = tess::tolerances();
    let spans = knots.windows(2).filter(|w| w[1] > w[0]).count().max(1);
    let knot_vector = ogeom::math::KnotVector::new(knots, degree).ok()?;
    let curve = match weights {
        Some(weights) if weights.len() == control_points.len() => {
            let weighted = control_points
                .into_iter()
                .zip(weights)
                .map(|(p, w)| ogeom::math::Weighted::new(p, w, tol))
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            ogeom::geom::BSpline2d::rational(knot_vector, weighted).ok()?
        }
        _ => ogeom::geom::BSpline2d::new(knot_vector, control_points, tol).ok()?,
    };
    let (t0, t1) = curve.domain();
    let n = spans * SAMPLES_PER_SPAN;
    (0..=n)
        .map(|i| {
            let t = t0 + (t1 - t0) * i as f64 / n as f64;
            curve.point_at(t, tol).ok().map(|p| [p.x, p.y])
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_polylines_come_through_marked_hidden_or_not() {
        let text = ogeom::io::dxf::write_dxf(
            &[vec![Point2::new(0.0, 0.0), Point2::new(4.0, 0.0)]],
            &[vec![Point2::new(1.0, 1.0), Point2::new(2.0, 3.5)]],
        );
        let drawing = read_dxf(&text).unwrap();
        let shown: Vec<(bool, Vec<[f64; 2]>)> = drawing
            .curves
            .into_iter()
            .map(|c| match c.shape {
                DrawingShape::Polyline { points, .. } => (c.hidden, points),
                other => panic!("a polyline, not {other:?}"),
            })
            .collect();
        assert!(
            shown.contains(&(false, vec![[0.0, 0.0], [4.0, 0.0]])),
            "{shown:?}"
        );
        assert!(
            shown.contains(&(true, vec![[1.0, 1.0], [2.0, 3.5]])),
            "{shown:?}"
        );
    }
}
