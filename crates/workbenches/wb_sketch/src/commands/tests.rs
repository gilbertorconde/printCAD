use super::*;
use crate::sketch::ConstraintKind;
use core_document::{Document, WorkbenchRuntimeContext};

fn call(doc: &mut Document, id: &str, args: Value) -> CommandResult {
    let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
    run(id, args.as_object().unwrap(), &mut ctx)
}

/// A call that must succeed: its answer.
/// Whether the call is refused.
fn refused(doc: &mut Document, id: &str, args: Value) -> bool {
    call(doc, id, args).is_err()
}

fn ok(doc: &mut Document, id: &str, args: Value) -> Value {
    call(doc, id, args).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn feature_id(id: &Value) -> FeatureId {
    FeatureId(Uuid::parse_str(id.as_str().unwrap()).unwrap())
}

/// The stored sketch feature a command answered with the id of.
fn feature_of(doc: &Document, id: &Value) -> SketchFeature {
    SketchFeature::from_json(doc.get_feature_data(feature_id(id)).unwrap()).unwrap()
}

fn sketch_of(doc: &Document, id: &Value) -> Sketch {
    feature_of(doc, id).sketch
}

/// A small picture written as a PNG file, and its bytes.
fn reference_png() -> (std::path::PathBuf, Vec<u8>) {
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(4, 2, image::Rgba([200, 100, 50, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("printcad-picture-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("reference.png");
    std::fs::write(&path, png.get_ref()).unwrap();
    (path, png.into_inner())
}

/// A picture file comes in as a document asset and lies in a new sketch named after it; it
/// moves, sizes and goes by its id.
#[test]
fn a_reference_picture_comes_in_and_is_placed() {
    let (path, png) = reference_png();
    let mut doc = Document::new("t");
    let args = json!({"path": path.display().to_string(), "width": 40.0});
    let made = ok(&mut doc, "sketch.image", args);
    let sketch = FeatureId(Uuid::parse_str(made["sketch"].as_str().unwrap()).unwrap());
    let image = made["image"].as_str().unwrap().to_string();
    let feature = SketchFeature::from_json(doc.get_feature_data(sketch).unwrap()).unwrap();
    assert_eq!(doc.get_feature_meta(sketch).unwrap().name, "reference");
    let placed = &feature.sketch.images[0];
    assert_eq!(placed.width, 40.0);
    let kept = doc.asset_bytes(placed.asset).map(<[u8]>::to_vec);
    assert_eq!(kept, Some(png));

    ok(
        &mut doc,
        "sketch.set_image",
        json!({"sketch": sketch.0.to_string(), "image": image, "x": 5.0, "angle": 30.0}),
    );
    let feature = SketchFeature::from_json(doc.get_feature_data(sketch).unwrap()).unwrap();
    let moved = &feature.sketch.images[0];
    assert_eq!(
        (moved.center.x, moved.angle_deg, moved.width),
        (5.0, 30.0, 40.0)
    );

    ok(
        &mut doc,
        "sketch.set_image",
        json!({"sketch": sketch.0.to_string(), "image": image, "remove": true}),
    );
    let feature = SketchFeature::from_json(doc.get_feature_data(sketch).unwrap()).unwrap();
    assert!(feature.sketch.images.is_empty());
    assert!(refused(
        &mut doc,
        "sketch.image",
        json!({"path": "/nonexistent/picture.png"})
    ));
}

/// `sketch.new{on = datum}` draws the sketch on the datum and keeps it
/// there: the sketch records the datum and depends on it; a plane set
/// outright later lets it go.
#[test]
fn a_sketch_made_on_a_datum_keeps_to_it() {
    use core_document::{AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape};
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let datum = doc
        .add_feature_in_body(
            DatumFeature {
                shape: DatumShape::CoordinateSystem { size: 10.0 },
                attachment: DatumAttachment::BasePlane(BasePlane::XY),
                offset: AttachmentOffset {
                    tilt: [0.0; 2],
                    translation: [0.0, 0.0, 5.0],
                    rotation_deg: 0.0,
                    flip: false,
                },
            },
            "Frame".into(),
            Some(body),
        )
        .unwrap();
    let made = ok(
        &mut doc,
        "sketch.new",
        json!({"on": datum.0.to_string(), "plane": "xz", "offset": 2.0}),
    );
    let id = feature_id(&made);
    let feature = feature_of(&doc, &made);
    assert_eq!(
        feature.support,
        Some(DatumSupport {
            datum,
            plane: Some("XZ".into()),
            offset: 2.0,
            shift: [0.0, 0.0],
            turn: 0.0,
        })
    );
    assert_eq!(doc.feature_tree().dependencies(id), vec![datum]);
    assert_eq!(doc.get_feature_meta(id).unwrap().body, Some(body));

    ok(
        &mut doc,
        "sketch.set_plane",
        json!({"sketch": made, "normal": [0, 0, 1]}),
    );
    let feature = SketchFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap();
    assert_eq!(feature.support, None);
    assert!(doc.feature_tree().dependencies(id).is_empty());
}

#[test]
fn a_new_sketch_can_be_a_generators() {
    let mut doc = Document::new("t");
    let made = ok(
        &mut doc,
        "sketch.new",
        json!({"plane": "XZ", "generator": "sprocket"}),
    );
    let id = feature_id(&made);
    let feature = feature_of(&doc, &made);
    assert!(matches!(
        feature.generator,
        Some(crate::generator::Generator::Sprocket(_))
    ));
    assert!(!feature.sketch.geometry.is_empty());
    assert_eq!(feature.plane.normal, SketchPlane::xz().normal);
    assert_eq!(doc.get_feature_meta(id).unwrap().name, "Sprocket");
    let again = ok(&mut doc, "sketch.new", json!({"generator": "sprocket"}));
    let again = feature_id(&again);
    assert_eq!(doc.get_feature_meta(again).unwrap().name, "Sprocket_2");
    assert!(refused(&mut doc, "sketch.new", json!({"generator": "cam"})));
}

#[test]
fn scripted_text_is_laid_out_and_changed() {
    let mut doc = Document::new("t");
    let sketch = ok(&mut doc, "sketch.new", json!({"plane": "XY"}));
    let made = ok(
        &mut doc,
        "sketch.text",
        json!({"sketch": sketch, "text": "Hi", "at": [2, 3], "size": 8}),
    );
    let id = feature_id(&sketch);
    let read = |doc: &Document| {
        SketchFeature::from_json(doc.get_feature_data(id).unwrap())
            .unwrap()
            .sketch
    };
    let s = read(&doc);
    assert_eq!(s.texts.len(), 1);
    assert_eq!(made["point"], json!(s.texts[0].anchor.to_string()));
    let wires = crate::profile::extract_wires(&s).unwrap().len();
    assert!(wires >= 3, "an H and an i: {wires}");
    ok(
        &mut doc,
        "sketch.text_edit",
        json!({"sketch": sketch, "block": made["text"], "text": "HHH"}),
    );
    let s = read(&doc);
    assert_eq!(s.texts[0].text, "HHH");
    assert_eq!(crate::profile::extract_wires(&s).unwrap().len(), 3);
    assert!(refused(
        &mut doc,
        "sketch.text",
        json!({"sketch": sketch, "text": "x", "at": [0, 0], "font": "/no/such.ttf"})
    ));
}

#[test]
fn a_scripted_rectangle_is_one_closed_profile() {
    let mut doc = Document::new("t");
    let sketch = ok(&mut doc, "sketch.new", json!({"plane": "XZ"}));
    let lines = ok(
        &mut doc,
        "sketch.rect",
        json!({"sketch": sketch, "x": 0, "y": 0, "width": 20, "height": 10}),
    );
    assert_eq!(lines.as_array().unwrap().len(), 4);
    let id = feature_id(&sketch);
    let feature = feature_of(&doc, &sketch);
    let points = feature
        .sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::Point(_)))
        .count();
    assert_eq!(points, 4, "corners are shared");
    assert_eq!(
        crate::profile::extract_wires(&feature.sketch)
            .unwrap()
            .len(),
        1
    );
    assert!(doc.get_feature_meta(id).unwrap().body.is_some());
}

/// A negative horizontal or vertical distance puts the end that far
/// the other way; the line keeps its length and nothing conflicts.
#[test]
fn a_negative_axis_distance_runs_the_other_way() {
    for (kind, axis) in [("distance_x", 0), ("distance_y", 1)] {
        let mut doc = Document::new("t");
        let sketch = ok(&mut doc, "sketch.new", json!({}));
        let line = ok(
            &mut doc,
            "sketch.line",
            json!({"sketch": sketch, "x1": 0, "y1": 0, "x2": 5, "y2": 3}),
        );
        ok(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": sketch, "kind": kind, "items": [line], "value": -7}),
        );
        let status = ok(&mut doc, "sketch.status", json!({"sketch": sketch}));
        assert_eq!(status["conflicting"], json!([]), "{kind}");
        let geometry = ok(&mut doc, "sketch.geometry", json!({"sketch": sketch}));
        let points = geometry
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["kind"] == "line")
            .unwrap()["points"]
            .clone();
        let d = points[1][axis].as_f64().unwrap() - points[0][axis].as_f64().unwrap();
        assert!((d + 7.0).abs() < 1e-3, "{kind}: {points}");
    }
}

/// What is left free: 5 for an arc, however drawn, and for an ellipse.
#[test]
fn arcs_and_ellipses_count_five_degrees_of_freedom() {
    let dof = |tool: &str, points: Value| {
        let mut doc = Document::new("t");
        let sketch = ok(&mut doc, "sketch.new", json!({}));
        ok(
            &mut doc,
            "sketch.draw",
            json!({"sketch": sketch, "tool": tool, "points": points}),
        );
        ok(&mut doc, "sketch.status", json!({"sketch": sketch}))["dof"].clone()
    };
    assert_eq!(dof("arc3", json!([[0, 0], [10, 0], [5, 3]])), json!(5));
    assert_eq!(dof("ellipse", json!([[0, 0], [10, 0], [0, 4]])), json!(5));
}

/// A rounded rectangle, a slot and an arc slot keep their shape when
/// dragged: their tangents and equal radii are part of them, and none
/// of those constraints says again what the others already do.
#[test]
fn shapes_with_rounded_ends_hold_together_when_dragged() {
    for (tool, points, params, dof) in [
        (
            "rect_rounded",
            json!([[0, 0], [20, 10]]),
            json!({"fillet_radius": 2}),
            5,
        ),
        (
            "slot",
            json!([[1, 1], [21, 1]]),
            json!({"slot_width": 6}),
            5,
        ),
        (
            "arc_slot",
            json!([[0, 0], [20, 0], [0, 20]]),
            json!({"slot_width": 4}),
            6,
        ),
    ] {
        let mut doc = Document::new("t");
        let sketch = ok(&mut doc, "sketch.new", json!({}));
        ok(
            &mut doc,
            "sketch.draw",
            json!({"sketch": sketch, "tool": tool, "points": points, "params": params}),
        );
        let status = ok(&mut doc, "sketch.status", json!({"sketch": sketch}));
        assert_eq!(status["dof"], json!(dof), "{tool}");
        assert_eq!(status["redundant"], json!([]), "{tool}");
        let geometry = ok(&mut doc, "sketch.geometry", json!({"sketch": sketch}));
        let first = geometry
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["kind"] == "line" || g["kind"] == "arc")
            .unwrap()["id"]
            .clone();
        ok(
            &mut doc,
            "sketch.drag",
            json!({"sketch": sketch, "items": [first], "by": [1.5, 2.5]}),
        );
        let geometry = ok(&mut doc, "sketch.geometry", json!({"sketch": sketch}));
        let radii: Vec<f64> = geometry
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|g| g["radius"].as_f64())
            .collect();
        // The ends (all four corners, both caps) stay one size.
        let ends: Vec<f64> = match tool {
            "arc_slot" => radii[2..].to_vec(),
            _ => radii.clone(),
        };
        assert!(
            ends.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-3),
            "{tool}: {radii:?}"
        );
    }
}

/// Rows and columns are the array's, not a drawing tool's: sketch.draw
/// says so rather than ignoring them.
#[test]
fn array_settings_given_to_a_drawing_tool_are_refused() {
    let mut doc = Document::new("t");
    let sketch = ok(&mut doc, "sketch.new", json!({}));
    let error = call(
        &mut doc,
        "sketch.draw",
        json!({"sketch": sketch, "tool": "translate", "points": [[0, 0], [5, 0]],
               "params": {"array_rows": 3}}),
    )
    .unwrap_err();
    assert!(error.to_string().contains("sketch.array"), "{error}");
}

/// A rectangle keeps its sides level and upright, as the drawing
/// tool's does, and so do a polyline's level and upright segments.
#[test]
fn a_rectangle_and_a_polyline_hold_their_level_and_upright_sides() {
    let mut doc = Document::new("t");
    let sketch = ok(&mut doc, "sketch.new", json!({}));
    ok(
        &mut doc,
        "sketch.rect",
        json!({"sketch": sketch, "x": 0, "y": 0, "width": 10, "height": 5}),
    );
    ok(
        &mut doc,
        "sketch.polyline",
        json!({"sketch": sketch, "points": [[20, 0], [30, 0], [34, 6]]}),
    );
    let kinds: Vec<String> = ok(&mut doc, "sketch.constraints", json!({"sketch": sketch}))
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap().to_string())
        .collect();
    let count = |k: &str| kinds.iter().filter(|x| *x == k).count();
    assert_eq!(
        (count("Horizontal"), count("Vertical")),
        (3, 2),
        "{kinds:?}"
    );
}

#[test]
fn lines_drawn_end_to_end_share_their_points() {
    let mut doc = Document::new("t");
    let sketch = ok(&mut doc, "sketch.new", json!({}));
    for (x1, y1, x2, y2) in [(0, 0, 10, 0), (10, 0, 0, 10), (0, 10, 0, 0)] {
        ok(
            &mut doc,
            "sketch.line",
            json!({"sketch": sketch, "x1": x1, "y1": y1, "x2": x2, "y2": y2}),
        );
    }
    let listed = ok(&mut doc, "sketch.geometry", json!({"sketch": sketch}));
    let points = listed
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["kind"] == "point")
        .count();
    assert_eq!(points, 3);
}

/// A face given as an attachment is on the sketch's own body: the
/// sketch follows it unless told not to.
#[test]
fn an_attached_sketch_follows_its_face_unless_told_not_to() {
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    for (given, follows) in [(None, true), (Some(false), false)] {
        let mut face = json!({"point": [0, 0, 10], "normal": [0, 0, 1]});
        if let Some(given) = given {
            face["follows"] = json!(given);
        }
        let sketch = ok(
            &mut doc,
            "sketch.new",
            json!({"body": body.0.to_string(), "attachment": {"Face": {"face": face}}}),
        );
        let feature = feature_of(&doc, &sketch);
        let Some(core_document::DatumAttachment::Face { face }) =
            feature.attached.map(|a| a.attachment)
        else {
            panic!("attached on the face");
        };
        assert_eq!(face.follows, follows);
    }
}

#[test]
fn a_sketch_attached_by_three_points_lies_on_them() {
    use core_document::{DatumAttachment, PointAnchor};
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let at = |point| PointAnchor::At { point };
    let attachment = DatumAttachment::ThreePoints {
        points: [
            at([0.0, 0.0, 5.0]),
            at([10.0, 0.0, 5.0]),
            at([0.0, 10.0, 5.0]),
        ],
    };
    let sketch = ok(
        &mut doc,
        "sketch.new",
        json!({"body": body.0.to_string(), "attachment": attachment}),
    );
    let feature = feature_of(&doc, &sketch);
    assert_eq!(feature.attached.map(|a| a.attachment), Some(attachment));
    let (o, n) = (feature.plane.origin, feature.plane.normal);
    assert!((o[2] - 5.0).abs() < 1e-5, "{o:?}");
    assert!((n[2].abs() - 1.0).abs() < 1e-5, "{n:?}");
    assert!(refused(
        &mut doc,
        "sketch.new",
        json!({"body": body.0.to_string(), "attachment": {"Nonsense": 1}})
    ));
}

#[test]
fn a_sketch_shifts_and_turns_on_its_datum_or_off_its_plane() {
    use core_document::{AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape};
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let datum = doc
        .add_feature_in_body(
            DatumFeature {
                shape: DatumShape::Plane { size: 20.0 },
                attachment: DatumAttachment::BasePlane(BasePlane::XY),
                offset: AttachmentOffset::default(),
            },
            "Datum".into(),
            Some(body),
        )
        .unwrap();
    let sketch = ok(&mut doc, "sketch.new", json!({"on": datum.0.to_string()}));
    ok(
        &mut doc,
        "sketch.attachment",
        json!({"sketch": sketch, "offset": 3.0, "shift": [5.0, 0.0], "turn": 90.0}),
    );
    let feature = feature_of(&doc, &sketch);
    let near = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5);
    assert!(
        near(feature.plane.origin, [5.0, 0.0, 3.0]),
        "{:?}",
        feature.plane
    );
    assert!(
        near(feature.plane.x_axis, [0.0, 1.0, 0.0]),
        "{:?}",
        feature.plane
    );
    assert!(near(feature.plane.y_axis, [-1.0, 0.0, 0.0]));
    // A sketch on a base plane moves off it without a datum; given
    // again, each value replaces the last rather than adding to it.
    let free = ok(&mut doc, "sketch.new", json!({}));
    let placed = feature_of(&doc, &free).plane;
    for _ in 0..2 {
        ok(
            &mut doc,
            "sketch.attachment",
            json!({"sketch": free, "offset": 10.0, "shift": [2.0, 0.0], "turn": 90.0}),
        );
    }
    let feature = feature_of(&doc, &free);
    let lift = |v: [f64; 3], by: [f64; 3]| [v[0] + by[0], v[1] + by[1], v[2] + by[2]];
    let n = placed.normal.map(|c| c * 10.0);
    let x = placed.x_axis.map(|c| c * 2.0);
    assert!(
        near(feature.plane.origin, lift(lift(placed.origin, n), x)),
        "{:?}",
        feature.plane
    );
    assert!(
        near(feature.plane.x_axis, placed.y_axis),
        "{:?}",
        feature.plane
    );
    assert!(near(
        feature.offset.undo(feature.plane).origin,
        placed.origin
    ));
    ok(
        &mut doc,
        "sketch.attachment",
        json!({"sketch": free, "offset": 0.0, "shift": [0.0, 0.0], "turn": 0.0}),
    );
    assert!(near(feature_of(&doc, &free).plane.x_axis, placed.x_axis));
    assert!(near(feature_of(&doc, &free).plane.origin, placed.origin));
}

#[test]
fn another_sketch_and_a_datum_come_in_as_external_geometry_that_follows_them() {
    use core_document::{AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape};
    let mut doc = Document::new("t");
    let body = doc.create_body(None);
    let first = ok(&mut doc, "sketch.new", json!({"body": body.0.to_string()}));
    for (x1, y1, x2, y2) in [(0, 0, 10, 0), (10, 0, 10, 6), (10, 6, 0, 6), (0, 6, 0, 0)] {
        ok(
            &mut doc,
            "sketch.line",
            json!({"sketch": first, "x1": x1, "y1": y1, "x2": x2, "y2": y2}),
        );
    }
    let second = ok(&mut doc, "sketch.new", json!({"body": body.0.to_string()}));
    let made = ok(
        &mut doc,
        "sketch.external_from",
        json!({"sketch": second, "from": first}),
    );
    assert_eq!(
        made["elements"].as_array().map(Vec::len),
        Some(12),
        "four lines and their points: {made}"
    );
    let copied = sketch_of(&doc, &second);
    assert_eq!(
        copied.external.len(),
        4,
        "the lines; their points go with them"
    );
    assert!(copied.external.values().all(|s| s.reference.is_some()));
    assert!(
        copied.external.values().all(|s| !s.defining),
        "said nothing, it only guides"
    );
    // Asked to count, it closes a profile of its own.
    let third = ok(&mut doc, "sketch.new", json!({"body": body.0.to_string()}));
    ok(
        &mut doc,
        "sketch.external_from",
        json!({"sketch": third, "from": first, "counts": true}),
    );
    let counted = sketch_of(&doc, &third);
    assert!(counted.external.values().all(|s| s.defining));
    let wires = crate::profile::extract_wires(&counted).expect("the rectangle counts");
    assert_eq!(wires.len(), 1);
    let corner = copied.geometry.iter().any(|g| matches!(
        g,
        GeometryElement::Point(p) if (p.position.to_glam() - glam::DVec2::new(10.0, 6.0)).length() < 1e-4
    ));
    assert!(
        corner,
        "the far corner lands where it is in the other sketch"
    );

    // A datum plane standing on the XZ plane crosses this one along x.
    let datum = doc
        .add_feature_in_body(
            DatumFeature {
                shape: DatumShape::Plane { size: 20.0 },
                attachment: DatumAttachment::BasePlane(BasePlane::XZ),
                offset: AttachmentOffset::default(),
            },
            "Datum".into(),
            Some(body),
        )
        .unwrap();
    ok(
        &mut doc,
        "sketch.external_from",
        json!({"sketch": second, "from": datum.0.to_string()}),
    );
    let with_datum = sketch_of(&doc, &second);
    let crossing = with_datum.geometry.iter().any(|g| matches!(
        g,
        GeometryElement::Point(p) if (p.position.to_glam() - glam::DVec2::new(10.0, 0.0)).length() < 1e-3
    ));
    assert!(
        crossing,
        "the crossing runs along x, 20 long about the origin"
    );
    assert!(
        refused(
            &mut doc,
            "sketch.external_from",
            json!({"sketch": second, "from": second})
        ),
        "not from itself"
    );

    // The first sketch's far corner moves; brought up to it again, the
    // copy follows.
    let first_id = feature_id(&first);
    let mut data = doc.get_feature_data(first_id).unwrap().clone();
    let mut edited = SketchFeature::from_json(&data).unwrap();
    for g in &mut edited.sketch.geometry {
        if let GeometryElement::Point(p) = g
            && (p.position.to_glam() - glam::DVec2::new(10.0, 6.0)).length() < 1e-4
        {
            p.position = crate::sketch::Vec2D::new(12.0, 7.0);
        }
    }
    data = edited.to_json();
    doc.update_feature_data(first_id, data).unwrap();
    let second_id = feature_id(&second);
    let mut feature = SketchFeature::from_json(doc.get_feature_data(second_id).unwrap()).unwrap();
    let placed = crate::placed_plane(&feature.plane, &crate::sketch_placement(&doc, second_id));
    for (source, group) in crate::external::groups(&feature.sketch) {
        let projected =
            crate::external_ref::project(&doc, &placed, source.reference.unwrap()).unwrap();
        crate::external::refresh_group(&mut feature.sketch, source, &group, &projected);
    }
    let moved = feature.sketch.geometry.iter().any(|g| matches!(
        g,
        GeometryElement::Point(p) if (p.position.to_glam() - glam::DVec2::new(12.0, 7.0)).length() < 1e-4
    ));
    assert!(moved, "the copy's corner followed");
}

fn line_length(sketch: &Sketch, id: &Value) -> f64 {
    let id = Uuid::parse_str(id.as_str().unwrap()).unwrap();
    let Some(GeometryElement::Line(l)) = sketch.get_geometry(id) else {
        panic!("not a line")
    };
    let (a, b) = (
        sketch.point_position(l.start).unwrap(),
        sketch.point_position(l.end).unwrap(),
    );
    (b - a).to_glam().length()
}

#[test]
fn constraints_drive_the_geometry_and_the_status_counts_them() {
    let mut doc = Document::new("t");
    let s = ok(&mut doc, "sketch.new", json!({}));
    let line = ok(
        &mut doc,
        "sketch.line",
        json!({"sketch": s, "x1": 1, "y1": 1, "x2": 9, "y2": 2}),
    );
    let free = ok(&mut doc, "sketch.status", json!({"sketch": s}))["dof"]
        .as_i64()
        .unwrap();
    ok(
        &mut doc,
        "sketch.constrain",
        json!({"sketch": s, "kind": "horizontal", "items": [line]}),
    );
    let dim = ok(
        &mut doc,
        "sketch.constrain",
        json!({"sketch": s, "kind": "dimension", "items": [line], "value": 25}),
    );
    let sketch = sketch_of(&doc, &s);
    assert!((line_length(&sketch, &line) - 25.0).abs() < 1e-3);
    let status = ok(&mut doc, "sketch.status", json!({"sketch": s}));
    assert_eq!(status["dof"].as_i64().unwrap(), free - 2);
    assert_eq!(status["solved"], json!(true));

    ok(
        &mut doc,
        "sketch.set_value",
        json!({"sketch": s, "constraint": dim[0], "value": 40}),
    );
    assert!((line_length(&sketch_of(&doc, &s), &line) - 40.0).abs() < 1e-3);
    let listed = ok(&mut doc, "sketch.constraints", json!({"sketch": s}));
    assert_eq!(listed.as_array().unwrap().len(), 2);

    // The origin takes constraints by name.
    let start = sketch_of(&doc, &s)
        .geometry
        .iter()
        .find_map(|g| match g {
            GeometryElement::Line(l) => Some(l.start),
            _ => None,
        })
        .unwrap();
    ok(
        &mut doc,
        "sketch.constrain",
        json!({"sketch": s, "kind": "coincident", "items": [start.to_string(), "origin"]}),
    );
    let sketch = sketch_of(&doc, &s);
    assert!(sketch.point_position(start).unwrap().to_glam().length() < 1e-3);

    ok(
        &mut doc,
        "sketch.delete",
        json!({"sketch": s, "items": [dim[0]]}),
    );
    assert_eq!(sketch_of(&doc, &s).constraints.len(), 2);
    assert!(
        refused(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "parallel", "items": [line]})
        ),
        "parallel takes two lines"
    );
}

#[test]
fn a_sketch_takes_a_plane_of_its_own() {
    let mut doc = Document::new("t");
    let s = ok(
        &mut doc,
        "sketch.new",
        json!({"normal": [1, 0, 0], "origin": [5, 0, 0], "x_axis": [0, 1, 0]}),
    );
    let plane = sketch_of(&doc, &s).plane;
    assert_eq!(plane.origin, [5.0, 0.0, 0.0]);
    assert_eq!(plane.x_axis, [0.0, 1.0, 0.0]);
    assert_eq!(plane.y_axis, [0.0, 0.0, 1.0]);
    assert!(refused(
        &mut doc,
        "sketch.new",
        json!({"normal": [0, 0, 0]})
    ));
}

#[test]
fn bad_arguments_are_refused() {
    let mut doc = Document::new("t");
    let sketch = ok(&mut doc, "sketch.new", json!({}));
    assert!(refused(
        &mut doc,
        "sketch.circle",
        json!({"sketch": sketch, "x": 0, "y": 0, "radius": -1})
    ));
    assert!(refused(&mut doc, "sketch.new", json!({"plane": "AB"})));
    let not_a_sketch = Uuid::new_v4().to_string();
    assert!(refused(
        &mut doc,
        "sketch.point",
        json!({"sketch": not_a_sketch, "x": 0, "y": 0})
    ));
}

fn registry() -> core_document::DocumentService {
    let mut registry = core_document::DocumentService::default();
    registry
        .register_workbench(Box::new(crate::SketchWorkbench::default()))
        .unwrap();
    registry
}

/// The radius of the sketch's one circle, as `sketch.geometry` reads it.
fn listed_radius(doc: &mut Document, sketch: &Value) -> f64 {
    let listed = ok(doc, "sketch.geometry", json!({"sketch": sketch}));
    listed
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["kind"] == "circle")
        .and_then(|g| g["radius"].as_f64())
        .unwrap()
}

/// A sketch whose projected line's source is deleted warns of it on its
/// row, through the registry as the tree asks.
#[test]
fn a_projection_whose_source_is_gone_warns() {
    let registry = registry();
    let mut doc = Document::new("t");
    let base = ok(&mut doc, "sketch.new", json!({}));
    let line = ok(
        &mut doc,
        "sketch.line",
        json!({"sketch": base, "x1": 0, "y1": 0, "x2": 5, "y2": 1}),
    );
    let top = ok(&mut doc, "sketch.new", json!({"offset": 10}));
    ok(
        &mut doc,
        "sketch.external_from",
        json!({"sketch": top, "from": base}),
    );
    let warning = |doc: &mut Document| {
        registry.evaluate(doc);
        let id = feature_id(&top);
        let node = doc.feature_tree().get_node(id).unwrap().clone();
        registry.feature_warning(doc, None, id, &node)
    };
    assert_eq!(warning(&mut doc), None, "its source stands");

    ok(
        &mut doc,
        "sketch.delete",
        json!({"sketch": base, "items": [line]}),
    );
    let said = warning(&mut doc).expect("a warning");
    assert!(said.starts_with("1 projected element"), "{said}");
}

/// A circle copied from another sketch takes the source's new radius
/// when the document is worked out, set by hand or by a formula, with
/// the copy never opened; its own stored data is the projection it was
/// made with.
#[test]
fn geometry_from_another_sketch_follows_it_without_the_sketch_being_opened() {
    let registry = registry();
    let mut doc = Document::new("t");
    let base = ok(&mut doc, "sketch.new", json!({}));
    let circle = ok(
        &mut doc,
        "sketch.circle",
        json!({"sketch": base, "x": 0, "y": 0, "radius": 2}),
    );
    let dim = ok(
        &mut doc,
        "sketch.constrain",
        json!({"sketch": base, "kind": "radius", "items": [circle], "value": 2}),
    )[0]
    .clone();
    let top = ok(&mut doc, "sketch.new", json!({"offset": 10}));
    ok(
        &mut doc,
        "sketch.external_from",
        json!({"sketch": top, "from": base, "counts": true}),
    );
    registry.evaluate(&mut doc);
    assert!((listed_radius(&mut doc, &top) - 2.0).abs() < 1e-5);

    ok(
        &mut doc,
        "sketch.set_value",
        json!({"sketch": base, "constraint": dim, "value": 3}),
    );
    registry.evaluate(&mut doc);
    assert!((listed_radius(&mut doc, &top) - 3.0).abs() < 1e-5);
    let built = crate::stored_sketch(&doc, feature_id(&top)).unwrap();
    let wires = crate::profile::extract_wires(&built.sketch).unwrap();
    assert_eq!(wires.len(), 1, "the copy is the profile");

    doc.set_feature_formula(
        feature_id(&base),
        dim.as_str().unwrap(),
        Some("4 mm".into()),
    )
    .unwrap();
    registry.evaluate(&mut doc);
    assert!((listed_radius(&mut doc, &base) - 4.0).abs() < 1e-5);
    assert!((listed_radius(&mut doc, &top) - 4.0).abs() < 1e-5);
    let listed = ok(&mut doc, "sketch.constraints", json!({"sketch": base}));
    assert!((listed[0]["value"].as_f64().unwrap() - 4.0).abs() < 1e-5);
    // Derived: the copy's own data is as it was made.
    assert!(
        sketch_of(&doc, &top)
            .geometry
            .iter()
            .any(|g| matches!(g, GeometryElement::Circle(c) if (c.radius - 2.0).abs() < 1e-5))
    );
}

/// The `elements` an external call answers with go back to
/// `sketch.external_defining` as they come, its curves' end points with
/// them; anything else is still refused.
#[test]
fn external_elements_are_made_to_count_as_they_were_returned() {
    let mut doc = Document::new("t");
    let base = ok(&mut doc, "sketch.new", json!({}));
    ok(
        &mut doc,
        "sketch.rect",
        json!({"sketch": base, "x": 0, "y": 0, "width": 4, "height": 4}),
    );
    let top = ok(&mut doc, "sketch.new", json!({"offset": 10}));
    let made = ok(
        &mut doc,
        "sketch.external_from",
        json!({"sketch": top, "from": base}),
    );
    assert!(crate::profile::extract_wires(&sketch_of(&doc, &top)).is_err());
    ok(
        &mut doc,
        "sketch.external_defining",
        json!({"sketch": top, "items": made["elements"]}),
    );
    assert_eq!(
        crate::profile::extract_wires(&sketch_of(&doc, &top))
            .unwrap()
            .len(),
        1
    );
    let own = ok(
        &mut doc,
        "sketch.point",
        json!({"sketch": top, "x": 20, "y": 20}),
    );
    assert!(refused(
        &mut doc,
        "sketch.external_defining",
        json!({"sketch": top, "items": [own]})
    ));
    // The same sketch again brings nothing twice.
    let again = call(
        &mut doc,
        "sketch.external_from",
        json!({"sketch": top, "from": base}),
    );
    assert!(again.is_err(), "{again:?}");
}

/// A radius turned into a diameter keeps its id and the circle's size,
/// its formula doubled; turned back, the formula is what it was.
#[test]
fn a_size_switches_between_radius_and_diameter_or_length_and_sweep() {
    let mut doc = Document::new("t");
    let s = ok(&mut doc, "sketch.new", json!({}));
    let circle = ok(
        &mut doc,
        "sketch.circle",
        json!({"sketch": s, "x": 0, "y": 0, "radius": 2}),
    );
    let dim = ok(
        &mut doc,
        "sketch.constrain",
        json!({"sketch": s, "kind": "radius", "items": [circle], "value": 2}),
    )[0]
    .clone();
    let key = dim.as_str().unwrap().to_string();
    doc.set_feature_formula(feature_id(&s), &key, Some("1 mm + 1 mm".into()))
        .unwrap();
    let switch = |doc: &mut Document, to: &str| {
        call(
            doc,
            "sketch.set_constraint",
            json!({"sketch": s, "items": [dim], "as": to}),
        )
    };
    switch(&mut doc, "diameter").unwrap();
    let held = sketch_of(&doc, &s)
        .constraints
        .iter()
        .find(|c| c.id.to_string() == key)
        .map(|c| c.kind.clone());
    assert!(
        matches!(held, Some(ConstraintKind::Diameter { diameter, .. }) if (diameter - 4.0).abs() < 1e-5),
        "{held:?}"
    );
    assert_eq!(
        doc.feature_formula(feature_id(&s), &key),
        Some("(1 mm + 1 mm) * 2")
    );
    switch(&mut doc, "radius").unwrap();
    assert_eq!(
        doc.feature_formula(feature_id(&s), &key),
        Some("1 mm + 1 mm")
    );
    assert!(switch(&mut doc, "chord").is_err());
    let line = ok(
        &mut doc,
        "sketch.line",
        json!({"sketch": s, "x1": 0, "y1": 0, "x2": 5, "y2": 1}),
    );
    let length = ok(
        &mut doc,
        "sketch.constrain",
        json!({"sketch": s, "kind": "distance", "items": [line], "value": 5}),
    );
    let refused = call(
        &mut doc,
        "sketch.set_constraint",
        json!({"sketch": s, "items": length, "as": "diameter"}),
    );
    assert!(refused.is_err(), "a length is no circle's size");
    // An arc's length dimension measures its sweep instead, at what the
    // arc measures, and back; a line's length takes neither.
    let arc = ok(
        &mut doc,
        "sketch.arc",
        json!({"sketch": s, "x": 20, "y": 0, "radius": 4, "start": 0, "end": 90}),
    );
    let along = ok(
        &mut doc,
        "sketch.constrain",
        json!({"sketch": s, "kind": "arc_length", "items": [arc]}),
    );
    let kind_of = |doc: &Document| {
        sketch_of(doc, &s)
            .constraints
            .iter()
            .find(|c| json!([c.id.to_string()]) == along)
            .map(|c| c.kind.clone())
    };
    let to = |doc: &mut Document, as_: &str| {
        call(
            doc,
            "sketch.set_constraint",
            json!({"sketch": s, "items": along, "as": as_}),
        )
    };
    to(&mut doc, "angle").unwrap();
    let held = kind_of(&doc);
    assert!(
        matches!(held, Some(ConstraintKind::ArcAngle { angle_rad, .. }) if (angle_rad - std::f64::consts::FRAC_PI_2).abs() < 1e-3),
        "{held:?}"
    );
    to(&mut doc, "length").unwrap();
    let held = kind_of(&doc);
    assert!(
        matches!(held, Some(ConstraintKind::ArcLength { length, .. }) if (length - std::f64::consts::TAU).abs() < 1e-2),
        "{held:?}"
    );
    assert!(
        call(
            &mut doc,
            "sketch.set_constraint",
            json!({"sketch": s, "items": length, "as": "angle"})
        )
        .is_err()
    );
}

/// Text in a font file named by its path keeps the font in the
/// document: changed after the file is gone, it is made again from the
/// kept copy, and a second text in the same file keeps no second one.
#[test]
fn a_font_file_is_kept_in_the_document() {
    let path = std::env::temp_dir().join(format!("printcad-font-{}.ttf", Uuid::new_v4()));
    std::fs::write(&path, crate::text::FONTS[2].1).unwrap();
    let font = path.to_string_lossy().to_string();
    let mut doc = Document::new("t");
    let s = ok(&mut doc, "sketch.new", json!({}));
    let fonts = |doc: &Document| {
        doc.assets()
            .filter(|a| a.metadata.get("kind").and_then(|k| k.as_str()) == Some("font"))
            .count()
    };
    let made = ok(
        &mut doc,
        "sketch.text",
        json!({"sketch": s, "text": "A", "at": [0, 0], "font": font}),
    );
    assert_eq!(fonts(&doc), 1);
    std::fs::remove_file(&path).unwrap();
    call(
        &mut doc,
        "sketch.text_edit",
        json!({"sketch": s, "block": made["text"], "text": "AB", "size": 14}),
    )
    .expect("made again from the kept font");
    let block = sketch_of(&doc, &s).texts[0].clone();
    assert_eq!((block.text.as_str(), block.size), ("AB", 14.0));
    assert!(block.font_asset.is_some());

    std::fs::write(&path, crate::text::FONTS[2].1).unwrap();
    ok(
        &mut doc,
        "sketch.text",
        json!({"sketch": s, "text": "C", "at": [0, 20], "font": font}),
    );
    assert_eq!(fonts(&doc), 1, "the same file is kept once");
    std::fs::remove_file(&path).unwrap();
    let missing = call(
        &mut doc,
        "sketch.text",
        json!({"sketch": s, "text": "D", "at": [0, 40], "font": "/nowhere/font.ttf"}),
    );
    assert!(
        missing
            .unwrap_err()
            .to_string()
            .contains("cannot read the font")
    );
}

/// A font found installed by its family is kept in the document under
/// the family the font gives, and text in it is made again from the
/// kept copy once the document is read back where no such font is
/// installed.
#[test]
fn an_installed_font_is_kept_and_reopens_without_it() {
    const FAMILY: &str = "Printcad Kept Family";
    let installed = |family: &str| {
        family
            .eq_ignore_ascii_case(FAMILY)
            .then(|| crate::text::InstalledFont {
                family: FAMILY.to_string(),
                bytes: crate::text::FONTS[2].1.to_vec(),
                index: 0,
                file: Some("kept.ttf".to_string()),
            })
    };
    let mut doc = Document::new("t");
    let s = ok(&mut doc, "sketch.new", json!({}));
    let mut spec = crate::text::TextSpec {
        text: "A".to_string(),
        font: "printcad kept family".to_string(),
        ..Default::default()
    };
    crate::text::load_font_from(&mut doc, &mut spec, &installed).unwrap();
    assert_eq!(spec.font, FAMILY, "the family as the font names it");
    let asset = spec.font_asset.expect("kept");
    assert_eq!(
        doc.get_asset(asset).unwrap().metadata["family"],
        json!(FAMILY)
    );
    let mut args = crate::text_args(&spec, None);
    args["sketch"] = s.clone();
    args["at"] = json!([0, 0]);
    let made = ok(&mut doc, "sketch.text", args);

    let bytes = doc.save_to_bytes(core_document::Compression::None).unwrap();
    let mut reopened = Document::load_from_bytes(bytes).unwrap();
    assert!(crate::text::installed_font(FAMILY).is_none());
    call(
        &mut reopened,
        "sketch.text_edit",
        json!({"sketch": s, "block": made["text"], "text": "AB"}),
    )
    .expect("made again from the kept font");
    let block = sketch_of(&reopened, &s).texts[0].clone();
    assert_eq!(block.text, "AB");
    assert_eq!(block.font, FAMILY);
    assert_eq!(block.font_asset, Some(asset));

    let missing = call(
        &mut reopened,
        "sketch.text",
        json!({"sketch": s, "text": "C", "at": [0, 20], "font": FAMILY}),
    );
    assert!(
        missing
            .unwrap_err()
            .to_string()
            .contains("is installed here"),
        "a family neither installed nor kept is refused with why"
    );
}

/// A text block's size set by a variable: the outlines the document
/// builds from are made again at its value, keeping their elements,
/// and the block's stored data is left as it was made.
#[test]
fn a_variable_sets_a_texts_size() {
    let registry = registry();
    let mut doc = Document::new("t");
    let s = ok(&mut doc, "sketch.new", json!({}));
    let made = ok(
        &mut doc,
        "sketch.text",
        json!({"sketch": s, "text": "I", "at": [0, 0], "size": 10}),
    );
    let block = made["text"].as_str().unwrap().to_string();
    let sizes = doc.add_variable_set("Label").unwrap();
    doc.set_variable(sizes, "height", "10 mm", None).unwrap();
    doc.set_feature_formula(
        feature_id(&s),
        format!("{block}/size"),
        Some("Label.height".into()),
    )
    .unwrap();
    let height = |doc: &Document| {
        let built = crate::stored_sketch(doc, feature_id(&s)).unwrap().sketch;
        let ys: Vec<f64> = crate::text::outline_points(&built)
            .iter()
            .filter_map(|p| built.point_position(*p))
            .map(|p| p.y)
            .collect();
        let elements = built.texts[0].elements.clone();
        let top = ys.iter().copied().fold(f64::MIN, f64::max);
        let bottom = ys.iter().copied().fold(f64::MAX, f64::min);
        (top - bottom, elements)
    };
    registry.evaluate(&mut doc);
    let (small, elements) = height(&doc);
    doc.set_variable(sizes, "height", "25 mm", None).unwrap();
    registry.evaluate(&mut doc);
    let (large, kept) = height(&doc);
    assert!((large / small - 2.5).abs() < 1e-6, "{small} then {large}");
    assert_eq!(kept, elements, "the same elements, reshaped");
    let stored = sketch_of(&doc, &s).texts[0].clone();
    assert_eq!(stored.size, 10.0, "the stored data is as it was made");
}

#[test]
fn constraint_kinds_and_flags_are_checked() {
    let mut doc = Document::new("t");
    let s = ok(&mut doc, "sketch.new", json!({}));
    let line = ok(
        &mut doc,
        "sketch.line",
        json!({"sketch": s, "x1": 0, "y1": 0, "x2": 5, "y2": 1}),
    );
    let unknown = call(
        &mut doc,
        "sketch.constrain",
        json!({"sketch": s, "kind": "bogus", "items": [line]}),
    )
    .unwrap_err()
    .to_string();
    assert!(
        unknown.contains("no constraint") && unknown.contains("horizontal"),
        "{unknown}"
    );
    let flat = ok(
        &mut doc,
        "sketch.constrain",
        json!({"sketch": s, "kind": "horizontal", "items": [line]}),
    );
    let set = |doc: &mut Document, args: Value| {
        let mut args = args;
        args["sketch"] = s.clone();
        call(doc, "sketch.set_constraint", args)
    };
    assert!(set(&mut doc, json!({"items": [line], "active": false})).is_err());
    assert!(set(&mut doc, json!({"items": flat, "driving": false})).is_err());
    assert!(set(&mut doc, json!({"items": flat, "active": false})).is_ok());
}

/// Two lengths for one line: the sketch does not solve, says so, and
/// gives no profile to build from until one goes.
#[test]
fn a_sketch_whose_constraints_conflict_gives_no_profile() {
    let mut doc = Document::new("t");
    let s = ok(&mut doc, "sketch.new", json!({}));
    let sides = ok(
        &mut doc,
        "sketch.rect",
        json!({"sketch": s, "x": 0, "y": 0, "width": 4, "height": 4}),
    );
    let length = |doc: &mut Document, value: f64| {
        ok(
            doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "distance", "items": [sides[0]], "value": value}),
        )
    };
    length(&mut doc, 4.0);
    let second = length(&mut doc, 8.0);
    let status = ok(&mut doc, "sketch.status", json!({"sketch": s}));
    assert_eq!(status["solved"], false);
    let why = crate::profile::extract_wires(&sketch_of(&doc, &s)).unwrap_err();
    assert!(why.to_string().contains("does not solve"), "{why}");
    ok(
        &mut doc,
        "sketch.delete",
        json!({"sketch": s, "items": second}),
    );
    assert!(crate::profile::extract_wires(&sketch_of(&doc, &s)).is_ok());
}

/// `ORDER` names every registered command once and nothing else, so a new
/// command takes its place in the scripting reference on purpose.
#[test]
fn the_order_names_every_command() {
    let mut registered = core_document::WorkbenchContext::default();
    register(&mut registered);
    let mut ids: Vec<&str> = registered
        .commands()
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let mut listed = ORDER.to_vec();
    ids.sort_unstable();
    listed.sort_unstable();
    assert_eq!(ids, listed);
}
