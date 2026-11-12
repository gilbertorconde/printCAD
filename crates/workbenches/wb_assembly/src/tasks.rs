//! The task a feature opens and what the task panel shows.

use crate::{
    AssemblyWorkbench, COUPLING_KIND, Coupling, EXPLODED_KIND, GROUP_KIND, JOINT_KIND,
    JointFeature, MOTION_KIND, MotionStudy, RigidGroup, STATE_KIND, Stepping, Studying, Task,
    all_placements, body_name, exploded, feature_name, parts,
};
use core_document::{FeatureId, WorkbenchFeature, WorkbenchRuntimeContext};

impl AssemblyWorkbench {
    pub(crate) fn open_task(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
        let kind = ctx
            .document
            .get_feature_meta(id)
            .map(|n| n.workbench_id.as_str().to_string());
        if self.tab.picking.is_some() {
            return;
        }
        if kind.as_deref() == Some(parts::PARTS_KIND) {
            self.tab.task = Some(Task::Parts);
            return;
        }
        if kind.as_deref() == Some(STATE_KIND) {
            self.restore_state(ctx, id);
            return;
        }
        if kind.as_deref() == Some(MOTION_KIND) {
            let draft = ctx
                .document
                .get_feature_data(id)
                .and_then(|d| MotionStudy::from_json(d).ok())
                .unwrap_or_default();
            self.put_back_motion(ctx);
            self.tab.task = Some(Task::Motion(Box::new(Studying {
                study: Some(id),
                draft,
                frames: None,
                frame: 0,
                playing: false,
                clock: 0.0,
                placements: all_placements(ctx),
                traces: Vec::new(),
                tracing: false,
            })));
            return;
        }
        if kind.as_deref() == Some(EXPLODED_KIND) {
            self.put_back_explosion(ctx);
            let placements = all_placements(ctx);
            let at = exploded::view_of(ctx.document, id).map_or(0.0, |v| v.steps.len() as f32);
            self.tab.task = Some(Task::Explode {
                placements,
                spread: 0.0,
                steps: Box::new(Stepping {
                    view: Some(id),
                    at,
                    shift: [0.0, 0.0, 20.0],
                    ..Stepping::default()
                }),
            });
            self.show_steps(ctx);
            return;
        }
        if kind.as_deref() == Some(GROUP_KIND) {
            let members = ctx
                .document
                .get_feature_data(id)
                .and_then(|d| RigidGroup::from_json(d).ok())
                .map(|g| g.bodies())
                .unwrap_or_default();
            self.tab.group_seen = ctx.selected_body_id;
            self.tab.task = Some(Task::Group {
                editing: Some(id),
                members,
            });
            return;
        }
        if kind.as_deref() == Some(COUPLING_KIND) {
            self.tab.task = Some(Task::Coupling {
                id,
                before: ctx.document.get_feature_data(id).cloned(),
                placements: all_placements(ctx),
            });
            self.tab.verdict = None;
            return;
        }
        if kind.as_deref() != Some(JOINT_KIND) {
            return;
        }
        self.tab.task = Some(Task::Joint {
            id,
            before: ctx.document.get_feature_data(id).cloned(),
            placements: all_placements(ctx),
        });
        self.tab.verdict = None;
    }

    pub(crate) fn task_of(&self, ctx: &WorkbenchRuntimeContext) -> Option<core_document::TaskInfo> {
        if let Some(picking) = &self.tab.picking {
            return Some(core_document::TaskInfo {
                title: picking.kind.label().to_string(),
                icon: picking.kind.icon(),
                confirmable: false,
                stepwise: false,
                ..Default::default()
            });
        }
        match self.tab.task.as_ref()? {
            Task::Joint { id, .. } => {
                let kind = ctx
                    .document
                    .get_feature_data(*id)
                    .and_then(|d| JointFeature::from_json(d).ok())
                    .map(|j| j.kind)?;
                Some(core_document::TaskInfo {
                    title: kind.label().to_string(),
                    subtitle: feature_name(ctx, *id),
                    icon: kind.icon(),
                    confirmable: true,
                    stepwise: false,
                    ..Default::default()
                })
            }
            Task::Coupling { id, .. } => Some(core_document::TaskInfo {
                title: ctx
                    .document
                    .get_feature_data(*id)
                    .and_then(|d| Coupling::from_json(d).ok())
                    .map_or("Coupling", |c| c.gearing.label())
                    .to_string(),
                subtitle: feature_name(ctx, *id),
                icon: "involute-gear",
                confirmable: true,
                stepwise: false,
                ..Default::default()
            }),
            Task::Move { body, .. } => Some(core_document::TaskInfo {
                title: "Move body".to_string(),
                subtitle: body_name(ctx, *body),
                icon: "move-geometry",
                confirmable: true,
                stepwise: false,
                ..Default::default()
            }),
            Task::Interference { .. } => Some(core_document::TaskInfo {
                title: "Interference".to_string(),
                icon: "check-geometry",
                confirmable: false,
                stepwise: false,
                ..Default::default()
            }),
            Task::Explode { .. } => Some(core_document::TaskInfo {
                title: "Exploded view".to_string(),
                icon: "scale-geometry",
                confirmable: false,
                stepwise: false,
                ..Default::default()
            }),
            Task::Parts => Some(core_document::TaskInfo {
                title: "Parts list".to_string(),
                icon: "file-document",
                confirmable: false,
                stepwise: false,
                ..Default::default()
            }),
            Task::Replace { .. } => Some(core_document::TaskInfo {
                title: "Replace body".to_string(),
                icon: "carbon-copy",
                confirmable: true,
                stepwise: false,
                ..Default::default()
            }),
            Task::Copies { .. } => Some(core_document::TaskInfo {
                title: "Insert linked copies".to_string(),
                icon: "clone",
                confirmable: true,
                stepwise: false,
                ..Default::default()
            }),
            Task::Group { .. } => Some(core_document::TaskInfo {
                title: "Rigid group".to_string(),
                icon: "tree-group",
                confirmable: true,
                stepwise: false,
                ..Default::default()
            }),
            Task::Motion(_) => Some(core_document::TaskInfo {
                title: "Motion over time".to_string(),
                icon: "polar-pattern",
                confirmable: false,
                stepwise: false,
                ..Default::default()
            }),
            Task::Mass { .. } => Some(core_document::TaskInfo {
                title: "Mass".to_string(),
                icon: "measure",
                confirmable: false,
                stepwise: false,
                ..Default::default()
            }),
        }
    }
}
