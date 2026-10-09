//! Borrowed geometry through the full stack: one body's sketch, faces and
//! edges taken by another body's features where the two bodies sit,
//! followed while live and kept while frozen, built by the kernel as the
//! app's recompute driver builds them.

use core_document::{BodyId, BodyPlacement, Document, FeatureId, WorkbenchFeature};
use kernel_api::TessellationSettings;
use kernel_ogeom::OgeomKernel;
use wb_design::{BorrowSource, BorrowedRef, DesignFeature, EdgePick, FacePick};
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{Circle, GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};

const PI: f64 = std::f64::consts::PI;

/// A rectangle from (x0, y0) to (x1, y1) on the XY plane.
fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> SketchFeature {
    let mut sketch = Sketch::new("r");
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
        .map(|(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))));
    for i in 0..4 {
        sketch.add_geometry(GeometryElement::Line(Line::new(
            corners[i],
            corners[(i + 1) % 4],
        )));
    }
    let plane = sketch.plane;
    SketchFeature::new(sketch, plane)
}

/// A circle on the XY plane.
fn circle(cx: f32, cy: f32, r: f32) -> SketchFeature {
    let mut sketch = Sketch::new("c");
    let center = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(cx, cy))));
    sketch.add_geometry(GeometryElement::Circle(Circle::new(center, r)));
    SketchFeature::new(sketch, SketchPlane::xy())
}

/// A pad or a pocket of `sketch` with every other field at its default,
/// then `fields` set the way `design.set` sets them.
fn extrude(kind: &str, sketch: FeatureId, fields: serde_json::Value) -> DesignFeature {
    let length = if kind == "Pad" { "length" } else { "depth" };
    let mut value = serde_json::json!({ kind: {
        "sketch": sketch.0.to_string(),
        length: 10.0,
        "reversed": false,
    }});
    let inner = value[kind].as_object_mut().unwrap();
    for (k, v) in fields.as_object().unwrap() {
        inner.insert(k.clone(), v.clone());
    }
    DesignFeature::from_json(&value).unwrap()
}

/// A body holding a `w` × `h` block `t` thick from its XY plane up.
fn block(doc: &mut Document, name: &str, w: f32, h: f32, t: f32) -> BodyId {
    let body = doc.create_body(Some(name.into()));
    let sketch = doc
        .add_feature_in_body(rect(0.0, 0.0, w, h), "Block sketch".into(), Some(body))
        .unwrap();
    doc.add_feature_in_body(
        extrude("Pad", sketch, serde_json::json!({ "length": t })),
        "Block".into(),
        Some(body),
    )
    .unwrap();
    body
}

/// Rebuild every body the way the host does until nothing is left to
/// rebuild; a failed build fails the test.
fn settle(doc: &mut Document, kernel: &mut OgeomKernel) {
    for _ in 0..16 {
        let jobs = wb_design::rebuild_jobs(doc);
        if jobs.is_empty() {
            return;
        }
        for job in jobs {
            let plan = job
                .plan
                .unwrap_or_else(|e| panic!("planning: {}", e.message));
            if plan.ops.is_empty() {
                continue;
            }
            let result = kernel
                .execute_solid_chain(&plan.ops, &TessellationSettings::default())
                .unwrap_or_else(|e| panic!("building: {e}"));
            doc.set_imported_brep_data(job.body, result.brep_blob, Vec::new());
            doc.set_imported_geometry(
                job.body,
                core_document::ImportedGeometry {
                    mesh: std::sync::Arc::new(result.mesh),
                    source_asset: None,
                    revision: 0,
                    bounds_mm: result.bounds_mm,
                    brep_blob_path: None,
                    mesh_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
        }
    }
    panic!("the rebuilds never settled");
}

fn volume(doc: &Document, kernel: &mut OgeomKernel, body: BodyId) -> f64 {
    let blob = doc.imported_brep_blob_arc(body).expect("a solid");
    kernel
        .physical_properties(&blob)
        .unwrap()
        .volume_mm3
        .expect("a closed solid")
}

fn assert_close(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() <= want.abs() * 1e-4 + 1e-6,
        "{what}: {got}, want {want}"
    );
}

/// A borrow of `source` in `body`.
fn borrow(doc: &mut Document, body: BodyId, source: BorrowSource) -> FeatureId {
    doc.add_feature_in_body(
        DesignFeature::Borrow {
            options: Default::default(),
            source,
            frozen: None,
        },
        "Borrowed".into(),
        Some(body),
    )
    .unwrap()
}

/// Freeze the borrow `id` as it is now, the way `design.freeze` does.
fn freeze(doc: &mut Document, id: FeatureId) {
    let body = doc.get_feature_meta(id).unwrap().body.unwrap();
    let Ok(DesignFeature::Borrow { source, .. }) =
        DesignFeature::from_json(doc.get_feature_data(id).unwrap())
    else {
        panic!("not a borrow");
    };
    let frozen = wb_design::freeze(
        doc,
        Some(&kernel_ogeom::QUERIES),
        body,
        &source,
        &Default::default(),
    )
    .unwrap();
    let feature = DesignFeature::Borrow {
        options: Default::default(),
        source,
        frozen: Some(frozen),
    };
    doc.update_feature_data(id, feature.to_json()).unwrap();
    doc.set_feature_dependencies(id, feature.dependencies());
    doc.mark_feature_dirty(id);
}

/// Change a sketch's data, as an edit in the sketcher would.
fn replace_sketch(doc: &mut Document, id: FeatureId, sketch: SketchFeature) {
    doc.update_feature_data(id, sketch.to_json()).unwrap();
    doc.mark_feature_dirty(id);
}

/// The area of a disc of radius `r` whose centre lies `d` from a line,
/// on the far side of the line from the centre.
fn segment_beyond(r: f64, d: f64) -> f64 {
    r * r * (d / r).acos() - d * (r * r - d * d).sqrt()
}

/// One hole through two bodies: body A's sketch drills A and, borrowed,
/// the pocket of body B sitting on top of it. The borrow follows the sketch
/// while live, and keeps the circle it took once frozen.
#[test]
fn a_borrowed_sketch_drives_a_pocket_in_another_body_live_or_frozen() {
    let mut doc = Document::new("t");
    let a = block(&mut doc, "A", 20.0, 20.0, 10.0);
    let hole = doc
        .add_feature_in_body(circle(10.0, 10.0, 3.0), "Hole sketch".into(), Some(a))
        .unwrap();
    doc.add_feature_in_body(
        extrude(
            "Pocket",
            hole,
            serde_json::json!({ "mode": "ThroughAll", "through_all": true, "reversed": true }),
        ),
        "Hole A".into(),
        Some(a),
    )
    .unwrap();
    // B sits on top of A.
    let b = block(&mut doc, "B", 20.0, 20.0, 10.0);
    doc.set_body_placement(
        b,
        BodyPlacement {
            translation: [0.0, 0.0, 10.0],
            ..BodyPlacement::IDENTITY
        },
    );
    let borrowed = borrow(&mut doc, b, BorrowSource::Sketch(hole));
    doc.add_feature_in_body(
        extrude(
            "Pocket",
            borrowed,
            serde_json::json!({ "mode": "ThroughAll", "through_all": true, "reversed": true }),
        ),
        "Hole B".into(),
        Some(b),
    )
    .unwrap();

    wb_design::mark_all_design_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    let drilled = |r: f64| 4000.0 - PI * r * r * 10.0;
    assert_close(volume(&doc, &mut kernel, a), drilled(3.0), "A drilled");
    assert_close(volume(&doc, &mut kernel, b), drilled(3.0), "B drilled");

    // Live: a smaller circle drills both.
    replace_sketch(&mut doc, hole, circle(10.0, 10.0, 2.0));
    assert!(
        wb_design::pending_body_rebuilds(&doc).contains(&b),
        "the sketch's change reaches the body that borrows it"
    );
    settle(&mut doc, &mut kernel);
    assert_close(volume(&doc, &mut kernel, b), drilled(2.0), "B follows");

    // Frozen: B keeps the circle it took.
    freeze(&mut doc, borrowed);
    settle(&mut doc, &mut kernel);
    replace_sketch(&mut doc, hole, circle(10.0, 10.0, 4.0));
    settle(&mut doc, &mut kernel);
    assert_close(volume(&doc, &mut kernel, a), drilled(4.0), "A follows");
    assert_close(volume(&doc, &mut kernel, b), drilled(2.0), "B keeps");
}

/// The borrowed sketch lies where the source body puts it, seen from the
/// borrowing body: moving or turning the borrowing body moves the hole
/// within it, while the frozen copy keeps its place in the body.
#[test]
fn a_borrowed_sketch_lies_where_the_bodies_sit() {
    let mut doc = Document::new("t");
    let a = doc.create_body(Some("A".into()));
    let hole = doc
        .add_feature_in_body(circle(10.0, 10.0, 3.0), "Hole sketch".into(), Some(a))
        .unwrap();
    let b = block(&mut doc, "B", 20.0, 20.0, 10.0);
    let borrowed = borrow(&mut doc, b, BorrowSource::Sketch(hole));
    doc.add_feature_in_body(
        extrude(
            "Pocket",
            borrowed,
            serde_json::json!({ "mode": "ThroughAll", "through_all": true, "reversed": true }),
        ),
        "Hole".into(),
        Some(b),
    )
    .unwrap();
    wb_design::mark_all_design_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    let full = 4000.0 - PI * 90.0;
    assert_close(volume(&doc, &mut kernel, b), full, "centred");

    // B moved 12 mm along x: in B's frame the circle's centre is at x = -2,
    // outside B, and only the sliver over x >= 0 is cut.
    doc.set_body_placement(
        b,
        BodyPlacement {
            translation: [12.0, 0.0, 0.0],
            ..BodyPlacement::IDENTITY
        },
    );
    settle(&mut doc, &mut kernel);
    let partial = 4000.0 - 10.0 * segment_beyond(3.0, 2.0);
    assert_close(volume(&doc, &mut kernel, b), partial, "moved");

    // B turned a quarter about z, and A's sketch at (-5, 2): B's frame has
    // it at (2, 5), inside B 2 mm from its x = 0 side, which cuts all but
    // the sliver beyond that side. Unturned, or turned the wrong way, it
    // would miss B altogether.
    replace_sketch(&mut doc, hole, circle(-5.0, 2.0, 3.0));
    let quarter = std::f32::consts::FRAC_PI_4;
    doc.set_body_placement(
        b,
        BodyPlacement {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, quarter.sin(), quarter.cos()],
        },
    );
    settle(&mut doc, &mut kernel);
    let turned = 4000.0 - 10.0 * (PI * 9.0 - segment_beyond(3.0, 2.0));
    assert_close(volume(&doc, &mut kernel, b), turned, "turned");

    // Frozen, a move of B carries the hole with it.
    freeze(&mut doc, borrowed);
    settle(&mut doc, &mut kernel);
    doc.set_body_placement(
        b,
        BodyPlacement {
            translation: [50.0, 50.0, 0.0],
            rotation: [0.0, 0.0, quarter.sin(), quarter.cos()],
        },
    );
    settle(&mut doc, &mut kernel);
    assert_close(volume(&doc, &mut kernel, b), turned, "frozen, moved");
}

/// A face of body A as the face body B's pad stops on, with the two bodies
/// placed apart: the pad runs up to where A's face is seen from B, follows
/// A as it moves, and stays once the face is frozen.
#[test]
fn a_borrowed_face_is_the_face_a_pad_in_another_body_stops_on() {
    let mut doc = Document::new("t");
    // A slab whose underside is at A's z = 0, A lifted to z = 30.
    let a = block(&mut doc, "A", 40.0, 40.0, 5.0);
    doc.set_body_placement(
        a,
        BodyPlacement {
            translation: [0.0, 0.0, 30.0],
            ..BodyPlacement::IDENTITY
        },
    );
    let b = doc.create_body(Some("B".into()));
    doc.set_body_placement(
        b,
        BodyPlacement {
            translation: [3.0, 4.0, 2.0],
            ..BodyPlacement::IDENTITY
        },
    );
    let underside = borrow(
        &mut doc,
        b,
        BorrowSource::Solid {
            body: a,
            faces: vec![FacePick {
                name: 0,
                point: [20.0, 20.0, 0.0],
                normal: [0.0, 0.0, -1.0],
            }],
            edges: Vec::new(),
        },
    );
    let square = doc
        .add_feature_in_body(rect(0.0, 0.0, 10.0, 10.0), "Square".into(), Some(b))
        .unwrap();
    let face = BorrowedRef {
        borrow: underside,
        index: 0,
    };
    doc.add_feature_in_body(
        extrude(
            "Pad",
            square,
            serde_json::json!({ "mode": { "UpToBorrowed": face } }),
        ),
        "Post".into(),
        Some(b),
    )
    .unwrap();

    wb_design::mark_all_design_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    // From B's z = 0 (world 2) up to world 30.
    assert_close(volume(&doc, &mut kernel, b), 100.0 * 28.0, "up to A");

    // A lifted: the post follows.
    doc.set_body_placement(
        a,
        BodyPlacement {
            translation: [0.0, 0.0, 40.0],
            ..BodyPlacement::IDENTITY
        },
    );
    let jobs = wb_design::rebuild_jobs(&mut doc);
    assert!(
        jobs.iter().any(|job| job.body == b),
        "moving the source rebuilds the body that borrows from it"
    );
    for job in jobs {
        let plan = job.plan.unwrap();
        let result = kernel
            .execute_solid_chain(&plan.ops, &TessellationSettings::default())
            .unwrap();
        doc.set_imported_brep_data(job.body, result.brep_blob, Vec::new());
    }
    assert_close(volume(&doc, &mut kernel, b), 100.0 * 38.0, "follows A");

    // Frozen: A moves on, the post stays.
    freeze(&mut doc, underside);
    settle(&mut doc, &mut kernel);
    doc.set_body_placement(
        a,
        BodyPlacement {
            translation: [0.0, 0.0, 20.0],
            ..BodyPlacement::IDENTITY
        },
    );
    settle(&mut doc, &mut kernel);
    assert_close(volume(&doc, &mut kernel, b), 100.0 * 38.0, "frozen");
}

/// A curved borrowed face stops the pad exactly on its surface: a post
/// under a lying cylinder of another body ends on the cylinder's underside,
/// frozen or live.
#[test]
fn a_pad_stops_on_a_curved_borrowed_face() {
    let mut doc = Document::new("t");
    let a = doc.create_body(Some("A".into()));
    doc.add_feature_in_body(
        DesignFeature::Primitive {
            attached: None,
            kind: kernel_api::PrimitiveKind::Cylinder {
                radius: 10.0,
                height: 40.0,
                angle_deg: 360.0,
            },
            placement: kernel_api::Placement {
                origin: [-20.0, 0.0, 30.0],
                x_axis: [0.0, 1.0, 0.0],
                z_axis: [1.0, 0.0, 0.0],
            },
            subtractive: false,
            refine: false,
        },
        "Roller".into(),
        Some(a),
    )
    .unwrap();
    let b = doc.create_body(Some("B".into()));
    doc.set_body_placement(
        b,
        BodyPlacement {
            translation: [0.0, 0.0, -5.0],
            ..BodyPlacement::IDENTITY
        },
    );
    let under = borrow(
        &mut doc,
        b,
        BorrowSource::Solid {
            body: a,
            faces: vec![FacePick {
                name: 0,
                point: [2.0, 0.0, 20.0],
                normal: [0.0, 0.0, -1.0],
            }],
            edges: Vec::new(),
        },
    );
    let square = doc
        .add_feature_in_body(rect(0.0, -2.0, 4.0, 2.0), "Square".into(), Some(b))
        .unwrap();
    doc.add_feature_in_body(
        extrude(
            "Pad",
            square,
            serde_json::json!({
                "mode": { "UpToBorrowed": BorrowedRef { borrow: under, index: 0 } },
            }),
        ),
        "Post".into(),
        Some(b),
    )
    .unwrap();
    wb_design::mark_all_design_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    // Height at y: from B's z = 0 (world -5) up to 30 - sqrt(100 - y²).
    let (r, half) = (10.0f64, 2.0f64);
    let arc = half * (r * r - half * half).sqrt() + r * r * (half / r).asin();
    let want = 4.0 * (35.0 * 2.0 * half - arc);
    assert_close(volume(&doc, &mut kernel, b), want, "live");

    freeze(&mut doc, under);
    doc.set_body_placement(
        a,
        BodyPlacement {
            translation: [0.0, 0.0, 10.0],
            ..BodyPlacement::IDENTITY
        },
    );
    settle(&mut doc, &mut kernel);
    assert_close(volume(&doc, &mut kernel, b), want, "frozen");
}

/// A pad runs along an edge another body lends, turned as that body is
/// turned: a vertical edge of a body tipped 45° about x runs the pad at
/// 45° to its sketch, so it rises only cos 45° of its length.
#[test]
fn a_pad_runs_along_a_borrowed_edge_where_its_body_turns_it() {
    let mut doc = Document::new("t");
    let a = block(&mut doc, "A", 20.0, 20.0, 10.0);
    let eighth = std::f32::consts::FRAC_PI_8;
    doc.set_body_placement(
        a,
        BodyPlacement {
            translation: [0.0; 3],
            rotation: [eighth.sin(), 0.0, 0.0, eighth.cos()],
        },
    );
    let b = doc.create_body(Some("B".into()));
    let lent = borrow(
        &mut doc,
        b,
        BorrowSource::Solid {
            body: a,
            faces: Vec::new(),
            edges: vec![EdgePick {
                faces: [0, 0],
                point: [0.0, 0.0, 5.0],
                direction: [0.0, 0.0, 1.0],
            }],
        },
    );
    let square = doc
        .add_feature_in_body(rect(0.0, 0.0, 10.0, 10.0), "Square".into(), Some(b))
        .unwrap();
    let edge = BorrowedRef {
        borrow: lent,
        index: 0,
    };
    doc.add_feature_in_body(
        extrude(
            "Pad",
            square,
            serde_json::json!({ "direction": { "Borrowed": edge } }),
        ),
        "Leaning".into(),
        Some(b),
    )
    .unwrap();
    wb_design::mark_all_design_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    let want = 100.0 * 10.0 * std::f64::consts::FRAC_1_SQRT_2;
    assert_close(volume(&doc, &mut kernel, b), want, "leaning");
}

/// A borrow of a sketch is offered, and taken, as a profile of the
/// borrowing body: the command that makes a pocket accepts it as its
/// sketch.
#[test]
fn a_borrowed_sketch_is_a_profile_of_the_borrowing_body() {
    let mut doc = Document::new("t");
    let a = doc.create_body(Some("A".into()));
    let hole = doc
        .add_feature_in_body(circle(10.0, 10.0, 3.0), "Hole sketch".into(), Some(a))
        .unwrap();
    let b = block(&mut doc, "B", 20.0, 20.0, 10.0);
    let borrowed = borrow(&mut doc, b, BorrowSource::Sketch(hole));
    let listed = wb_design::sketches_of_body(&doc, b);
    assert!(listed.iter().any(|(id, _)| *id == borrowed), "{listed:?}");
    assert!(
        !listed.iter().any(|(id, _)| *id == hole),
        "A's own stays A's"
    );
}

/// Two bodies that borrow faces from each other would rebuild each other
/// forever: their borrows follow nothing, everything settles, and a
/// feature that stops on one of them says why it cannot.
#[test]
fn bodies_that_borrow_from_each_other_settle_and_say_so() {
    let mut doc = Document::new("t");
    let a = block(&mut doc, "A", 20.0, 20.0, 10.0);
    let b = block(&mut doc, "B", 20.0, 20.0, 10.0);
    let top = |body| BorrowSource::Solid {
        body,
        faces: vec![FacePick {
            name: 0,
            point: [10.0, 10.0, 10.0],
            normal: [0.0, 0.0, 1.0],
        }],
        edges: Vec::new(),
    };
    let from_a = borrow(&mut doc, b, top(a));
    borrow(&mut doc, a, top(b));
    wb_design::mark_all_design_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);

    let square = doc
        .add_feature_in_body(rect(0.0, 0.0, 5.0, 5.0), "Square".into(), Some(b))
        .unwrap();
    let face = BorrowedRef {
        borrow: from_a,
        index: 0,
    };
    let post = doc
        .add_feature_in_body(
            extrude(
                "Pad",
                square,
                serde_json::json!({ "mode": { "UpToBorrowed": face } }),
            ),
            "Post".into(),
            Some(b),
        )
        .unwrap();
    doc.mark_feature_dirty(post);
    let jobs = wb_design::rebuild_jobs(&mut doc);
    let job = jobs.iter().find(|job| job.body == b).expect("B rebuilds");
    let plan = job.plan.as_ref().expect("B builds what comes before");
    let error = plan.failed.as_ref().expect("the post is refused");
    assert_eq!(error.feature, Some(post));
    assert!(error.message.contains("in turn"), "{}", error.message);
    assert!(
        wb_design::rebuild_jobs(&mut doc).is_empty(),
        "nothing comes back"
    );
}

/// A borrow moved by its offset: the borrowed circle shifted to cross the
/// block's edge drills only what of it lies inside.
#[test]
fn an_offset_moves_what_a_borrow_lends() {
    let mut doc = Document::new("t");
    let a = doc.create_body(Some("A".into()));
    let hole = doc
        .add_feature_in_body(circle(10.0, 10.0, 3.0), "Hole sketch".into(), Some(a))
        .unwrap();
    let b = block(&mut doc, "B", 20.0, 20.0, 10.0);
    let options = wb_design::BorrowOptions {
        offset: core_document::AttachmentOffset {
            translation: [8.0, 0.0, 0.0],
            ..Default::default()
        },
        ..Default::default()
    };
    let borrowed = doc
        .add_feature_in_body(
            DesignFeature::Borrow {
                source: BorrowSource::Sketch(hole),
                frozen: None,
                options,
            },
            "Borrowed".into(),
            Some(b),
        )
        .unwrap();
    doc.add_feature_in_body(
        extrude(
            "Pocket",
            borrowed,
            serde_json::json!({ "mode": "ThroughAll", "through_all": true, "reversed": true }),
        ),
        "Hole".into(),
        Some(b),
    )
    .unwrap();
    wb_design::mark_all_design_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    // Centred at x = 18, the circle runs 1 mm past the block's side.
    let inside = PI * 9.0 - segment_beyond(3.0, 2.0);
    assert_close(
        volume(&doc, &mut kernel, b),
        4000.0 - inside * 10.0,
        "shifted",
    );
}

/// Edges borrowed from another body's solid that close into a loop lend
/// the face they bound: a body pads it.
#[test]
fn closed_borrowed_edges_fill_to_a_face_a_pad_takes() {
    let mut doc = Document::new("t");
    let a = block(&mut doc, "A", 20.0, 20.0, 10.0);
    let b = doc.create_body(Some("B".into()));
    let top = [
        ([10.0, 0.0, 10.0], [1.0, 0.0, 0.0]),
        ([20.0, 10.0, 10.0], [0.0, 1.0, 0.0]),
        ([10.0, 20.0, 10.0], [1.0, 0.0, 0.0]),
        ([0.0, 10.0, 10.0], [0.0, 1.0, 0.0]),
    ];
    let edges = top
        .iter()
        .map(|&(point, direction)| EdgePick {
            faces: [0, 0],
            point,
            direction,
        })
        .collect();
    let borrowed = doc
        .add_feature_in_body(
            DesignFeature::Borrow {
                source: BorrowSource::Solid {
                    body: a,
                    faces: Vec::new(),
                    edges,
                },
                frozen: None,
                options: wb_design::BorrowOptions {
                    fill: true,
                    ..Default::default()
                },
            },
            "Top".into(),
            Some(b),
        )
        .unwrap();
    doc.add_feature_in_body(
        extrude("Pad", borrowed, serde_json::json!({ "length": 5.0 })),
        "Slab".into(),
        Some(b),
    )
    .unwrap();
    wb_design::mark_all_design_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    assert_close(volume(&doc, &mut kernel, b), 2000.0, "the top padded");
}

/// A face another body lends is a pad's profile, live and frozen: the
/// pad stands on it where the two bodies sit.
#[test]
fn a_borrowed_face_is_a_pad_s_profile() {
    for frozen in [false, true] {
        let mut kernel = OgeomKernel::new();
        let mut doc = Document::new("t");
        let a = block(&mut doc, "A", 20.0, 20.0, 10.0);
        let b = doc.create_body(Some("B".into()));
        wb_design::mark_all_design_features_dirty(&mut doc);
        settle(&mut doc, &mut kernel);
        let top = borrow(
            &mut doc,
            b,
            BorrowSource::Solid {
                body: a,
                faces: vec![FacePick {
                    name: 0,
                    point: [10.0, 10.0, 10.0],
                    normal: [0.0, 0.0, 1.0],
                }],
                edges: Vec::new(),
            },
        );
        if frozen {
            freeze(&mut doc, top);
        }
        let pad = serde_json::json!({ "Pad": {
            "sketch": null,
            "length": 5.0,
            "reversed": false,
            "profile_borrowed": { "borrow": top.0.to_string(), "index": 0 },
        }});
        doc.add_feature_in_body(
            DesignFeature::from_json(&pad).unwrap(),
            "Pad".into(),
            Some(b),
        )
        .unwrap();
        wb_design::mark_all_design_features_dirty(&mut doc);
        settle(&mut doc, &mut kernel);
        assert_close(
            volume(&doc, &mut kernel, b),
            20.0 * 20.0 * 5.0,
            "a pad on A's top",
        );
    }
}

/// Up to shape takes borrowed faces as stop faces: a first pad of body B
/// under two slabs of body A at different heights ends on each where it
/// runs under it, with no material of its own to stop on.
#[test]
fn an_up_to_shape_pad_stops_on_borrowed_faces() {
    let mut doc = Document::new("t");
    // Two slabs: one over x 0..10 at z 20, one over x 10..20 at z 30.
    let a = doc.create_body(Some("A".into()));
    for (x0, z) in [(0.0f32, 20.0f32), (10.0, 30.0)] {
        let mut slab = rect(x0, 0.0, x0 + 10.0, 10.0);
        slab.plane = SketchPlane::from_frame([0.0, 0.0, z], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        slab.sketch.plane = slab.plane;
        let sketch = doc
            .add_feature_in_body(slab, "Slab sketch".into(), Some(a))
            .unwrap();
        doc.add_feature_in_body(
            extrude("Pad", sketch, serde_json::json!({ "length": 5.0 })),
            "Slab".into(),
            Some(a),
        )
        .unwrap();
    }
    let b = doc.create_body(Some("B".into()));
    let unders = borrow(
        &mut doc,
        b,
        BorrowSource::Solid {
            body: a,
            faces: vec![
                FacePick {
                    name: 0,
                    point: [5.0, 5.0, 20.0],
                    normal: [0.0, 0.0, -1.0],
                },
                FacePick {
                    name: 0,
                    point: [15.0, 5.0, 30.0],
                    normal: [0.0, 0.0, -1.0],
                },
            ],
            edges: Vec::new(),
        },
    );
    let square = doc
        .add_feature_in_body(rect(0.0, 0.0, 20.0, 10.0), "Base".into(), Some(b))
        .unwrap();
    let both = [0, 1].map(|index| BorrowedRef {
        borrow: unders,
        index,
    });
    doc.add_feature_in_body(
        extrude(
            "Pad",
            square,
            serde_json::json!({
                "mode": "UpToShape",
                "extras": { "up_to_shape_borrowed": both },
            }),
        ),
        "Posts".into(),
        Some(b),
    )
    .unwrap();
    wb_design::mark_all_design_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    assert_close(
        volume(&doc, &mut kernel, b),
        100.0 * 20.0 + 100.0 * 30.0,
        "each half up to the slab over it",
    );
}

/// A revolution turns until it meets a face another body lends: a
/// profile beside the Z axis, turned about it, stops on body A's side
/// that holds the axis a quarter turn on.
#[test]
fn a_revolution_stops_on_a_borrowed_face() {
    let mut doc = Document::new("t");
    // A block over x -20..0, y 0..20, its side at x = 0 facing +x.
    let a = doc.create_body(Some("A".into()));
    let side = doc
        .add_feature_in_body(rect(-20.0, 0.0, 0.0, 20.0), "Block sketch".into(), Some(a))
        .unwrap();
    doc.add_feature_in_body(
        extrude("Pad", side, serde_json::json!({ "length": 10.0 })),
        "Block".into(),
        Some(a),
    )
    .unwrap();
    let b = doc.create_body(Some("B".into()));
    let wall = borrow(
        &mut doc,
        b,
        BorrowSource::Solid {
            body: a,
            faces: vec![FacePick {
                name: 0,
                point: [0.0, 10.0, 5.0],
                normal: [1.0, 0.0, 0.0],
            }],
            edges: Vec::new(),
        },
    );
    // On XZ: x 5..10, z 0..4, turned about the sketch's y, world Z.
    let mut profile = rect(5.0, 0.0, 10.0, 4.0);
    profile.plane = SketchPlane::xz();
    profile.sketch.plane = profile.plane;
    let profile = doc
        .add_feature_in_body(profile, "Profile".into(), Some(b))
        .unwrap();
    let revolution = serde_json::json!({ "Revolution": {
        "sketch": profile.0.to_string(),
        "angle_deg": 360.0,
        "mode": { "UpToBorrowed": BorrowedRef { borrow: wall, index: 0 } },
    }});
    doc.add_feature_in_body(
        DesignFeature::from_json(&revolution).unwrap(),
        "Turned".into(),
        Some(b),
    )
    .unwrap();
    wb_design::mark_all_design_features_dirty(&mut doc);
    let mut kernel = OgeomKernel::new();
    settle(&mut doc, &mut kernel);
    let quarter = PI / 4.0 * (100.0 - 25.0) * 4.0;
    assert_close(volume(&doc, &mut kernel, b), quarter, "a quarter turn");
}
