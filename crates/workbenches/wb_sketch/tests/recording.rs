//! A session recorded through the UI replays as a script to the same
//! sketch: clicks, typed dimensions, constraint tools, a drag and a delete
//! go in as viewport events, come out as `pc.sketch.*` calls, and run
//! again on the document as it was before.

use core_document::{
    CommandArgs, CommandError, CommandResult, CommandSpec, Document, FeatureId, HookOutcome,
    KeyCode, MouseButton, Recorded, Workbench, WorkbenchContext, WorkbenchFeature,
    WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use glam::{Mat4, Vec3};
use wb_sketch::sketch::{GeometryElement, Sketch};
use wb_sketch::{SketchFeature, SketchWorkbench};

const VIEWPORT: (u32, u32, u32, u32) = (0, 0, 800, 600);
const CAM_POS: [f32; 3] = [0.0, 0.0, 50.0];

fn view_proj() -> [[f32; 4]; 4] {
    let proj = glam::camera::rh::proj::directx::perspective(
        60f32.to_radians(),
        800.0 / 600.0,
        0.1,
        1000.0,
    );
    let flip_y = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0));
    let view = glam::camera::rh::view::look_at_mat4(Vec3::from_array(CAM_POS), Vec3::ZERO, Vec3::Y);
    (flip_y * proj * view).to_cols_array_2d()
}

/// The app shell's part: hooks run, their outcome carried and what they
/// recorded kept.
struct Session {
    doc: Document,
    wb: SketchWorkbench,
    active: Option<FeatureId>,
    recorded: Vec<Recorded>,
}

impl Session {
    fn ctx(&mut self) -> WorkbenchRuntimeContext<'_> {
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0, 0.0, 0.0], VIEWPORT);
        ctx.view_proj = Some(view_proj());
        ctx.active_document_object = self.active;
        ctx
    }

    fn event(&mut self, event: WorkbenchInputEvent, tool: Option<&str>) {
        let active = self.active;
        let mut ctx = WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0; 3], VIEWPORT);
        ctx.view_proj = Some(view_proj());
        ctx.active_document_object = active;
        self.wb.on_input(&event, tool, &mut ctx);
        let outcome = HookOutcome::take(&mut ctx);
        self.active = outcome.active_document_object;
        self.recorded.extend(outcome.recorded);
        let mut ctx = WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0; 3], VIEWPORT);
        ctx.view_proj = Some(view_proj());
        ctx.active_document_object = self.active;
        self.wb.on_frame(0.016, &mut ctx);
        self.recorded.extend(HookOutcome::take(&mut ctx).recorded);
    }

    fn px(&mut self, x: f32, y: f32) -> (f32, f32) {
        self.ctx().world_to_viewport([x, y, 0.0]).unwrap()
    }

    fn press(&mut self, x: f32, y: f32, tool: &str) {
        let viewport_pos = self.px(x, y);
        self.event(
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            },
            Some(tool),
        );
    }

    fn release(&mut self, x: f32, y: f32, tool: &str) {
        let viewport_pos = self.px(x, y);
        self.event(
            WorkbenchInputEvent::MouseRelease {
                button: MouseButton::Left,
                viewport_pos,
            },
            Some(tool),
        );
    }

    fn click(&mut self, x: f32, y: f32, tool: &str) {
        self.press(x, y, tool);
        self.release(x, y, tool);
    }

    fn move_to(&mut self, x: f32, y: f32, tool: &str) {
        let viewport_pos = self.px(x, y);
        self.event(WorkbenchInputEvent::MouseMove { viewport_pos }, Some(tool));
    }

    fn key(&mut self, key: KeyCode, tool: Option<&str>) {
        self.event(WorkbenchInputEvent::KeyPress { key }, tool);
    }

    fn sketch(&self, id: FeatureId) -> Sketch {
        SketchFeature::from_json(self.doc.get_feature_data(id).unwrap())
            .unwrap()
            .sketch
    }
}

/// The bench's commands, run on a document, for the replay.
struct Replay {
    doc: Document,
    wb: SketchWorkbench,
    specs: Vec<CommandSpec>,
}

impl scripting::Host for Replay {
    fn commands(&self) -> Vec<CommandSpec> {
        self.specs.clone()
    }

    fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
        let spec = self
            .specs
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
        spec.check(&args)?;
        let mut ctx = WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0; 3], VIEWPORT);
        self.wb.run_command(id, &args, &mut ctx)
    }
}

/// What a sketch comes to, in a form two sketches compare by: its points
/// where they are, what kinds of curve it has, and its constraints by kind.
fn summary(sketch: &Sketch) -> (Vec<(i64, i64)>, Vec<&'static str>, Vec<String>) {
    let mut points: Vec<(i64, i64)> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(p) => Some((
                (p.position.x * 1000.0).round() as i64,
                (p.position.y * 1000.0).round() as i64,
            )),
            _ => None,
        })
        .collect();
    points.sort();
    let mut curves: Vec<&'static str> = sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Point(_) => None,
            GeometryElement::Line(_) => Some("line"),
            GeometryElement::Circle(_) => Some("circle"),
            GeometryElement::Arc(_) => Some("arc"),
            GeometryElement::Ellipse(_) => Some("ellipse"),
            GeometryElement::BSpline(_) => Some("bspline"),
        })
        .collect();
    curves.sort();
    let mut constraints: Vec<String> = sketch
        .constraints
        .iter()
        .map(|c| format!("{:?}", std::mem::discriminant(&c.kind)))
        .collect();
    constraints.sort();
    (points, curves, constraints)
}

#[test]
fn a_session_recorded_through_the_ui_replays_to_the_same_sketch() {
    let (mut s, id, before) = session_on_a_sketch();

    // A rectangle, a line chain of two segments ending on the
    // rectangle's corner, a circle, and a slanted line.
    s.click(2.0, 2.0, "sketch.rect");
    s.click(14.0, 9.0, "sketch.rect");
    s.click(-10.0, -3.0, "sketch.line");
    s.click(-4.0, -8.0, "sketch.line");
    s.click(2.0, 2.0, "sketch.line");
    s.key(KeyCode::Escape, Some("sketch.line"));
    s.click(-9.0, 7.0, "sketch.circle");
    s.click(-6.5, 7.0, "sketch.circle");
    s.click(3.0, -12.0, "sketch.line");
    s.click(12.0, -6.0, "sketch.line");
    s.key(KeyCode::Escape, Some("sketch.line"));

    // The slanted line, dimensioned at its measured length, then dragged
    // by its middle.
    s.click(7.5, -9.0, "sketch.select");
    s.key(KeyCode::A, Some("sketch.constrain.dimension"));
    s.key(KeyCode::Escape, Some("sketch.select"));
    s.press(7.5, -9.0, "sketch.select");
    s.move_to(8.5, -7.0, "sketch.select");
    s.release(8.5, -7.0, "sketch.select");
    s.key(KeyCode::Escape, Some("sketch.select"));

    // The circle deleted.
    s.click(-6.5, 7.0, "sketch.select");
    s.key(KeyCode::Delete, Some("sketch.select"));
    s.key(KeyCode::A, None);

    let done = summary(&s.sketch(id));
    assert!(!done.0.is_empty());
    let kinds: Vec<&str> = s.recorded.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "sketch.draw",
            "sketch.draw",
            "sketch.draw",
            "sketch.draw",
            "sketch.constrain",
            "sketch.drag",
            "sketch.delete"
        ],
        "{:#?}",
        s.recorded
    );

    assert_replays(&s.recorded, before, id, &done);
}

/// Write `recorded` as a script, run it on `before`, and check sketch `id`
/// comes out as `done`.
fn assert_replays(
    recorded: &[Recorded],
    before: Document,
    id: FeatureId,
    done: &(Vec<(i64, i64)>, Vec<&'static str>, Vec<String>),
) {
    let mut recorder = scripting::Recorder::default();
    for call in recorded {
        recorder.push(call);
    }
    let script = recorder.script("Recorded in a test");
    let mut context = WorkbenchContext::default();
    SketchWorkbench::default().configure(&mut context);
    let mut replay = Replay {
        doc: before,
        wb: SketchWorkbench::default(),
        specs: context.commands().to_vec(),
    };
    let out = scripting::ScriptEngine::new().run_script(&script, "recorded.lua", &mut replay);
    assert_eq!(out.error, None, "{script}");
    let replayed = summary(
        &SketchFeature::from_json(replay.doc.get_feature_data(id).unwrap())
            .unwrap()
            .sketch,
    );
    assert_eq!(&replayed, done, "{script}");
}

fn session_on_a_sketch() -> (Session, FeatureId, Document) {
    let mut s = Session {
        doc: Document::new("recorded"),
        wb: SketchWorkbench::default(),
        active: None,
        recorded: Vec::new(),
    };
    let plane = Sketch::new("s").plane;
    let id = s
        .doc
        .add_feature_in_body(
            SketchFeature::new(Sketch::new("s"), plane),
            "s".into(),
            None,
        )
        .unwrap();
    s.active = Some(id);
    s.key(KeyCode::A, None);
    let before = s.doc.clone();
    (s, id, before)
}

#[test]
fn typed_lengths_polyline_arcs_and_a_spline_replay_too() {
    let (mut s, id, before) = session_on_a_sketch();
    // A line whose length is typed, committed with Enter: a driving
    // distance comes with it.
    s.click(1.0, 1.0, "sketch.line");
    s.move_to(6.0, 1.5, "sketch.line");
    s.key(KeyCode::Key1, Some("sketch.line"));
    s.key(KeyCode::Key2, Some("sketch.line"));
    s.key(KeyCode::Enter, Some("sketch.line"));
    s.key(KeyCode::Escape, Some("sketch.line"));
    // A polyline: a straight segment, then tangent arcs.
    s.click(-12.0, -2.0, "sketch.polyline");
    s.click(-6.0, -2.0, "sketch.polyline");
    s.event(
        WorkbenchInputEvent::Action {
            id: "sketch.polyline_arc".into(),
        },
        Some("sketch.polyline"),
    );
    s.click(-3.0, -5.0, "sketch.polyline");
    s.key(KeyCode::Escape, Some("sketch.polyline"));
    // A spline through four points, finished with Enter.
    for (x, y) in [(-10.0, 6.0), (-7.0, 9.0), (-3.0, 6.0), (0.0, 9.0)] {
        s.click(x, y, "sketch.bspline");
    }
    s.key(KeyCode::Enter, Some("sketch.bspline"));
    s.key(KeyCode::A, None);

    let sketch = s.sketch(id);
    assert!(
        sketch
            .constraints
            .iter()
            .any(|c| matches!(c.kind, wb_sketch::sketch::ConstraintKind::Distance { distance, .. } if (distance - 12.0).abs() < 1e-4)),
        "the typed length is a constraint"
    );
    let done = summary(&sketch);
    assert!(
        done.1.contains(&"arc") && done.1.contains(&"bspline"),
        "{done:?}"
    );
    assert_replays(&s.recorded, before, id, &done);
}

impl Session {
    /// A menu command of the bench, as the Edit menu runs it.
    fn edit_menu(&mut self, id: &str) {
        let active = self.active;
        let mut ctx = WorkbenchRuntimeContext::new(&mut self.doc, CAM_POS, [0.0; 3], VIEWPORT);
        ctx.view_proj = Some(view_proj());
        ctx.active_document_object = active;
        self.wb
            .on_command(id, &core_document::MenuScope::EditMenu, &mut ctx);
        self.recorded.extend(HookOutcome::take(&mut ctx).recorded);
    }
}

/// What a sketch comes to: see [`summary`].
type Summary = (Vec<(i64, i64)>, Vec<&'static str>, Vec<String>);

/// Every sketch of a document by name, with its plane and what it holds.
fn all_sketches(doc: &Document) -> Vec<(String, [i64; 9], Summary)> {
    let r = |v: f32| (v * 1000.0).round() as i64;
    let mut out: Vec<_> = doc
        .feature_tree()
        .all_nodes()
        .filter_map(|(_, n)| {
            let f = SketchFeature::from_json(&n.data).ok()?;
            let p = f.plane;
            let plane = [
                r(p.origin[0]),
                r(p.origin[1]),
                r(p.origin[2]),
                r(p.normal[0]),
                r(p.normal[1]),
                r(p.normal[2]),
                r(p.x_axis[0]),
                r(p.x_axis[1]),
                r(p.x_axis[2]),
            ];
            Some((n.name.clone(), plane, summary(&f.sketch)))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn arrays_clipboard_mirrors_planes_and_clearing_replay_too() {
    let (mut s, _, before) = session_on_a_sketch();
    s.click(2.0, 2.0, "sketch.rect");
    s.click(10.0, 8.0, "sketch.rect");
    s.click(-8.0, 6.0, "sketch.circle");
    s.click(-5.5, 6.0, "sketch.circle");
    s.key(KeyCode::Escape, Some("sketch.select"));
    // The circle, repeated in a two by two array.
    s.click(-5.5, 6.0, "sketch.select");
    s.key(KeyCode::A, Some("sketch.array"));
    s.key(KeyCode::Escape, Some("sketch.select"));
    // The rectangle's bottom edge cut, and pasted where the cursor is.
    s.click(6.0, 2.0, "sketch.select");
    s.edit_menu("edit.cut");
    s.move_to(-12.0, -10.0, "sketch.select");
    s.edit_menu("edit.paste");
    s.key(KeyCode::Escape, Some("sketch.select"));
    // A mirrored copy as a sketch of its own, then this one's plane
    // flipped and every constraint cleared.
    s.key(KeyCode::A, Some("sketch.mirror_sketch"));
    s.key(KeyCode::A, Some("sketch.reorient"));
    s.key(KeyCode::A, Some("sketch.delete_all_constraints"));
    s.key(KeyCode::A, None);

    let kinds: Vec<&str> = s.recorded.iter().map(|r| r.id.as_str()).collect();
    for kind in [
        "sketch.array",
        "sketch.delete",
        "sketch.paste",
        "sketch.mirror_sketch",
        "sketch.set_plane",
    ] {
        assert!(kinds.contains(&kind), "{kind} in {kinds:?}");
    }
    let done = all_sketches(&s.doc);
    assert_eq!(done.len(), 2, "the sketch and its mirror");

    let mut recorder = scripting::Recorder::default();
    for call in &s.recorded {
        recorder.push(call);
    }
    let script = recorder.script("Recorded in a test");
    let mut context = WorkbenchContext::default();
    SketchWorkbench::default().configure(&mut context);
    let mut replay = Replay {
        doc: before,
        wb: SketchWorkbench::default(),
        specs: context.commands().to_vec(),
    };
    let out = scripting::ScriptEngine::new().run_script(&script, "recorded.lua", &mut replay);
    assert_eq!(out.error, None, "{script}");
    assert_eq!(all_sketches(&replay.doc), done, "{script}");
}
