//! The task panel driven as the app draws it: frames of `ui_task_panel`
//! with pointer events at where its rows and fields were drawn.

use super::*;
use wb_sketch::generator::{GearSpec, Generator};
use wb_sketch::sketch::{ConstraintKind, Line};

/// A panel drawn frame by frame for a harness.
struct Panel {
    ctx: egui::Context,
    /// Where each text was drawn last frame.
    texts: Vec<(String, egui::Rect)>,
    /// The filled rectangles drawn last frame, with their colours.
    fills: Vec<(egui::Rect, egui::Color32)>,
}

impl Panel {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ui_kit::theme::apply_theme(&ctx);
        Self {
            ctx,
            texts: Vec::new(),
            fills: Vec::new(),
        }
    }

    fn frame(&mut self, h: &mut Harness, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(360.0, 2400.0),
            )),
            events,
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| {
            let mut wbctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0; 3], VIEWPORT);
            wbctx.view_proj = Some(h.vp);
            wbctx.active_document_object = h.active_object;
            let _ = h.wb.ui_task_panel(ui, &mut wbctx, Default::default());
            h.recorded
                .extend(core_document::HookOutcome::take(&mut wbctx).recorded);
        });
        out.textures_delta.clear();
        self.texts.clear();
        self.fills.clear();
        for clipped in &out.shapes {
            match &clipped.shape {
                egui::Shape::Text(text) => self
                    .texts
                    .push((text.galley.job.text.clone(), text.visual_bounding_rect())),
                egui::Shape::Rect(rect) => self.fills.push((rect.rect, rect.fill)),
                _ => {}
            }
        }
    }

    /// Two frames, so the panel draws from what the first one left.
    fn settle(&mut self, h: &mut Harness) {
        self.frame(h, Vec::new());
        self.frame(h, Vec::new());
    }

    /// Where `text` was drawn, the `nth` time.
    fn at(&self, text: &str, nth: usize) -> egui::Pos2 {
        self.texts
            .iter()
            .filter(|(t, _)| t == text)
            .nth(nth)
            .unwrap_or_else(|| panic!("`{text}` drawn {} times", nth + 1))
            .1
            .center()
    }

    fn shows(&self, text: &str) -> bool {
        self.texts.iter().any(|(t, _)| t == text)
    }

    /// Whether the row through `at` is filled `color`.
    fn row_filled(&self, at: egui::Pos2, color: egui::Color32) -> bool {
        self.fills
            .iter()
            .any(|(r, c)| *c == color && r.contains(at) && r.width() > 100.0)
    }

    fn move_to(&mut self, h: &mut Harness, at: egui::Pos2) {
        self.frame(h, vec![egui::Event::PointerMoved(at)]);
        self.settle(h);
    }

    fn click_with(&mut self, h: &mut Harness, at: egui::Pos2, button: egui::PointerButton) {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(h, vec![egui::Event::PointerMoved(at)]);
        self.frame(h, vec![press(true)]);
        self.frame(h, vec![press(false)]);
        self.settle(h);
    }

    fn click(&mut self, h: &mut Harness, at: egui::Pos2) {
        self.click_with(h, at, egui::PointerButton::Primary);
    }

    fn type_text(&mut self, h: &mut Harness, text: &str) {
        self.frame(h, vec![egui::Event::Text(text.into())]);
        let enter = |pressed| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        self.frame(h, vec![enter(true), enter(false)]);
        self.settle(h);
    }
}

/// A sketch of a rectangle, its bottom line sized, and a construction
/// diagonal, open in the panel.
fn rectangle() -> (Harness, Panel) {
    let mut h = Harness::sketching();
    h.edit_sketch(|sketch| {
        let corners = [(0.0, 0.0), (60.0, 0.0), (60.0, 40.0), (0.0, 40.0)];
        let points: Vec<_> = corners.iter().map(|(x, y)| point(sketch, *x, *y)).collect();
        let mut lines = Vec::new();
        for i in 0..4 {
            let line = Line::new(points[i], points[(i + 1) % 4]);
            lines.push(sketch.add_geometry(GeometryElement::Line(line)));
        }
        sketch.add_constraint(ConstraintKind::Length {
            line: lines[0],
            length: 60.0,
        });
        let diagonal = sketch.add_geometry(GeometryElement::Line(Line::new(points[0], points[2])));
        sketch.set_construction(diagonal, true);
    });
    let mut panel = Panel::new();
    panel.settle(&mut h);
    (h, panel)
}

fn length_of(h: &Harness) -> f64 {
    h.sketch()
        .constraints
        .iter()
        .find_map(|c| match c.kind {
            ConstraintKind::Length { length, .. } => Some(length),
            _ => None,
        })
        .expect("a length")
}

/// The lists show every element with its size and every constraint with
/// the elements it ties; a click on a row picks it, a second takes it
/// back, and with something picked the array fold offers itself.
#[test]
fn a_row_click_picks_its_element_and_a_second_takes_it_back() {
    let (mut h, mut panel) = rectangle();
    for text in [
        "ELEMENTS",
        "CONSTRAINTS",
        "Origin",
        "60.00 mm",
        "construction",
        "Length",
    ] {
        assert!(panel.shows(text), "{text} listed");
    }
    assert!(!panel.shows("ARRAY"), "nothing picked yet");
    let line = panel.at("Line 1", 0);
    panel.click(&mut h, line);
    assert!(panel.shows("ARRAY"), "the line is picked");
    assert!(panel.row_filled(panel.at("Line 1", 0), ui_kit::tokens::ACCENT_DIM));
    panel.click(&mut h, panel.at("Line 1", 0));
    assert!(!panel.shows("ARRAY"), "a second click takes it back");
}

/// The pointer on a constraint's chip lights the element it names, and
/// the chip's click picks that element.
#[test]
fn a_chip_lights_and_picks_the_element_it_names() {
    let (mut h, mut panel) = rectangle();
    // The element row first, then the chip on the Length row.
    let chip = panel.at("Line 1", 1);
    panel.move_to(&mut h, chip);
    let row = panel.at("Line 1", 0);
    assert!(
        panel.row_filled(row, ui_kit::tokens::BG3),
        "the element row is lit"
    );
    panel.click(&mut h, chip);
    assert!(panel.shows("ARRAY"), "the chip picked its element");
    // Off the panel, nothing stays lit.
    panel.move_to(&mut h, egui::pos2(5.0, 2300.0));
    assert!(!panel.row_filled(panel.at("Line 1", 0), ui_kit::tokens::BG3));
}

/// A value typed into a picked dimension's row opens that dimension's
/// card, nothing stored until OK: a formula reading the dimension itself
/// is refused as a loop, an unknown name says so, a negative length says
/// why, each holding OK back; OK stores the value as a recorded call that
/// replays, and Save as variable binds it to a new variable.
#[test]
fn a_value_typed_into_a_dimension_row_opens_its_card() {
    let (mut h, mut panel) = rectangle();
    let id = h.active_object.unwrap();
    h.edit_sketch(|s| s.constraints[0].name = Some("width".into()));
    let before = h.doc.clone();
    let name = h.doc.get_feature_meta(id).unwrap().name.clone();
    panel.settle(&mut h);
    panel.click(&mut h, panel.at("width", 0));
    let row = panel.at("width", 0);
    let field = panel
        .texts
        .iter()
        .find(|(t, r)| t != "width" && (r.center().y - row.y).abs() < 8.0 && r.center().x > row.x)
        .map(|(_, r)| r.center())
        .expect("the row's field");
    panel.click(&mut h, field);
    select_all(&mut panel, &mut h);
    panel.type_text(&mut h, "25 + 5");
    let card = h.card().expect("the dimension's card");
    assert_eq!(card.title, "width");
    assert!((length_of(&h) - 60.0).abs() < 1e-9, "nothing stored yet");

    for (text, says) in [
        (format!("{name}.width * 2"), "loop"),
        ("nothing.here".to_string(), "nothing"),
        ("-4".to_string(), "greater than zero"),
    ] {
        h.card_type("dim.text", &text);
        let refusal = h.card().unwrap().refusal.unwrap_or_default();
        assert!(refusal.contains(says), "{text}: says {says}: {refusal}");
        h.close_card(true);
        assert!((length_of(&h) - 60.0).abs() < 1e-9, "{text}: not applied");
    }
    h.card_type("dim.text", "45");
    h.close_card(true);
    assert!(h.card().is_none(), "the card closed");
    assert!((length_of(&h) - 45.0).abs() < 1e-9, "{}", length_of(&h));
    super::recording::assert_replays(&h.recorded, before, id, &h.sketch());

    // Saved as a variable: the set is made and the dimension reads it.
    panel.settle(&mut h);
    panel.click(&mut h, field);
    select_all(&mut panel, &mut h);
    panel.type_text(&mut h, "50");
    h.card_event(bench_api::PanelEvent::Button {
        id: "dim.save".into(),
    });
    assert_eq!(h.card().unwrap().ok, "Save");
    h.close_card(true);
    let key = h.sketch().constraints[0].id.to_string();
    assert_eq!(h.doc.feature_formula(id, &key), Some("Variables.width"));
    assert!((length_of(&h) - 50.0).abs() < 1e-9, "{}", length_of(&h));
}

/// The right-click menus: an element made construction, a constraint
/// renamed by typing into its row, then deleted.
#[test]
fn row_menus_change_rename_and_delete() {
    let (mut h, mut panel) = rectangle();
    panel.click_with(
        &mut h,
        panel.at("Line 2", 0),
        egui::PointerButton::Secondary,
    );
    panel.click(&mut h, panel.at("Make construction", 0));
    let line = h.sketch().geometry[5].id();
    assert!(h.sketch().is_construction(line), "Line 2 is construction");

    panel.click_with(
        &mut h,
        panel.at("Length", 0),
        egui::PointerButton::Secondary,
    );
    panel.click(&mut h, panel.at("Rename", 0));
    panel.type_text(&mut h, "width");
    panel.settle(&mut h);
    let constraint = &h.sketch().constraints[0];
    assert_eq!(constraint.name.as_deref(), Some("width"));

    panel.click_with(&mut h, panel.at("width", 0), egui::PointerButton::Secondary);
    panel.click(&mut h, panel.at("Delete", 0));
    assert!(h.sketch().constraints.is_empty(), "the dimension is gone");
}

/// What is typed in the filter narrows the elements, the bench reading
/// it.
#[test]
fn the_filter_narrows_the_elements() {
    let (mut h, mut panel) = rectangle();
    panel.click(&mut h, panel.at("Filter", 0));
    panel.type_text(&mut h, "line 5");
    assert!(panel.shows("Line 5"), "the diagonal");
    assert!(!panel.shows("Line 1") && !panel.shows("Origin"));
}

/// A generated sketch is edited by its numbers: a switch makes the
/// curves again.
#[test]
fn a_generated_sketch_takes_its_switches_from_the_panel() {
    let mut h = Harness::new();
    let sketch = Sketch::new("gear");
    let mut feature = SketchFeature::new(sketch.clone(), sketch.plane);
    feature.generator = Some(Generator::Gear(GearSpec::default()));
    wb_sketch::generator::regenerate(&mut feature).expect("a gear");
    let id = h
        .doc
        .add_feature_in_body(feature, "sketch".into(), None)
        .expect("the gear's sketch");
    h.active_object = Some(id);
    h.event(WorkbenchInputEvent::KeyPress { key: KeyCode::A }, None);
    let mut panel = Panel::new();
    panel.settle(&mut h);
    assert!(panel.shows("Involute gear") && panel.shows("Teeth"));
    panel.click(&mut h, panel.at("Internal (ring) gear", 0));
    let internal = h
        .doc
        .get_feature_data(id)
        .and_then(|d| d.pointer("/generator/Gear/internal").cloned());
    assert_eq!(internal, Some(serde_json::json!(true)));
    assert!(panel.shows("Ring rim"), "a ring gear shows its rim");
}

/// Run command `id` on the harness's document, as a script would.
fn command(h: &mut Harness, id: &str, args: serde_json::Value) -> serde_json::Value {
    let mut ctx = WorkbenchRuntimeContext::new(&mut h.doc, CAM_POS, [0.0; 3], VIEWPORT);
    ctx.active_document_object = h.active_object;
    let args = serde_json::from_value(args).unwrap();
    h.wb.run_command(id, &args, &mut ctx).expect(id)
}

fn spline(h: &Harness) -> wb_sketch::sketch::BSpline {
    h.sketch()
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::BSpline(b) => Some(b.clone()),
            _ => None,
        })
        .expect("a spline")
}

/// The spline fold: a knot made to stand twice and the degree raised by
/// their steppers, then a control point's weight typed in.
#[test]
fn a_spline_takes_its_knots_degree_and_weights_from_the_panel() {
    let mut h = Harness::sketching();
    for (x, y) in [(0.0, 0.0), (10.0, 10.0), (20.0, 0.0), (30.0, 10.0)] {
        h.click(x, y, "sketch.bspline");
    }
    h.right_click(30.0, 10.0, "sketch.bspline");
    let id = spline(&h).id;
    let sketch = h.active_object.unwrap().0.to_string();
    command(
        &mut h,
        "sketch.insert_knot",
        serde_json::json!({"sketch": sketch, "spline": id.to_string(), "at": [15.0, 5.0]}),
    );
    let mut panel = Panel::new();
    panel.settle(&mut h);
    panel.click(&mut h, panel.at("B-spline 1", 0));
    assert!(
        panel.shows("Degree") && panel.shows("×1"),
        "the spline fold"
    );

    // The second "+" is the knot's, the first the degree's.
    panel.click(&mut h, panel.at("+", 1));
    let knots = wb_sketch::spline_edit::knots_of(&h.sketch(), id);
    assert_eq!(knots.iter().map(|k| k.1).collect::<Vec<_>>(), [2]);
    panel.click(&mut h, panel.at("+", 0));
    assert_eq!(spline(&h).degree, 4, "raised once");

    // A control point's weight, typed in.
    panel.click(&mut h, panel.at("B-spline 1", 0));
    panel.click(&mut h, panel.at("Point 2", 0));
    assert!(panel.shows("Weight"));
    let row = panel.at("Weight", 0);
    let field = panel
        .texts
        .iter()
        .find(|(t, r)| t.starts_with("1.000") && (r.center().y - row.y).abs() < 8.0)
        .map(|(_, r)| r.center())
        .expect("the weight's field");
    panel.click(&mut h, field);
    select_all(&mut panel, &mut h);
    panel.type_text(&mut h, "2");
    let b = spline(&h);
    let point = h.sketch().geometry[1].id();
    let at = b
        .control_points
        .iter()
        .position(|p| *p == point)
        .expect("a pole");
    assert_eq!(b.weights.get(at).copied(), Some(2.0), "{:?}", b.weights);
}

/// Ctrl+A in the focused field.
fn select_all(panel: &mut Panel, h: &mut Harness) {
    panel.frame(
        h,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
}

/// The panel follows a shape as it is drawn: each click of a polyline
/// lists the line it made before the shape is done.
#[test]
fn the_lists_follow_a_shape_while_it_is_drawn() {
    let mut h = Harness::sketching();
    let mut panel = Panel::new();
    panel.settle(&mut h);
    assert!(!panel.shows("Line 1"));
    h.click(0.0, 0.0, "sketch.polyline");
    h.click(20.0, 0.0, "sketch.polyline");
    panel.settle(&mut h);
    assert!(
        panel.shows("Line 1") && !panel.shows("Line 2"),
        "one segment"
    );
    h.click(20.0, 15.0, "sketch.polyline");
    panel.settle(&mut h);
    assert!(panel.shows("Line 2"), "the next segment, mid shape");
}

/// Rows add to the selection click by click, with or without Ctrl, as
/// the view does; the view draws every one of them picked.
#[test]
fn rows_pick_several_elements_for_the_view() {
    let (mut h, mut panel) = rectangle();
    panel.click(&mut h, panel.at("Line 1", 0));
    let ctrl = panel.at("Line 2", 0);
    let press = |pressed| egui::Event::PointerButton {
        pos: ctrl,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::COMMAND,
    };
    panel.frame(&mut h, vec![egui::Event::PointerMoved(ctrl)]);
    panel.frame(&mut h, vec![press(true)]);
    panel.frame(&mut h, vec![press(false)]);
    panel.click(&mut h, panel.at("Line 3", 0));
    panel.settle(&mut h);
    let picked = h
        .overlays()
        .iter()
        .filter(|o| same_color(o.color, pal().selected))
        .count();
    assert_eq!(picked, 3, "three lines drawn picked");
    for row in ["Line 1", "Line 2", "Line 3"] {
        assert!(panel.row_filled(panel.at(row, 0), ui_kit::tokens::ACCENT_DIM));
    }
}

/// The edited sketch as stored.
fn json(h: &Harness) -> serde_json::Value {
    serde_json::to_value(h.sketch()).unwrap()
}

/// A fillet picked in the view opens a card beside it, its radius taking
/// the keys, the view showing the corner cut: a radius too large or below
/// zero says why and holds OK back, and OK stores what the view showed as
/// one undo step and one recorded call that replays to the same sketch.
#[test]
fn a_fillet_waits_in_its_card_until_ok_and_replays() {
    let (mut h, id, before) = super::recording::session_on_a_sketch();
    h.full_click(0.0, 0.0, "sketch.rect");
    h.full_click(20.0, 12.0, "sketch.rect");
    let drawn = (json(&h), h.overlays().len());
    h.full_click(20.0, 12.0, "sketch.fillet");
    let card = h.card().expect("the card");
    assert_eq!(
        (card.title.as_str(), card.focus.as_deref(), card.refusal),
        ("Fillet", Some("corner.size"), None)
    );
    assert!(card.at.is_some(), "by the corner");
    assert_eq!(json(&h), drawn.0, "nothing stored before OK");
    assert!(h.overlays().len() > drawn.1, "the view shows the cut");
    // Another tab's turn: the card goes with its tab and comes back.
    let kept = h.wb.suspend_session();
    assert!(h.card().is_none(), "not in another tab");
    h.wb.resume_session(kept);
    assert!(h.card().is_some(), "back with its tab");

    for refused in ["30", "-1", "nope"] {
        h.card_type("corner.size", refused);
        assert!(h.card().unwrap().refusal.is_some(), "{refused}: says why");
        h.close_card(true);
        assert_eq!(json(&h), drawn.0, "{refused}: nothing applied");
    }
    h.card_type("corner.size", "5");
    h.close_card(true);
    assert_eq!(h.gestures_closed, 1, "OK is one undo step");
    assert!(h.card().is_none(), "the card closed");
    let arcs = all_arcs(&h.sketch());
    assert_eq!(arcs.len(), 1);
    assert!((arcs[0].radius - 5.0).abs() < 1e-9, "{}", arcs[0].radius);
    let draws = h.recorded.iter().filter(|r| r.id == "sketch.draw").count();
    assert_eq!(draws, 2, "the rectangle and the fillet: {:#?}", h.recorded);
    super::recording::assert_replays(&h.recorded, before, id, &h.sketch());
}

/// Cancel (or Escape), another tool and an edit from elsewhere each put
/// an open chamfer away, storing and recording nothing.
#[test]
fn an_open_chamfer_goes_with_cancel_a_tool_or_an_outside_edit() {
    type Leave = fn(&mut Harness);
    let ways: [(&str, Leave); 3] = [
        ("cancel", |h| h.close_card(false)),
        ("tool", |h| h.activate("sketch.line")),
        ("outside", |h| {
            h.edit_sketch(|s| s.name = "renamed".into());
            h.mouse_move(5.0, 5.0, "sketch.chamfer");
        }),
    ];
    for (what, leave) in ways {
        let mut h = Harness::sketching();
        h.full_click(0.0, 0.0, "sketch.rect");
        h.full_click(20.0, 12.0, "sketch.rect");
        let drawn = (h.counts(), h.sketch().constraints.len());
        let recorded = h.recorded.len();
        h.full_click(20.0, 12.0, "sketch.chamfer");
        assert_eq!(h.card().map(|c| c.title), Some("Chamfer".into()), "{what}");
        h.card_type("corner.size", "3");
        leave(&mut h);
        assert!(h.card().is_none(), "{what}: put away");
        let now = (h.counts(), h.sketch().constraints.len());
        assert_eq!(now, drawn, "{what}: nothing cut");
        assert_eq!(h.recorded.len(), recorded, "{what}: nothing recorded");
    }
}
