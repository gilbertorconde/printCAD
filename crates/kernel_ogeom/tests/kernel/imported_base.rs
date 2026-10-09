//! An imported solid takes features: the first one gives the body a base
//! shape, the imported solid, and builds on it, as a primitive's body
//! builds on the primitive.

use core_document::WorkbenchFeature as _;
use core_document::{
    BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, Document, DocumentService,
    ImportedGeometry, WorkbenchId, WorkbenchRuntimeContext,
};
use kernel_api::{Kernel, TessellationSettings};
use kernel_ogeom::OgeomKernel;
use scripting::ScriptEngine;
use wb_design::DesignFeature;

pub(crate) struct Benches {
    registry: DocumentService,
    pub(crate) document: Document,
}

impl scripting::Host for Benches {
    fn commands(&self) -> Vec<CommandSpec> {
        self.registry
            .commands()
            .into_iter()
            .map(|(_, c)| c.clone())
            .collect()
    }

    fn call(&mut self, id: &str, args: CommandArgs) -> CommandResult {
        let (bench, spec): (WorkbenchId, _) = self
            .registry
            .command(id)
            .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
        spec.check(&args)?;
        let id = spec.id.clone();
        let wb = self.registry.workbench_mut(&bench).unwrap();
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.document, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.kernel = Some(&kernel_ogeom::QUERIES);
        wb.run_command(&id, &args, &mut ctx)
    }
}

/// The bundled STEP box, in a document as an import leaves it.
fn imported_box() -> (Document, BodyId, [f32; 3], [f32; 3]) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/box_native.step");
    let model = OgeomKernel::new()
        .import_step(&path, &TessellationSettings::default())
        .unwrap();
    let imported = &model.bodies[0];
    let mut document = Document::new("imported");
    let body = document.create_body(Some("Box".into()));
    let mesh = std::sync::Arc::new(imported.mesh.clone());
    let (lo, hi) = mesh.bounds().unwrap();
    document.set_imported_geometry(
        body,
        ImportedGeometry {
            bounds_mm: mesh.bounds(),
            mesh,
            // Any asset: what matters is that the import stamped one.
            source_asset: Some(body.0),
            revision: 0,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: None,
        },
    );
    document.set_imported_brep_data(body, imported.brep_blob.clone(), Vec::new());
    (document, body, lo, hi)
}

pub(crate) fn benches(document: Document) -> Benches {
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
        .unwrap();
    registry
        .register_workbench(Box::new(wb_design::DesignWorkbench::default()))
        .unwrap();
    Benches { registry, document }
}

#[test]
fn a_pocket_on_an_imported_box_builds_on_the_box() {
    let (document, body, lo, hi) = imported_box();
    assert!(document.body_solid_is_imported(body));
    let mut host = benches(document);
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"
            local s = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.rect{{sketch = s, x = {x}, y = {y}, width = 2, height = 2}}
            pocket = pc.design.pocket{{sketch = s, depth = 1}}
            "#,
            body = body.0,
            top = hi[2],
            x = cx - 1.0,
            y = cy - 1.0,
        ),
        "pocket.lua",
        &mut host,
    );
    assert_eq!(out.error, None);

    // The body is the one imported, its history the box and then the
    // pocket.
    let document = &host.document;
    assert_eq!(document.bodies().len(), 1, "no second body");
    assert!(document.has_base_solid(body));
    let features = wb_design::design_features_of_body(document, body);
    assert!(matches!(features[0].1, DesignFeature::Base {}));
    assert!(matches!(features[1].1, DesignFeature::Pocket { .. }));

    let ops = wb_design::body_build_ops(document, body).unwrap().ops;
    let built = OgeomKernel::new()
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let (blo, bhi) = built.mesh.bounds().unwrap();
    for axis in 0..3 {
        assert!((blo[axis] - lo[axis]).abs() < 1e-3 && (bhi[axis] - hi[axis]).abs() < 1e-3);
    }
    let whole = OgeomKernel::new()
        .execute_solid_chain(&ops[..1], &TessellationSettings::default())
        .unwrap();
    let volume = |r: &kernel_api::SolidBuildResult| {
        OgeomKernel::new()
            .physical_properties(&r.brep_blob)
            .unwrap()
            .volume_mm3
            .expect("a closed solid")
    };
    let cut = volume(&whole) - volume(&built);
    assert!((cut - 4.0).abs() < 1e-3, "a 2 × 2 × 1 pocket: {cut}");
}

#[test]
fn the_base_goes_last_and_takes_the_body_back_to_its_import() {
    let (document, body, _, hi) = imported_box();
    let mut host = benches(document);
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"
            local s = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.rect{{sketch = s, x = 0, y = 0, width = 1, height = 1}}
            pocket = pc.design.pocket{{sketch = s, depth = 0.5}}
            "#,
            body = body.0,
            top = hi[2],
        ),
        "pocket.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let features = wb_design::design_features_of_body(&host.document, body);
    let (base, pocket) = (features[0].0, features[1].0);
    assert!(
        !wb_design::delete_feature(&mut host.document, base),
        "the pocket builds on it"
    );
    assert!(wb_design::delete_feature(&mut host.document, pocket));
    assert!(wb_design::delete_feature(&mut host.document, base));
    assert!(!host.document.has_base_solid(body));
    assert!(
        host.document.body_solid_is_imported(body),
        "the imported box again"
    );
    assert!(host.document.imported_brep_blob(body).is_some());
}

#[test]
fn a_step_file_reads_as_its_first_solid() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/box_native.step");
    let read = OgeomKernel::new()
        .read_solid(&path, &TessellationSettings::default())
        .unwrap();
    assert!(read.closed);
    assert!(read.brep_blob.starts_with(b"ogeom"));
    assert!(read.mesh.bounds().is_some());
}

/// A STEP file's solid is read as it is, not converted from its mesh: a
/// screw keeps its cylinders.
#[test]
fn a_step_file_with_curved_faces_reads_exact() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/M5x16_BHCS_original_encoding.step");
    let read = OgeomKernel::new()
        .read_solid(&path, &TessellationSettings::default())
        .unwrap();
    assert!(!read.health.faceted, "read as facets: {:?}", read.summary);
    let text = String::from_utf8_lossy(&read.brep_blob);
    assert!(text.contains("Cylinder") || text.contains("cylinder"));
}

#[test]
fn deleting_a_bore_closes_it_again() {
    let (document, body, lo, hi) = imported_box();
    let mut host = benches(document);
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"
            local s = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.circle{{sketch = s, x = {cx}, y = {cy}, radius = 2}}
            pc.design.pocket{{sketch = s, through_all = true}}
            deleted = pc.design.delete_faces{{body = "{body}",
                face_point = {{{bx}, {cy}, {mz}}}, face_normal = {{-1, 0, 0}}}}
            "#,
            body = body.0,
            top = hi[2],
            cx = cx,
            cy = cy,
            bx = cx + 2.0,
            mz = (lo[2] + hi[2]) / 2.0,
        ),
        "bore.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let ops = wb_design::body_build_ops(&host.document, body).unwrap().ops;
    assert!(matches!(
        ops.last(),
        Some(kernel_api::SolidOp::RemoveFaces { .. })
    ));
    let volume = |ops: &[kernel_api::SolidOp]| {
        let built = OgeomKernel::new()
            .execute_solid_chain(ops, &TessellationSettings::default())
            .unwrap();
        OgeomKernel::new()
            .physical_properties(&built.brep_blob)
            .unwrap()
            .volume_mm3
            .unwrap()
    };
    let whole = volume(&ops[..1]);
    let bored = volume(&ops[..ops.len() - 1]);
    let closed = volume(&ops);
    assert!(
        bored < whole - 1.0,
        "the bore takes material: {bored} of {whole}"
    );
    assert!(
        (closed - whole).abs() < 1e-3,
        "closed again: {closed} of {whole}"
    );
}

/// Holes drilled with the Hole feature into the imported box, found again
/// in the solid they made.
#[test]
fn drilled_holes_are_recognized_as_they_were_drilled() {
    use kernel_api::KernelQueries;
    let (document, body, lo, hi) = imported_box();
    let mut host = benches(document);
    let top = hi[2];
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"
            local s = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.point{{sketch = s, x = {ax}, y = {y}}}
            pc.design.hole{{sketch = s, diameter = 3, through_all = true}}
            local b = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.point{{sketch = b, x = {bx}, y = {y}}}
            pc.design.hole{{sketch = b, diameter = 4, depth = 5}}
            local p = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.point{{sketch = p, x = {px}, y = {y}}}
            pc.design.hole{{sketch = p, diameter = 5, depth = 6,
                drill_point = {{Angled = {{angle_deg = 118}}}}}}
            "#,
            body = body.0,
            top = top,
            ax = lo[0] + 4.0,
            bx = lo[0] + 10.0,
            px = lo[0] + 16.0,
            y = (lo[1] + hi[1]) / 2.0,
        ),
        "holes.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let ops = wb_design::body_build_ops(&host.document, body).unwrap().ops;
    let built = OgeomKernel::new()
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let (mut holes, unknown) = kernel_ogeom::QUERIES
        .recognize_holes(&built.brep_blob)
        .unwrap();
    assert_eq!(unknown, 0);
    holes.sort_by(|a, b| a.diameter.total_cmp(&b.diameter));
    assert_eq!(holes.len(), 3, "{holes:#?}");
    let near = |a: f64, b: f64| (a - b).abs() < 1e-3;

    let through = &holes[0];
    assert!(near(through.diameter, 3.0) && through.through);
    assert!(near(through.depth, f64::from(hi[2] - lo[2])));

    let flat = &holes[1];
    assert!(near(flat.diameter, 4.0) && !flat.through);
    assert!(near(flat.depth, 5.0), "{flat:?}");
    assert!(near(flat.entry[2], f64::from(top)) && near(flat.direction[2], -1.0));
    assert_eq!(flat.drill_point_deg, None);
    assert_eq!(flat.faces.len(), 2, "the bore and its bottom");

    let pointed = &holes[2];
    assert!(
        near(pointed.diameter, 5.0) && near(pointed.depth, 6.0),
        "{pointed:?}"
    );
    assert!(near(pointed.drill_point_deg.unwrap(), 118.0), "{pointed:?}");
}

/// A solid with holes, as a file would bring it: recognized, its holes
/// become Hole features that build the same solid, and whose sizes change
/// it.
#[test]
fn recognized_holes_rebuild_the_solid_and_take_new_sizes() {
    let volume = |blob: &[u8]| {
        OgeomKernel::new()
            .physical_properties(blob)
            .unwrap()
            .volume_mm3
            .unwrap()
    };
    // The holed box, built once and then kept as an imported shape.
    let (document, body, lo, hi) = imported_box();
    let mut maker = benches(document);
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"
            local s = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.point{{sketch = s, x = {ax}, y = {y}}}
            pc.sketch.point{{sketch = s, x = {bx}, y = {y}}}
            pc.design.hole{{sketch = s, diameter = 3, through_all = true}}
            local p = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.point{{sketch = p, x = {px}, y = {y}}}
            pc.design.hole{{sketch = p, diameter = 5, depth = 6,
                drill_point = {{Angled = {{angle_deg = 118}}}}}}
            "#,
            body = body.0,
            top = hi[2],
            ax = lo[0] + 4.0,
            bx = lo[0] + 10.0,
            px = lo[0] + 16.0,
            y = (lo[1] + hi[1]) / 2.0,
        ),
        "holes.lua",
        &mut maker,
    );
    assert_eq!(out.error, None);
    let ops = wb_design::body_build_ops(&maker.document, body)
        .unwrap()
        .ops;
    let holed = OgeomKernel::new()
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();

    let mut document = Document::new("holed");
    let part = document.create_body(Some("Holed".into()));
    let mesh = std::sync::Arc::new(holed.mesh.clone());
    document.set_imported_geometry(
        part,
        ImportedGeometry {
            bounds_mm: mesh.bounds(),
            mesh,
            source_asset: Some(part.0),
            revision: 0,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: None,
        },
    );
    document.set_imported_brep_data(part, holed.brep_blob.clone(), Vec::new());
    let mut host = benches(document);
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"
            local made = pc.design.recognize_holes{{body = "{part}"}}
            assert(made.holes == 3, "three holes: " .. made.holes)
            assert(made.left == 0)
            "#,
            part = part.0
        ),
        "recognize.lua",
        &mut host,
    );
    assert_eq!(out.error, None);

    let features = wb_design::design_features_of_body(&host.document, part);
    let holes: Vec<_> = features
        .iter()
        .filter(|(_, f)| matches!(f, DesignFeature::Hole { .. }))
        .collect();
    assert_eq!(holes.len(), 2, "the two through holes are one feature");
    let build = |doc: &Document| {
        let ops = wb_design::body_build_ops(doc, part).unwrap().ops;
        OgeomKernel::new()
            .execute_solid_chain(&ops, &TessellationSettings::default())
            .unwrap()
    };
    let rebuilt = build(&host.document);
    assert!(
        (volume(&rebuilt.brep_blob) - volume(&holed.brep_blob)).abs() < 1e-2,
        "the same solid again: {} and {}",
        volume(&rebuilt.brep_blob),
        volume(&holed.brep_blob)
    );

    // A recognized size is a number to change.
    let through = holes
        .iter()
        .find(|(_, f)| {
            matches!(
                f,
                DesignFeature::Hole {
                    through_all: true,
                    ..
                }
            )
        })
        .unwrap()
        .0;
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"pc.design.set{{feature = "{}", diameter = 4}}"#,
            through.0
        ),
        "resize.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let resized = build(&host.document);
    let depth = f64::from(hi[2] - lo[2]);
    let expected = 2.0 * std::f64::consts::PI * (2.0f64.powi(2) - 1.5f64.powi(2)) * depth;
    let taken = volume(&rebuilt.brep_blob) - volume(&resized.brep_blob);
    assert!(
        (taken - expected).abs() < 1e-2,
        "{taken} against {expected}"
    );
}

/// Faces of an imported solid pushed, pulled and moved, the faces around
/// them following.
#[test]
fn faces_offset_and_move_with_their_neighbours_following() {
    let (document, body, lo, hi) = imported_box();
    let mut host = benches(document);
    let size = |i: usize| f64::from(hi[i] - lo[i]);
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"
            local s = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.circle{{sketch = s, x = {cx}, y = {cy}, radius = 2}}
            pc.design.pocket{{sketch = s, through_all = true}}
            top = pc.design.offset_faces{{body = "{body}", face_point = {{{fx}, {fy}, {top}}},
                face_normal = {{0, 0, 1}}, distance = 2}}
            bore = pc.design.offset_faces{{body = "{body}", face_point = {{{bx}, {cy}, {mz}}},
                face_normal = {{-1, 0, 0}}, distance = -0.5}}
            side = pc.design.move_faces{{body = "{body}", face_point = {{{right}, {cy}, {mz}}},
                face_normal = {{1, 0, 0}}, translation = {{3, 0, 0}}}}
            "#,
            body = body.0,
            top = hi[2],
            cx = cx,
            cy = cy,
            fx = lo[0] + 1.0,
            fy = lo[1] + 1.0,
            bx = cx + 2.0,
            mz = (lo[2] + hi[2]) / 2.0,
            right = hi[0],
        ),
        "faces.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let ops = wb_design::body_build_ops(&host.document, body).unwrap().ops;
    let built = |n: usize| {
        OgeomKernel::new()
            .execute_solid_chain(&ops[..n], &TessellationSettings::default())
            .unwrap()
    };
    let volume = |r: &kernel_api::SolidBuildResult| {
        OgeomKernel::new()
            .physical_properties(&r.brep_blob)
            .unwrap()
            .volume_mm3
            .unwrap()
    };
    let near = |a: f64, b: f64| (a - b).abs() < 1e-2;
    let n = ops.len();
    let (bored, raised, widened, moved) = (built(n - 3), built(n - 2), built(n - 1), built(n));
    let area = size(0) * size(1) - std::f64::consts::PI * 4.0;
    assert!(
        near(volume(&raised) - volume(&bored), area * 2.0),
        "the top up 2 mm"
    );
    let height = size(2) + 2.0;
    let wider = std::f64::consts::PI * (2.5f64.powi(2) - 2.0f64.powi(2)) * height;
    assert!(
        near(volume(&raised) - volume(&widened), wider),
        "the bore Ø4 to Ø5"
    );
    let (mlo, mhi) = moved.mesh.bounds().unwrap();
    assert!(
        near(f64::from(mhi[0] - mlo[0]), size(0) + 3.0),
        "the side out 3 mm"
    );
    assert!(near(f64::from(mhi[2] - mlo[2]), height));
}

/// A sketch placed on a side face of an imported solid stands on that
/// face once the document is worked out, not on an origin plane.
#[test]
fn a_sketch_on_an_imported_side_face_stays_on_that_face() {
    let (document, body, lo, hi) = imported_box();
    let mut host = benches(document);
    let point = [hi[0], (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let face = core_document::FaceRef {
        point,
        normal: [1.0, 0.0, 0.0],
        surface: None,
        name: 0,
    };
    let plane = wb_sketch::sketch::SketchPlane::from_face(point, face.normal);
    let mut sketch = wb_sketch::sketch::Sketch::new("side");
    sketch.plane = plane;
    let mut sketch = wb_sketch::SketchFeature::new(sketch, plane);
    sketch.face =
        wb_sketch::FaceSupport::from_origin(&face, core_document::FaceOrigin::OwnSolid, plane);
    let id = host
        .document
        .add_feature_in_body(sketch, "side".into(), Some(body))
        .unwrap();
    for _ in 0..3 {
        host.registry.evaluate(&mut host.document);
        let _ = host.registry.rebuild_jobs(&mut host.document);
    }
    let placed = wb_sketch::SketchFeature::from_json(host.document.feature_values(id).unwrap())
        .unwrap()
        .plane;
    assert!(
        (placed.normal[0] - 1.0).abs() < 1e-3,
        "normal {:?}, origin {:?}",
        placed.normal,
        placed.origin
    );
    assert!(
        (placed.origin[0] - hi[0]).abs() < 1e-3,
        "origin {:?}",
        placed.origin
    );
}

/// What a pad reversed `length` mm into a converted part builds, and the
/// part's own volume: all of it made from the file each time, by the code
/// the application runs. The part is a user's STL (not bundled; named by
/// `PRINTCAD_TEST_MESH_PART`), converted to a solid and refined; the sketch stands on
/// its top face, made of that face's own edges; the pad goes down into the
/// part along the face's outline. The file is only read. `None` without it.
fn pad_into_converted_part(
    length: f64,
) -> Option<(
    Result<kernel_api::SolidBuildResult, kernel_api::ChainError>,
    f64,
)> {
    use kernel_api::KernelQueries;
    let path = std::path::PathBuf::from(std::env::var_os("PRINTCAD_TEST_MESH_PART")?);
    let mut kernel = OgeomKernel::new();
    let detail = TessellationSettings::default();
    let model = kernel.import_step_full_mesh(&path, &detail).unwrap();
    let coarse = kernel
        .mesh_to_solid(&model.bodies[0].mesh, &detail)
        .unwrap();
    let solid = kernel
        .refine_brep(&coarse.brep_blob, &coarse.face_colors, &detail)
        .unwrap();
    assert_eq!(solid.health.broken, 0, "the part converts sound");

    let mut document = Document::new("part");
    let body = document.create_body(Some("Part".into()));
    let mesh = std::sync::Arc::new(solid.mesh.clone());
    let (_, hi) = mesh.bounds().unwrap();
    document.set_imported_geometry(
        body,
        ImportedGeometry {
            bounds_mm: mesh.bounds(),
            mesh,
            source_asset: Some(body.0),
            revision: 0,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: None,
        },
    );
    document.set_imported_brep_data(body, solid.brep_blob.clone(), Vec::new());

    // The part's top face: its small tab at the left end.
    let top = hi[2];
    let edges = kernel_ogeom::QUERIES
        .face_edges(&solid.brep_blob, [105.157, 89.76, f64::from(top)])
        .unwrap();
    let list: Vec<String> = edges
        .iter()
        .map(|(p, d)| {
            format!(
                "{{body = \"{}\", point = {{{}, {}, {}}}, direction = {{{}, {}, {}}}}}",
                body.0, p[0], p[1], p[2], d[0], d[1], d[2]
            )
        })
        .collect();
    let mut host = benches(document);
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"
            s = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.external{{sketch = s, counts = true, edges = {{{edges}}}}}
            pc.design.pad{{sketch = s, length = {length}, reversed = true}}
            "#,
            body = body.0,
            edges = list.join(",\n"),
        ),
        "pad.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    host.registry.evaluate(&mut host.document);
    let ops = wb_design::body_build_ops(&host.document, body).unwrap().ops;
    let volume = |r: &kernel_api::SolidBuildResult| {
        OgeomKernel::new()
            .physical_properties(&r.brep_blob)
            .unwrap()
            .volume_mm3
            .expect("a closed solid")
    };
    let base = kernel.execute_solid_chain(&ops[..1], &detail).unwrap();
    Some((kernel.execute_solid_chain(&ops, &detail), volume(&base)))
}

fn volume_of(built: &kernel_api::SolidBuildResult) -> f64 {
    OgeomKernel::new()
        .physical_properties(&built.brep_blob)
        .unwrap()
        .volume_mm3
        .expect("a closed solid")
}

/// A pad 3 mm into a converted part, along its face's own outline: it is
/// all inside the part, so the part is what the fuse leaves.
#[test]
fn a_pad_along_a_converted_solid_s_sides_fuses() {
    let Some((built, part)) = pad_into_converted_part(3.0) else {
        return;
    };
    let built = built.expect("the fuse settles");
    let volume = volume_of(&built);
    assert!(
        (volume - part).abs() < 1e-3 * part,
        "{volume} vs the part's {part}"
    );
}

/// The same pad 10 mm deep, out through the part's bottom: the fuse keeps
/// what leaves it, so the part grows.
#[test]
fn a_pad_out_through_a_converted_solid_s_bottom_fuses() {
    let Some((built, part)) = pad_into_converted_part(10.0) else {
        return;
    };
    let built = built.expect("the fuse settles");
    let volume = volume_of(&built);
    assert!(volume > part + 1.0, "{volume} vs the part's {part}");
}

/// A face picked for projection brings every edge around it: the box's
/// top comes in as its four sides and closes a profile; a bore's face
/// brings its rims, not the seam it meets itself along.
#[test]
fn a_face_s_edges_come_in_around_it() {
    use kernel_api::KernelQueries;
    let (document, body, lo, hi) = imported_box();
    let brep = document.imported_brep_blob(body).unwrap().to_vec();
    let centre = [
        f64::from(lo[0] + hi[0]) / 2.0,
        f64::from(lo[1] + hi[1]) / 2.0,
        f64::from(hi[2]),
    ];
    let edges = kernel_ogeom::QUERIES.face_edges(&brep, centre).unwrap();
    assert_eq!(edges.len(), 4, "{edges:?}");

    let mut host = benches(document);
    let list: Vec<String> = edges
        .iter()
        .map(|(p, d)| {
            format!(
                "{{body = \"{}\", point = {{{}, {}, {}}}, direction = {{{}, {}, {}}}}}",
                body.0, p[0], p[1], p[2], d[0], d[1], d[2]
            )
        })
        .collect();
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"
            s = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.external{{sketch = s, edges = {{{list}}}, counts = true}}
            "#,
            body = body.0,
            top = hi[2],
            list = list.join(", "),
        ),
        "face.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let sketch = host
        .document
        .feature_tree()
        .all_nodes()
        .find(|(_, n)| n.workbench_id.as_str() == "wb.sketch")
        .map(|(id, _)| *id)
        .unwrap();
    let feature =
        wb_sketch::SketchFeature::from_json(host.document.get_feature_data(sketch).unwrap())
            .unwrap();
    let wires =
        wb_sketch::profile::extract_wires(&feature.sketch).expect("the top's outline closes");
    assert_eq!(wires.len(), 1);
    assert_eq!(wires[0].segments.len(), 4);

    // A through bore: its face's rims, not its seam.
    let out = ScriptEngine::new().run_script(
        &format!(
            r#"
            local c = pc.sketch.new{{body = "{body}", plane = "XY", offset = {top}}}
            pc.sketch.circle{{sketch = c, x = {cx}, y = {cy}, radius = 2}}
            pc.design.pocket{{sketch = c, through_all = true}}
            "#,
            body = body.0,
            top = hi[2],
            cx = centre[0],
            cy = centre[1],
        ),
        "bore.lua",
        &mut host,
    );
    assert_eq!(out.error, None);
    let ops = wb_design::body_build_ops(&host.document, body).unwrap().ops;
    let bored = OgeomKernel::new()
        .execute_solid_chain(&ops, &TessellationSettings::default())
        .unwrap();
    let on_bore = [centre[0] + 2.0, centre[1], f64::from(lo[2] + hi[2]) / 2.0];
    let rims = kernel_ogeom::QUERIES
        .face_edges(&bored.brep_blob, on_bore)
        .unwrap();
    // The rims, each as the arcs the kernel keeps it in; nothing along the
    // bore, where its seam runs.
    assert!(!rims.is_empty());
    assert!(
        rims.iter().all(|(_, d)| d[2].abs() < 1e-6),
        "no seam: {rims:?}"
    );
    let heights: std::collections::BTreeSet<i64> = rims
        .iter()
        .map(|(p, _)| (p[2] * 1000.0).round() as i64)
        .collect();
    assert_eq!(heights.len(), 2, "both rims: {rims:?}");
}
