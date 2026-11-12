//! Checks run away from the window: interference and clearance, mass,
//! and a joint's motion for collisions.

use crate::{
    AssemblyWorkbench, SWEEP_STEPS, Task, body_name, interference, mass, parts, sweep_check,
};
use core_document::jobs::Job;
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

impl AssemblyWorkbench {
    /// Look for clashes among the visible solid bodies, or between
    /// `around` and the others, and show them.
    pub(crate) fn check_interference(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        around: Option<BodyId>,
    ) {
        self.check_bodies(ctx, around, None);
    }

    /// Look for pairs nearer than `clearance` mm, as a check of shared
    /// material does for clashes.
    pub(crate) fn check_clearance(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        around: Option<BodyId>,
        clearance: f32,
    ) {
        self.check_bodies(ctx, around, Some(clearance));
    }

    fn check_bodies(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        around: Option<BodyId>,
        clearance: Option<f32>,
    ) {
        let Some(kernel) = ctx.kernel else {
            ctx.log_warn("No kernel to check interference with");
            return;
        };
        let mut check = match clearance {
            Some(gap) => interference::plan_clearance(ctx.document, None, f64::from(gap)),
            None => interference::plan(ctx.document, None),
        };
        if let Some(body) = around {
            check = check.around(body);
        }
        let total = check.pairs();
        let spawned = Job::spawn("printcad-interference", total, move |done, stop| {
            check.run(kernel, done, stop)
        });
        match spawned {
            Ok(job) => self.tab.checking = Some(job),
            Err(why) => {
                ctx.log_warn(format!("The interference check could not start: {why}"));
                return;
            }
        }
        self.tab.task = Some(Task::Interference {
            found: None,
            seq: ctx.document.mutation_seq(),
            around,
            clearance,
        });
    }

    /// A finished check's answer, when it has come: shown, and said in the
    /// log.
    pub(crate) fn collect_interference(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if !matches!(self.tab.task, Some(Task::Interference { .. })) {
            // Closed while it ran: nobody waits for it.
            self.tab.checking = None;
            return;
        }
        let Some(answer) = self
            .tab
            .checking
            .as_ref()
            .and_then(|job| job.answer("the check ended without an answer"))
        else {
            return;
        };
        self.tab.checking = None;
        match answer {
            Ok(found) => {
                let stopped = if found.stopped {
                    " (stopped early)"
                } else {
                    ""
                };
                ctx.log_info(match (found.clearance, found.clashes.len()) {
                    (Some(gap), _) => format!(
                        "{} pair{} nearer than {gap} mm among {} bodies{stopped}",
                        found.near.len(),
                        if found.near.len() == 1 { "" } else { "s" },
                        found.checked
                    ),
                    (None, 0) => format!("No interference among {} bodies{stopped}", found.checked),
                    (None, n) => format!(
                        "{n} clash{} among {} bodies{stopped}",
                        if n == 1 { "" } else { "es" },
                        found.checked
                    ),
                });
                for u in &found.unchecked {
                    ctx.log_warn(format!(
                        "{} and {} could not be checked: {}",
                        body_name(ctx, u.a),
                        body_name(ctx, u.b),
                        u.why
                    ));
                }
                if let Some(Task::Interference { found: slot, .. }) = &mut self.tab.task {
                    *slot = Some(found);
                }
            }
            Err(why) => {
                ctx.log_warn(format!("The interference check failed: {why}"));
                if matches!(self.tab.task, Some(Task::Interference { .. })) {
                    self.tab.task = None;
                }
            }
        }
    }

    /// How far a running check has got: pairs asked about, of how many.
    pub(crate) fn interference_progress(&self) -> Option<(usize, usize)> {
        self.tab.checking.as_ref().map(Job::progress)
    }

    /// Stop a running check; what it found so far still comes.
    pub(crate) fn stop_interference(&self) {
        if let Some(checking) = &self.tab.checking {
            checking.stop();
        }
    }

    /// Measure the visible solid bodies away from the window and show
    /// their mass and centre of mass.
    pub(crate) fn measure_mass(&mut self, ctx: &mut WorkbenchRuntimeContext, density: f32) {
        let Some(kernel) = ctx.kernel else {
            ctx.log_warn("No kernel to measure with");
            return;
        };
        let bought = parts::bought_bodies(ctx.document, &ctx.bought_kinds);
        let weighing = mass::plan(ctx.document, None, &bought);
        let total = weighing.len();
        let spawned = Job::spawn("printcad-mass", total, move |done, stop| {
            weighing.run(kernel, done, stop)
        });
        match spawned {
            Ok(job) => self.tab.measuring = Some(job),
            Err(why) => {
                ctx.log_warn(format!("The measuring could not start: {why}"));
                return;
            }
        }
        self.tab.task = Some(Task::Mass {
            found: None,
            density,
        });
    }

    /// Whether a panel's own work runs: the parts list's volumes being
    /// measured, or the open joint's drive swept.
    pub(crate) fn panel_running(&self) -> bool {
        #[cfg(feature = "egui")]
        return self.volumes.running()
            || self.tab.playing.is_some_and(
                |p| matches!(&self.tab.task, Some(Task::Joint { id, .. }) if *id == p.joint),
            );
        #[cfg(not(feature = "egui"))]
        false
    }

    /// A finished measuring's report, when it has come.
    pub(crate) fn collect_mass(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        if !matches!(self.tab.task, Some(Task::Mass { .. })) {
            self.tab.measuring = None;
            return;
        }
        let Some(answer) = self
            .tab
            .measuring
            .as_ref()
            .and_then(|job| job.answer("the measuring ended without an answer"))
        else {
            return;
        };
        self.tab.measuring = None;
        match answer {
            Ok(report) => {
                if let Some(Task::Mass { found, .. }) = &mut self.tab.task {
                    *found = Some(report);
                }
            }
            Err(why) => {
                ctx.log_warn(format!("The bodies could not be measured: {why}"));
                self.tab.task = None;
            }
        }
    }

    /// Check `joint`'s motion from `low` to `high` for collisions, away
    /// from the window.
    pub(crate) fn check_sweep(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        joint: FeatureId,
        (low, high): (f32, f32),
    ) {
        let Some(kernel) = ctx.kernel else {
            ctx.log_warn("No kernel to check collisions with");
            return;
        };
        let Some(check) = sweep_check::plan(ctx.document, joint, low, high, SWEEP_STEPS) else {
            self.tab.motion_clashes = Some((joint, Ok(Vec::new())));
            ctx.log_info("Nothing moves through this joint's range");
            return;
        };
        let total = check.pairs();
        let spawned = Job::spawn("printcad-sweep-check", total, move |done, stop| {
            check.run(kernel, done, stop)
        });
        match spawned {
            Ok(job) => {
                self.tab.motion_clashes = None;
                self.tab.sweeping = Some((joint, job));
            }
            Err(why) => ctx.log_warn(format!("The check could not start: {why}")),
        }
    }

    /// A finished motion check's answer, when it has come.
    pub(crate) fn collect_sweep(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some((joint, answer)) = self.tab.sweeping.as_ref().and_then(|(joint, job)| {
            Some((*joint, job.answer("the check ended without an answer")?))
        }) else {
            return;
        };
        self.tab.sweeping = None;
        match &answer {
            Ok(found) if found.is_empty() => ctx.log_info("No collisions through the motion"),
            Ok(found) => ctx.log_warn(format!(
                "{} collision{} through the motion",
                found.len(),
                if found.len() == 1 { "" } else { "s" }
            )),
            Err(why) => ctx.log_warn(format!("The motion could not be checked: {why}")),
        }
        self.tab.motion_clashes = Some((joint, answer));
    }

    /// How far a motion check has got, and for which joint.
    pub(crate) fn sweep_progress(&self) -> Option<(FeatureId, usize, usize)> {
        let (joint, job) = self.tab.sweeping.as_ref()?;
        let (done, total) = job.progress();
        Some((*joint, done, total))
    }

    /// How far a measuring has got: bodies done, of how many.
    pub(crate) fn mass_progress(&self) -> Option<(usize, usize)> {
        self.tab.measuring.as_ref().map(Job::progress)
    }
}
