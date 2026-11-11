//! A coupling: two joints' motions tied together, as gears, a belt, a rack
//! and pinion or a screw tie them. Each end is a hinge or a slider; the
//! driven one's travel follows the driving one's by a ratio.
//!
//! A hinge's angle wraps round at a half turn, but a gear train remembers
//! every turn: after a whole turn of a driver at 1:2 the driven gear is half
//! a turn on, not back where it started. So the coupling counts its
//! driver's whole turns (`turns`), and a solve that carries the driver past
//! the wrap moves the count with it.

use std::collections::HashMap;

use core_document::{BodyId, Document, FeatureId, WorkbenchFeature, WorkbenchId};
use serde::{Deserialize, Serialize};

use crate::joint::{JointFeature, JointKind, Rigid};
use crate::solve::Joint;

/// The feature kind couplings are stored as.
pub const COUPLING_KIND: &str = "wb.assembly.coupling";

/// What ties the two motions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gearing {
    /// Two hinges turning opposite ways, `ratio` turns of the driven one to
    /// each of the driver's: meshed gears.
    Gears,
    /// Two hinges turning the same way: pulleys on a belt, sprockets on a
    /// chain.
    Belt,
    /// A hinge and a slider: the slider moves the pinion's pitch circle
    /// (`ratio`, its radius in mm) for each radian the hinge turns.
    RackAndPinion,
    /// A hinge and a slider: the slider moves `ratio` mm, the lead, for each
    /// whole turn of the hinge.
    Screw,
}

impl Gearing {
    pub const ALL: [Gearing; 4] = [
        Gearing::Gears,
        Gearing::Belt,
        Gearing::RackAndPinion,
        Gearing::Screw,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Gearing::Gears => "Gears",
            Gearing::Belt => "Belt",
            Gearing::RackAndPinion => "Rack and pinion",
            Gearing::Screw => "Screw",
        }
    }

    /// Its name in commands.
    pub fn word(self) -> &'static str {
        match self {
            Gearing::Gears => "gears",
            Gearing::Belt => "belt",
            Gearing::RackAndPinion => "rack",
            Gearing::Screw => "screw",
        }
    }

    pub fn of_word(word: &str) -> Option<Gearing> {
        Self::ALL.into_iter().find(|g| g.word() == word)
    }

    /// What its ratio is called, and whether it is a length.
    pub fn ratio_label(self) -> (&'static str, bool) {
        match self {
            Gearing::Gears | Gearing::Belt => ("Ratio", false),
            Gearing::RackAndPinion => ("Pitch radius", true),
            Gearing::Screw => ("Lead", true),
        }
    }

    /// The ratio a new coupling of this kind starts with.
    pub fn default_ratio(self) -> f32 {
        match self {
            Gearing::Gears | Gearing::Belt => 1.0,
            Gearing::RackAndPinion => 10.0,
            Gearing::Screw => 2.0,
        }
    }

    /// Whether it ties a turn to a turn (`true`) or a turn to a slide.
    fn turns_to_turns(self) -> bool {
        matches!(self, Gearing::Gears | Gearing::Belt)
    }

    /// Whether it can tie these two joints, driver first.
    pub fn fits(self, driver: &JointKind, driven: &JointKind) -> bool {
        let hinge = |k: &JointKind| matches!(k, JointKind::Hinge { .. });
        let slider = |k: &JointKind| matches!(k, JointKind::Slider { .. });
        if self.turns_to_turns() {
            hinge(driver) && hinge(driven)
        } else {
            (hinge(driver) && slider(driven)) || (slider(driver) && hinge(driven))
        }
    }

    /// The kind that suits two joints first: gears for two hinges, a rack
    /// and pinion for a hinge and a slider.
    pub fn suiting(driver: &JointKind, driven: &JointKind) -> Option<Gearing> {
        Self::ALL.into_iter().find(|g| g.fits(driver, driven))
    }

    pub fn summary(self) -> &'static str {
        match self {
            Gearing::Gears => "The driven hinge turns the other way, by the ratio",
            Gearing::Belt => "The driven hinge turns the same way, by the ratio",
            Gearing::RackAndPinion => {
                "The slider moves the pinion's pitch circle as the hinge turns"
            }
            Gearing::Screw => "The slider moves the lead for each whole turn of the hinge",
        }
    }
}

/// A coupling, stored as a feature of the body its driven joint moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coupling {
    pub gearing: Gearing,
    /// The joint whose motion leads.
    pub driver: FeatureId,
    /// The joint whose motion follows.
    pub driven: FeatureId,
    /// Turns per turn, or the pitch radius or the lead in mm, as `gearing`
    /// reads it.
    pub ratio: f32,
    /// The driven motion goes the other way.
    #[serde(default)]
    pub reverse: bool,
    /// Where the two motions stood when the coupling was made: the driver's
    /// counting its whole turns, in degrees or mm.
    pub driver_at: f64,
    pub driven_at: f64,
    /// The driver's whole turns since, when it is a hinge.
    #[serde(default)]
    pub turns: i64,
}

impl WorkbenchFeature for Coupling {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(COUPLING_KIND)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        vec![self.driver, self.driven]
    }

    fn name(&self) -> &str {
        self.gearing.label()
    }
}

/// A joint's motion from two bodies' placements: a hinge's angle, a
/// slider's position.
fn travel(joint: &Joint, placements: &HashMap<BodyId, Rigid>) -> Option<f64> {
    joint.feature.travel(
        placements.get(&joint.body)?,
        placements.get(&joint.feature.other_body)?,
    )
}

/// The coupling that drives `joint`, if one does.
pub fn driving(document: &Document, joint: FeatureId) -> Option<Coupling> {
    document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == COUPLING_KIND && !node.suppressed)
        .filter_map(|(id, _)| {
            serde_json::from_value::<Coupling>(document.feature_values(*id)?.clone()).ok()
        })
        .find(|c| c.driven == joint)
}

/// Every coupling's count of its driver's whole turns, as stored.
pub(crate) fn turn_counts(document: &Document) -> Vec<(FeatureId, i64)> {
    document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == COUPLING_KIND)
        .filter_map(|(id, node)| {
            let coupling = serde_json::from_value::<Coupling>(node.data.clone()).ok()?;
            Some((*id, coupling.turns))
        })
        .collect()
}

/// Store `counts` as the couplings' whole turns; a coupling gone or
/// already there is left alone.
pub(crate) fn set_turn_counts(document: &mut Document, counts: &[(FeatureId, i64)]) {
    for (id, turns) in counts {
        let Some(mut coupling) = document
            .get_feature_data(*id)
            .and_then(|d| serde_json::from_value::<Coupling>(d.clone()).ok())
        else {
            continue;
        };
        if coupling.turns == *turns {
            continue;
        }
        coupling.turns = *turns;
        if document
            .update_feature_data(*id, coupling.to_json())
            .is_ok()
        {
            document.clear_feature_dirty(*id);
        }
    }
}

pub(crate) fn angular(feature: &JointFeature) -> bool {
    matches!(feature.kind, JointKind::Hinge { .. })
}

/// `degrees` wrapped into a half turn either way.
fn wrap(degrees: f64) -> f64 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

impl Coupling {
    /// A coupling of `driver` and `driven` where they stand now, so making
    /// it moves nothing; `None` when a joint is not there, or `gearing`
    /// cannot tie them.
    pub fn new(
        gearing: Gearing,
        driver: &Joint,
        driven: &Joint,
        ratio: f32,
        reverse: bool,
        placements: &HashMap<BodyId, Rigid>,
    ) -> Option<Coupling> {
        if driver.id == driven.id || !gearing.fits(&driver.feature.kind, &driven.feature.kind) {
            return None;
        }
        Some(Coupling {
            gearing,
            driver: driver.id,
            driven: driven.id,
            ratio,
            reverse,
            driver_at: travel(driver, placements)?,
            driven_at: travel(driven, placements)?,
            turns: 0,
        })
    }

    /// How far the driver goes (degrees or mm) to take the driven motion
    /// `step` along (its own degrees or mm).
    pub fn driver_step(&self, step: f64, driver_angular: bool) -> Option<f64> {
        let factor = self.factor(driver_angular);
        (factor.abs() > 1e-12).then(|| step / factor)
    }

    /// How far the driven motion goes, in its own units (degrees or mm),
    /// for each unit (degree or mm) of the driver's.
    fn factor(&self, driver_angular: bool) -> f64 {
        let ratio = f64::from(self.ratio);
        let sign = if self.reverse { -1.0 } else { 1.0 };
        sign * match self.gearing {
            Gearing::Gears => -ratio,
            Gearing::Belt => ratio,
            // mm per degree of the pinion, or degrees per mm of the rack.
            Gearing::RackAndPinion if driver_angular => ratio.to_radians(),
            Gearing::RackAndPinion => (1.0 / ratio.max(1e-9)).to_degrees(),
            Gearing::Screw if driver_angular => ratio / 360.0,
            Gearing::Screw => 360.0 / ratio.max(1e-9),
        }
    }
}

/// A coupling as the solver reads it: its two joints, and the driver's
/// motion where the solve started, so it is followed through the wrap.
#[derive(Debug, Clone)]
pub struct Link {
    pub id: FeatureId,
    pub name: String,
    pub coupling: Coupling,
    pub driver: Joint,
    pub driven: Joint,
    /// The driver's travel where the solve started, as its joint reads it.
    start: f64,
}

/// A driven motion's mismatch weighs like a joint's: a degree as much as
/// the arc it sweeps at arm's length, a millimetre as itself.
const ARM_MM: f64 = 50.0;

impl Link {
    /// The driver's travel counting whole turns, with the bodies at
    /// `placements`: continued from where the solve started, so a step
    /// across the half turn does not jump.
    fn driver_travel(&self, placements: &HashMap<BodyId, Rigid>) -> Option<f64> {
        let now = travel(&self.driver, placements)?;
        if !angular(&self.driver.feature) {
            return Some(now);
        }
        let counted = self.start + 360.0 * self.coupling.turns as f64;
        Some(counted + wrap(now - self.start))
    }

    /// How far the driven joint is from where the driver puts it: one
    /// residual, weighed like a joint's.
    pub fn residual(&self, placements: &HashMap<BodyId, Rigid>, out: &mut Vec<f64>) {
        let (Some(lead), Some(now)) = (
            self.driver_travel(placements),
            travel(&self.driven, placements),
        ) else {
            out.push(0.0);
            return;
        };
        let c = &self.coupling;
        let driver_angular = angular(&self.driver.feature);
        let wanted = c.driven_at + c.factor(driver_angular) * (lead - c.driver_at);
        if angular(&self.driven.feature) {
            out.push(wrap(now - wanted).to_radians() * ARM_MM);
        } else {
            out.push(now - wanted);
        }
    }

    /// The coupling with its turn count moved to where `placements` carry
    /// the driver; `None` when the count stands.
    pub fn counted(&self, placements: &HashMap<BodyId, Rigid>) -> Option<Coupling> {
        if !angular(&self.driver.feature) {
            return None;
        }
        let lead = self.driver_travel(placements)?;
        let now = travel(&self.driver, placements)?;
        let turns = ((lead - now) / 360.0).round() as i64;
        (turns != self.coupling.turns).then(|| Coupling {
            turns,
            ..self.coupling.clone()
        })
    }
}

/// Every coupling not suppressed whose two joints are among `joints`,
/// read against the bodies at `placements`.
pub fn links(
    document: &Document,
    joints: &[Joint],
    placements: &HashMap<BodyId, Rigid>,
) -> Vec<Link> {
    let mut found: Vec<(u64, Link)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, node)| node.workbench_id.as_str() == COUPLING_KIND && !node.suppressed)
        .filter_map(|(id, node)| {
            let coupling: Coupling =
                serde_json::from_value(document.feature_values(*id)?.clone()).ok()?;
            let joint = |id: FeatureId| joints.iter().find(|j| j.id == id).cloned();
            let (driver, driven) = (joint(coupling.driver)?, joint(coupling.driven)?);
            if !coupling
                .gearing
                .fits(&driver.feature.kind, &driven.feature.kind)
            {
                return None;
            }
            let start = travel(&driver, placements)?;
            Some((
                node.seq,
                Link {
                    id: *id,
                    name: node.name.clone(),
                    coupling,
                    driver,
                    driven,
                    start,
                },
            ))
        })
        .collect();
    found.sort_by_key(|(seq, link)| (*seq, link.id));
    found.into_iter().map(|(_, link)| link).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_ties_to_a_turn_and_a_slide_to_a_turn() {
        let hinge = JointKind::Hinge {
            offset: 0.0,
            zero: [0.0, 0.0, 0.0, 1.0],
            drive: Default::default(),
        };
        let slider = JointKind::Slider {
            turn: [0.0, 0.0, 0.0, 1.0],
            drive: Default::default(),
        };
        assert!(Gearing::Gears.fits(&hinge, &hinge));
        assert!(!Gearing::Gears.fits(&hinge, &slider));
        assert!(Gearing::Screw.fits(&hinge, &slider));
        assert!(Gearing::RackAndPinion.fits(&slider, &hinge));
        assert!(!Gearing::RackAndPinion.fits(&slider, &slider));
        assert_eq!(Gearing::suiting(&hinge, &hinge), Some(Gearing::Gears));
        assert_eq!(
            Gearing::suiting(&hinge, &slider),
            Some(Gearing::RackAndPinion)
        );
        assert_eq!(Gearing::suiting(&slider, &slider), None);
    }

    #[test]
    fn each_kind_moves_its_driven_joint_by_its_ratio() {
        let c = |gearing, ratio, reverse| Coupling {
            gearing,
            driver: FeatureId(uuid::Uuid::nil()),
            driven: FeatureId(uuid::Uuid::nil()),
            ratio,
            reverse,
            driver_at: 0.0,
            driven_at: 0.0,
            turns: 0,
        };
        assert_eq!(c(Gearing::Gears, 0.5, false).factor(true), -0.5);
        assert_eq!(c(Gearing::Belt, 2.0, false).factor(true), 2.0);
        assert_eq!(c(Gearing::Belt, 2.0, true).factor(true), -2.0);
        // A 10 mm pinion moves its rack 2π·10 mm a turn.
        let rack = c(Gearing::RackAndPinion, 10.0, false);
        assert!((rack.factor(true) * 360.0 - 20.0 * std::f64::consts::PI).abs() < 1e-9);
        assert!((rack.factor(false) * rack.factor(true) - 1.0).abs() < 1e-12);
        // A 2 mm lead: 2 mm a turn.
        assert!((c(Gearing::Screw, 2.0, false).factor(true) * 360.0 - 2.0).abs() < 1e-12);
    }
}
