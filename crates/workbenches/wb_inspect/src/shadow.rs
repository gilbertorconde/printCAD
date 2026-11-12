//! A scan's shadow on a plane, outlined clear of it by a play in lines,
//! arcs and circles, and stood up as a block: the pocket, bin or insert a
//! part sits in. The plane is kept in the scan's frame; the outline is
//! worked out from the scan's mesh during evaluation (`derive`), in the
//! document's frame, so the block follows the scan when it is aligned.
//! The shadow lives on a body of its own, whose solid is the block.

use core_document::{BodyId, Document, FeatureId, WorkbenchFeature, WorkbenchId};
use glam::DVec3;
use kernel_api::{
    BooleanOp, ExtrudeTermination, KernelQueries, Profile, ProfilePlane, ProfileSegment,
    ProfileWire, ShadowRequest, SolidOp, SweepKind, TriMesh,
};
use serde::{Deserialize, Serialize};

use crate::vec::{basis, unit};

/// The feature kind of every shadow.
pub const KIND: &str = "wb.inspect.shadow";

/// One shadow of a scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub name: String,
    pub scan: BodyId,
    /// The plane in the scan's frame, at the scan's foot along `normal`.
    pub origin: [f64; 3],
    pub normal: [f64; 3],
    pub x_axis: [f64; 3],
    /// The outline's least clearance from the shadow, mm; 0 exact,
    /// negative a press fit.
    #[serde(default = "play")]
    pub play: f64,
    /// Half the outline's band, mm.
    #[serde(default = "tolerance")]
    pub tolerance: f64,
    /// Corners turning less than this, degrees, are rounded.
    #[serde(default = "round_below")]
    pub round_below_deg: f64,
    /// Holes and notches no wider than this, mm, are filled; 0 keeps them.
    #[serde(default)]
    pub fill_below: f64,
    /// Stand the outline up as a block.
    #[serde(default = "yes")]
    pub block: bool,
    /// The block's height, mm; the part's thickness along the normal when
    /// unset.
    #[serde(default)]
    pub height: Option<f64>,
    /// What the outline came to, worked out during evaluation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made: Option<Made>,
}

fn play() -> f64 {
    1.0
}

fn tolerance() -> f64 {
    0.1
}

fn round_below() -> f64 {
    30.0
}

fn yes() -> bool {
    true
}

/// The outline as last worked out, and what it was worked out from.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Made {
    pub key: u64,
    /// The plane in the document's frame.
    pub plane: ProfilePlane,
    /// The outline's loops, in the plane's coordinates.
    pub outline: Vec<ProfileWire>,
    /// The part's thickness along the normal from the plane, mm.
    pub depth: f64,
    /// The shadow's area, mm².
    pub area: f64,
    /// The least and largest clearance of the outline from the shadow,
    /// sampled along it, mm.
    pub band: [f64; 2],
    #[serde(default)]
    pub error: Option<String>,
}

impl WorkbenchFeature for Shadow {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// The plane at the foot of `mesh` seen along `normal`, in its frame.
pub fn foot(mesh: &TriMesh, normal: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
    let n = unit(normal.into())?;
    let (mut lo, mut lo_at) = (f64::MAX, DVec3::ZERO);
    let (mut sum, mut count) = (DVec3::ZERO, 0.0);
    for p in &mesh.positions {
        let p = DVec3::from(p.map(f64::from));
        let along = p.dot(n);
        if along < lo {
            (lo, lo_at) = (along, p);
        }
        sum += p;
        count += 1.0;
    }
    if count == 0.0 {
        return None;
    }
    let centre = sum * (1.0 / count);
    // The centre moved down onto the foot.
    let origin = centre - n * (centre - lo_at).dot(n);
    Some((origin.to_array(), basis(n)?.0.to_array()))
}

/// What the outline is worked out from: the scan's geometry, where it
/// stands, and the shadow's settings.
pub fn key(document: &Document, s: &Shadow) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    document
        .imported_geometry(s.scan)
        .map(|g| g.revision)
        .hash(&mut h);
    format!("{:?}", document.body_placement(s.scan).rows()).hash(&mut h);
    let settings = Shadow {
        made: None,
        ..s.clone()
    };
    serde_json::to_string(&settings)
        .unwrap_or_default()
        .hash(&mut h);
    h.finish()
}

/// The outline of `s` as the scan stands now.
pub fn make(document: &Document, kernel: &dyn KernelQueries, s: &Shadow) -> Made {
    let key = key(document, s);
    let failed = |error: String| Made {
        key,
        error: Some(error),
        ..Made::default()
    };
    let Some((mesh, _)) = document.local_geometry(s.scan) else {
        return failed("the scan has no mesh".into());
    };
    let (Some(n), Some(x)) = (unit(s.normal.into()), unit(s.x_axis.into())) else {
        return failed("the plane has no direction".into());
    };
    let local = ProfilePlane {
        origin: s.origin,
        x_axis: x.to_array(),
        y_axis: n.cross(x).to_array(),
        normal: n.to_array(),
    };
    let answer = match kernel.mesh_shadow(
        &mesh,
        &ShadowRequest {
            plane: local,
            play: s.play,
            tolerance: s.tolerance,
            round_below_deg: s.round_below_deg,
            fill_below: s.fill_below,
        },
    ) {
        Ok(answer) => answer,
        Err(e) => return failed(e.to_string()),
    };
    let depth = mesh
        .positions
        .iter()
        .map(|p| (DVec3::from(p.map(f64::from)) - DVec3::from(s.origin)).dot(n))
        .fold(0.0, f64::max);
    let area = answer
        .silhouette
        .iter()
        .map(|l| kernel_api::polygon::signed_area(l))
        .sum::<f64>();
    let mut band = [f64::MAX, f64::MIN];
    for p in answer.outline.iter().flat_map(samples) {
        let d = answer
            .silhouette
            .iter()
            .flat_map(|l| (0..l.len()).map(move |i| (l[i], l[(i + 1) % l.len()])))
            .map(|(a, b)| to_segment(p, a, b))
            .fold(f64::MAX, f64::min);
        band = [band[0].min(d), band[1].max(d)];
    }
    let rows = document.body_placement(s.scan).rows();
    let point = |p: [f64; 3]| {
        std::array::from_fn(|r| {
            rows[r][0] * p[0] + rows[r][1] * p[1] + rows[r][2] * p[2] + rows[r][3]
        })
    };
    let turn = |v: [f64; 3]| {
        std::array::from_fn(|r| rows[r][0] * v[0] + rows[r][1] * v[1] + rows[r][2] * v[2])
    };
    Made {
        key,
        plane: ProfilePlane {
            origin: point(local.origin),
            x_axis: turn(local.x_axis),
            y_axis: turn(local.y_axis),
            normal: turn(local.normal),
        },
        outline: answer.outline,
        depth,
        area,
        band: if band[0] <= band[1] { band } else { [0.0; 2] },
        error: None,
    }
}

/// The block the outline stands up as, as one op; none when the shadow
/// is not a block or has no outline yet.
pub fn ops(s: &Shadow) -> Vec<SolidOp> {
    let Some(made) = s.made.as_ref().filter(|m| s.block && !m.outline.is_empty()) else {
        return Vec::new();
    };
    let distance = s.height.unwrap_or(made.depth);
    if distance <= 0.0 {
        return Vec::new();
    }
    vec![SolidOp::Sweep {
        profile: Profile {
            plane: made.plane,
            wires: made.outline.clone(),
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        },
        op: BooleanOp::NewSolid,
    }]
}

/// The point at `u`, `v` along a plane's axes, in the plane's frame.
pub(crate) fn on_plane(p: &ProfilePlane, [u, v]: [f64; 2]) -> [f64; 3] {
    let at = DVec3::from(p.origin) + (DVec3::from(p.x_axis) * u + DVec3::from(p.y_axis) * v);
    at.to_array()
}

/// The outline drawn as lines in the document's frame.
pub fn lines(made: &Made) -> TriMesh {
    let mut mesh = TriMesh::default();
    let at = |q: [f64; 2]| on_plane(&made.plane, q);
    for wire in &made.outline {
        let points: Vec<[f64; 2]> = samples(wire).collect();
        let base = mesh.positions.len() as u32;
        for q in &points {
            mesh.positions.push(at(*q).map(|c| c as f32));
            mesh.normals.push([0.0, 0.0, 1.0]);
        }
        let n = points.len() as u32;
        for i in 0..n {
            mesh.edges.extend([base + i, base + (i + 1) % n]);
        }
    }
    mesh
}

/// Points along a wire, in order, closing back to the first: lines by
/// their ends, arcs and circles every few degrees.
pub(crate) fn samples(wire: &ProfileWire) -> impl Iterator<Item = [f64; 2]> + '_ {
    wire.segments.iter().flat_map(|s| match *s {
        ProfileSegment::Line { start, end } => vec![start, lerp(start, end, 0.5)],
        ProfileSegment::Arc { start, mid, end } => arc_points(start, mid, end),
        ProfileSegment::Circle { center, radius } => (0..72)
            .map(|k| {
                let t = f64::from(k) * std::f64::consts::TAU / 72.0;
                [center[0] + radius * t.cos(), center[1] + radius * t.sin()]
            })
            .collect(),
        _ => Vec::new(),
    })
}

/// An arc through three points, every 5° from `a` to just short of `c`.
fn arc_points(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Vec<[f64; 2]> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    if d.abs() < 1e-12 {
        return vec![a, b];
    }
    let sq = |p: [f64; 2]| p[0] * p[0] + p[1] * p[1];
    let ux = (sq(a) * (b[1] - c[1]) + sq(b) * (c[1] - a[1]) + sq(c) * (a[1] - b[1])) / d;
    let uy = (sq(a) * (c[0] - b[0]) + sq(b) * (a[0] - c[0]) + sq(c) * (b[0] - a[0])) / d;
    let r = (a[0] - ux).hypot(a[1] - uy);
    let angle = |p: [f64; 2]| (p[1] - uy).atan2(p[0] - ux);
    let tau = std::f64::consts::TAU;
    let (t0, tb, tc) = (angle(a), angle(b), angle(c));
    let ccw = |t: f64| (t - t0).rem_euclid(tau);
    // Counter-clockwise when the middle comes before the end that way.
    let sweep = if ccw(tb) < ccw(tc) {
        ccw(tc)
    } else {
        ccw(tc) - tau
    };
    let steps = ((sweep.abs() / 5f64.to_radians()).ceil() as usize).max(2);
    (0..steps)
        .map(|k| {
            let t = t0 + sweep * k as f64 / steps as f64;
            [ux + r * t.cos(), uy + r * t.sin()]
        })
        .collect()
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let [x, y] = kernel_api::nearest::nearest_on_segment(p, a, b);
    (p[0] - x).hypot(p[1] - y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An arc through three points runs through its middle, either way
    /// round, and a ring's samples lie on it.
    #[test]
    fn arcs_are_sampled_along_their_circle() {
        for (mid, ends_up) in [([0.0, 1.0], true), ([0.0, -1.0], false)] {
            let pts = arc_points([1.0, 0.0], mid, [-1.0, 0.0]);
            assert!(pts.iter().all(|p| (p[0].hypot(p[1]) - 1.0).abs() < 1e-12));
            assert!(
                pts.iter().skip(1).all(|p| (p[1] > 0.0) == ends_up),
                "{pts:?}"
            );
        }
    }
}
