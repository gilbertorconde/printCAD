//! Between the interface's data (`bench_api`) and the app's own types.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use core_document::runtime::{
    CameraOrientRequest, HostRequest, MouseButton, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use core_document::{Document, FeatureNode};

/// `text` for as long as the app runs. The app's presentation types name
/// icons and short labels as `&'static str`; a package's are kept once
/// each, so the set only grows by the distinct names packages use.
pub(crate) fn intern(text: &str) -> &'static str {
    static SET: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(Default::default);
    let Ok(mut set) = SET.lock() else {
        return "";
    };
    if let Some(known) = set.get(text) {
        return known;
    }
    let kept: &'static str = Box::leak(text.to_owned().into_boxed_str());
    set.insert(kept);
    kept
}

/// A feature as a bench sees it: with its formulas' values in.
pub(crate) fn node(document: &Document, node: &FeatureNode) -> bench_api::Node {
    bench_api::Node {
        id: node.id.0.to_string(),
        kind: node.workbench_id.as_str().to_string(),
        name: node.name.clone(),
        body: node.body.map(|b| b.0.to_string()),
        data: document
            .feature_values(node.id)
            .cloned()
            .unwrap_or_else(|| node.data.clone()),
        visible: node.visible,
        made_by: node.made_by.clone(),
        deps: document
            .feature_tree()
            .dependencies(node.id)
            .iter()
            .map(|d| d.0.to_string())
            .collect(),
        seq: node.seq,
    }
}

/// A feature with its data as given, when no document is at hand.
pub(crate) fn bare_node(node: &FeatureNode, data: &serde_json::Value) -> bench_api::Node {
    bench_api::Node {
        id: node.id.0.to_string(),
        kind: node.workbench_id.as_str().to_string(),
        name: node.name.clone(),
        body: node.body.map(|b| b.0.to_string()),
        data: data.clone(),
        visible: node.visible,
        made_by: node.made_by.clone(),
        deps: Vec::new(),
        seq: node.seq,
    }
}

pub(crate) fn parameter(p: bench_api::Parameter) -> core_document::Parameter {
    use core_document::expr::Dim;
    core_document::Parameter {
        key: p.key,
        name: p.name,
        label: p.label,
        dim: match p.dim {
            bench_api::Dim::Length => Dim::LENGTH,
            bench_api::Dim::Angle => Dim::ANGLE,
            bench_api::Dim::Number => Dim::NUMBER,
        },
        pointer: p.pointer,
        scale: p.scale,
        integer: p.integer,
    }
}

/// What the cursor is over and what is selected, for an input.
pub(crate) fn pointer(
    ctx: &WorkbenchRuntimeContext,
    position: Option<(f32, f32)>,
) -> bench_api::Pointer {
    let position = position.or(ctx.cursor_viewport_pos);
    bench_api::Pointer {
        position: position.map(|(x, y)| [x, y]),
        ray: position.and_then(|p| ctx.viewport_to_ray(p)),
        world: ctx.hovered_world_pos,
        body: ctx.hovered_body_id.map(|b| b.to_string()),
        selected_body: ctx.selected_body_id.map(|b| b.to_string()),
        face: ctx.selected_face.map(|f| bench_api::FaceRef {
            point: f.point,
            normal: f.normal,
            surface: f.surface,
        }),
        edges: ctx
            .selected_edges
            .iter()
            .map(|e| bench_api::EdgeRef {
                body: e.body.to_string(),
                point: e.point,
                direction: e.direction,
                length: e.length_mm,
            })
            .collect(),
        active_feature: ctx.active_document_object.map(|f| f.0.to_string()),
        ctrl: ctx.ctrl_down,
    }
}

fn button(b: MouseButton) -> bench_api::Button {
    match b {
        MouseButton::Left => bench_api::Button::Left,
        MouseButton::Middle => bench_api::Button::Middle,
        MouseButton::Right => bench_api::Button::Right,
        _ => bench_api::Button::Other,
    }
}

/// The event and where it happened.
pub(crate) fn event(event: &WorkbenchInputEvent) -> (bench_api::Event, Option<(f32, f32)>) {
    use bench_api::Event;
    match event {
        WorkbenchInputEvent::MousePress {
            button: b,
            viewport_pos,
        } => (Event::Press { button: button(*b) }, Some(*viewport_pos)),
        WorkbenchInputEvent::MouseRelease {
            button: b,
            viewport_pos,
        } => (Event::Release { button: button(*b) }, Some(*viewport_pos)),
        WorkbenchInputEvent::MouseMove { viewport_pos } => (Event::Move, Some(*viewport_pos)),
        WorkbenchInputEvent::KeyPress { key } => (
            Event::Key {
                key: format!("{key:?}"),
                down: true,
            },
            None,
        ),
        WorkbenchInputEvent::KeyRelease { key } => (
            Event::Key {
                key: format!("{key:?}"),
                down: false,
            },
            None,
        ),
        WorkbenchInputEvent::ToolActivated => (Event::ToolActivated, None),
        WorkbenchInputEvent::Action { id } => (Event::Action { id: id.clone() }, None),
    }
}

/// What a bench asked for, as the host's request; `None` for one the
/// package may not make or that names nothing.
pub(crate) fn request(
    request: bench_api::Request,
    granted: &bench_api::Capabilities,
) -> Result<HostRequest, String> {
    use bench_api::Request as R;
    Ok(match request {
        R::ActivateTool { tool } => HostRequest::ActivateTool(tool),
        R::SelectBody { body } => HostRequest::SelectBody(crate::host::body_id(&body)?),
        R::JournalLabel { label } => HostRequest::JournalLabel(label),
        R::SwitchWorkbench { workbench } => {
            HostRequest::SwitchWorkbench(core_document::WorkbenchId::new(workbench))
        }
        R::OrientCamera { origin, normal, up } => HostRequest::OrientCamera(CameraOrientRequest {
            plane_origin: origin,
            plane_normal: normal,
            plane_up: up,
        }),
        R::FinishEditing => HostRequest::FinishEditing,
        R::SaveFile {
            name,
            kind,
            extension,
            contents,
        } => {
            if !granted.save_dialog {
                return Err("the package has not been allowed to save files".into());
            }
            HostRequest::SaveFile {
                name,
                kind,
                extension,
                contents,
            }
        }
    })
}

/// A mesh with smooth normals, for the scene.
pub(crate) fn tri_mesh(positions: Vec<[f32; 3]>, indices: Vec<u32>) -> kernel_api::TriMesh {
    use glam::Vec3;
    let mut normals = vec![Vec3::ZERO; positions.len()];
    let valid = |i: u32| (i as usize) < positions.len();
    let indices: Vec<u32> = indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|t| t.iter().all(|i| valid(*i)))
        .flatten()
        .copied()
        .collect();
    for t in indices.as_chunks::<3>().0 {
        let [a, b, c] = t.map(|i| Vec3::from_array(positions[i as usize]));
        let n = (b - a).cross(c - a);
        for i in t {
            normals[*i as usize] += n;
        }
    }
    kernel_api::TriMesh {
        normals: normals
            .into_iter()
            .map(|n| n.normalize_or(Vec3::Z).to_array())
            .collect(),
        positions,
        indices,
        ..Default::default()
    }
}

pub(crate) fn menu_scope(scope: &core_document::MenuScope) -> bench_api::MenuScope {
    use core_document::MenuScope as M;
    match scope {
        M::ViewportBody(b) => bench_api::MenuScope::ViewportBody(b.0.to_string()),
        M::TreeFeature(f) => bench_api::MenuScope::TreeFeature(f.0.to_string()),
        M::TreeBody(b) => bench_api::MenuScope::TreeBody(b.0.to_string()),
        M::StartPage => bench_api::MenuScope::StartPage,
        M::EditMenu => bench_api::MenuScope::EditMenu,
    }
}
