//! Centre line: the middle path of a tube-like solid between two of its
//! faces, measured.
//!
//! The tool takes two faces of one body, the one selected when it starts
//! or the next clicked, then the other, and asks the kernel for the path
//! the solid's sections centre on between them. The path draws over the
//! scene with its length, and the task panel reports the length, how
//! closely the sections follow it, and whether it is straight. Nothing is
//! added to the document.

use core_document::{
    Args, BodyId, CommandError, CommandResult, ScreenSpaceLabel, ScreenSpaceMark,
    ScreenSpaceOverlay, ToolHint, ViewportHud, WorkbenchRuntimeContext,
};
use kernel_api::{CentreLine, FaceProbe};
use serde_json::{Value, json};

use crate::PartDesignWorkbench;

/// What the path is held to, in millimetres, unless the command says.
pub(crate) const TOLERANCE_MM: f64 = 0.02;

/// The tool's icon.
pub(crate) const ICON: &str = "datum-line";

/// A face picked for the path, in its body's frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Picked {
    pub body: BodyId,
    pub probe: FaceProbe,
}

/// The tool, from its first pick to the path shown.
#[derive(Debug, Clone, Default)]
pub(crate) struct CentreTask {
    pub first: Option<Picked>,
    /// The last face pick taken, so a pick is taken once.
    seen: Option<(uuid::Uuid, [u32; 3])>,
    /// The path found, in its body's frame.
    pub found: Option<(BodyId, CentreLine)>,
    /// Why the last pair of faces gave no path.
    pub refused: Option<String>,
}

impl CentreTask {
    /// What the user is to do next.
    pub fn prompt(&self) -> &'static str {
        match (&self.first, &self.found) {
            (_, Some(_)) => "Pick two other faces to measure again",
            (None, None) => "Pick the face where the tube starts",
            (Some(_), None) => "Pick the face where it ends",
        }
    }
}

/// The path between two faces of `body`, given in its own frame.
pub(crate) fn measure(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    from: &FaceProbe,
    to: &FaceProbe,
    tolerance: f64,
) -> Result<CentreLine, String> {
    let blob = ctx
        .document
        .imported_brep_blob(body)
        .ok_or("the body has no solid yet")?;
    let kernel = ctx.kernel.ok_or("no kernel to measure with")?;
    kernel
        .centre_line(blob, from, to, tolerance)
        .map_err(|e| e.to_string())
}

/// The command's answer.
pub(crate) fn report(line: &CentreLine) -> Value {
    json!({
        "length": line.length,
        "points": line.points,
        "deviation": line.deviation,
        "straight": line.straight,
    })
}

/// A probe as the command takes it.
fn probe_args(probe: &FaceProbe) -> (Value, Value) {
    (json!(probe.point), json!(probe.normal))
}

/// `part.centre_line`.
pub(crate) fn command(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let body = BodyId(a.id("body")?);
    if !ctx.document.bodies().iter().any(|b| b.id == body) {
        return Err(CommandError::bad("body", "is not a body of this document"));
    }
    let probe = |point: &str, normal: &str| -> Result<FaceProbe, CommandError> {
        Ok(FaceProbe {
            point: crate::commands::vector3(a.0.get(point), point)?.map(f64::from),
            normal: crate::commands::vector3(a.0.get(normal), normal)?.map(f64::from),
        })
    };
    let from = probe("from_point", "from_normal")?;
    let to = probe("to_point", "to_normal")?;
    let tolerance = match a.opt_number("tolerance")? {
        Some(t) if t > 0.0 => t,
        Some(_) => return Err(CommandError::bad("tolerance", "must be more than zero")),
        None => TOLERANCE_MM,
    };
    let line = measure(ctx, body, &from, &to, tolerance).map_err(CommandError::failed)?;
    Ok(report(&line))
}

fn length_text(line: &CentreLine) -> String {
    format!("{:.2} mm", line.length)
}

impl PartDesignWorkbench {
    /// Start the tool; a face already selected is its first pick.
    pub(crate) fn start_centre_line(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        self.centre = Some(CentreTask::default());
        self.take_centre_pick(ctx);
    }

    /// Take a new face pick for the tool, and measure once there are two.
    pub(crate) fn take_centre_pick(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(task) = self.centre.as_mut() else {
            return;
        };
        let (Some(body), Some(face)) = (ctx.selected_body_id, ctx.selected_face) else {
            return;
        };
        let signature = (body, face.point.map(f32::to_bits));
        if task.seen == Some(signature) {
            return;
        }
        task.seen = Some(signature);
        let body = BodyId(body);
        let Some(local) = ctx.selected_face_in(body) else {
            return;
        };
        let picked = Picked {
            body,
            probe: FaceProbe {
                point: local.point.map(f64::from),
                normal: local.normal.map(f64::from),
            },
        };
        let first = match task.first {
            Some(first) if first.body == body => first,
            Some(_) => {
                ctx.log_warn("Pick the second face on the same body");
                return;
            }
            None => {
                task.first = Some(picked);
                task.found = None;
                task.refused = None;
                return;
            }
        };
        task.first = None;
        match measure(ctx, body, &first.probe, &picked.probe, TOLERANCE_MM) {
            Ok(line) => {
                ctx.log_info(format!(
                    "Centre line {}{}",
                    length_text(&line),
                    if line.straight { ", straight" } else { "" }
                ));
                let (from_point, from_normal) = probe_args(&first.probe);
                let (to_point, to_normal) = probe_args(&picked.probe);
                ctx.record(
                    "part.centre_line",
                    crate::commands::object(json!({
                        "body": body.0.to_string(),
                        "from_point": from_point,
                        "from_normal": from_normal,
                        "to_point": to_point,
                        "to_normal": to_normal,
                    })),
                    report(&line),
                );
                task.found = Some((body, line));
                task.refused = None;
            }
            Err(err) => {
                ctx.log_warn(format!("No centre line between those faces: {err}"));
                task.found = None;
                task.refused = Some(err);
            }
        }
    }

    /// The path in the scene, as points where its body sits.
    fn centre_points(&self, ctx: &WorkbenchRuntimeContext) -> Option<(Vec<[f32; 3]>, &CentreLine)> {
        let (body, line) = self.centre.as_ref()?.found.as_ref()?;
        let placement = ctx.document.body_placement(*body);
        let points = line
            .points
            .iter()
            .map(|p| placement.point(p.map(|v| v as f32)))
            .collect();
        Some((points, line))
    }

    pub(crate) fn centre_overlays(&self, ctx: &WorkbenchRuntimeContext) -> Vec<ScreenSpaceOverlay> {
        let Some((points, _)) = self.centre_points(ctx) else {
            return Vec::new();
        };
        let px: Vec<Option<(f32, f32)>> =
            points.iter().map(|p| ctx.world_to_viewport(*p)).collect();
        px.windows(2)
            .filter_map(|w| match (w[0], w[1]) {
                (Some(a), Some(b)) => Some(ScreenSpaceOverlay::new(
                    [a.0, a.1],
                    [b.0, b.1],
                    ctx.sketch_palette.centre_line,
                    2.0,
                )),
                _ => None,
            })
            .collect()
    }

    /// A dot on each end of the path, and on a first face picked.
    pub(crate) fn centre_marks(&self, ctx: &WorkbenchRuntimeContext) -> Vec<ScreenSpaceMark> {
        let color = ctx.sketch_palette.centre_line;
        let mut at: Vec<[f32; 3]> = Vec::new();
        if let Some((points, _)) = self.centre_points(ctx) {
            at.extend(points.first());
            at.extend(points.last());
        }
        if let Some(first) = self.centre.as_ref().and_then(|t| t.first) {
            let placement = ctx.document.body_placement(first.body);
            at.push(placement.point(first.probe.point.map(|v| v as f32)));
        }
        at.into_iter()
            .filter_map(|p| ctx.world_to_viewport(p))
            .map(|(x, y)| ScreenSpaceMark::dot([x, y], 4.0, color))
            .collect()
    }

    /// The path's length beside its middle.
    pub(crate) fn centre_labels(&self, ctx: &WorkbenchRuntimeContext) -> Vec<ScreenSpaceLabel> {
        let Some((points, line)) = self.centre_points(ctx) else {
            return Vec::new();
        };
        let Some((x, y)) = points
            .get(points.len() / 2)
            .and_then(|p| ctx.world_to_viewport(*p))
        else {
            return Vec::new();
        };
        vec![
            ScreenSpaceLabel::new(
                [x, y - 16.0],
                length_text(line),
                ctx.sketch_palette.centre_line,
                12.0,
            )
            .pill()
            .mono(),
        ]
    }

    pub(crate) fn centre_hud(&self) -> Option<ViewportHud> {
        let task = self.centre.as_ref()?;
        Some(ViewportHud {
            tool: Some(ToolHint {
                icon: ICON,
                name: "Centre line".to_string(),
                prompt: task.prompt().to_string(),
                keys: vec![("Esc".to_string(), "close")],
            }),
            footer: task
                .found
                .as_ref()
                .map(|(_, line)| format!("Centre line {}", length_text(line)))
                .into_iter()
                .collect(),
            ..ViewportHud::default()
        })
    }

    /// The task panel while the tool is out.
    #[cfg(feature = "egui")]
    pub(crate) fn centre_panel(
        &mut self,
        ui: &mut egui::Ui,
        request: core_document::TaskRequest,
    ) -> core_document::TaskOutcome {
        use egui::RichText;
        use ui_kit::sans;
        use ui_kit::tokens::*;
        use ui_kit::widgets::{Note, note_card};

        if request.accept || request.cancel {
            self.centre = None;
            return core_document::TaskOutcome::Cancelled;
        }
        let Some(task) = &self.centre else {
            return core_document::TaskOutcome::Open;
        };
        ui.label(
            RichText::new(task.prompt())
                .font(sans(FONT_SM))
                .color(TEXT1),
        );
        ui.add_space(SPACE_2);
        if let Some((_, line)) = &task.found {
            let row = |ui: &mut egui::Ui, label: &str, value: String| {
                ui.horizontal(|ui| {
                    crate::editors::label_cell(ui, label);
                    ui.label(
                        RichText::new(value)
                            .font(ui_kit::mono(FONT_SM))
                            .color(TEXT1),
                    );
                });
            };
            row(ui, "Length", length_text(line));
            row(ui, "Deviation", format!("{:.3} mm", line.deviation));
            row(
                ui,
                "Shape",
                if line.straight { "Straight" } else { "Curved" }.to_string(),
            );
        } else if let Some(refused) = &task.refused {
            note_card(ui, Note::Error, Some("No centre line"), refused);
        }
        ui.add_space(SPACE_2);
        ui.label(
            RichText::new(
                "The line runs through the centres of the solid's sections, from \
                 the first face to the second. Both faces are on one body.",
            )
            .font(sans(FONT_XS))
            .color(TEXT3),
        );
        core_document::TaskOutcome::Open
    }
}
