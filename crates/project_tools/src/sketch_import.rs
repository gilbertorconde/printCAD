//! Translate local geometry without solving it; the saved state is the oracle's input.

use crate::numeric::scalar;
use crate::report::{Finding, note};
use glam::DVec2;
use project_archive::archive::FcstdError;
use project_archive::coordinates::Point2;
use project_archive::sketch_decode::{
    FcConstraint, FcConstraintType as C, FcGeoIndex, FcGeometry, FcPointPos as P, FcShape, FcSketch,
};
use uuid::Uuid;
use wb_sketch::sketch::{
    Arc, BSpline, Circle, Constraint, ConstraintKind as K, GeometryElement as G, Line, ORIGIN_ID,
    Point, Sketch, Vec2D, X_AXIS_ID, Y_AXIS_ID,
};

pub struct ImportedSketch {
    pub sketch: Sketch,
    pub findings: Vec<Finding>,
    /// Original f64 positions/radii, keyed by the native geometry they became.
    pub source_points: Vec<(Uuid, DVec2)>,
    pub source_radii: Vec<(Uuid, f64)>,
}

pub fn import_sketch(source: &FcSketch, name: &str) -> Result<ImportedSketch, FcstdError> {
    let mut imported = ImportedSketch {
        sketch: Sketch::new(name),
        findings: Vec::new(),
        source_points: Vec::new(),
        source_radii: Vec::new(),
    };
    let mut elements = Vec::new();
    let mut externals = Vec::new();
    let drawn = (source.geometry.iter().enumerate())
        .map(|(i, g)| (format!("Geometry[{i}]"), g, g.construction, false));
    // Projected curves come in as their last projection, held where they
    // are: in the profile when they define it, construction otherwise.
    let projected = (source.external.iter().enumerate()).map(|(i, e)| {
        (
            format!("ExternalGeo[{}]", i + 2),
            &e.geometry,
            !e.defining,
            true,
        )
    });
    // Points of projected guides, and points drawn on their own: what lies
    // on them stays a constraint rather than a shared point, so each keeps
    // its own role (a hole's centre stays a point of the sketch).
    let mut guide_points = std::collections::HashSet::new();
    for (list, geometry, construction, external) in drawn.chain(projected) {
        let first_point = imported.source_points.len();
        let id = add_element(&mut imported, name, &list, geometry, construction)?;
        if !external {
            if !construction && matches!(geometry.shape, FcShape::Point(_)) {
                guide_points.extend(id);
            }
            elements.push(id);
            continue;
        }
        if let Some(element) = id {
            (imported.sketch.constraints).push(Constraint::new(K::Block { element }));
        }
        if construction {
            guide_points.extend(
                imported.source_points[first_point..]
                    .iter()
                    .map(|(p, _)| *p),
            );
        }
        externals.push(id);
    }
    if !source.external.is_empty() {
        note(
            &mut imported.findings,
            name,
            "external geometry",
            format!(
                "{} external items kept fixed where they were last projected",
                source.external.len()
            ),
        );
    }
    for (index, constraint) in source.constraints.iter().enumerate() {
        for geo in [constraint.first, constraint.second, constraint.third] {
            if geo.0 >= source.geometry.len() as i64 {
                return Err(FcstdError::Format {
                    object: name.into(),
                    reason: format!(
                        "constraint #{index}: geometry index {} is out of range",
                        geo.0
                    ),
                });
            }
        }
        let value = scalar(
            name,
            &format!("Constraints[{index}].Value"),
            constraint.value,
        )?;
        match translate(&imported.sketch, (&elements, &externals), constraint, value) {
            Some(kinds) => {
                for kind in kinds {
                    let mut native = Constraint::new(kind);
                    native.active = constraint.active;
                    native.driving = constraint.driving;
                    native.name = (!constraint.name.is_empty()).then(|| constraint.name.clone());
                    if native.kind.is_dimensional() {
                        note(
                            &mut imported.findings,
                            name,
                            "dimension label",
                            format!(
                                "#{index}: label position reset (source distance {})",
                                constraint.label_distance
                            ),
                        );
                    }
                    imported.sketch.constraints.push(native);
                }
            }
            None => note(
                &mut imported.findings,
                name,
                "constraint",
                format!(
                    "#{index}: {} {:?}/{:?}, {:?}/{:?}",
                    constraint.kind.name(),
                    constraint.first,
                    constraint.first_pos,
                    constraint.second,
                    constraint.second_pos
                ),
            ),
        }
    }
    // Active coincidence becomes native shared-vertex topology. Other constraints
    // remain constraints; proximity alone never merges unrelated source points.
    let mut aliases = std::collections::HashMap::new();
    let canonical = |mut id, aliases: &std::collections::HashMap<Uuid, Uuid>| {
        while let Some(&next) = aliases.get(&id) {
            id = next;
        }
        id
    };
    for constraint in &imported.sketch.constraints {
        if constraint.active
            && constraint.driving
            && let K::Coincident { point1, point2 } = constraint.kind
            && !guide_points.contains(&point1)
            && !guide_points.contains(&point2)
        {
            let (a, b) = (canonical(point1, &aliases), canonical(point2, &aliases));
            if a != b {
                if a == ORIGIN_ID {
                    aliases.insert(b, a);
                } else {
                    aliases.insert(a, b);
                }
            }
        }
    }
    let aliases: std::collections::HashMap<_, _> = aliases
        .keys()
        .map(|&id| (id, canonical(id, &aliases)))
        .collect();
    wb_sketch::operations::join_points(&mut imported.sketch, &aliases).map_err(|error| {
        FcstdError::Format {
            object: name.into(),
            reason: error.to_string(),
        }
    })?;
    for (id, _) in &mut imported.source_points {
        *id = canonical(*id, &aliases);
    }
    Ok(imported)
}

/// One curve or point of the source, added to the sketch; `None` where it
/// has no counterpart, which is named in the findings.
fn add_element(
    imported: &mut ImportedSketch,
    name: &str,
    list: &str,
    geometry: &FcGeometry,
    construction: bool,
) -> Result<Option<Uuid>, FcstdError> {
    let field = |part: &str| format!("{list}.{part}");
    // Internal geometry of an unsupported parent is not an independent drawn curve.
    if geometry.internal != 0 {
        note(
            &mut imported.findings,
            name,
            "geometry",
            "internal geometry omitted",
        );
        return Ok(None);
    }
    let element = match geometry.shape {
        FcShape::Line { start, end } if start != end => {
            let a = point(imported, name, &field("LineSegment.start"), start)?;
            let b = point(imported, name, &field("LineSegment.end"), end)?;
            G::Line(Line::new(a, b))
        }
        FcShape::Circle { center, radius } if radius > 0.0 => {
            let center = point(imported, name, &field("Circle.center"), center)?;
            G::Circle(Circle::new(
                center,
                scalar(name, &field("Circle.radius"), radius)?,
            ))
        }
        FcShape::Arc {
            center,
            radius,
            start,
            end,
        } if radius > 0.0 && start != end => {
            let center = point(imported, name, &field("Arc.center"), center)?;
            let start = point(imported, name, &field("Arc.start"), start)?;
            let end = point(imported, name, &field("Arc.end"), end)?;
            G::Arc(Arc::new(
                center,
                start,
                end,
                scalar(name, &field("Arc.radius"), radius)?,
            ))
        }
        FcShape::BSpline {
            ref poles,
            ref weights,
            ref knots,
            degree,
            periodic,
        } if !periodic && degree >= 1 && poles.len() > degree as usize => {
            let mut control_points = Vec::new();
            for (i, pole) in poles.iter().enumerate() {
                let id = point(imported, name, &field(&format!("Pole[{i}]")), *pole)?;
                imported.sketch.set_construction(id, true);
                control_points.push(id);
            }
            let mut spline = BSpline::new(control_points, false);
            spline.degree = degree;
            spline.knots = knots
                .iter()
                .flat_map(|&(value, mult)| std::iter::repeat_n(value, mult as usize))
                .collect();
            if weights.iter().any(|&w| w != 1.0) {
                spline.weights = weights.clone();
            }
            G::BSpline(spline)
        }
        FcShape::Point(p) => {
            let id = point(imported, name, &field("Point"), p)?;
            imported.sketch.set_construction(id, construction);
            return Ok(Some(id));
        }
        ref unsupported => {
            note(
                &mut imported.findings,
                name,
                "geometry",
                format!("unsupported or degenerate {unsupported:?}"),
            );
            return Ok(None);
        }
    };
    let radius = match geometry.shape {
        FcShape::Circle { radius, .. } | FcShape::Arc { radius, .. } => Some(radius),
        _ => None,
    };
    let id = imported.sketch.add_geometry(element);
    if let Some(radius) = radius {
        imported.source_radii.push((id, radius));
    }
    imported.sketch.set_construction(id, construction);
    Ok(Some(id))
}

fn point(
    imported: &mut ImportedSketch,
    name: &str,
    field: &str,
    p: Point2,
) -> Result<Uuid, FcstdError> {
    let id = imported.sketch.add_geometry(G::Point(Point::new(Vec2D::new(
        scalar(name, &format!("{field}.x"), p.x)?,
        scalar(name, &format!("{field}.y"), p.y)?,
    ))));
    imported
        .source_points
        .push((id, DVec2::from_array(p.to_array())));
    Ok(id)
}

fn element(elements: Elements, index: FcGeoIndex) -> Option<Uuid> {
    match index {
        FcGeoIndex::H_AXIS => Some(X_AXIS_ID),
        FcGeoIndex::V_AXIS => Some(Y_AXIS_ID),
        FcGeoIndex(i) if i >= 0 => elements.0.get(i as usize).copied().flatten(),
        FcGeoIndex(i) => elements
            .1
            .get(usize::try_from(-3 - i).ok()?)
            .copied()
            .flatten(),
    }
}

/// The sketch's elements by source index: its own geometry, then its
/// external geometry (`-3` the first).
type Elements<'a> = (&'a [Option<Uuid>], &'a [Option<Uuid>]);

fn endpoint(sketch: &Sketch, elements: Elements, index: FcGeoIndex, pos: P) -> Option<Uuid> {
    if index == FcGeoIndex::H_AXIS && pos == P::Start {
        return Some(ORIGIN_ID);
    }
    let id = element(elements, index)?;
    match (sketch.get_geometry(id)?, pos) {
        (G::Point(_), P::Start | P::Edge) => Some(id),
        (G::Line(l), P::Start) => Some(l.start),
        (G::Line(l), P::End) => Some(l.end),
        (G::Arc(a), P::Start) => Some(a.start),
        (G::Arc(a), P::End) => Some(a.end),
        (G::Arc(a), P::Mid) => Some(a.center),
        // A clamped spline starts and ends on its end poles.
        (G::BSpline(b), P::Start) => b.control_points.first().copied(),
        (G::BSpline(b), P::End) => b.control_points.last().copied(),
        (G::Circle(c), P::Mid) => Some(c.center),
        _ => None,
    }
}

fn translate(sketch: &Sketch, elements: Elements, c: &FcConstraint, value: f64) -> Option<Vec<K>> {
    (|| {
        let a = element(elements, c.first)?;
        let b = element(elements, c.second);
        let p = endpoint(sketch, elements, c.first, c.first_pos);
        let q = endpoint(sketch, elements, c.second, c.second_pos);
        let line = |id| {
            id == X_AXIS_ID
                || id == Y_AXIS_ID
                || matches!(sketch.get_geometry(id), Some(G::Line(_)))
        };
        let round = |id| matches!(sketch.get_geometry(id), Some(G::Arc(_) | G::Circle(_)));
        let pair = c.first_pos == P::Edge && c.second_pos == P::Edge;
        let kinds = match c.kind {
            C::Coincident => vec![K::Coincident {
                point1: p?,
                point2: q?,
            }],
            C::Horizontal | C::Vertical if c.first_pos != P::Edge => {
                if c.kind == C::Horizontal {
                    vec![K::HorizontalPoints {
                        point1: p?,
                        point2: q?,
                    }]
                } else {
                    vec![K::VerticalPoints {
                        point1: p?,
                        point2: q?,
                    }]
                }
            }
            C::Horizontal if line(a) => vec![K::Horizontal { element: a }],
            C::Vertical if line(a) => vec![K::Vertical { element: a }],
            C::Parallel if pair && line(a) && line(b?) => vec![K::Parallel {
                line1: a,
                line2: b?,
            }],
            C::Perpendicular if pair && line(a) && line(b?) => vec![K::Perpendicular {
                line1: a,
                line2: b?,
            }],
            C::Angle if pair && line(a) && line(b?) => vec![K::Angle {
                line1: a,
                line2: b?,
                angle_rad: value,
            }],
            C::Tangent if (line(a) || round(a)) && (line(b?) || round(b?)) => {
                let mut kinds = vec![K::Tangent {
                    line_or_circle1: a,
                    item2: b?,
                }];
                if !pair {
                    kinds.push(K::Coincident {
                        point1: p?,
                        point2: q?,
                    });
                }
                kinds
            }
            C::Equal if line(a) && line(b?) => vec![K::EqualLength {
                line1: a,
                line2: b?,
            }],
            C::Equal if round(a) && round(b?) => vec![K::EqualRadius {
                circle1: a,
                circle2: b?,
            }],
            C::Radius if round(a) => vec![K::Radius {
                circle: a,
                radius: value,
            }],
            C::Diameter if round(a) => vec![K::Diameter {
                circle: a,
                diameter: value,
            }],
            C::PointOnObject if line(b?) => vec![K::PointOnLine {
                point: p?,
                line: b?,
            }],
            C::PointOnObject if round(b?) => vec![K::PointOnCircle {
                point: p?,
                circle: b?,
            }],
            C::Distance if c.second.is_none() && c.first_pos == P::Edge && line(a) => {
                vec![K::Length {
                    line: a,
                    length: value,
                }]
            }
            C::Distance => vec![K::Distance {
                point1: p?,
                point2: q?,
                distance: value,
            }],
            C::DistanceX | C::DistanceY => {
                let (a, b) = if c.first_pos == P::Edge {
                    let G::Line(l) = sketch.get_geometry(a)? else {
                        return None;
                    };
                    (l.start, Some(l.end))
                } else if c.second.is_none() {
                    (p?, None)
                } else {
                    (p?, Some(q?))
                };
                // The native signed distance only fixes the side for negative values.
                // A positive source distance pointing backwards cannot be represented.
                let pa = sketch.point_position(a)?;
                let pb = b
                    .and_then(|id| sketch.point_position(id))
                    .unwrap_or(Vec2D::new(0.0, 0.0));
                let delta = if b.is_none() {
                    if c.kind == C::DistanceX { pa.x } else { pa.y }
                } else if c.kind == C::DistanceX {
                    pb.x - pa.x
                } else {
                    pb.y - pa.y
                };
                if value > 0.0 && delta < 0.0 {
                    return None;
                }
                if c.kind == C::DistanceX {
                    vec![K::DistanceX { a, b, value }]
                } else {
                    vec![K::DistanceY { a, b, value }]
                }
            }
            C::Symmetric if c.third_pos == P::Edge => vec![K::Symmetric {
                point1: p?,
                point2: q?,
                line: element(elements, c.third)?,
            }],
            C::Symmetric => vec![K::SymmetricAboutPoint {
                point1: p?,
                point2: q?,
                center: endpoint(sketch, elements, c.third, c.third_pos)?,
            }],
            C::Block => vec![K::Block { element: a }],
            _ => return None,
        };
        Some(kinds)
    })()
}
