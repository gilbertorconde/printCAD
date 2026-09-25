//! The wall thickness check: how thin the sketch's closed profile gets,
//! for a part that has to print.
//!
//! The kernel reads each region of the profile into its medial axis, the
//! centres of the largest discs that fit inside, with each disc's radius
//! (the clearance) along it. Twice the clearance is the wall there. The
//! axis draws over the sketch, in the warning colour where the wall is
//! under the minimum, and the thinnest place is marked with its width.

use core_document::{
    FeatureId, ScreenSpaceLabel, ScreenSpaceMark, ScreenSpaceOverlay, SketchPalette,
    WorkbenchRuntimeContext,
};
use kernel_api::{MedialPath, MedialRegion, Profile, ProfilePlane, ProfileWire};
use serde_json::{Value, json};

use crate::overlay::SketchProjector;
use crate::profile;
use crate::sketch::Vec2D;

/// The minimum wall a printer is taken to manage when nothing else is
/// said: two lines of a 0.4 mm nozzle.
pub const DEFAULT_MINIMUM_MM: f32 = 0.8;

/// What the medial axis is held to, in millimetres.
const AXIS_TOLERANCE_MM: f64 = 1e-3;

/// A check made on one sketch, kept for drawing until the sketch changes.
#[derive(Debug, Clone)]
pub(crate) struct WallCheck {
    pub sketch: FeatureId,
    /// The profile it was made on: a sketch whose profile differs has
    /// changed since.
    pub wires: Vec<ProfileWire>,
    pub regions: Vec<MedialRegion>,
    /// The wall under which a place counts as thin, in millimetres.
    pub minimum: f64,
    /// The document's edit count when the profile was last compared.
    pub seen_seq: u64,
}

impl WallCheck {
    /// The thinnest wall of all the regions and where it is.
    pub fn thinnest(&self) -> Option<(f64, [f64; 2])> {
        self.regions
            .iter()
            .filter_map(|r| r.narrowest)
            .min_by(|a, b| a.clearance.total_cmp(&b.clearance))
            .map(|n| (2.0 * n.clearance, n.at))
    }

    /// One line for the log and the status bar.
    pub fn summary(&self) -> String {
        match self.thinnest() {
            Some((wall, at)) => {
                let verdict = if wall < self.minimum {
                    format!("under the {} minimum", mm(self.minimum))
                } else {
                    format!("at least the {} minimum", mm(self.minimum))
                };
                format!(
                    "Thinnest wall {} at ({:.2}, {:.2}), {verdict}",
                    mm(wall),
                    at[0],
                    at[1]
                )
            }
            None => "The profile has no wall to measure".to_string(),
        }
    }

    /// The command's answer.
    pub fn report(&self) -> Value {
        let place = |at: [f64; 2]| json!({"x": at[0], "y": at[1]});
        let regions: Vec<Value> = self
            .regions
            .iter()
            .map(|r| match r.narrowest {
                Some(n) => json!({"thinnest": 2.0 * n.clearance, "where": place(n.at)}),
                None => json!({"thinnest": Value::Null, "where": Value::Null}),
            })
            .collect();
        let (thinnest, at) = match self.thinnest() {
            Some((wall, at)) => (json!(wall), place(at)),
            None => (Value::Null, Value::Null),
        };
        json!({
            "thinnest": thinnest,
            "where": at,
            "minimum": self.minimum,
            "thin": self.thinnest().is_some_and(|(wall, _)| wall < self.minimum),
            "regions": regions,
        })
    }
}

fn mm(value: f64) -> String {
    format!("{value:.2} mm")
}

/// Check `sketch`'s closed profile against a `minimum` wall.
pub(crate) fn check(
    ctx: &WorkbenchRuntimeContext,
    sketch: FeatureId,
    minimum: f64,
) -> Result<WallCheck, String> {
    let feature = crate::stored_sketch(ctx.document, sketch).ok_or("that is not a sketch")?;
    let wires = profile::extract_wires(&feature.sketch).map_err(|e| e.to_string())?;
    let kernel = ctx.kernel.ok_or("no kernel to measure with")?;
    // The sketch's own coordinates are the plane's.
    let profile = Profile {
        plane: ProfilePlane {
            origin: [0.0; 3],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        },
        wires: wires.clone(),
    };
    let regions = kernel
        .medial_axis(&profile, AXIS_TOLERANCE_MM)
        .map_err(|e| e.to_string())?;
    Ok(WallCheck {
        sketch,
        wires,
        regions,
        minimum,
        seen_seq: ctx.document.mutation_seq(),
    })
}

/// The clearance each point of a path stands for. Towards an end on the
/// boundary the clearance falls to a corner without the region getting
/// thinner, so that run takes the clearance where it starts to fall.
fn effective_clearance(path: &MedialPath) -> Vec<f64> {
    let mut out = path.clearance.clone();
    let n = out.len();
    if path.boundary_ends[0] {
        hold_rise(&mut out, &(0..n).collect::<Vec<_>>());
    }
    if path.boundary_ends[1] {
        hold_rise(&mut out, &(0..n).rev().collect::<Vec<_>>());
    }
    out
}

/// Along `order`, from its first index while the clearance keeps rising,
/// every value takes the one the rise ends at.
fn hold_rise(clearance: &mut [f64], order: &[usize]) {
    let mut top = 0;
    while top + 1 < order.len() && clearance[order[top + 1]] >= clearance[order[top]] {
        top += 1;
    }
    let Some(&peak_at) = order.get(top) else {
        return;
    };
    let peak = clearance[peak_at];
    for &i in &order[..top] {
        clearance[i] = peak;
    }
}

/// The axis, each stretch in the colour of the wall it stands for.
pub(crate) fn overlays(
    check: &WallCheck,
    proj: &SketchProjector,
    pal: &SketchPalette,
) -> Vec<ScreenSpaceOverlay> {
    let mut out = Vec::new();
    for path in check.regions.iter().flat_map(|r| &r.paths) {
        let clearance = effective_clearance(path);
        let px: Vec<Option<[f32; 2]>> = path
            .points
            .iter()
            .map(|p| proj.to_px(Vec2D::new(p[0] as f32, p[1] as f32)))
            .collect();
        for i in 1..px.len() {
            let (Some(a), Some(b)) = (px[i - 1], px[i]) else {
                continue;
            };
            let wall = 2.0 * clearance[i - 1].min(clearance[i]);
            let color = if wall < check.minimum {
                pal.wall_thin
            } else {
                pal.wall
            };
            out.push(ScreenSpaceOverlay::new(a, b, color, 2.0));
        }
    }
    out
}

/// A dot on each region's narrowest place.
pub(crate) fn marks(
    check: &WallCheck,
    proj: &SketchProjector,
    pal: &SketchPalette,
) -> Vec<ScreenSpaceMark> {
    check
        .regions
        .iter()
        .filter_map(|r| r.narrowest)
        .filter_map(|n| {
            let color = if 2.0 * n.clearance < check.minimum {
                pal.wall_thin
            } else {
                pal.wall
            };
            let pos = proj.to_px(Vec2D::new(n.at[0] as f32, n.at[1] as f32))?;
            Some(ScreenSpaceMark::dot(pos, 4.0, color))
        })
        .collect()
}

/// The thinnest wall's width beside its dot.
pub(crate) fn labels(
    check: &WallCheck,
    proj: &SketchProjector,
    pal: &SketchPalette,
) -> Vec<ScreenSpaceLabel> {
    let Some((wall, at)) = check.thinnest() else {
        return Vec::new();
    };
    let Some([x, y]) = proj.to_px(Vec2D::new(at[0] as f32, at[1] as f32)) else {
        return Vec::new();
    };
    let color = if wall < check.minimum {
        pal.wall_thin
    } else {
        pal.wall
    };
    vec![
        ScreenSpaceLabel::new([x, y - 16.0], mm(wall), color, 12.0)
            .pill()
            .mono(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(clearance: &[f64], boundary_ends: [bool; 2]) -> MedialPath {
        MedialPath {
            points: vec![[0.0, 0.0]; clearance.len()],
            clearance: clearance.to_vec(),
            boundary_ends,
        }
    }

    #[test]
    fn a_branch_into_a_corner_stands_for_the_wall_it_leaves() {
        let got = effective_clearance(&path(&[0.0, 0.3, 0.6, 1.0], [true, false]));
        assert_eq!(got, vec![1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn a_neck_between_branch_points_stays_thin() {
        let got = effective_clearance(&path(&[1.0, 0.4, 1.0], [false, false]));
        assert_eq!(got, vec![1.0, 0.4, 1.0]);
    }

    #[test]
    fn a_corner_run_stops_where_the_clearance_turns() {
        let got = effective_clearance(&path(&[0.0, 0.5, 0.8, 0.6, 0.9], [true, false]));
        assert_eq!(got, vec![0.8, 0.8, 0.8, 0.6, 0.9]);
    }
}
