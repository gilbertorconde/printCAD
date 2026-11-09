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

/// A picked dimension's value is a field: what is typed in sets it,
/// through the sketcher's own path, a sum read as its value.
#[test]
fn a_picked_dimension_takes_a_value_typed_into_its_row() {
    let (mut h, mut panel) = rectangle();
    panel.click(&mut h, panel.at("Length", 0));
    // The field: the value drawn on the Length row.
    let row = panel.at("Length", 0);
    let field = panel
        .texts
        .iter()
        .find(|(t, r)| t.starts_with("60.00") && (r.center().y - row.y).abs() < 8.0)
        .map(|(_, r)| r.center())
        .expect("the row's field");
    panel.click(&mut h, field);
    // The field opens on its value; Ctrl+A takes it all.
    panel.frame(
        &mut h,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    panel.type_text(&mut h, "25 + 5");
    assert!((length_of(&h) - 30.0).abs() < 1e-9, "{}", length_of(&h));
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
