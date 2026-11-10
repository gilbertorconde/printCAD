//! Pattern transforms: the occurrences of linear, circular, point, polar
//! and path patterns, and the matrices they are made of.

use super::{dot3, edge_probe_of, load_sketch, mirror_plane};
use core_document::{Document, FeatureId, WorkbenchFeature};
use wb_sketch::SketchFeature;
use wb_sketch::profile;
use wb_sketch::sketch::GeometryElement;

use crate::feature::{DesignFeature, PatternAxis, TransformStep};

// Pattern transforms: row-major 4x4, last row 0 0 0 1.

pub(crate) type Mat4 = [[f64; 4]; 4];

fn mat_identity() -> Mat4 {
    let mut m = [[0.0; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    m
}

pub(super) fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for r in 0..4 {
        for c in 0..4 {
            out[r][c] = (0..4).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

pub(super) fn mat_translation(v: [f64; 3]) -> Mat4 {
    let mut m = mat_identity();
    m[0][3] = v[0];
    m[1][3] = v[1];
    m[2][3] = v[2];
    m
}

pub(super) fn normalize(v: [f64; 3]) -> Result<[f64; 3], String> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-12 {
        return Err("axis direction is zero".into());
    }
    Ok([v[0] / len, v[1] / len, v[2] / len])
}

pub(super) fn mat_rotation(
    origin: [f64; 3],
    dir: [f64; 3],
    angle_deg: f64,
) -> Result<Mat4, String> {
    let [x, y, z] = normalize(dir)?;
    let a = angle_deg.to_radians();
    let (s, c) = a.sin_cos();
    let t = 1.0 - c;
    let r = [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ];
    let mut m = mat_identity();
    for i in 0..3 {
        m[i][..3].copy_from_slice(&r[i]);
        // Affine part: rotate about `origin`, not the world origin.
        m[i][3] = origin[i] - r[i][0] * origin[0] - r[i][1] * origin[1] - r[i][2] * origin[2];
    }
    Ok(m)
}

fn mat_mirror(point: [f64; 3], normal: [f64; 3]) -> Mat4 {
    let n = normalize(normal).unwrap_or([0.0, 0.0, 1.0]);
    let d = point[0] * n[0] + point[1] * n[1] + point[2] * n[2];
    let mut m = mat_identity();
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (if i == j { 1.0 } else { 0.0 }) - 2.0 * n[i] * n[j];
        }
        m[i][3] = 2.0 * d * n[i];
    }
    m
}

fn mat_scale(center: [f64; 3], factor: f64) -> Mat4 {
    let mut m = mat_identity();
    for i in 0..3 {
        m[i][i] = factor;
        m[i][3] = center[i] * (1.0 - factor);
    }
    m
}

/// A pattern's axis as a point and a unit direction, in the body's own
/// frame: a picked edge as it was picked, a datum line where the datum
/// sits now, a sketch's axis where the sketch sits now.
pub(crate) fn pattern_axis(
    document: &Document,
    axis: &PatternAxis,
) -> Result<([f64; 3], [f64; 3]), String> {
    let (origin, dir) = match axis {
        PatternAxis::X => ([0.0; 3], [1.0, 0.0, 0.0]),
        PatternAxis::Y => ([0.0; 3], [0.0, 1.0, 0.0]),
        PatternAxis::Z => ([0.0; 3], [0.0, 0.0, 1.0]),
        PatternAxis::Custom { origin, dir } => (origin.map(f64::from), dir.map(f64::from)),
        PatternAxis::Edge(edge) => (edge.point.map(f64::from), edge.direction.map(f64::from)),
        PatternAxis::Datum(id) => {
            let data = document
                .feature_values(*id)
                .ok_or("the pattern's datum line is missing")?;
            let datum = core_document::DatumFeature::from_json(data)
                .map_err(|_| "the pattern's axis is not a datum".to_string())?;
            if !matches!(datum.shape, core_document::DatumShape::Line { .. }) {
                return Err("the pattern's datum is not a line".into());
            }
            let frame = datum.frame();
            (frame.origin.map(f64::from), frame.x_axis.map(f64::from))
        }
        PatternAxis::Sketch { sketch, axis } => {
            let data = document
                .feature_values(*sketch)
                .ok_or("the pattern axis's sketch is missing")?;
            let sketch = SketchFeature::from_json(data)
                .map_err(|_| "the pattern's axis is not a sketch's".to_string())?;
            let plane = profile::plane_of(&sketch.plane);
            let dir = match axis {
                crate::feature::SketchAxis::Horizontal => plane.x_axis,
                crate::feature::SketchAxis::Vertical => plane.y_axis,
                crate::feature::SketchAxis::Normal => plane.normal,
            };
            (plane.origin, dir)
        }
    };
    let dir = normalize(dir).map_err(|_| "the pattern axis has no direction".to_string())?;
    Ok((origin, dir))
}

/// The offsets from the original, first copy to last, of `count` copies
/// spaced `even` apart, save where `gaps` gives the gap before a copy.
fn uneven_offsets(count: u32, even: f64, gaps: &[f32]) -> Vec<f64> {
    let mut at = 0.0;
    (0..count as usize)
        .map(|i| {
            at += gaps.get(i).map_or(even, |gap| f64::from(*gap));
            at
        })
        .collect()
}

fn linear_transforms(
    (_, dir): ([f64; 3], [f64; 3]),
    length: f32,
    occurrences: u32,
    spacing_mode: bool,
    reversed: bool,
    spacings: &[f32],
) -> Result<Vec<Mat4>, String> {
    // One occurrence is the original alone: nothing to copy.
    if occurrences == 0 {
        return Err("a linear pattern needs at least 1 occurrence".into());
    }
    if occurrences == 1 {
        return Ok(Vec::new());
    }
    let sign = if reversed { -1.0 } else { 1.0 };
    let spacing = if spacing_mode {
        f64::from(length)
    } else {
        f64::from(length) / f64::from(occurrences - 1)
    };
    let gaps = spacings.len().min(occurrences as usize - 1);
    if spacing.abs() < 1e-9 && gaps < occurrences as usize - 1 {
        return Err("pattern spacing is zero".into());
    }
    if let Some(i) = spacings[..gaps].iter().position(|gap| gap.abs() < 1e-6) {
        return Err(format!("spacing {} of the pattern is zero", i + 1));
    }
    Ok(uneven_offsets(occurrences - 1, spacing, spacings)
        .into_iter()
        .map(|offset| {
            let d = offset * sign;
            mat_translation([dir[0] * d, dir[1] * d, dir[2] * d])
        })
        .collect())
}

/// Concentric rings about the axis through `origin` along `dir`: ring `i`
/// moved `i × spacing` along `outward` (square to the axis), then turned
/// round it in `counts[i]` even steps; the original, ring 0's first, left
/// out.
pub(super) fn circular_transforms(
    (origin, dir): ([f64; 3], [f64; 3]),
    outward: [f64; 3],
    spacing: f32,
    counts: &[u32],
) -> Result<Vec<Mat4>, String> {
    if counts.is_empty() || counts.contains(&0) {
        return Err("every ring needs at least one occurrence".into());
    }
    let axis = normalize(dir)?;
    let along = dot3(outward, axis);
    let out = normalize([
        outward[0] - axis[0] * along,
        outward[1] - axis[1] * along,
        outward[2] - axis[2] * along,
    ])
    .map_err(|_| "the rings' outward direction runs along the axis".to_string())?;
    let mut transforms = Vec::new();
    for (ring, count) in counts.iter().enumerate() {
        let step = f64::from(spacing) * ring as f64;
        let shift = mat_translation(out.map(|c| c * step));
        for k in 0..*count {
            if ring == 0 && k == 0 {
                continue;
            }
            let turn = mat_rotation(origin, axis, 360.0 * f64::from(k) / f64::from(*count))?;
            transforms.push(mat_mul(&turn, &shift));
        }
    }
    Ok(transforms)
}

/// A shift to every point of `sketch` after its first, by how far it is
/// from the first, in the sketch's body frame.
pub(super) fn point_transforms(
    document: &Document,
    sketch: FeatureId,
) -> Result<Vec<Mat4>, String> {
    let feature = load_sketch(document, sketch)?;
    let plane = profile::plane_of(&feature.plane);
    let points: Vec<[f64; 3]> = feature
        .sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) if !feature.sketch.is_construction(p.id) => Some(p.position),
            _ => None,
        })
        .map(|p| {
            let (x, y) = (p.x, p.y);
            std::array::from_fn(|k| plane.origin[k] + plane.x_axis[k] * x + plane.y_axis[k] * y)
        })
        .collect();
    let Some(first) = points.first().copied() else {
        return Err("the points sketch has no points".into());
    };
    Ok(points[1..]
        .iter()
        .map(|p| mat_translation(std::array::from_fn(|k| p[k] - first[k])))
        .collect())
}

pub(super) fn polar_transforms(
    (origin, dir): ([f64; 3], [f64; 3]),
    angle_deg: f32,
    occurrences: u32,
    reversed: bool,
    step_mode: bool,
    angles: &[f32],
) -> Result<Vec<Mat4>, String> {
    if occurrences == 0 {
        return Err("a polar pattern needs at least 1 occurrence".into());
    }
    if occurrences == 1 {
        return Ok(Vec::new());
    }
    let full_circle = (f64::from(angle_deg) - 360.0).abs() < 1e-6;
    let step = if step_mode {
        f64::from(angle_deg)
    } else if full_circle {
        // 360° spreads evenly without doubling the original position.
        f64::from(angle_deg) / f64::from(occurrences)
    } else {
        f64::from(angle_deg) / f64::from(occurrences - 1)
    };
    let gaps = angles.len().min(occurrences as usize - 1);
    if step.abs() < 1e-9 && gaps < occurrences as usize - 1 {
        return Err("the pattern's angle between occurrences is zero".into());
    }
    if let Some(i) = angles[..gaps].iter().position(|gap| gap.abs() < 1e-6) {
        return Err(format!("step angle {} of the pattern is zero", i + 1));
    }
    let sign = if reversed { -1.0 } else { 1.0 };
    uneven_offsets(occurrences - 1, step, angles)
        .into_iter()
        .map(|angle| mat_rotation(origin, dir, angle * sign))
        .collect()
}

/// Where every occurrence of a pattern or mirror stands, in its body's
/// own frame: the original's (the identity) first, then each copy's, in
/// the order `skipped` counts them. Empty for any other feature; an error
/// for one with a path step, whose places the kernel finds
/// (`occurrence_steps`).
pub(crate) fn occurrence_transforms(
    document: &Document,
    feature: &DesignFeature,
) -> Result<Vec<Mat4>, String> {
    let steps = occurrence_steps(document, feature)?;
    if steps.is_empty() {
        return Ok(Vec::new());
    }
    compose_steps(&steps)
        .ok_or_else(|| "the pattern's path is followed on the solid as it builds".to_string())
}

/// A pattern's or mirror's occurrences as steps the kernel composes, each
/// applying to every occurrence of the steps before it (the original
/// among them). Empty for any other feature.
pub(crate) fn occurrence_steps(
    document: &Document,
    feature: &DesignFeature,
) -> Result<Vec<kernel_api::PatternStep>, String> {
    use kernel_api::PatternStep;
    let copies = match feature {
        DesignFeature::Mirrored { plane, .. } => {
            let (point, normal) = mirror_plane(document, plane)?;
            vec![mat_mirror(point, normal)]
        }
        DesignFeature::LinearPattern {
            axis,
            length,
            occurrences,
            spacing_mode,
            reversed,
            spacings,
            ..
        } => linear_transforms(
            pattern_axis(document, axis)?,
            *length,
            *occurrences,
            *spacing_mode,
            *reversed,
            spacings,
        )?,
        DesignFeature::PolarPattern {
            axis,
            angle_deg,
            occurrences,
            reversed,
            step_mode,
            angles,
            ..
        } => polar_transforms(
            pattern_axis(document, axis)?,
            *angle_deg,
            *occurrences,
            *reversed,
            *step_mode,
            angles,
        )?,
        DesignFeature::CircularPattern {
            axis,
            outward,
            spacing,
            counts,
            ..
        } => circular_transforms(
            pattern_axis(document, axis)?,
            pattern_axis(document, outward)?.1,
            *spacing,
            counts,
        )?,
        DesignFeature::PointPattern { sketch, .. } => point_transforms(document, *sketch)?,
        DesignFeature::PathPattern { path, layout, .. } => {
            return Ok(vec![PatternStep::Path(path_layout(
                document, path, layout,
            )?)]);
        }
        DesignFeature::MultiTransform { steps, .. } => {
            if steps.is_empty() {
                return Err("a multi-transform needs at least one step".into());
            }
            return steps.iter().map(|step| step_of(document, step)).collect();
        }
        _ => return Ok(Vec::new()),
    };
    Ok(vec![PatternStep::Transforms(copies)])
}

/// Every occurrence of `steps`, the identity first, when none follows a
/// path: each step's copies of every occurrence before it.
pub(super) fn compose_steps(steps: &[kernel_api::PatternStep]) -> Option<Vec<Mat4>> {
    let mut accumulated = vec![mat_identity()];
    for step in steps {
        let kernel_api::PatternStep::Transforms(own) = step else {
            return None;
        };
        let mut next = Vec::with_capacity(accumulated.len() * (own.len() + 1));
        for base in &accumulated {
            next.push(*base);
            for t in own {
                next.push(mat_mul(t, base));
            }
        }
        accumulated = next;
    }
    Some(accumulated)
}

/// What the kernel walks for a path pattern's `path`, spaced as `layout`
/// says, in the body's own frame.
fn path_layout(
    document: &Document,
    path: &crate::feature::PatternPath,
    layout: &crate::feature::PathSpacing,
) -> Result<kernel_api::PathLayout, String> {
    use crate::feature::PatternPath;
    let path = match path {
        PatternPath::Sketch { sketch, chain } => {
            let feature = load_sketch(document, *sketch)?;
            let chains = profile::extract_chains(&feature.sketch)
                .map_err(|e| format!("the pattern's path sketch: {e}"))?;
            let count = chains.len();
            let wire = chains.into_iter().nth(*chain).ok_or_else(|| {
                if count == 0 {
                    "the pattern's path sketch has no curves to follow".to_string()
                } else {
                    format!(
                        "the pattern's path sketch has {count} chain{} of curves; chain {} \
                         is not one",
                        if count == 1 { "" } else { "s" },
                        chain
                    )
                }
            })?;
            kernel_api::PatternPath::Wire {
                plane: profile::plane_of(&feature.plane),
                wire,
            }
        }
        PatternPath::Edge(pick) => kernel_api::PatternPath::Edge(edge_probe_of(pick)),
    };
    if layout.occurrences == 0 {
        return Err("a path pattern needs at least 1 occurrence".into());
    }
    if layout.spacing_mode && !(layout.spacing.is_finite() && layout.spacing > 0.0) {
        return Err("the spacing along the path must be more than zero".into());
    }
    Ok(kernel_api::PathLayout {
        path,
        count: layout.occurrences,
        spacing: layout.spacing_mode.then_some(f64::from(layout.spacing)),
        reversed: layout.reversed,
        align: layout.align,
    })
}

/// `point` moved by `transform`.
pub(crate) fn transform_point(transform: &Mat4, point: [f64; 3]) -> [f64; 3] {
    let m = transform;
    [
        m[0][0] * point[0] + m[0][1] * point[1] + m[0][2] * point[2] + m[0][3],
        m[1][0] * point[0] + m[1][1] * point[1] + m[1][2] * point[2] + m[1][3],
        m[2][0] * point[0] + m[2][1] * point[1] + m[2][2] * point[2] + m[2][3],
    ]
}

/// Cartesian composition: each step's occurrences (including the identity)
/// apply to every result of the previous steps; the pure identity is dropped
/// because the base solid already contains the original. A path step,
/// whose places the kernel finds, is refused.
#[cfg(test)]
pub(super) fn multi_transforms(
    document: &Document,
    steps: &[TransformStep],
) -> Result<Vec<Mat4>, String> {
    if steps.is_empty() {
        return Err("a multi-transform needs at least one step".into());
    }
    let steps: Vec<kernel_api::PatternStep> = steps
        .iter()
        .map(|step| step_of(document, step))
        .collect::<Result<_, _>>()?;
    let accumulated = compose_steps(&steps).ok_or("a path step is followed as the body builds")?;
    // Drop the identity (index 0 by construction).
    Ok(accumulated.into_iter().skip(1).collect())
}

/// One multi-transform step's copies, the original left out.
fn step_of(document: &Document, step: &TransformStep) -> Result<kernel_api::PatternStep, String> {
    let copies: Vec<Mat4> = match step {
        TransformStep::Linear {
            axis,
            length,
            occurrences,
        } => linear_transforms(
            pattern_axis(document, axis)?,
            *length,
            *occurrences,
            false,
            false,
            &[],
        )?,
        TransformStep::Polar {
            axis,
            angle_deg,
            occurrences,
        } => polar_transforms(
            pattern_axis(document, axis)?,
            *angle_deg,
            *occurrences,
            false,
            false,
            &[],
        )?,
        TransformStep::Mirror { plane } => {
            let (point, normal) = mirror_plane(document, plane)?;
            vec![mat_mirror(point, normal)]
        }
        TransformStep::Scale {
            factor,
            center,
            occurrences,
        } => {
            if *factor <= 0.0 {
                return Err("scale factor must be positive".into());
            }
            let occ = (*occurrences).max(2);
            // The factor applies to the LAST occurrence; the rest
            // interpolate evenly.
            (1..occ)
                .map(|k| {
                    let f = 1.0 + (f64::from(*factor) - 1.0) * f64::from(k) / f64::from(occ - 1);
                    mat_scale([center[0] as f64, center[1] as f64, center[2] as f64], f)
                })
                .collect()
        }
        TransformStep::Circular {
            axis,
            outward,
            spacing,
            counts,
        } => circular_transforms(
            pattern_axis(document, axis)?,
            pattern_axis(document, outward)?.1,
            *spacing,
            counts,
        )?,
        TransformStep::Points { sketch } => point_transforms(document, *sketch)?,
        TransformStep::Path { path, layout } => {
            return Ok(kernel_api::PatternStep::Path(path_layout(
                document, path, layout,
            )?));
        }
    };
    Ok(kernel_api::PatternStep::Transforms(copies))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polar_full_circle_spacing_avoids_overlap() {
        let z = ([0.0; 3], [0.0, 0.0, 1.0]);
        let transforms = polar_transforms(z, 360.0, 4, false, false, &[]).unwrap();
        assert_eq!(transforms.len(), 3);
        // First occurrence at 90°: X axis maps to Y.
        let t = &transforms[0];
        assert!((t[0][0]).abs() < 1e-9 && (t[1][0] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn uneven_spacing_puts_each_copy_its_own_gap_past_the_last() {
        let x = ([0.0; 3], [1.0, 0.0, 0.0]);
        let transforms = linear_transforms(x, 30.0, 4, false, false, &[5.0, 15.0]).unwrap();
        let at: Vec<f64> = transforms.iter().map(|t| t[0][3]).collect();
        // 5, then 15 more, then the even 10 more.
        assert_eq!(at, vec![5.0, 20.0, 30.0]);
        let reversed = linear_transforms(x, 30.0, 3, false, true, &[5.0, 15.0]).unwrap();
        assert_eq!(reversed[1][0][3], -20.0);
        let err = linear_transforms(x, 30.0, 3, false, false, &[5.0, 0.0]).unwrap_err();
        assert!(err.contains("spacing 2"), "{err}");
    }

    #[test]
    fn mirror_transform_reflects_across_plane() {
        let m = mat_mirror([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]);
        // Point at z=2 reflects to z=8.
        let z = m[2][0] * 0.0 + m[2][1] * 0.0 + m[2][2] * 2.0 + m[2][3];
        assert!((z - 8.0).abs() < 1e-9);
    }

    #[test]
    fn multi_transform_composes_cartesian_product() {
        let steps = vec![
            TransformStep::Linear {
                axis: PatternAxis::X,
                length: 10.0,
                occurrences: 2,
            },
            TransformStep::Linear {
                axis: PatternAxis::Y,
                length: 10.0,
                occurrences: 2,
            },
        ];
        let transforms = multi_transforms(&Document::new("t"), &steps).unwrap();
        // 2x2 grid minus the original.
        assert_eq!(transforms.len(), 3);
    }
}
