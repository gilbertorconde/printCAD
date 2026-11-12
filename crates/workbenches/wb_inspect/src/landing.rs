//! Moving a scan and keeping how as an alignment, and the best fit's
//! answer landed once it is in.

use crate::{BEST_FIT, InspectionWorkbench, align, best_fit, making::numbered};
use core_document::{BodyId, CommandArgs, WorkbenchRuntimeContext};
use serde_json::{Value, json};

/// Move `scan` to `after` and keep how as an alignment; the answer says
/// where it went.
pub(crate) fn record_alignment(
    ctx: &mut WorkbenchRuntimeContext,
    scan: BodyId,
    how: align::How,
    after: core_document::BodyPlacement,
) -> Result<Value, String> {
    if ctx.document.body_scan(scan).is_none() {
        return Err("only a scan is aligned".into());
    }
    let before = ctx.document.body_placement(scan);
    let a = align::Alignment {
        name: numbered(ctx.document, "Alignment"),
        scan,
        how,
        before,
        after,
    };
    // The step that can refuse goes first, so a refusal moves nothing.
    let id = ctx
        .document
        .add_feature_in_body(a.clone(), a.name.clone(), Some(scan))
        .map_err(|e| e.to_string())?;
    ctx.document.set_body_placement(scan, after);
    ctx.log_info(format!("{}: {}", a.how.word(), a.how.summary()));
    let mut out = json!({
        "alignment": id.0.to_string(),
        "name": a.name,
        "translation": after.translation,
        "rotation": after.rotation,
    });
    if let align::How::BestFit {
        rms, inliers, free, ..
    } = a.how
    {
        out["rms"] = json!(rms);
        out["inliers"] = json!(inliers);
        out["free"] = json!(free);
    }
    Ok(out)
}

pub(crate) fn best_fit_how(
    reference: BodyId,
    search: bool,
    fit: &kernel_api::Registration,
) -> align::How {
    align::How::BestFit {
        marked: false,
        reference,
        search,
        rms: fit.rms,
        inliers: fit.inliers,
        sampled: fit.sampled,
        free: fit.free.len(),
        converged: fit.converged,
    }
}

impl InspectionWorkbench {
    /// Apply the running best fit once its answer is in, while the scan
    /// and reference are still as it fitted them; a stale answer is
    /// dropped with a word in the log.
    pub(crate) fn land_best_fit(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let (job, answer) = match self.tab.fitting.landed(ctx.document) {
            None => return,
            Some(best_fit::Landed::Stale) => {
                ctx.log_warn("Best fit dropped: the scan or its reference changed while it ran");
                return;
            }
            Some(best_fit::Landed::Fresh(landed)) => *landed,
        };
        let (after, fit) = match answer {
            Ok(fitted) => fitted,
            Err(why) => return ctx.log_warn(format!("Best fit: {why}")),
        };
        let how = best_fit_how(job.reference, job.search, &fit);
        match record_alignment(ctx, job.scan, how, after) {
            Ok(answer) => {
                let mut args = CommandArgs::new();
                args.insert("scan".into(), json!(job.scan.0.to_string()));
                args.insert("reference".into(), json!(job.reference.0.to_string()));
                ctx.record(BEST_FIT, args, answer);
            }
            Err(why) => ctx.log_warn(why),
        }
    }
}
