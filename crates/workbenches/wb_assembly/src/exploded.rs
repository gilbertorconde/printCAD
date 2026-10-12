//! Exploded views kept in the document: ordered steps, each moving some
//! bodies (turned about an axis, pushed out from a centre, shifted),
//! played from the assembly as it stands. Showing one moves nothing for
//! good: the bodies go back when the view closes.

use core_document::{
    BodyId, BodyPlacement, Document, DocumentResult, FeatureError, FeatureId, WorkbenchFeature,
    WorkbenchId,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// The feature kind exploded views are stored as.
pub const EXPLODED_KIND: &str = "wb.assembly.exploded";

/// One step: these bodies turned by `turn`, pushed out by `radial`, then
/// moved by `shift` (world, mm), each by the share of the step played.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExplodeStep {
    pub bodies: Vec<BodyId>,
    pub shift: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<StepTurn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radial: Option<StepRadial>,
}

/// A step's turn: `degrees` about the line through `point` along `axis`
/// (world).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StepTurn {
    pub point: [f32; 3],
    pub axis: [f32; 3],
    pub degrees: f32,
}

/// A step's spread: each body pushed `distance` mm straight away from
/// `centre`, or with `axis`, square away from the line through `centre`
/// along it; measured from the middle of the body's box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StepRadial {
    pub centre: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<[f32; 3]>,
    pub distance: f32,
}

impl StepRadial {
    /// The way a body whose box middle is at `middle` is pushed: none for
    /// one on the centre or the axis.
    fn away(&self, middle: Vec3) -> Vec3 {
        let mut out = middle - Vec3::from_array(self.centre);
        if let Some(axis) = self.axis.map(Vec3::from_array).map(Vec3::normalize_or_zero) {
            out -= axis * out.dot(axis);
        }
        out.normalize_or_zero()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExplodedView {
    pub steps: Vec<ExplodeStep>,
}

impl WorkbenchFeature for ExplodedView {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(EXPLODED_KIND)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            core_document::DocumentError::Feature(FeatureError::Deserialization(e.to_string()))
        })
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        "Exploded view"
    }
}

impl ExplodedView {
    /// Every body `start` names, where the view has it at `at` steps in:
    /// each step whole before it, the one it is in by the share it has got
    /// through. `document` gives the bodies' boxes a radial step pushes
    /// from.
    pub fn placed_at(
        &self,
        document: &Document,
        start: &[(BodyId, BodyPlacement)],
        at: f32,
    ) -> Vec<(BodyId, BodyPlacement)> {
        start
            .iter()
            .map(|(body, placement)| {
                let mut placed = *placement;
                for (i, step) in self.steps.iter().enumerate() {
                    let share = (at - i as f32).clamp(0.0, 1.0);
                    if share > 0.0 && step.bodies.contains(body) {
                        placed = step.applied(document, *body, placed, share);
                    }
                }
                (*body, placed)
            })
            .collect()
    }
}

impl ExplodeStep {
    /// `placement` of `body` moved by `share` of this step.
    fn applied(
        &self,
        document: &Document,
        body: BodyId,
        placement: BodyPlacement,
        share: f32,
    ) -> BodyPlacement {
        let mut quat = placement.quat();
        let mut offset = placement.offset();
        if let Some(turn) = self.turn
            && let Some(axis) = Vec3::from_array(turn.axis).try_normalize()
        {
            let spin = glam::Quat::from_axis_angle(axis, (turn.degrees * share).to_radians());
            let point = Vec3::from_array(turn.point);
            quat = (spin * quat).normalize();
            offset = spin * (offset - point) + point;
        }
        if let Some(radial) = self.radial {
            let now = BodyPlacement::new(quat, offset);
            let middle = centre(document, body, &now)
                .map(Vec3::from_array)
                .unwrap_or(offset);
            offset += radial.away(middle) * radial.distance * share;
        }
        BodyPlacement::new(quat, offset + Vec3::from_array(self.shift) * share)
    }

    /// What the step does, in a few words: its turn, spread and shift,
    /// each left out when it does nothing ("turn 90° about Z, spread
    /// 5 mm, shift (0, 0, 30) mm").
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(turn) = self.turn.filter(|t| t.degrees != 0.0) {
            parts.push(format!(
                "turn {}° about {}",
                short(turn.degrees),
                axis_name(turn.axis)
            ));
        }
        if let Some(radial) = self.radial.filter(|r| r.distance != 0.0) {
            let mut said = format!("spread {} mm", short(radial.distance));
            if let Some(axis) = radial.axis {
                said += &format!(" from {}", axis_name(axis));
            }
            parts.push(said);
        }
        if self.shift != [0.0; 3] {
            let [x, y, z] = self.shift.map(short);
            parts.push(format!("shift ({x}, {y}, {z}) mm"));
        }
        if parts.is_empty() {
            "stays".into()
        } else {
            parts.join(", ")
        }
    }
}

/// `v` to two decimals at most, trailing zeros dropped.
fn short(v: f32) -> String {
    let text = format!("{v:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

/// X, Y or Z for an axis along one (either way), else its direction.
fn axis_name(axis: [f32; 3]) -> String {
    let Some(dir) = Vec3::from_array(axis).try_normalize() else {
        return "no axis".into();
    };
    for (k, name) in ["X", "Y", "Z"].into_iter().enumerate() {
        if dir[k].abs() > 1.0 - 1e-5 {
            return if dir[k] < 0.0 {
                format!("-{name}")
            } else {
                name.into()
            };
        }
    }
    let [x, y, z] = dir.to_array().map(short);
    format!("({x}, {y}, {z})")
}

/// The view `id` as stored.
pub fn view_of(document: &Document, id: FeatureId) -> Option<ExplodedView> {
    ExplodedView::from_json(document.get_feature_data(id)?).ok()
}

/// The middle of `body`'s box where `placement` puts it.
pub fn centre(document: &Document, body: BodyId, placement: &BodyPlacement) -> Option<[f32; 3]> {
    let (mesh, bounds) = document.local_geometry(body)?;
    let (lo, hi) = bounds.or_else(|| mesh.bounds())?;
    let middle = (Vec3::from_array(lo) + Vec3::from_array(hi)) * 0.5;
    Some(placement.point(middle.to_array()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_play_in_order_each_by_its_share() {
        let (a, b) = (BodyId::new(), BodyId::new());
        let view = ExplodedView {
            steps: vec![
                ExplodeStep {
                    bodies: vec![a],
                    shift: [0.0, 0.0, 10.0],
                    turn: None,
                    radial: None,
                },
                ExplodeStep {
                    bodies: vec![a, b],
                    shift: [5.0, 0.0, 0.0],
                    turn: None,
                    radial: None,
                },
            ],
        };
        let start = [(a, BodyPlacement::default()), (b, BodyPlacement::default())];
        let document = Document::new("t");
        let at = |t: f32| view.placed_at(&document, &start, t);
        assert_eq!(at(0.0)[0].1.offset(), Vec3::ZERO);
        assert_eq!(at(0.5)[0].1.offset(), Vec3::new(0.0, 0.0, 5.0));
        assert_eq!(at(1.5)[0].1.offset(), Vec3::new(2.5, 0.0, 10.0));
        assert_eq!(at(1.5)[1].1.offset(), Vec3::new(2.5, 0.0, 0.0));
        assert_eq!(at(2.0)[1].1.offset(), Vec3::new(5.0, 0.0, 0.0));
    }

    #[test]
    fn a_step_s_summary_names_its_turn_spread_and_shift() {
        let mut step = ExplodeStep {
            bodies: vec![BodyId::new()],
            shift: [0.0, 0.0, 30.0],
            turn: None,
            radial: None,
        };
        assert_eq!(step.summary(), "shift (0, 0, 30) mm");
        step.turn = Some(StepTurn {
            point: [1.0, 2.0, 3.0],
            axis: [0.0, 0.0, 1.0],
            degrees: 90.0,
        });
        step.radial = Some(StepRadial {
            centre: [0.0; 3],
            axis: None,
            distance: 12.5,
        });
        assert_eq!(
            step.summary(),
            "turn 90° about Z, spread 12.5 mm, shift (0, 0, 30) mm"
        );
        step.shift = [0.0; 3];
        step.turn = Some(StepTurn {
            point: [0.0; 3],
            axis: [0.0, -2.0, 0.0],
            degrees: -45.0,
        });
        step.radial = Some(StepRadial {
            centre: [0.0; 3],
            axis: Some([3.0, 4.0, 0.0]),
            distance: 5.0,
        });
        assert_eq!(
            step.summary(),
            "turn -45° about -Y, spread 5 mm from (0.6, 0.8, 0)"
        );
        step.turn = None;
        step.radial = None;
        assert_eq!(step.summary(), "stays");
    }
}

#[cfg(test)]
mod turned_and_radial {
    use super::*;

    fn close(a: Vec3, b: [f32; 3]) -> bool {
        (a - Vec3::from_array(b)).length() < 1e-4
    }

    /// Half a step of a quarter turn about Z through (10, 0, 0) turns the
    /// body an eighth round that point; the whole step a quarter.
    #[test]
    fn a_turning_step_turns_its_bodies_about_its_axis() {
        let a = BodyId::new();
        let view = ExplodedView {
            steps: vec![ExplodeStep {
                bodies: vec![a],
                shift: [0.0; 3],
                turn: Some(StepTurn {
                    point: [10.0, 0.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                    degrees: 90.0,
                }),
                radial: None,
            }],
        };
        let document = Document::new("t");
        let start = [(a, BodyPlacement::default())];
        let whole = view.placed_at(&document, &start, 1.0)[0].1;
        assert!(
            close(whole.offset(), [10.0, -10.0, 0.0]),
            "{:?}",
            whole.offset()
        );
        assert!(close(whole.quat() * Vec3::X, [0.0, 1.0, 0.0]));
        let half = view.placed_at(&document, &start, 0.5)[0].1;
        let r = std::f32::consts::FRAC_1_SQRT_2 * 10.0;
        assert!(
            close(half.offset(), [10.0 - r, -r, 0.0]),
            "{:?}",
            half.offset()
        );
    }

    /// A radial step pushes each body straight out from the centre, or
    /// square out from the axis, the distance asked.
    #[test]
    fn a_radial_step_pushes_bodies_away_from_its_centre() {
        let (a, b) = (BodyId::new(), BodyId::new());
        let at =
            |x: f32, y: f32, z: f32| BodyPlacement::new(glam::Quat::IDENTITY, Vec3::new(x, y, z));
        let start = [(a, at(10.0, 0.0, 5.0)), (b, at(0.0, -4.0, 0.0))];
        let step = |axis| ExplodedView {
            steps: vec![ExplodeStep {
                bodies: vec![a, b],
                shift: [0.0; 3],
                turn: None,
                radial: Some(StepRadial {
                    centre: [0.0; 3],
                    axis,
                    distance: 6.0,
                }),
            }],
        };
        let document = Document::new("t");
        let about_z = step(Some([0.0, 0.0, 1.0])).placed_at(&document, &start, 1.0);
        assert!(
            close(about_z[0].1.offset(), [16.0, 0.0, 5.0]),
            "{:?}",
            about_z[0].1.offset()
        );
        assert!(close(about_z[1].1.offset(), [0.0, -10.0, 0.0]));
        let from_point = step(None).placed_at(&document, &start, 1.0);
        let out = Vec3::new(10.0, 0.0, 5.0).normalize() * 6.0;
        assert!(close(
            from_point[0].1.offset(),
            (Vec3::new(10.0, 0.0, 5.0) + out).to_array()
        ));
    }

    /// Views kept before turns and spreads existed read as they were.
    #[test]
    fn a_kept_view_without_turns_reads_as_before() {
        let step: ExplodeStep =
            serde_json::from_value(serde_json::json!({"bodies": [], "shift": [1.0, 2.0, 3.0]}))
                .unwrap();
        assert_eq!((step.turn, step.radial), (None, None));
    }
}
