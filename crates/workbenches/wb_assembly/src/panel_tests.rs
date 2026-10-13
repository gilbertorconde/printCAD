//! The Assembly's declared panels: each makes the edits, and records the
//! calls, its fields and buttons stand for.

use super::*;
use crate::test_support::{activate, context};
use crate::tests::{face_up, frame, scene, task_frame};
use core_document::{Document, Recorded, Workbench};

/// The bench with `task` open.
fn with_task(task: Task) -> AssemblyWorkbench {
    AssemblyWorkbench {
        task: Some(task),
        ..AssemblyWorkbench::default()
    }
}

/// The copies task on `part`: one copy 10 mm along X.
fn copies(part: BodyId) -> Task {
    Task::Copies {
        body: part,
        count: 1,
        step: [10.0, 0.0, 0.0],
        around: None,
        mirror: None,
    }
}

/// Hand `event` to the open task's panel, as the renderer hands it; what
/// that closed with, and what it recorded.
fn send(
    wb: &mut AssemblyWorkbench,
    doc: &mut Document,
    event: PanelEvent,
) -> (Option<TaskOutcome>, Vec<Recorded>) {
    let mut ctx = context(doc);
    let outcome = wb.task_event(&mut ctx, &event);
    (outcome, core_document::HookOutcome::take(&mut ctx).recorded)
}

/// The body `event` asks the host to select, if any.
fn selects(wb: &mut AssemblyWorkbench, doc: &mut Document, event: PanelEvent) -> Option<BodyId> {
    let mut ctx = context(doc);
    wb.task_event(&mut ctx, &event);
    core_document::HookOutcome::take(&mut ctx)
        .requests
        .into_iter()
        .find_map(|r| match r {
            core_document::HostRequest::SelectBody(body) => Some(body),
            _ => None,
        })
}

fn select(id: &str, index: usize) -> PanelEvent {
    PanelEvent::Select {
        id: id.into(),
        index,
    }
}

fn number(id: &str, value: f64) -> PanelEvent {
    PanelEvent::Number {
        id: id.into(),
        value,
    }
}

fn button(id: &str) -> PanelEvent {
    PanelEvent::Button { id: id.into() }
}

fn text(id: &str, value: &str) -> PanelEvent {
    PanelEvent::Text {
        id: id.into(),
        value: value.into(),
    }
}

/// Text typed into a table's cell.
fn cell(id: &str, row: usize, column: usize, value: &str) -> PanelEvent {
    PanelEvent::CellText {
        id: id.into(),
        row,
        column,
        value: value.into(),
    }
}

/// A table's check cell set.
fn check(id: &str, row: usize, column: usize, on: bool) -> PanelEvent {
    PanelEvent::CellCheck {
        id: id.into(),
        row,
        column,
        on,
    }
}

fn widgets(wb: &AssemblyWorkbench, doc: &mut Document) -> Vec<Widget> {
    wb.task_widgets(&context(doc))
}

/// Every widget, those in rows, hints and groups too.
fn flat(widgets: &[Widget]) -> Vec<&Widget> {
    let mut all = Vec::new();
    for widget in widgets {
        all.push(widget);
        match widget {
            Widget::Row { children } | Widget::Group { children, .. } => all.extend(flat(children)),
            Widget::Hinted { widget, .. } => all.extend(flat(std::slice::from_ref(widget))),
            _ => {}
        }
    }
    all
}

/// The widget with id `id`.
fn field<'a>(widgets: &'a [Widget], id: &str) -> Option<&'a Widget> {
    flat(widgets).into_iter().find(|w| match w {
        Widget::Number { id: i, .. }
        | Widget::Choice { id: i, .. }
        | Widget::Toggle { id: i, .. }
        | Widget::TextField { id: i, .. }
        | Widget::Button { id: i, .. }
        | Widget::Slider { id: i, .. }
        | Widget::List { id: i, .. }
        | Widget::Table { id: i, .. }
        | Widget::Sheet { id: i, .. } => i == id,
        _ => false,
    })
}

fn value_of(widgets: &[Widget], id: &str) -> f64 {
    match field(widgets, id) {
        Some(Widget::Number { value, .. } | Widget::Slider { value, .. }) => *value,
        other => panic!("{id}: {other:?}"),
    }
}

/// Everything the panel says, in words.
fn words(widgets: &[Widget]) -> String {
    flat(widgets)
        .iter()
        .filter_map(|w| match w {
            Widget::Text { text, .. } | Widget::Heading { text } => Some(text.clone()),
            Widget::Note { title, text, .. } => {
                Some(format!("{} {text}", title.clone().unwrap_or_default()))
            }
            Widget::Value { label, value, .. } => Some(format!("{label}: {value}")),
            Widget::Header { title, .. } => Some(title.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The panel drawn once through the renderer, nothing touched: it draws,
/// records nothing and leaves the task open.
fn draws(wb: &mut AssemblyWorkbench, doc: &mut Document) {
    let recorded = task_frame(wb, doc, TaskRequest::default());
    assert!(recorded.is_empty(), "{recorded:?}");
    assert!(wb.task.is_some() || wb.picking.is_some());
}

fn accept(wb: &mut AssemblyWorkbench, doc: &mut Document) -> Vec<Recorded> {
    task_frame(
        wb,
        doc,
        TaskRequest {
            accept: true,
            cancel: false,
        },
    )
}

fn close_to(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
}

/// Two bodies mated by picks, the joint's task open.
fn mated() -> (AssemblyWorkbench, Document, BodyId, BodyId, FeatureId) {
    let (mut doc, base, part) = scene();
    let mut wb = AssemblyWorkbench::default();
    activate(&mut wb, &mut context(&mut doc), "asm.mate");
    frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
    frame(&mut wb, &mut doc, Some((base, face_up(0.0))));
    let Some(Task::Joint { id, .. }) = wb.task.clone() else {
        panic!("the mate's task is open");
    };
    (wb, doc, base, part, id)
}

#[test]
fn the_move_panel_moves_the_body_as_asm_place_does_and_records_it() {
    let (mut doc, _, part) = scene();
    let mut wb = with_task(Task::Move {
        body: part,
        placements: crate::all_placements(&context(&mut doc)),
    });
    draws(&mut wb, &mut doc);
    let panel = widgets(&wb, &mut doc);
    assert_eq!(value_of(&panel, "x"), 30.0);
    assert_eq!(value_of(&panel, "z"), 40.0);
    let before = doc.clone();

    send(&mut wb, &mut doc, number("x", 12.0));
    send(&mut wb, &mut doc, number("turn_z", 90.0));
    let mut by_hand = before;
    crate::components::move_with_unit(
        &mut by_hand,
        part,
        BodyPlacement::new(
            glam::Quat::from_euler(glam::EulerRot::XYZ, 0.0, 0.0, 90f32.to_radians()),
            glam::Vec3::new(12.0, 0.0, 40.0),
        ),
    );
    let (moved, want) = (doc.body_placement(part), by_hand.body_placement(part));
    assert!(close_to(moved.translation, want.translation), "{moved:?}");
    assert!(moved.quat().angle_between(want.quat()) < 1e-4, "{moved:?}");
    assert_eq!(value_of(&widgets(&wb, &mut doc), "turn_z").round(), 90.0);

    let recorded = accept(&mut wb, &mut doc);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].id, "asm.place");
    assert_eq!(
        recorded[0].args["translation"],
        serde_json::json!(moved.translation)
    );

    wb.task = Some(Task::Move {
        body: part,
        placements: Vec::new(),
    });
    send(&mut wb, &mut doc, button("home"));
    assert_eq!(doc.body_placement(part), BodyPlacement::IDENTITY);
}

#[test]
fn the_picking_prompt_offers_the_origin_and_takes_its_plane_as_the_second_face() {
    let (mut doc, _, part) = scene();
    let mut wb = AssemblyWorkbench::default();
    activate(&mut wb, &mut context(&mut doc), "asm.mate");
    let panel = widgets(&wb, &mut doc);
    assert!(words(&panel).contains("The first body moves"));
    assert!(field(&panel, "origin:0").is_none(), "nothing to pair yet");
    draws(&mut wb, &mut doc);

    frame(&mut wb, &mut doc, Some((part, face_up(40.0))));
    let panel = widgets(&wb, &mut doc);
    let Some(Widget::Button { label, .. }) = field(&panel, "origin:0") else {
        panic!("{panel:?}");
    };
    assert_eq!(label, crate::ORIGIN[0].0);
    draws(&mut wb, &mut doc);

    send(&mut wb, &mut doc, button("origin:0"));
    assert!(wb.picking.is_none());
    let Some(Task::Joint { id, .. }) = wb.task.clone() else {
        panic!("the mate's task is open");
    };
    let joint = crate::joints(&doc)
        .into_iter()
        .find(|j| j.id == id)
        .unwrap();
    assert_eq!(joint.feature.other_body, crate::WORLD);
    assert_eq!(joint.body, part);
    let placed = doc.body_placement(part);
    assert!(
        placed.point([2.0, 2.0, 0.0])[2].abs() < 1e-3,
        "the part's face on the XY plane: {placed:?}"
    );
}

#[test]
fn a_body_its_joints_place_shows_its_place_without_fields() {
    let (mut wb, mut doc, _, part, _) = mated();
    wb.task = Some(Task::Move {
        body: part,
        placements: Vec::new(),
    });
    let panel = widgets(&wb, &mut doc);
    assert!(field(&panel, "x").is_none());
    assert!(field(&panel, "home").is_none());
    assert!(
        field(&panel, "handles").is_none(),
        "its handles do not show"
    );
    assert!(words(&panel).contains("Placed by its joints"));
    assert!(words(&panel).contains("Position X: "));
    draws(&mut wb, &mut doc);
}

/// The Move panel's choice switches the handles between moving and turning,
/// recording nothing: it changes what the view shows, not the document.
#[test]
fn the_move_panel_switches_its_handles_between_moving_and_turning() {
    let (mut doc, _, part) = scene();
    let mut wb = with_task(Task::Move {
        body: part,
        placements: Vec::new(),
    });
    let selected =
        |wb: &AssemblyWorkbench, doc: &mut Document| match field(&widgets(wb, doc), "handles") {
            Some(Widget::Choice {
                options, selected, ..
            }) => (options.clone(), *selected),
            other => panic!("{other:?}"),
        };
    assert_eq!(
        selected(&wb, &mut doc),
        (vec!["Move".into(), "Turn".into()], 0)
    );
    let before = doc.body_placement(part);
    let (outcome, recorded) = send(&mut wb, &mut doc, choose("handles", 1));
    assert!(outcome.is_none() && recorded.is_empty());
    assert_eq!(selected(&wb, &mut doc).1, 1);
    assert_eq!(doc.body_placement(part), before);
    draws(&mut wb, &mut doc);
}

fn toggle(id: &str, on: bool) -> PanelEvent {
    PanelEvent::Toggle { id: id.into(), on }
}

fn choose(id: &str, index: usize) -> PanelEvent {
    PanelEvent::Choice {
        id: id.into(),
        index,
    }
}

/// Run a command on `doc` as a script would.
fn run(doc: &mut Document, id: &str, args: serde_json::Value) -> serde_json::Value {
    AssemblyWorkbench::default()
        .run_command(id, &crate::commands::object(args), &mut context(doc))
        .unwrap()
}

/// The bodies of `doc` not in `before`, and where each sits.
fn new_bodies(doc: &Document, before: &Document) -> Vec<BodyPlacement> {
    doc.bodies()
        .iter()
        .filter(|b| !before.bodies().iter().any(|o| o.id == b.id))
        .map(|b| b.placement)
        .collect()
}

#[test]
fn copies_turned_about_an_axis_are_inserted_and_recorded_as_asm_copy() {
    let (mut doc, _, part) = scene();
    let before = doc.clone();
    let mut wb = with_task(copies(part));
    draws(&mut wb, &mut doc);
    assert!(field(&widgets(&wb, &mut doc), "step_0").is_some());
    send(&mut wb, &mut doc, number("count", 3.0));
    send(&mut wb, &mut doc, toggle("around", true));
    send(&mut wb, &mut doc, choose("axis", 0));
    send(&mut wb, &mut doc, number("through_1", 5.0));
    send(&mut wb, &mut doc, number("over", 180.0));
    let panel = widgets(&wb, &mut doc);
    assert!(field(&panel, "step_0").is_none());
    assert_eq!(value_of(&panel, "over"), 180.0);
    draws(&mut wb, &mut doc);

    let recorded = accept(&mut wb, &mut doc);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].id, "asm.copy");
    let args = &recorded[0].args;
    assert_eq!(args["count"], serde_json::json!(3));
    assert_eq!(
        args["around"]["direction"],
        serde_json::json!([1.0, 0.0, 0.0])
    );
    assert_eq!(args["around"]["point"], serde_json::json!([0.0, 5.0, 0.0]));
    assert_eq!(args["around"]["angle"], serde_json::json!(180.0));

    let mut by_command = before.clone();
    run(
        &mut by_command,
        "asm.copy",
        serde_json::Value::Object(args.clone()),
    );
    let (made, want) = (new_bodies(&doc, &before), new_bodies(&by_command, &before));
    assert_eq!(made.len(), 3);
    assert_eq!(made, want);
    assert!(wb.task.is_none());
}

#[test]
fn a_mirror_image_across_a_plane_is_recorded_as_asm_mirror() {
    let (mut doc, _, part) = scene();
    let mut wb = with_task(copies(part));
    send(&mut wb, &mut doc, toggle("mirror", true));
    send(&mut wb, &mut doc, choose("plane", 2));
    send(&mut wb, &mut doc, number("mirror_2", 5.0));
    let panel = widgets(&wb, &mut doc);
    assert!(field(&panel, "around").is_none());
    assert!(field(&panel, "count").is_some());
    draws(&mut wb, &mut doc);
    let recorded = accept(&mut wb, &mut doc);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].id, "asm.mirror");
    assert_eq!(
        recorded[0].args["normal"],
        serde_json::json!([0.0, 0.0, 1.0])
    );
    assert_eq!(
        recorded[0].args["point"],
        serde_json::json!([0.0, 0.0, 5.0])
    );
    let copy = recorded[0].result.as_str().unwrap();
    assert!(doc.bodies().iter().any(|b| b.id.0.to_string() == copy));
}

#[test]
fn the_replace_panel_names_both_bodies() {
    let (mut doc, base, part) = scene();
    let mut wb = with_task(Task::Replace {
        old: part,
        new: None,
    });
    let said = words(&widgets(&wb, &mut doc));
    assert!(said.contains("Replace: Part"), "{said}");
    assert!(said.contains("With: click a body"), "{said}");
    wb.task = Some(Task::Replace {
        old: part,
        new: Some(base),
    });
    assert!(words(&widgets(&wb, &mut doc)).contains("With: Base"));
    draws(&mut wb, &mut doc);
}

#[test]
fn a_group_takes_bodies_out_and_is_dissolved_as_doc_delete() {
    let (mut doc, base, part) = scene();
    let mut wb = with_task(Task::Group {
        editing: None,
        members: vec![base, part],
    });
    let panel = widgets(&wb, &mut doc);
    assert!(field(&panel, "remove:1").is_some());
    assert!(
        field(&panel, "dissolve").is_none(),
        "nothing to dissolve yet"
    );
    draws(&mut wb, &mut doc);
    send(&mut wb, &mut doc, button("remove:0"));
    let Some(Task::Group { members, .. }) = &wb.task else {
        panic!("still picking");
    };
    assert_eq!(members, &vec![part]);
    assert!(words(&widgets(&wb, &mut doc)).contains("two bodies or more"));

    let group = run(
        &mut doc,
        "asm.group",
        serde_json::json!({"bodies": [base.0.to_string(), part.0.to_string()]}),
    );
    let group = FeatureId(uuid::Uuid::parse_str(group.as_str().unwrap()).unwrap());
    wb.task = Some(Task::Group {
        editing: Some(group),
        members: vec![base, part],
    });
    draws(&mut wb, &mut doc);
    let (outcome, recorded) = send(&mut wb, &mut doc, button("dissolve"));
    assert!(matches!(outcome, Some(TaskOutcome::Accepted { .. })));
    assert_eq!(recorded[0].id, "doc.delete");
    assert!(doc.get_feature_meta(group).is_none());
    assert!(wb.task.is_none());
}

#[test]
fn an_interference_check_lists_its_clashes_and_a_click_selects_the_first_body() {
    let (mut doc, base, part) = scene();
    let mut wb = with_task(Task::Interference {
        found: None,
        seq: doc.mutation_seq(),
        around: Some(part),
        clearance: None,
    });
    let panel = widgets(&wb, &mut doc);
    assert!(matches!(
        flat(&panel)[1],
        Widget::Progress {
            fraction: Some(_),
            ..
        }
    ));
    assert!(field(&panel, "stop").is_some());
    draws(&mut wb, &mut doc);

    wb.task = Some(Task::Interference {
        found: Some(crate::Interference {
            clashes: vec![crate::Clash {
                a: part,
                b: base,
                volume_mm3: 2.5,
                centre: [0.0; 3],
                mesh: std::sync::Arc::new(core_document::TriMesh::default()),
            }],
            checked: 2,
            skipped: 1,
            ..crate::Interference::default()
        }),
        seq: doc.mutation_seq(),
        around: Some(part),
        clearance: None,
    });
    let panel = widgets(&wb, &mut doc);
    let said = words(&panel);
    assert!(said.contains("1 clash"), "{said}");
    assert!(said.contains("1 visible body without a solid"), "{said}");
    assert!(said.contains("Part against every other body"), "{said}");
    let Some(Widget::List { items, .. }) = field(&panel, "clashes") else {
        panic!("{panel:?}");
    };
    assert_eq!(items[0].label, "Part and Base: 2.50 mm³");
    assert_eq!(value_of(&panel, "clearance"), 0.5);
    assert!(field(&panel, "every_pair").is_some());
    draws(&mut wb, &mut doc);

    assert_eq!(selects(&mut wb, &mut doc, select("clashes", 0)), Some(part));
    send(&mut wb, &mut doc, number("clearance", 2.0));
    assert_eq!(wb.clearance_mm, Some(2.0));
    assert_eq!(value_of(&widgets(&wb, &mut doc), "clearance"), 2.0);
}

#[test]
fn the_mass_panel_weighs_at_the_density_typed_and_selects_a_body_clicked() {
    let (mut doc, base, part) = scene();
    let mut wb = with_task(Task::Mass {
        found: Some(crate::MassReport {
            bodies: vec![
                crate::BodyMass {
                    body: base,
                    volume_mm3: 1000.0,
                    centre: [0.0; 3],
                    density: None,
                    inertia: Some(kernel_api::Inertia::default()),
                    bought: false,
                },
                crate::BodyMass {
                    body: part,
                    volume_mm3: 1000.0,
                    centre: [10.0, 0.0, 0.0],
                    density: None,
                    inertia: Some(kernel_api::Inertia::default()),
                    bought: true,
                },
            ],
            skipped: 0,
            stopped: false,
        }),
        density: 1.0,
    });
    let said = words(&widgets(&wb, &mut doc));
    assert!(said.contains("Mass: 2.00 g"), "{said}");
    // Two point masses 10 mm apart: 1 g · 5² each about Y and Z.
    assert!(said.contains("I3: 50.00 g·mm² about"), "{said}");
    draws(&mut wb, &mut doc);
    send(&mut wb, &mut doc, number("density", 2.5));
    let panel = widgets(&wb, &mut doc);
    assert!(words(&panel).contains("Mass: 5.00 g"));
    let Some(Widget::Table { rows, .. }) = field(&panel, "bodies") else {
        panic!("{panel:?}");
    };
    assert_eq!(
        rows[1],
        vec!["Part (bought)".to_string(), "2.50 g".to_string()]
    );
    assert_eq!(selects(&mut wb, &mut doc, select("bodies", 1)), Some(part));
}

#[test]
fn the_exploded_view_spreads_bodies_and_keeps_its_steps_as_asm_exploded_view() {
    let (mut doc, base, part) = scene();
    let placements = crate::all_placements(&context(&mut doc));
    let mut wb = with_task(Task::Explode {
        placements: placements.clone(),
        spread: 0.0,
        steps: Box::default(),
    });
    draws(&mut wb, &mut doc);
    let mut by_hand = doc.clone();
    crate::explode(&mut context(&mut by_hand), &placements, 1.5);
    send(&mut wb, &mut doc, number("spread", 1.5));
    assert_eq!(doc.body_placement(part), by_hand.body_placement(part));
    assert_eq!(doc.body_placement(base), by_hand.body_placement(base));
    assert_eq!(value_of(&widgets(&wb, &mut doc), "spread"), 1.5);

    // Bodies clicked: the panel turns to the steps.
    if let Some(Task::Explode { steps, .. }) = &mut wb.task {
        steps.picked = vec![part];
    }
    let panel = widgets(&wb, &mut doc);
    assert!(field(&panel, "spread").is_none());
    assert!(field(&panel, "add_step").is_some());
    draws(&mut wb, &mut doc);
    send(&mut wb, &mut doc, number("shift_2", 10.0));
    let (_, recorded) = send(&mut wb, &mut doc, button("add_step"));
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].id, "asm.exploded_view");
    let Some(Task::Explode { steps, .. }) = &wb.task else {
        panic!("still open");
    };
    let view = steps.view.expect("the view is kept");
    assert_eq!(recorded[0].result, serde_json::json!(view.0.to_string()));
    let kept = crate::exploded::view_of(&doc, view).unwrap();
    assert_eq!(kept.steps.len(), 1);
    assert_eq!(kept.steps[0].bodies, vec![part]);
    assert_eq!(kept.steps[0].shift, [0.0, 0.0, 10.0]);
    // Played to its end: the part shifted, the base where it sat.
    assert!(close_to(
        doc.body_placement(part).translation,
        [30.0, 0.0, 50.0]
    ));
    assert_eq!(doc.body_placement(base), placements[0].1);
    assert_eq!(value_of(&widgets(&wb, &mut doc), "at"), 1.0);

    send(&mut wb, &mut doc, number("at", 0.0));
    assert!(close_to(
        doc.body_placement(part).translation,
        [30.0, 0.0, 40.0]
    ));
    send(&mut wb, &mut doc, button("play"));
    assert!(matches!(&wb.task, Some(Task::Explode { steps, .. }) if steps.playing));

    let (_, recorded) = send(&mut wb, &mut doc, button("remove_step:0"));
    assert_eq!(recorded[0].args["steps"], serde_json::json!([]));
    assert!(
        crate::exploded::view_of(&doc, view)
            .unwrap()
            .steps
            .is_empty()
    );
    draws(&mut wb, &mut doc);
}

/// Every text the task panel draws in a panel `width` points wide, with
/// how many lines each took.
fn drawn_texts(wb: &mut AssemblyWorkbench, doc: &mut Document, width: f32) -> Vec<(String, usize)> {
    let egui_ctx = egui::Context::default();
    ui_kit::apply_theme(&egui_ctx);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, 900.0),
        )),
        ..Default::default()
    };
    let mut output = egui_ctx.run_ui(input, |ui| {
        let mut ctx = context(doc);
        wb.ui_task_panel(ui, &mut ctx, TaskRequest::default());
    });
    output.textures_delta.clear();
    fn texts(shape: &egui::Shape, out: &mut Vec<(String, usize)>) {
        match shape {
            egui::Shape::Text(text) => {
                out.push((text.galley.text().to_string(), text.galley.rows.len()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &output.shapes {
        texts(&clipped.shape, &mut out);
    }
    out
}

#[test]
fn an_exploded_step_says_what_it_does_and_its_remove_button_keeps_one_line() {
    let (mut doc, base, part) = scene();
    let placements = crate::all_placements(&context(&mut doc));
    let mut wb = with_task(Task::Explode {
        placements,
        spread: 0.0,
        steps: Box::default(),
    });
    for bodies in [vec![part], vec![base, part]] {
        if let Some(Task::Explode { steps, .. }) = &mut wb.task {
            steps.picked = bodies;
        }
        send(&mut wb, &mut doc, number("shift_2", 10.0));
        send(&mut wb, &mut doc, choose("turn_axis", 2));
        send(&mut wb, &mut doc, number("turn_degrees", 90.0));
        send(&mut wb, &mut doc, number("radial", 5.0));
        send(&mut wb, &mut doc, button("add_step"));
    }
    let said = words(&widgets(&wb, &mut doc));
    assert!(
        said.contains("2. Base, Part: turn 90° about Z, spread 5 mm, shift (0, 0, 10) mm"),
        "{said}"
    );
    // Whatever the panel's width, a step's line wraps and its button
    // keeps its label on one line.
    for width in (240..=480).step_by(4) {
        let drawn = drawn_texts(&mut wb, &mut doc, width as f32);
        let removes: Vec<_> = drawn.iter().filter(|(t, _)| t == "Remove").collect();
        assert_eq!(removes.len(), 2, "{width}: {drawn:?}");
        assert!(
            removes.iter().all(|(_, lines)| *lines == 1),
            "{width}: {drawn:?}"
        );
    }
}

#[test]
fn the_parts_list_keeps_its_cells_through_asm_parts_table() {
    let (mut doc, _, _) = scene();
    let mut wb = with_task(Task::Parts);
    draws(&mut wb, &mut doc);
    let parts = crate::parts_list(&doc, &[]);
    let Some(Widget::Sheet { rows, columns, .. }) =
        field(&widgets(&wb, &mut doc), "parts").cloned()
    else {
        panic!("the list is a sheet");
    };
    assert_eq!(rows.len(), parts.len());
    assert_eq!(columns.len(), 9);

    send(&mut wb, &mut doc, text("new_column", "Maker"));
    let (_, recorded) = send(&mut wb, &mut doc, button("add_column"));
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].id, "asm.parts_table");
    assert!(wb.parts_column.is_empty());
    let Some(Widget::Sheet { columns, .. }) = field(&widgets(&wb, &mut doc), "parts").cloned()
    else {
        panic!("the list is a sheet");
    };
    assert_eq!(columns.last().map(String::as_str), Some("Maker"));

    send(&mut wb, &mut doc, cell("parts", 0, 9, "ACME"));
    send(&mut wb, &mut doc, check("parts", 0, 7, true));
    assert_eq!(
        crate::parts_list(&doc, &[])[0].print,
        0,
        "a bought part prints none"
    );
    send(&mut wb, &mut doc, cell("parts", 0, 3, "5"));
    assert_eq!(crate::parts_list(&doc, &[])[0].print, 5);
    send(&mut wb, &mut doc, choose("print_material", 1));
    let (_, table) = crate::parts::table_of(&doc).unwrap();
    assert_eq!(table.material.name, "PETG");
    assert!(field(&widgets(&wb, &mut doc), "print_density").is_none());
    send(
        &mut wb,
        &mut doc,
        choose("print_material", crate::parts::PRINT_MATERIALS.len()),
    );
    send(&mut wb, &mut doc, number("print_density", 1.3));
    let (_, table) = crate::parts::table_of(&doc).unwrap();
    assert_eq!(
        (table.material.name.as_str(), table.material.density),
        ("Custom", 1.3)
    );
    send(&mut wb, &mut doc, button("number"));
    let first = &crate::parts_list(&doc, &[])[0];
    assert_eq!(first.values.get("Maker").map(String::as_str), Some("ACME"));
    assert!(first.bought);
    assert_eq!(first.number, Some(1));
    // Numbered already: nothing changes, nothing is recorded.
    let (_, recorded) = send(&mut wb, &mut doc, button("number"));
    assert!(recorded.is_empty(), "nothing changed, nothing recorded");
    assert_eq!(
        selects(&mut wb, &mut doc, select("parts", 0)),
        Some(first.bodies[0])
    );

    send(&mut wb, &mut doc, button("copy"));
    assert!(wb.copied.as_deref().is_some_and(|csv| csv.contains("ACME")));
    draws(&mut wb, &mut doc);
    assert!(wb.copied.is_none(), "drawn onto the clipboard");

    send(&mut wb, &mut doc, button("remove_column:0"));
    assert!(crate::parts_list(&doc, &[])[0].values.is_empty());
}

/// Two hinges on a base, the second geared to the first.
fn geared() -> (Document, FeatureId, FeatureId) {
    let mut doc = Document::new("t");
    let [base, g1, g2] = [
        doc.create_body(None),
        doc.create_body(None),
        doc.create_body(None),
    ];
    let pin = |x: f32| serde_json::json!({"axis": {"point": [x, 0, 0], "direction": [0, 0, 1]}});
    let mut hinge = |body: BodyId, x: f32| {
        run(
            &mut doc,
            "asm.hinge",
            serde_json::json!({"body": body.0.to_string(), "face": pin(x),
                               "other": base.0.to_string(), "other_face": pin(x)}),
        )
    };
    let (h1, h2) = (hinge(g1, -20.0), hinge(g2, 10.0));
    let coupling = run(
        &mut doc,
        "asm.couple",
        serde_json::json!({"driver": h1, "driven": h2, "gearing": "gears", "ratio": 2}),
    );
    let id = |v: serde_json::Value| FeatureId(uuid::Uuid::parse_str(v.as_str().unwrap()).unwrap());
    (doc, id(coupling), id(h1))
}

#[test]
fn a_coupling_s_fields_change_it_as_asm_set_does_and_record_the_change() {
    let (mut doc, coupling, _) = geared();
    let before = doc.clone();
    let mut wb = with_task(Task::Coupling {
        id: coupling,
        before: doc.get_feature_data(coupling).cloned(),
        placements: crate::all_placements(&context(&mut doc)),
    });
    draws(&mut wb, &mut doc);
    let panel = widgets(&wb, &mut doc);
    assert_eq!(value_of(&panel, "ratio"), 2.0);
    let Some(Widget::Number {
        bind: Some(bind), ..
    }) = field(&panel, "ratio")
    else {
        panic!("the ratio takes formulas");
    };
    assert_eq!(bind.key, "/ratio");
    let Some(Widget::Choice { options, .. }) = field(&panel, "gearing") else {
        panic!("{panel:?}");
    };
    let belt = options
        .iter()
        .position(|o| o == Gearing::Belt.label())
        .expect("two hinges may take a belt");
    assert!(
        !options.iter().any(|o| o == Gearing::RackAndPinion.label()),
        "a rack needs a slider"
    );

    send(&mut wb, &mut doc, number("ratio", 3.0));
    send(&mut wb, &mut doc, toggle("reverse", true));
    send(&mut wb, &mut doc, choose("gearing", belt));
    let mut by_command = before.clone();
    run(
        &mut by_command,
        "asm.set",
        serde_json::json!({"joint": coupling.0.to_string(), "ratio": 3.0, "reverse": true,
                           "gearing": "belt"}),
    );
    assert_eq!(
        doc.get_feature_data(coupling),
        by_command.get_feature_data(coupling)
    );
    draws(&mut wb, &mut doc);

    let recorded = accept(&mut wb, &mut doc);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].id, "asm.set");
    let mut replay = before;
    run(
        &mut replay,
        "asm.set",
        serde_json::Value::Object(recorded[0].args.clone()),
    );
    assert_eq!(
        replay.get_feature_data(coupling),
        doc.get_feature_data(coupling)
    );
}

fn joint_of(doc: &Document, id: FeatureId) -> JointFeature {
    JointFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap()
}

/// A joint's settings task open on an existing joint.
fn editing(doc: &mut Document, id: FeatureId) -> AssemblyWorkbench {
    AssemblyWorkbench {
        task: Some(Task::Joint {
            id,
            before: doc.get_feature_data(id).cloned(),
            placements: crate::all_placements(&context(doc)),
        }),
        ..AssemblyWorkbench::default()
    }
}

#[test]
fn a_new_mate_s_gap_moves_its_body_as_asm_set_does_and_records_as_the_mate() {
    let (mut wb, mut doc, _, part, id) = mated();
    let before = doc.clone();
    let panel = widgets(&wb, &mut doc);
    let Some(Widget::Number {
        bind: Some(bind), ..
    }) = field(&panel, "offset")
    else {
        panic!("the gap takes formulas: {panel:?}");
    };
    assert_eq!(bind.key, "/kind/Mate/offset");
    assert!(field(&panel, "flip").is_some());
    assert!(
        field(&panel, "drive.drive").is_none(),
        "a mate has no drive"
    );
    draws(&mut wb, &mut doc);

    send(&mut wb, &mut doc, number("offset", 5.0));
    let mut by_command = before.clone();
    run(
        &mut by_command,
        "asm.set",
        serde_json::json!({"joint": id.0.to_string(), "offset": 5.0}),
    );
    assert_eq!(doc.get_feature_data(id), by_command.get_feature_data(id));
    assert_eq!(doc.body_placement(part), by_command.body_placement(part));
    assert!(matches!(&wb.verdict, Some(Ok(_))), "{:?}", wb.verdict);
    assert!(words(&widgets(&wb, &mut doc)).contains("every joint holds"));

    let recorded = accept(&mut wb, &mut doc);
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0].id, "asm.mate");
    assert_eq!(recorded[0].args["offset"], serde_json::json!(5.0));
}

#[test]
fn an_edited_joint_records_what_its_panel_changed_as_asm_set() {
    let (_, mut doc, _, part, id) = mated();
    let before = doc.clone();
    let mut wb = editing(&mut doc, id);
    send(&mut wb, &mut doc, toggle("flip", true));
    send(&mut wb, &mut doc, number("moving_end", 2.0));
    let recorded = accept(&mut wb, &mut doc);
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0].id, "asm.set");
    assert_eq!(recorded[0].args["flip"], serde_json::json!(true));
    assert_eq!(recorded[0].args["moving_end"], serde_json::json!(2.0));
    let mut replay = before;
    run(
        &mut replay,
        "asm.set",
        serde_json::Value::Object(recorded[0].args.clone()),
    );
    assert_eq!(replay.get_feature_data(id), doc.get_feature_data(id));
    let (a, b) = (replay.body_placement(part), doc.body_placement(part));
    assert!(close_to(a.translation, b.translation), "{a:?} {b:?}");
    assert!(a.quat().angle_between(b.quat()) < 1e-4, "{a:?} {b:?}");
}

#[test]
fn a_joint_s_kind_changes_as_asm_set_with_a_kind_does() {
    let (mut wb, mut doc, _, _, id) = mated();
    let distance = JointTool::ALL
        .iter()
        .position(|t| *t == JointTool::Distance)
        .unwrap();
    let (outcome, recorded) = send(&mut wb, &mut doc, choose("kind", distance));
    assert!(matches!(outcome, Some(TaskOutcome::Open)));
    assert!(matches!(
        joint_of(&doc, id).kind,
        JointKind::Distance { .. }
    ));
    let ids: Vec<&str> = recorded.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["asm.mate", "asm.set"]);
    assert_eq!(recorded[1].args["kind"], serde_json::json!("distance"));
    assert!(field(&widgets(&wb, &mut doc), "offset").is_some());
    draws(&mut wb, &mut doc);

    let (outcome, _) = send(&mut wb, &mut doc, button("repick"));
    assert!(matches!(outcome, Some(TaskOutcome::Open)));
    assert!(wb.picking.is_some(), "the faces are picked afresh");
}

#[test]
fn a_hinge_s_drive_holds_limits_sweeps_and_records_as_asm_set_does() {
    let (mut doc, _, hinge) = geared();
    let before = doc.clone();
    let mut wb = editing(&mut doc, hinge);
    let panel = widgets(&wb, &mut doc);
    assert!(words(&panel).contains("Angle now: "));
    assert!(field(&panel, "drive.to").is_none());
    assert!(field(&panel, "check_sweep").is_some());
    draws(&mut wb, &mut doc);

    send(&mut wb, &mut doc, toggle("drive.drive", true));
    let panel = widgets(&wb, &mut doc);
    let Some(Widget::Number {
        bind: Some(bind), ..
    }) = field(&panel, "drive.to")
    else {
        panic!("a held drive takes formulas: {panel:?}");
    };
    assert_eq!(bind.key, "/kind/Hinge/drive/to");
    assert!(field(&panel, "drive.play").is_some());
    assert!(field(&panel, "drive.record").is_some());
    send(&mut wb, &mut doc, number("drive.to", 30.0));
    send(&mut wb, &mut doc, toggle("drive.limits", true));
    send(&mut wb, &mut doc, number("drive.high", -40.0));
    // The limits start 45° either side of where the hinge stands, and an
    // end typed past the other swaps them.
    let JointKind::Hinge { drive, .. } = joint_of(&doc, hinge).kind else {
        panic!("a hinge");
    };
    let [low, high] = drive.limits.expect("limited");
    assert_eq!(low, -40.0);
    assert!((high + 15.0).abs() < 1e-3, "{high}");
    let mut by_command = before.clone();
    run(
        &mut by_command,
        "asm.set",
        serde_json::json!({"joint": hinge.0.to_string(), "drive": 30.0,
                           "limits": [low, high]}),
    );
    assert_eq!(
        doc.get_feature_data(hinge),
        by_command.get_feature_data(hinge)
    );
    draws(&mut wb, &mut doc);

    // Played, the drive sweeps away from where it was held, and stopping
    // puts it back.
    send(&mut wb, &mut doc, toggle("drive.limits", false));
    send(&mut wb, &mut doc, button("drive.play"));
    assert!(
        wb.playing
            .is_some_and(|p| p.joint == hinge && p.start == 30.0)
    );
    assert!(wb.advance_play(&mut context(&mut doc), hinge, 0.5));
    let JointKind::Hinge { drive, .. } = joint_of(&doc, hinge).kind else {
        panic!("a hinge");
    };
    assert!(
        drive.to.is_some_and(|to| (to - 30.0).abs() > 1.0),
        "{drive:?}"
    );
    send(&mut wb, &mut doc, button("drive.play"));
    assert!(wb.playing.is_none());
    let JointKind::Hinge { drive, .. } = joint_of(&doc, hinge).kind else {
        panic!("a hinge");
    };
    assert_eq!(drive.to, Some(30.0));

    let mut ctx = context(&mut doc);
    wb.task_event(&mut ctx, &button("drive.record"));
    let requests = core_document::HookOutcome::take(&mut ctx).requests;
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, core_document::HostRequest::RecordAnimation { .. }))
    );

    send(&mut wb, &mut doc, toggle("drive.drive", false));
    let JointKind::Hinge { drive, .. } = joint_of(&doc, hinge).kind else {
        panic!("a hinge");
    };
    assert_eq!(drive.to, None);
    let recorded = accept(&mut wb, &mut doc);
    assert!(recorded.is_empty(), "back as it was: {recorded:?}");
}

#[test]
fn a_joint_s_turn_and_delete_record_their_commands() {
    let (_, mut doc, _, _, id) = mated();
    let mut wb = editing(&mut doc, id);
    send(&mut wb, &mut doc, number("turn_by", 45.0));
    assert_eq!(wb.turn_by, Some(45.0));
    let (_, recorded) = send(&mut wb, &mut doc, button("turn"));
    assert_eq!(recorded[0].id, "asm.turn");
    assert_eq!(recorded[0].args["degrees"], serde_json::json!(45.0));
    let (_, recorded) = send(&mut wb, &mut doc, button("turn_over"));
    assert_eq!(recorded[0].id, "asm.flip");

    let (outcome, recorded) = send(&mut wb, &mut doc, button("delete"));
    assert!(matches!(outcome, Some(TaskOutcome::Accepted { .. })));
    assert_eq!(recorded[0].id, "doc.delete");
    assert!(doc.get_feature_meta(id).is_none());
}

#[test]
fn a_joint_s_quarter_turn_is_asm_turn90_and_offered_where_it_keeps_a_turn() {
    let (_, mut doc, _, _, mate) = mated();
    let wb = editing(&mut doc, mate);
    assert!(
        field(&widgets(&wb, &mut doc), "turn90").is_none(),
        "a mate keeps no turn of its own"
    );

    let (mut doc, _, hinge) = geared();
    let before = doc.clone();
    let mut wb = editing(&mut doc, hinge);
    assert!(field(&widgets(&wb, &mut doc), "turn90").is_some());
    draws(&mut wb, &mut doc);
    let (_, recorded) = send(&mut wb, &mut doc, button("turn90"));
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0].id, "asm.turn90");
    let mut by_command = before;
    run(
        &mut by_command,
        "asm.turn90",
        serde_json::Value::Object(recorded[0].args.clone()),
    );
    assert_eq!(
        doc.get_feature_data(hinge),
        by_command.get_feature_data(hinge)
    );
    for body in doc.bodies().iter().map(|b| b.id) {
        let (a, b) = (doc.body_placement(body), by_command.body_placement(body));
        assert!(close_to(a.translation, b.translation), "{a:?} {b:?}");
        assert!(a.quat().angle_between(b.quat()) < 1e-4, "{a:?} {b:?}");
    }
}

/// The joint panel lists the joints left apart and the redundant ones as
/// `asm.unmet` and `asm.redundant` name them, and a click selects one.
#[test]
fn the_solver_messages_name_unmet_and_redundant_joints_and_select_them() {
    let (_, mut doc, _, part, mate) = mated();
    let wb = editing(&mut doc, mate);
    assert!(
        field(&widgets(&wb, &mut doc), "solver_messages").is_none(),
        "every joint holds"
    );

    let mut extra = joint_of(&doc, mate);
    extra.kind = JointKind::Parallel;
    doc.add_feature_in_body(extra.clone(), "Extra".into(), Some(part))
        .unwrap();
    extra.kind = JointKind::Mate {
        flip: false,
        offset: 7.0,
    };
    let clash = doc
        .add_feature_in_body(extra, "Clash".into(), Some(part))
        .unwrap();
    let mut wb = editing(&mut doc, mate);
    let panel = widgets(&wb, &mut doc);
    let Some(Widget::List { items, .. }) = field(&panel, "solver_messages") else {
        panic!("a list of messages: {panel:?}");
    };
    let names: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    let unmet = run(&mut doc, "asm.unmet", serde_json::json!({}));
    let redundant = run(&mut doc, "asm.redundant", serde_json::json!({}));
    let named: Vec<&str> = unmet
        .as_array()
        .unwrap()
        .iter()
        .chain(redundant.as_array().unwrap())
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, named);
    assert!(names.contains(&"Clash"), "{names:?}");
    assert!(names.contains(&"Extra"), "{names:?}");
    draws(&mut wb, &mut doc);

    let at = names.iter().position(|n| *n == "Clash").unwrap();
    let mut ctx = context(&mut doc);
    wb.task_event(&mut ctx, &select("solver_messages", at));
    let requests = core_document::HookOutcome::take(&mut ctx).requests;
    assert!(
        requests.iter().any(|r| matches!(
            r,
            core_document::HostRequest::SelectFeatures(f) if f == &[clash]
        )),
        "{requests:?}"
    );
}

fn studying(wb: &AssemblyWorkbench) -> &crate::Studying {
    match &wb.task {
        Some(Task::Motion(studying)) => studying,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_motion_is_worked_out_kept_as_asm_motion_and_scrubbed() {
    let (mut doc, _, hinge) = geared();
    let mut wb = AssemblyWorkbench::default();
    activate(&mut wb, &mut context(&mut doc), "asm.motion");
    let panel = widgets(&wb, &mut doc);
    let Some(Widget::Choice { options, .. }) = field(&panel, "add_drive") else {
        panic!("{panel:?}");
    };
    assert_eq!(options.len(), 3, "a placeholder and the two hinges");
    assert!(field(&panel, "work_out").is_none(), "nothing driven yet");
    draws(&mut wb, &mut doc);

    send(&mut wb, &mut doc, choose("add_drive", 1));
    assert_eq!(studying(&wb).draft.drives[0].joint, hinge);
    assert_eq!(studying(&wb).draft.drives[0].formula, "90 * t");
    send(&mut wb, &mut doc, text("formula:0", "45 * t"));
    send(&mut wb, &mut doc, number("end", 1.0));
    send(&mut wb, &mut doc, number("step", 0.25));
    let (_, recorded) = send(&mut wb, &mut doc, button("work_out"));
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].id, "asm.motion");
    let study = studying(&wb).study.expect("kept in the document");
    assert_eq!(recorded[0].result, serde_json::json!(study.0.to_string()));
    assert_eq!(recorded[0].args["drives"][0]["formula"], "45 * t");
    let frames = studying(&wb).frames.clone().expect("worked out");
    let listed = run(
        &mut doc,
        "asm.motion_frames",
        serde_json::json!({"study": study.0.to_string()}),
    );
    assert_eq!(listed.as_array().unwrap().len(), frames.len());

    let panel = widgets(&wb, &mut doc);
    assert_eq!(value_of(&panel, "frame"), 0.0);
    assert!(
        flat(&panel)
            .iter()
            .any(|w| matches!(w, Widget::Diagram { .. })),
        "the drive's value is plotted"
    );
    draws(&mut wb, &mut doc);
    send(&mut wb, &mut doc, number("frame", 2.0));
    assert_eq!(studying(&wb).frame, 2);
    for (body, placement) in &frames[2].1 {
        assert_eq!(doc.body_placement(*body), *placement);
    }
    send(&mut wb, &mut doc, button("play"));
    assert!(studying(&wb).playing);
    // A setting changed: the frames worked out no longer hold.
    send(&mut wb, &mut doc, number("start", 0.5));
    assert!(studying(&wb).frames.is_none());
    assert!(!studying(&wb).playing);
    send(&mut wb, &mut doc, button("remove_drive:0"));
    assert!(studying(&wb).draft.drives.is_empty());
}
