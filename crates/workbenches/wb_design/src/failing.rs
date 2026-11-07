//! The edges a refused fillet or chamfer names: drawn in the danger colour
//! while the feature is selected or its task is open, and listed in the
//! task panel, where the row under the pointer lights its edge. The user
//! may leave them sharp: an edit of the feature's selection, never a
//! result built without them on its own.

use core_document::{
    CommandArgs, Document, FeatureId, ScreenSpaceOverlay, WorkbenchFeature, WorkbenchRuntimeContext,
};
use kernel_api::EdgeRefusal;
use serde_json::{Value, json};

use crate::DesignWorkbench;
use crate::feature::{DesignFeature, EdgePick, EdgeSel};

/// The tree menu's entry that leaves a refused feature's failing edges
/// sharp.
pub(crate) const LEAVE_SHARP: &str = "design.leave_sharp";

/// How many failing edges the feature `id` could leave sharp: its
/// refusal is current (the feature built since it was last changed) and
/// some edge would still be rounded. Zero otherwise.
pub(crate) fn sharp_count(document: &Document, id: FeatureId) -> usize {
    let Some(node) = document.get_feature_meta(id) else {
        return 0;
    };
    let refused = &node.refused_edges;
    if node.dirty || node.error.is_none() || refused.failing.len() >= refused.request.len() {
        return 0;
    }
    refused.failing.len()
}

/// "Leave this edge sharp", "Leave these 3 edges sharp".
pub(crate) fn sharp_label(count: usize) -> String {
    match count {
        1 => "Leave this edge sharp".into(),
        n => format!("Leave these {n} edges sharp"),
    }
}

/// `feature` taking every edge of its refused request but the failing
/// ones, each picked one by one: tangent chains are no longer followed,
/// since the request already holds them. `None` for a feature that takes
/// no edges.
fn without_failing(feature: &DesignFeature, refused: &EdgeRefusal) -> Option<DesignFeature> {
    let kept: Vec<EdgePick> = refused
        .request
        .iter()
        .enumerate()
        .filter(|(i, _)| !refused.failing.iter().any(|f| f.index == *i))
        .map(|(_, p)| EdgePick {
            point: p.point.map(|v| v as f32),
            direction: p.direction.map(|v| v as f32),
            faces: p.faces,
        })
        .collect();
    let mut feature = feature.clone();
    match &mut feature {
        DesignFeature::Fillet {
            edges,
            follow_tangent,
            ..
        }
        | DesignFeature::Chamfer {
            edges,
            follow_tangent,
            ..
        } => {
            *edges = EdgeSel::Edges(kept);
            *follow_tangent = false;
        }
        _ => return None,
    }
    Some(feature)
}

/// Take the failing edges of the refused feature `id` out of its
/// selection, as one edit recorded as `design.set`. The number left sharp.
pub(crate) fn leave_sharp(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
) -> Result<usize, String> {
    let count = sharp_count(ctx.document, id);
    if count == 0 {
        return Err("the feature has no failing edges to leave sharp".into());
    }
    let node = ctx.document.get_feature_meta(id).ok_or("no such feature")?;
    let feature = DesignFeature::from_json(&node.data).map_err(|e| e.to_string())?;
    let changed =
        without_failing(&feature, &node.refused_edges).ok_or("the feature takes no edges")?;
    let data = changed.to_json();
    ctx.document
        .update_feature_data(id, data.clone())
        .map_err(|e| e.to_string())?;
    ctx.document.mark_feature_dirty(id);
    let inner = data
        .as_object()
        .and_then(|m| m.values().next())
        .cloned()
        .unwrap_or(Value::Null);
    let mut args = CommandArgs::new();
    args.insert("feature".into(), json!(id.0.to_string()));
    for field in ["edges", "follow_tangent"] {
        args.insert(
            field.into(),
            inner.get(field).cloned().unwrap_or(Value::Null),
        );
    }
    ctx.record("design.set", args, Value::Null);
    Ok(count)
}

/// Line widths of a failing edge, and of the one whose row is hovered.
const WIDTH: f32 = 3.0;
const LIT_WIDTH: f32 = 6.0;

impl DesignWorkbench {
    /// The feature whose failing edges the view shows: the one open in the
    /// task panel, else the one selected.
    fn failing_feature(
        &self,
        ctx: &WorkbenchRuntimeContext,
        selected: Option<FeatureId>,
    ) -> Option<FeatureId> {
        #[cfg(feature = "egui")]
        if let Some((id, _)) = self.task_target(ctx) {
            return Some(id);
        }
        let _ = ctx;
        selected
    }

    pub(crate) fn failing_edge_overlays(
        &self,
        ctx: &WorkbenchRuntimeContext,
        selected: Option<FeatureId>,
    ) -> Vec<ScreenSpaceOverlay> {
        let Some(node) = self
            .failing_feature(ctx, selected)
            .and_then(|id| ctx.document.get_feature_meta(id))
        else {
            return Vec::new();
        };
        let Some(body) = node.body else {
            return Vec::new();
        };
        let placement = ctx.document.body_placement(body);
        let color = ctx.sketch_palette.conflict;
        let mut lines = Vec::new();
        for (i, edge) in node.refused_edges.failing.iter().enumerate() {
            let width = if self.hovered_failing == Some(i) {
                LIT_WIDTH
            } else {
                WIDTH
            };
            let px: Vec<Option<(f32, f32)>> = edge
                .points
                .iter()
                .map(|p| ctx.world_to_viewport(placement.point(p.map(|v| v as f32))))
                .collect();
            lines.extend(px.windows(2).filter_map(|w| match (w[0], w[1]) {
                (Some(a), Some(b)) => Some(ScreenSpaceOverlay::new(
                    [a.0, a.1],
                    [b.0, b.1],
                    color,
                    width,
                )),
                _ => None,
            }));
        }
        lines
    }
}

/// What the task panel's failing edges do: the row under the pointer, by
/// its place in the list, and whether the user asked to leave them sharp.
#[cfg(feature = "egui")]
pub(crate) struct Rows {
    pub hovered: Option<usize>,
    pub leave_sharp: bool,
}

/// One row per failing edge, its reason beside it, and the button that
/// leaves them sharp when `sharp` of them can be.
#[cfg(feature = "egui")]
pub(crate) fn rows(ui: &mut egui::Ui, failing: &[kernel_api::FailingEdge], sharp: usize) -> Rows {
    use ui_kit::tokens::DANGER;
    use ui_kit::widgets::{RowLook, list_row};
    let mut hovered = None;
    for (i, edge) in failing.iter().enumerate() {
        let row = list_row(
            ui,
            ("failing_edge", i),
            RowLook {
                problem: true,
                ..Default::default()
            },
            ("warning", DANGER),
            &format!("Edge {} of {}", edge.index + 1, edge.of),
            &edge.reason,
            |_| {},
        );
        if row.hovered() {
            hovered = Some(i);
        }
        row.on_hover_text(&edge.reason);
    }
    let leave_sharp = sharp > 0 && {
        ui.add_space(ui_kit::tokens::SPACE_1);
        ui_kit::widgets::secondary_button(ui, &sharp_label(sharp))
            .on_hover_text(
                "Takes them out of the feature's edges, which then builds as asked. \
                 Undo puts them back.",
            )
            .clicked()
    };
    Rows {
        hovered,
        leave_sharp,
    }
}
