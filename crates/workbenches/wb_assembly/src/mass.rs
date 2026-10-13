//! The assembly's mass, centre of mass and inertia: each visible solid
//! body measured by the kernel in its own frame, its centre placed and its
//! inertia turned where the body sits, and each weighed at its material's
//! density, or at one given density where it has no material.
//!
//! Bought parts weigh in: the parts list counts them as parts of the
//! assembly, and a screw or a bearing weighs and turns with the rest.
//! Each body says whether it is bought, so a figure for the printed parts
//! alone leaves them out.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use core_document::{BodyId, BodyPlacement, Document};
use kernel_api::KernelQueries;

/// One body as measured.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyMass {
    pub body: BodyId,
    pub volume_mm3: f64,
    /// World space.
    pub centre: [f64; 3],
    /// Its material's density, g/cm³, when it has one.
    pub density: Option<f64>,
    /// Its inertia at unit density about its centre, on the world's
    /// axes, when the kernel gave one.
    pub inertia: Option<kernel_api::Inertia>,
    /// The parts list counts it bought.
    pub bought: bool,
}

impl BodyMass {
    /// Its mass in grams, at `density` g/cm³ when it has no material.
    pub fn mass_g(&self, density: f64) -> f64 {
        self.volume_mm3 * self.grams_per_mm3(density)
    }

    fn grams_per_mm3(&self, density: f64) -> f64 {
        self.density.unwrap_or(density) / 1000.0
    }
}

/// What the measuring found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MassReport {
    pub bodies: Vec<BodyMass>,
    /// Visible bodies with no solid, or none that encloses a volume.
    pub skipped: usize,
    pub stopped: bool,
}

impl MassReport {
    pub fn volume_mm3(&self) -> f64 {
        self.bodies.iter().map(|b| b.volume_mm3).sum()
    }

    /// The mass in grams, bodies without a material at `density` g/cm³.
    pub fn mass_g(&self, density: f64) -> f64 {
        self.bodies.iter().map(|b| b.mass_g(density)).sum()
    }

    /// The centre of mass, bodies without a material at `density`, world
    /// space.
    pub fn centre(&self, density: f64) -> Option<[f64; 3]> {
        let total = self.mass_g(density);
        (total > 0.0).then(|| {
            let mut c = [0.0; 3];
            for b in &self.bodies {
                for (k, v) in c.iter_mut().enumerate() {
                    *v += b.centre[k] * b.mass_g(density) / total;
                }
            }
            c
        })
    }

    /// The inertia about the centre of mass, g·mm² on the world's axes,
    /// bodies without a material at `density`: each body's moved there by
    /// the parallel axis theorem. `None` when a body has no inertia or
    /// nothing weighs.
    pub fn inertia(&self, density: f64) -> Option<kernel_api::Inertia> {
        let shares = self
            .bodies
            .iter()
            .map(|b| {
                let per = b.grams_per_mm3(density);
                Some(core_document::inertia::Share {
                    mass: b.mass_g(density),
                    centre: b.centre,
                    about_centre: core_document::inertia::scaled(&b.inertia?.about_centre, per),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        core_document::inertia::combined(&shares).map(|(_, _, inertia)| inertia)
    }

    /// Without the bought parts' bodies.
    pub fn made_only(&self) -> MassReport {
        MassReport {
            bodies: self.bodies.iter().filter(|b| !b.bought).cloned().collect(),
            ..self.clone()
        }
    }
}

/// A body to measure: its snapshot, where it sits, its material's density
/// and whether it is bought.
type Solid = (BodyId, Arc<Vec<u8>>, BodyPlacement, Option<f64>, bool);

/// The bodies to measure, read from the document.
pub struct Weighing {
    solids: Vec<Solid>,
    skipped: usize,
}

/// The visible solid bodies, or only `among` when given; `bought` are the
/// bought parts' bodies.
pub fn plan(document: &Document, among: Option<&[BodyId]>, bought: &[BodyId]) -> Weighing {
    let mut solids = Vec::new();
    let mut skipped = 0;
    for body in document.bodies() {
        if among.is_some_and(|only| !only.contains(&body.id))
            || !document.imported_body_effective_visible(body.id)
        {
            continue;
        }
        match document.imported_brep_blob_arc(body.id) {
            Some(blob) => solids.push((
                body.id,
                blob,
                document.body_placement(body.id),
                body.material.as_ref().map(|m| f64::from(m.density)),
                bought.contains(&body.id),
            )),
            None => skipped += 1,
        }
    }
    Weighing { solids, skipped }
}

impl Weighing {
    pub fn len(&self) -> usize {
        self.solids.len()
    }

    /// Measure each body, counting them off in `done`, until `stop`.
    pub fn run(
        &self,
        kernel: &dyn KernelQueries,
        done: &AtomicUsize,
        stop: &AtomicBool,
    ) -> Result<MassReport, String> {
        let mut report = MassReport {
            skipped: self.skipped,
            ..MassReport::default()
        };
        for (body, blob, placement, density, bought) in &self.solids {
            if stop.load(Ordering::Relaxed) {
                report.stopped = true;
                break;
            }
            let measured = kernel.measure(blob).map_err(|e| e.to_string())?;
            done.fetch_add(1, Ordering::Relaxed);
            let Some(volume) = measured.volume_mm3.filter(|v| *v > 0.0) else {
                report.skipped += 1;
                continue;
            };
            let c = measured.centre_mm.map(|v| v as f32);
            report.bodies.push(BodyMass {
                body: *body,
                volume_mm3: volume,
                centre: placement.point(c).map(f64::from),
                density: *density,
                inertia: measured
                    .inertia
                    .map(|i| core_document::inertia::turned(&i, placement.quat().as_dquat())),
                bought: *bought,
            });
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_centre_weighs_each_body_by_its_volume() {
        let body = |volume, x| BodyMass {
            body: BodyId::new(),
            volume_mm3: volume,
            centre: [x, 0.0, 0.0],
            density: None,
            inertia: None,
            bought: false,
        };
        let report = MassReport {
            bodies: vec![body(1000.0, 0.0), body(3000.0, 10.0)],
            ..MassReport::default()
        };
        assert_eq!(report.centre(1.0), Some([7.5, 0.0, 0.0]));
        assert!((report.mass_g(1.25) - 5.0).abs() < 1e-9);
        assert_eq!(MassReport::default().centre(1.0), None);
        assert_eq!(report.inertia(1.0), None, "no body measured its inertia");
    }

    /// Two 10 mm cubes, one bought in steel, 20 mm apart along X: about
    /// their shared centre each adds its own moment and its mass times
    /// the square of its distance off the axis; left out, the made cube
    /// stands alone.
    #[test]
    fn the_inertia_adds_each_body_about_the_shared_centre() {
        let own = 1000.0 * 200.0 / 12.0;
        let cube = |x: f64, density, bought| BodyMass {
            body: BodyId::new(),
            volume_mm3: 1000.0,
            centre: [x, 0.0, 0.0],
            density,
            inertia: Some(kernel_api::Inertia {
                about_centre: [[own, 0.0, 0.0], [0.0, own, 0.0], [0.0, 0.0, own]],
                principal: [
                    (own, [1.0, 0.0, 0.0]),
                    (own, [0.0, 1.0, 0.0]),
                    (own, [0.0, 0.0, 1.0]),
                ],
            }),
            bought,
        };
        let report = MassReport {
            bodies: vec![cube(0.0, None, false), cube(20.0, Some(7.85), true)],
            ..MassReport::default()
        };
        // 1 g of the made cube at 1 g/cm³ and 7.85 g of steel.
        let (light, heavy) = (1.0, 7.85);
        let centre = report.centre(1.0).unwrap();
        assert!((centre[0] - 20.0 * heavy / (light + heavy)).abs() < 1e-9);
        let inertia = report.inertia(1.0).unwrap();
        let t = inertia.about_centre;
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9 * b.abs().max(1.0);
        assert!(close(t[0][0], own / 1000.0 * (light + heavy)), "{t:?}");
        let off = |m: f64, x: f64| m * (x - centre[0]).powi(2);
        let about_y = own / 1000.0 * (light + heavy) + off(light, 0.0) + off(heavy, 20.0);
        assert!(close(t[1][1], about_y) && close(t[2][2], about_y), "{t:?}");
        assert!(close(inertia.principal[0].0, t[0][0]));

        let made = report.made_only();
        assert_eq!(made.bodies.len(), 1);
        assert!(close(
            made.inertia(1.0).unwrap().about_centre[1][1],
            own / 1000.0
        ));
    }
}
