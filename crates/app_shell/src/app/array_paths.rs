//! The places a path or point array's copies stand, read from what the
//! array runs along: a feature's chain of curves or its points (asked of
//! the bench that owns it), or an edge of a body's solid (asked of the
//! kernel), each in the document's axes where its body sits.

use core_document::{
    ArrayPattern, BodyId, Document, DocumentService, FeatureId, PathSource, PathStation,
};
use kernel_api::{KernelQueries, PathFrames};

/// A path array's pattern: `count` stations along `along`, `spacing`
/// millimetres apart or spread over the whole path.
pub fn path_pattern(
    document: &Document,
    registry: &DocumentService,
    along: PathSource,
    count: u32,
    spacing: Option<f32>,
    align: bool,
) -> Result<ArrayPattern, String> {
    let (frames, normal) = read_path(document, registry, along, count, spacing)?;
    Ok(ArrayPattern::Path {
        along,
        stations: frames,
        spacing,
        align,
        normal,
    })
}

/// A point array's pattern: every point `from` lays out that stands alone.
pub fn points_pattern(
    document: &Document,
    registry: &DocumentService,
    from: FeatureId,
) -> Result<ArrayPattern, String> {
    let paths = registry
        .feature_paths(document, from)
        .ok_or("lays out no points")?;
    if paths.points.is_empty() {
        return Err("lays out no points that stand alone".into());
    }
    let placement = feature_placement(document, from);
    let points = paths
        .points
        .iter()
        .map(|&[u, v]| placement.point(on_plane(&paths.plane, u, v)))
        .collect();
    Ok(ArrayPattern::Points { from, points })
}

/// How long the path `along` is, mm.
pub fn path_length(
    document: &Document,
    registry: &DocumentService,
    along: PathSource,
) -> Result<f64, String> {
    frames_of(document, registry, along, &[]).map(|(f, _)| f.length)
}

/// The stations along `along` for `count` occurrences, and the normal of
/// the plane it lies in, when that is known.
fn read_path(
    document: &Document,
    registry: &DocumentService,
    along: PathSource,
    count: u32,
    spacing: Option<f32>,
) -> Result<(Vec<PathStation>, Option<[f32; 3]>), String> {
    let (first, _) = frames_of(document, registry, along, &[])?;
    let stations = core_document::path_stations(first.length, first.closed, count, spacing)
        .map_err(|e| format!("the count {e}"))?;
    let (frames, normal) = frames_of(document, registry, along, &stations)?;
    let stations = frames
        .frames
        .iter()
        .map(|f| PathStation {
            point: f.point.map(|v| v as f32),
            tangent: f.tangent.map(|v| v as f32),
        })
        .collect();
    Ok((stations, normal))
}

/// The kernel's frames along `along` at `stations`, in the document's
/// axes, and the normal of the plane it lies in.
fn frames_of(
    document: &Document,
    registry: &DocumentService,
    along: PathSource,
    stations: &[f64],
) -> Result<(PathFrames, Option<[f32; 3]>), String> {
    let kernel: &dyn KernelQueries = &kernel_ogeom::QUERIES;
    let (mut frames, placement, normal) = match along {
        PathSource::Sketch { feature, chain } => {
            let paths = registry
                .feature_paths(document, feature)
                .ok_or("lays out no curves")?;
            let wire = paths
                .chains
                .get(chain)
                .ok_or_else(|| format!("has {} chains of curves", paths.chains.len()))?;
            let frames = kernel
                .wire_frames(&paths.plane, wire, stations)
                .map_err(|e| e.to_string())?;
            let placement = feature_placement(document, feature);
            let normal = placement.direction(paths.plane.normal.map(|v| v as f32));
            (frames, placement, Some(normal))
        }
        PathSource::Edge { body, point } => {
            let blob = document
                .imported_brep_blob(body)
                .ok_or("the body has no solid to take an edge from")?;
            let frames = kernel
                .edge_frames(blob, point.map(f64::from), stations)
                .map_err(|e| e.to_string())?;
            (frames, document.body_placement(body), None)
        }
    };
    for frame in &mut frames.frames {
        frame.point = placement
            .point(frame.point.map(|v| v as f32))
            .map(f64::from);
        frame.tangent = placement
            .direction(frame.tangent.map(|v| v as f32))
            .map(f64::from);
    }
    Ok((frames, normal))
}

/// Where `body` sits, for what is in its own frame.
fn feature_placement(document: &Document, feature: FeatureId) -> core_document::BodyPlacement {
    document
        .get_feature_meta(feature)
        .and_then(|n| n.body)
        .map(|b: BodyId| document.body_placement(b))
        .unwrap_or_default()
}

fn on_plane(plane: &kernel_api::ProfilePlane, u: f64, v: f64) -> [f32; 3] {
    std::array::from_fn(|i| (plane.origin[i] + plane.x_axis[i] * u + plane.y_axis[i] * v) as f32)
}
