//! What the kernel tests share: the application's command path for
//! scripts, and the profiles, planes and transforms chains are built from.

use core_document::{
    BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, Document, DocumentService,
    FeatureId, ImportedGeometry, Workbench, WorkbenchFeature, WorkbenchId, WorkbenchRuntimeContext,
};
use kernel_api::{
    BooleanOp, ChainError, Placement, PrimitiveKind, ProfilePlane, ProfileSegment, ProfileWire,
    SolidBuildResult, SolidOp, TessellationSettings,
};
use kernel_ogeom::OgeomKernel;
use wb_design::DesignFeature;
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};

/// The benches a test registers and the document their commands edit: the
/// host a script's calls reach, the kernel answering the benches' queries.
pub struct Benches {
    pub registry: DocumentService,
    pub document: Document,
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
        // The command by the name it has now, whatever the script called it.
        let id = spec.id.clone();
        let wb = self.registry.workbench_mut(&bench).unwrap();
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.document, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.kernel = Some(&kernel_ogeom::QUERIES);
        wb.run_command(&id, &args, &mut ctx)
    }
}

/// Adding and changing features, which a test expects to succeed.
pub trait Features {
    /// Add `feature` to `body` as `name`.
    fn add_in<F: WorkbenchFeature>(&mut self, body: BodyId, name: &str, feature: F) -> FeatureId;
    /// Put `feature`'s data in place of feature `id`'s, as an edit does.
    fn set_feature<F: WorkbenchFeature>(&mut self, id: FeatureId, feature: F);
}

impl Features for Document {
    fn add_in<F: WorkbenchFeature>(&mut self, body: BodyId, name: &str, feature: F) -> FeatureId {
        self.add_feature_in_body(feature, name.into(), Some(body))
            .unwrap_or_else(|e| panic!("{name} adds: {e:?}"))
    }

    fn set_feature<F: WorkbenchFeature>(&mut self, id: FeatureId, feature: F) {
        self.update_feature_data(id, feature.to_json()).unwrap();
    }
}

/// A variable set of `(name, formula)` pairs, added to `doc` as `name`.
pub fn add_variables(doc: &mut Document, name: &str, pairs: &[(&str, &str)]) -> FeatureId {
    let variables = pairs
        .iter()
        .map(|(name, formula)| core_document::Variable {
            name: (*name).into(),
            formula: (*formula).into(),
            comment: String::new(),
        })
        .collect();
    doc.add_feature(core_document::VariableSet { variables }, name.into())
        .unwrap()
}

/// Give variable `index` of the set `set` the formula `formula`.
pub fn set_variable(doc: &mut Document, set: FeatureId, index: usize, formula: &str) {
    let data = doc.get_feature_data(set).unwrap();
    let mut variables = core_document::VariableSet::from_json(data).unwrap();
    variables.variables[index].formula = formula.into();
    doc.set_feature(set, variables);
}

/// What `registry`'s rebuild of `body` builds.
pub fn rebuilt(registry: &DocumentService, doc: &mut Document, body: BodyId) -> SolidBuildResult {
    let job = registry
        .rebuild_jobs(doc)
        .into_iter()
        .find(|j| j.body == body)
        .expect("a rebuild");
    chain(&job.plan.unwrap().ops)
}

/// Rebuild as the host does, probes, projections and face names all,
/// until nothing is left to rebuild; how many passes that took. A body
/// whose history does not plan is left as it stands.
pub fn settle_as_host(registry: &DocumentService, doc: &mut Document) -> usize {
    let mut kernel = OgeomKernel::new();
    for pass in 0..16 {
        registry.evaluate(doc);
        let jobs = registry.rebuild_jobs(doc);
        if jobs.is_empty() {
            return pass;
        }
        for job in jobs {
            let Ok(plan) = job.plan else { continue };
            if plan.ops.is_empty() {
                continue;
            }
            let asked: Vec<kernel_api::ChainProbe> = plan.probes.iter().map(|p| p.probe).collect();
            let projections: Vec<kernel_api::ChainProjection> =
                plan.projections.iter().map(|p| p.projection).collect();
            let tags: Vec<kernel_api::TopoName> = plan
                .op_features
                .iter()
                .map(|f| kernel_api::naming::name_of_id(f.0.as_bytes()))
                .collect();
            let result = kernel
                .execute_solid_chain_projecting(
                    &plan.ops,
                    &tags,
                    &TessellationSettings::default(),
                    None,
                    &asked,
                    &projections,
                )
                .unwrap_or_else(|e| panic!("{e:?}"));
            doc.store_probe_answers(&plan.probes, &result.probes);
            doc.store_projections(&plan.projections, &result.projections);
            store_solid(doc, job.body, result);
        }
    }
    panic!("the rebuilds never settled");
}

/// The volume of the solid `body` holds.
pub fn body_volume(doc: &Document, body: BodyId) -> f64 {
    volume(
        &doc.imported_brep_blob_arc(body)
            .expect("the body has a solid"),
    )
}

/// `document` with `benches` registered.
pub fn with_benches(
    document: Document,
    benches: impl IntoIterator<Item = Box<dyn Workbench>>,
) -> Benches {
    let mut registry = DocumentService::default();
    for bench in benches {
        registry.register_workbench(bench).unwrap();
    }
    Benches { registry, document }
}

/// `document` with the sketcher and Design registered, as the app has them.
pub fn benches(document: Document) -> Benches {
    Benches {
        registry: registry(),
        document,
    }
}

/// The sketcher and Design registered, as the app has them.
pub fn registry() -> DocumentService {
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
        .unwrap();
    registry
        .register_workbench(Box::new(wb_design::DesignWorkbench::default()))
        .unwrap();
    registry
}

/// Store a built solid on `body` as the host does.
pub fn store_solid(doc: &mut Document, body: BodyId, result: SolidBuildResult) {
    doc.set_imported_brep_data(body, result.brep_blob, Vec::new());
    doc.set_imported_geometry(
        body,
        ImportedGeometry {
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

/// A rectangle from (x0, y0) to (x1, y1) on `plane`.
pub fn rect_at(plane: SketchPlane, x0: f64, y0: f64, x1: f64, y1: f64) -> SketchFeature {
    let mut sketch = Sketch::new("r");
    sketch.plane = plane;
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
        .map(|(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))));
    for i in 0..4 {
        sketch.add_geometry(GeometryElement::Line(Line::new(
            corners[i],
            corners[(i + 1) % 4],
        )));
    }
    SketchFeature::new(sketch, plane)
}

/// A pad or a pocket of `sketch` with every other field at its default,
/// then `fields` set the way `design.set` sets them.
pub fn extrude(kind: &str, sketch: FeatureId, fields: serde_json::Value) -> DesignFeature {
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

/// A pad of `sketch`, `length` long.
pub fn pad(sketch: FeatureId, length: f32) -> DesignFeature {
    extrude("Pad", sketch, serde_json::json!({ "length": length }))
}

/// A pocket of `sketch`, `depth` deep.
pub fn pocket(sketch: FeatureId, depth: f32) -> DesignFeature {
    extrude("Pocket", sketch, serde_json::json!({ "depth": depth }))
}

/// What `ops` build at the default detail, or why they do not.
pub fn try_chain(ops: &[SolidOp]) -> Result<SolidBuildResult, ChainError> {
    new_kernel().execute_solid_chain(ops, &TessellationSettings::default())
}

/// [`try_chain`], which must succeed.
#[track_caller]
pub fn chain(ops: &[SolidOp]) -> SolidBuildResult {
    try_chain(ops).unwrap_or_else(|e| panic!("the chain builds: {e}"))
}

/// A primitive at the origin, the chain's first solid.
pub fn primitive(kind: PrimitiveKind) -> SolidOp {
    SolidOp::Primitive {
        kind,
        placement: Placement::default(),
        op: BooleanOp::NewSolid,
    }
}

pub fn new_kernel() -> OgeomKernel {
    use kernel_api::Kernel;
    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize kernel");
    kernel
}

pub fn xy_plane() -> ProfilePlane {
    ProfilePlane {
        origin: [0.0, 0.0, 0.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 1.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    }
}

pub fn plane_at_z(z: f64) -> ProfilePlane {
    ProfilePlane {
        origin: [0.0, 0.0, z],
        ..xy_plane()
    }
}

pub fn rect_wire(x0: f64, y0: f64, x1: f64, y1: f64) -> ProfileWire {
    let line = |start: [f64; 2], end: [f64; 2]| ProfileSegment::Line { start, end };
    ProfileWire {
        names: Vec::new(),
        segments: vec![
            line([x0, y0], [x1, y0]),
            line([x1, y0], [x1, y1]),
            line([x1, y1], [x0, y1]),
            line([x0, y1], [x0, y0]),
        ],
    }
}

pub fn circle_wire(cx: f64, cy: f64, radius: f64) -> ProfileWire {
    ProfileWire {
        names: Vec::new(),
        segments: vec![ProfileSegment::Circle {
            center: [cx, cy],
            radius,
        }],
    }
}

pub fn assert_close(actual: f32, expected: f32, tol: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: expected {expected} +/- {tol}, got {actual}"
    );
}

/// A move by `(x, y, z)`, row-major.
pub fn translate(x: f64, y: f64, z: f64) -> [[f64; 4]; 4] {
    [
        [1.0, 0.0, 0.0, x],
        [0.0, 1.0, 0.0, y],
        [0.0, 0.0, 1.0, z],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// The volume the kernel measures of a solid's snapshot.
pub fn volume(blob: &[u8]) -> f64 {
    OgeomKernel::new()
        .physical_properties(blob)
        .expect("the solid measures")
        .volume_mm3
        .expect("a closed solid")
}

/// A solid's snapshot read back into a model of its own.
pub fn read_shape(blob: &[u8]) -> (ogeom::topo::Model, ogeom::topo::Shape) {
    let mut model = ogeom::topo::Model::new();
    let text = std::str::from_utf8(blob).expect("a native snapshot is text");
    let shape = ogeom::io::native::read_into(&mut model, text)
        .expect("the snapshot reads")
        .shapes
        .remove(0);
    (model, shape)
}

/// How many shapes of `kind` a snapshot holds.
pub fn count_shapes(blob: &[u8], kind: ogeom::topo::ShapeType) -> usize {
    let (model, shape) = read_shape(blob);
    ogeom::topo::explore_unique(&model, &shape, kind)
        .unwrap()
        .len()
}

/// Whether the kernel's checker finds a snapshot's shape fit to use.
pub fn is_sound(blob: &[u8]) -> bool {
    let (model, shape) = read_shape(blob);
    ogeom::algo::check(&model, &shape, ogeom::core::Tolerances::millimetres())
        .unwrap()
        .is_usable()
}
