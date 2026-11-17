//! Editing a spline by its nodes in the select tool. A double click on a
//! spline brings it into node form (`nodes::edit_nodes`, the curve kept)
//! and shows its nodes; a click on one picks it and shows its two handles,
//! each joined to it by a line. Dragging a node carries its handles;
//! dragging a handle holds its node where it is, the other handle
//! following as the node's kind says (in line with it when smooth,
//! mirrored when symmetric, free at a corner). A right click on a node
//! opens a menu at the pointer (`HostRequest::OpenMenu`) to make it a
//! corner, smooth or symmetric, its kind ticked. Escape, another tool or a
//! click on anything else ends the edit.

use std::collections::HashSet;

use core_document::{HostRequest, MenuItem, WorkbenchRuntimeContext};
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::nodes::{self, NodeKind, Nodes};
use crate::sketch::{GeometryElement, Vec2D};
use crate::{DragState, SketchWorkbench, snap, step};
use core_document::InputResult;

/// The spline under node edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeEdit {
    pub spline: Uuid,
    /// The node picked, its handles shown.
    pub node: Option<Uuid>,
}

/// What the view draws for the spline under node edit.
pub(crate) struct NodeView {
    pub spline: Uuid,
    /// Every node, and whether it is the one picked.
    pub nodes: Vec<(Uuid, bool)>,
    /// The picked node's handles, each with the node it hangs from.
    pub handles: Vec<(Uuid, Uuid)>,
    /// The handles not shown, nor taken by a click.
    pub hidden: HashSet<Uuid>,
}

/// The node menu's entries, by kind, the id its pick comes back as.
const KIND_COMMANDS: [(NodeKind, &str); 3] = [
    (NodeKind::Corner, "sketch.node.corner"),
    (NodeKind::Smooth, "sketch.node.smooth"),
    (NodeKind::Symmetric, "sketch.node.symmetric"),
];

impl SketchWorkbench {
    /// The spline's nodes, while it is under node edit and still a path.
    fn edited_nodes(&self, feature: &SketchFeature) -> Option<(NodeEdit, Nodes)> {
        let edit = self.tab.node_edit?;
        Some((edit, nodes::nodes_of(&feature.sketch, edit.spline)?))
    }

    /// A double click on spline `spline`: into node form, its nodes shown.
    pub(crate) fn enter_node_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        mut feature: SketchFeature,
        spline: Uuid,
    ) -> InputResult {
        match nodes::edit_nodes(&mut feature.sketch, spline) {
            Ok(converted) => {
                if converted {
                    self.store_sketch(ctx, feature);
                    ctx.log_info("The spline is now nodes and handles, the curve as it was");
                }
                self.tab.selected.clear();
                self.tab.node_edit = Some(NodeEdit { spline, node: None });
            }
            Err(why) => ctx.log_warn(why),
        }
        InputResult::consumed()
    }

    /// The spline under node edit and the node picked on it, if any.
    pub fn node_edit_state(&self) -> Option<(Uuid, Option<Uuid>)> {
        self.tab.node_edit.map(|e| (e.spline, e.node))
    }

    /// End the node edit, if one is on.
    pub(crate) fn leave_node_edit(&mut self) -> bool {
        if self.tab.node_edit.is_none() {
            return false;
        }
        self.tab.node_edit = None;
        true
    }

    /// A press in the select tool while a spline is under node edit: a
    /// node, a shown handle or the spline itself is taken here; anything
    /// else ends the edit and is left to the select tool (`None`).
    pub(crate) fn node_press(
        &mut self,
        feature: &SketchFeature,
        cursor: Vec2D,
        tol: f64,
    ) -> Option<InputResult> {
        let Some((edit, nodes)) = self.edited_nodes(feature) else {
            self.leave_node_edit();
            return None;
        };
        let view = node_view(&feature.sketch, &edit, &nodes);
        let sketch = &feature.sketch;
        let hit = snap::hit_candidates(sketch, cursor, tol)
            .into_iter()
            .find(|id| !view.hidden.contains(id));
        let Some(hit) = hit else {
            self.leave_node_edit();
            return None;
        };
        if let Some(node) = nodes.node(hit) {
            let carried: Vec<Uuid> = std::iter::once(node.point).chain(node.handles()).collect();
            self.tab.node_edit = Some(NodeEdit {
                node: Some(node.point),
                ..edit
            });
            self.begin_held_drag(sketch, hit, &carried, Vec::new(), cursor);
        } else if let Some(owner) = view
            .handles
            .iter()
            .find(|(handle, _)| *handle == hit)
            .map(|(_, node)| *node)
        {
            self.begin_held_drag(sketch, hit, &[hit], vec![owner], cursor);
        } else if hit == edit.spline {
            self.tab.node_edit = Some(NodeEdit { node: None, ..edit });
            self.begin_held_drag(sketch, hit, &[hit], Vec::new(), cursor);
        } else {
            self.leave_node_edit();
            return None;
        }
        Some(InputResult::consumed())
    }

    /// Arm a drag of `elements`, `hit` the one pressed and selected, with
    /// `held` points kept where they are.
    fn begin_held_drag(
        &mut self,
        sketch: &crate::sketch::Sketch,
        hit: Uuid,
        elements: &[Uuid],
        held: Vec<Uuid>,
        cursor: Vec2D,
    ) {
        self.tab.selected.clear();
        self.tab.selected.insert(hit);
        self.tab.dragging = Some(DragState {
            points: step::drag_targets(sketch, elements),
            grab: cursor,
            hit,
            was_selected: false,
            moved: false,
            elements: elements.to_vec(),
            delta: Vec2D::new(0.0, 0.0),
            target: None,
            held,
        });
    }

    /// A right click at `cursor` while a spline is under node edit: on a
    /// node with both handles, the menu of its kinds opens at the pointer,
    /// the node picked. Answers whether it did.
    pub(crate) fn node_right_click(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        feature: &SketchFeature,
        cursor: Vec2D,
        tol: f64,
    ) -> bool {
        let Some((edit, nodes)) = self.edited_nodes(feature) else {
            return false;
        };
        let view = node_view(&feature.sketch, &edit, &nodes);
        let hit = snap::hit_candidates(&feature.sketch, cursor, tol)
            .into_iter()
            .find(|id| !view.hidden.contains(id));
        let Some(node) = hit.and_then(|id| nodes.node(id)) else {
            return false;
        };
        if node.before.is_none() || node.after.is_none() {
            return false;
        }
        let kind = nodes::kind_of(&feature.sketch, node);
        let items = KIND_COMMANDS
            .iter()
            .map(|(k, id)| MenuItem::new(*id, k.label()).checked(*k == kind))
            .collect();
        ctx.request(HostRequest::OpenMenu {
            bench: core_document::WorkbenchId::new(crate::KIND),
            tag: node.point.to_string(),
            items,
        });
        self.tab.node_edit = Some(NodeEdit {
            node: Some(node.point),
            ..edit
        });
        true
    }

    /// A pick from a node's menu: node `tag` of the spline under edit made
    /// the kind `id` names. Answers whether `id` is one of the menu's.
    pub(crate) fn node_command(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        id: &str,
        tag: &str,
    ) -> bool {
        let Some(&(kind, _)) = KIND_COMMANDS.iter().find(|(_, c)| *c == id) else {
            return false;
        };
        let (Some(edit), Ok(point)) = (self.tab.node_edit, Uuid::parse_str(tag)) else {
            return true;
        };
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return true;
        };
        let Some(node) =
            nodes::nodes_of(&feature.sketch, edit.spline).and_then(|n| n.node(point).copied())
        else {
            return true;
        };
        if nodes::set_kind(&mut feature.sketch, &node, kind) {
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
            ctx.request(HostRequest::JournalLabel(format!("{} node", kind.label())));
        }
        true
    }

    /// What the view draws for the spline under node edit.
    pub(crate) fn node_view(&self, feature: &SketchFeature) -> Option<NodeView> {
        let (edit, nodes) = self.edited_nodes(feature)?;
        Some(node_view(&feature.sketch, &edit, &nodes))
    }
}

fn node_view(sketch: &crate::sketch::Sketch, edit: &NodeEdit, nodes: &Nodes) -> NodeView {
    let picked = edit.node.and_then(|p| nodes.node(p));
    let handles: Vec<(Uuid, Uuid)> = picked
        .map(|n| n.handles().map(|h| (h, n.point)).collect())
        .unwrap_or_default();
    let mut hidden: HashSet<Uuid> = nodes.nodes.iter().flat_map(|n| n.handles()).collect();
    for (handle, _) in &handles {
        hidden.remove(handle);
    }
    // A handle that is also a node of another curve stays: it is not this
    // path's alone.
    hidden.retain(|id| {
        !sketch.geometry.iter().any(|g| {
            g.id() != edit.spline
                && !matches!(g, GeometryElement::Point(_))
                && crate::sketch::Sketch::curve_point_ids(g).contains(id)
        })
    });
    NodeView {
        spline: edit.spline,
        nodes: nodes
            .nodes
            .iter()
            .map(|n| (n.point, Some(n.point) == edit.node))
            .collect(),
        handles,
        hidden,
    }
}
