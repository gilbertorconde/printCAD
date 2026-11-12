//! One command per kind of step, its fields as arguments, with notes,
//! related commands and a working example; and the scan fits that work
//! out a step's arguments from a point on a scan.

use core_document::{
    BenchFnById, BodyId, CommandArgs, CommandError, CommandProse, CommandResult, CommandSpec,
    CommandTable, Document, FeatureId, ParamKind, WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use std::sync::OnceLock;

use crate::feature::{CurveRef, FitSheet, KINDS, Kind, SurfaceFeature};
use crate::{SurfaceWorkbench, panel_free};

const EDGE: &str = "{point, direction}, as pc.doc.edges lists an edge";

const CURVE: &str = "{Sketch = id}, {Edge = {point, direction}} for an edge of the body, or \
     {BodyEdge = {body = id, edge = {point, direction}}} for an edge of another body, in its frame";

const BY_CURVES: &str = "In place of `sketches`: each {Sketch = id}, {Edge = {point, direction}} for \
     an edge of the body, or {BodyEdge = {body = id, edge = {point, direction}}} for an edge of \
     another body, in its frame";

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SurfaceWorkbench>) {
    for kind in KINDS {
        if kind.waits.is_none() {
            let run: BenchFnById<SurfaceWorkbench> = match kind.tool {
                FIT_FREE => fit_free,
                FIT_SCAN => fit_scan,
                _ => create,
            };
            t.add_on_by_id(step(kind), run);
        }
    }
}

/// Make the step command `id` makes.
fn create(
    bench: &mut SurfaceWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    bench.create_by_command(id, args, ctx)
}

/// A free-form fit, its region worked out from a point on the scan when
/// given `at`.
fn fit_free(
    bench: &mut SurfaceWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    if !args.contains_key("at") {
        return bench.create_by_command(id, args, ctx);
    }
    let args = fit_free_args(args, ctx)?;
    bench.create_by_command(id, &args, ctx)
}

/// A plane or cylinder fit, worked out from a point on the scan when
/// given `at`.
fn fit_scan(
    bench: &mut SurfaceWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    if !args.contains_key("at") {
        return bench.create_by_command(id, args, ctx);
    }
    let args = fit_scan_args(args, ctx)?;
    bench.create_by_command(id, &args, ctx)
}

/// The command that makes a kind of step.
pub fn step(kind: &Kind) -> CommandSpec {
    let feature = SurfaceFeature::for_tool(kind.tool).expect("every kind has a step");
    let mut spec = CommandSpec::new(kind.tool, kind.summary);
    spec = if feature.needs_body() {
        spec.param(
            "body",
            ParamKind::Id,
            "The surface body it works on, or a feature in it",
        )
    } else {
        spec.optional(
            "body",
            ParamKind::Id,
            "The surface body it goes in, or a feature in it; else its first sketch's body \
             when that holds only drawings and surfaces, else a new one",
        )
    };
    spec = spec.optional("name", ParamKind::String, "Its name in the tree");
    if let Some(doc) = sketches(kind.tool) {
        spec = spec.optional("sketches", ParamKind::List, doc);
    }
    for (name, param, doc) in fields(kind.tool) {
        spec = spec.optional(name, *param, doc);
    }
    explained(kind.tool, spec.returns("The new feature's id"))
}

impl SurfaceWorkbench {
    fn create_by_command(
        &self,
        id: &str,
        args: &CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        let a = core_document::command::Args(args);
        let mut feature =
            SurfaceFeature::for_tool(id).ok_or_else(|| CommandError::Unknown(id.into()))?;
        let named = match a.opt_id("body")? {
            Some(raw) => {
                // A body, or a feature standing for the body it is in.
                let body = if ctx.document.body(BodyId(raw)).is_some() {
                    BodyId(raw)
                } else {
                    ctx.document
                        .get_feature_meta(FeatureId(raw))
                        .and_then(|n| n.body)
                        .ok_or_else(|| {
                            CommandError::bad("body", "is neither a body nor a feature in one")
                        })?
                };
                if !Self::takes_surfaces(ctx.document, body) {
                    return Err(CommandError::bad(
                        "body",
                        "is built by another workbench; surfaces go in a body of their own",
                    ));
                }
                Some(body)
            }
            None => None,
        };
        let curves = sketch_curves(&feature, args, ctx.document)?;
        let body = match named {
            Some(body) => body,
            None if feature.needs_body() => {
                return Err(CommandError::bad(
                    "body",
                    "is required: this step works on the surfaces a body holds",
                ));
            }
            // Beside its sketch, when the sketch's body takes surfaces.
            None => curves
                .iter()
                .find_map(|c| match c {
                    CurveRef::Sketch(id) => ctx.document.get_feature_meta(*id).and_then(|n| n.body),
                    CurveRef::Edge(_) | CurveRef::BodyEdge { .. } => None,
                })
                .filter(|b| Self::takes_surfaces(ctx.document, *b))
                .unwrap_or_else(|| ctx.document.create_body(Some("Surface".to_string()))),
        };
        panel_free::take_selection(&mut feature, &curves, &[]);
        let mut data = feature.to_json();
        merge_fields(&mut data, args)?;
        let feature = SurfaceFeature::from_json(&data).map_err(unreadable)?;
        if let Some(missing) = feature.missing() {
            return Err(CommandError::failed(format!("it needs {missing}")));
        }
        let made = self.add(ctx, body, feature).map_err(CommandError::failed)?;
        if let Some(name) = core_document::command::Args(args).opt_string("name")? {
            ctx.document.rename_feature(made, name);
        }
        Ok(json!(made.0.to_string()))
    }
}

/// The sketches a command names in `sketches`, each checked to be one, and
/// no more than the step takes.
pub(super) fn sketch_curves(
    feature: &SurfaceFeature,
    args: &CommandArgs,
    document: &Document,
) -> Result<Vec<CurveRef>, CommandError> {
    let mut curves = Vec::new();
    let Some(list) = args.get("sketches") else {
        return Ok(curves);
    };
    let list = list
        .as_array()
        .ok_or_else(|| CommandError::bad("sketches", "must list sketch ids"))?;
    for item in list {
        let raw = item
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .ok_or_else(|| CommandError::bad("sketches", "must list sketch ids"))?;
        let sketch = FeatureId(raw);
        let is_sketch = document
            .get_feature_meta(sketch)
            .is_some_and(|n| n.workbench_id.as_str() == wb_sketch::KIND);
        if !is_sketch {
            return Err(CommandError::bad(
                "sketches",
                format!("{raw} is not a sketch"),
            ));
        }
        curves.push(CurveRef::Sketch(sketch));
    }
    if matches!(feature, SurfaceFeature::Ruled { .. }) && curves.len() > 2 {
        return Err(CommandError::bad(
            "sketches",
            format!("gives {} curves; a ruled surface spans two", curves.len()),
        ));
    }
    Ok(curves)
}

/// A step's fields that do not read as one, with the form a `Custom`
/// direction, axis or plane takes when that is what went wrong.
pub(super) fn unreadable(error: core_document::DocumentError) -> CommandError {
    let e = error.to_string();
    let custom = [
        "unit variant",
        "expected struct variant",
        "expected newtype variant",
    ];
    let hint = if custom.iter().any(|c| e.contains(c)) {
        "; a `Custom` direction, axis or plane takes its numbers: \
         direction = {Custom = {0, 0, 1}}, \
         axis = {Custom = {origin = {0, 0, 0}, direction = {0, 0, 1}}}, \
         plane = {Custom = {origin = {0, 0, 0}, normal = {1, 0, 0}}}"
    } else {
        ""
    };
    CommandError::failed(format!("the fields do not make a surface: {e}{hint}"))
}

/// Named arguments laid over a step's fields.
/// The tool and command that fit a sheet to a scan.
const FIT_SCAN: &str = "surface.fit_scan";

/// And a free-form one.
const FIT_FREE: &str = "surface.fit_scan_spline";

/// The most points a free-form fit keeps from its region.
const FREE_POINTS: usize = 6000;

/// `surface.fit_scan_spline`'s arguments with `at` worked out into the
/// region's `points`, in the frame of the body it goes in: the scan's
/// triangles reached from the one at `at` across edges turning less than
/// `turn_deg`.
fn fit_free_args(
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> Result<CommandArgs, CommandError> {
    let a = core_document::command::Args(args);
    let scan = BodyId(a.id("scan")?);
    if ctx.document.body_scan(scan).is_none() {
        return Err(CommandError::bad("scan", "is not a scan"));
    }
    let at = a.vec3("at")?;
    let turn = a.opt_number("turn_deg")?.unwrap_or(8.0);
    let (mesh, _) = ctx
        .document
        .local_geometry(scan)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let scan_at = ctx.document.body_placement(scan);
    let seed = scan_at.inverse().point(at.map(|c| c as f32));
    let region = free_region(&mesh, seed, turn as f32);
    if region.len() < 50 {
        return Err(CommandError::failed(
            "the region there holds too few points to fit; widen `turn_deg`",
        ));
    }
    let body = a
        .opt_id("body")?
        .map(BodyId)
        .filter(|b| ctx.document.body(*b).is_some());
    let to = match body {
        Some(b) => ctx.document.body_placement(b).inverse().after(&scan_at),
        None => scan_at,
    };
    let step = region.len().div_ceil(FREE_POINTS).max(1);
    let points: Vec<[f32; 3]> = region.iter().step_by(step).map(|p| to.point(*p)).collect();
    let mut out = args.clone();
    out.remove("at");
    out.insert("points".into(), serde_json::json!(points));
    out.insert("scan".into(), serde_json::json!(scan.0.to_string()));
    Ok(out)
}

/// The points of `mesh`'s triangles reached from the one nearest `seed`
/// across edges whose triangles turn less than `turn_deg`, welded by
/// position.
fn free_region(mesh: &kernel_api::TriMesh, seed: [f32; 3], turn_deg: f32) -> Vec<[f32; 3]> {
    use std::collections::HashMap;
    let key = |p: [f32; 3]| p.map(f32::to_bits);
    let tris = mesh.indices.as_chunks::<3>().0;
    let at = |v: u32| glam::Vec3::from(mesh.positions[v as usize]);
    let normals: Vec<glam::Vec3> = tris
        .iter()
        .map(|[a, b, c]| (at(*b) - at(*a)).cross(at(*c) - at(*a)).normalize_or_zero())
        .collect();
    let mut by_edge: HashMap<([u32; 3], [u32; 3]), Vec<usize>> = HashMap::new();
    for (t, tri) in tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (
                key(mesh.positions[tri[k] as usize]),
                key(mesh.positions[tri[(k + 1) % 3] as usize]),
            );
            by_edge
                .entry(if a < b { (a, b) } else { (b, a) })
                .or_default()
                .push(t);
        }
    }
    let seed = glam::Vec3::from(seed);
    let Some(start) = (0..tris.len()).min_by(|x, y| {
        let d = |t: usize| {
            ((at(tris[t][0]) + at(tris[t][1]) + at(tris[t][2])) / 3.0).distance_squared(seed)
        };
        d(*x).total_cmp(&d(*y))
    }) else {
        return Vec::new();
    };
    let cos = turn_deg.to_radians().cos();
    let mut seen = vec![false; tris.len()];
    seen[start] = true;
    let mut queue = vec![start];
    let mut points: HashMap<[u32; 3], [f32; 3]> = HashMap::new();
    while let Some(t) = queue.pop() {
        let tri = tris[t];
        for k in 0..3 {
            let p = mesh.positions[tri[k] as usize];
            points.insert(key(p), p);
            let (a, b) = (key(p), key(mesh.positions[tri[(k + 1) % 3] as usize]));
            let edge = if a < b { (a, b) } else { (b, a) };
            for u in by_edge.get(&edge).into_iter().flatten() {
                if !seen[*u] && normals[*u].dot(normals[t]) >= cos {
                    seen[*u] = true;
                    queue.push(*u);
                }
            }
        }
    }
    let mut out: Vec<_> = points.into_iter().collect();
    out.sort_by_key(|(k, _)| *k);
    out.into_iter().map(|(_, p)| p).collect()
}

/// `surface.fit_scan`'s arguments with `at` and `shape` worked out into
/// the fitted `sheet`, in the frame of the body it goes in.
fn fit_scan_args(
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> Result<CommandArgs, CommandError> {
    use kernel_api::{FitKind, FittedShape, GrowRequest};
    let a = core_document::command::Args(args);
    let scan = BodyId(a.id("scan")?);
    if ctx.document.body_scan(scan).is_none() {
        return Err(CommandError::bad("scan", "is not a scan"));
    }
    let at = a.vec3("at")?;
    let kind = match a.opt_string("shape")?.unwrap_or("plane") {
        "plane" => FitKind::Plane,
        "cylinder" => FitKind::Cylinder,
        _ => return Err(CommandError::bad("shape", "is plane or cylinder")),
    };
    let (mesh, _) = ctx
        .document
        .local_geometry(scan)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to fit with"))?;
    let scan_at = ctx.document.body_placement(scan);
    let seed = scan_at.inverse().point(at.map(|c| c as f32)).map(f64::from);
    let grown = kernel
        .grow(
            &mesh,
            &GrowRequest {
                kind,
                seeds: vec![seed],
                sigma_cutoff: None,
                normal_limit_deg: None,
                peel: false,
                max_triangles: None,
            },
        )
        .map_err(|e| CommandError::failed(e.to_string()))?;
    if grown.seeds_in_region.first() == Some(&false) {
        return Err(CommandError::failed("the scan there is not that shape"));
    }
    // From the scan's frame to the frame of the body it goes in: a new
    // body's is the document's.
    let body = a
        .opt_id("body")?
        .map(BodyId)
        .filter(|b| ctx.document.body(*b).is_some());
    let to = match body {
        Some(b) => ctx.document.body_placement(b).inverse().after(&scan_at),
        None => scan_at,
    };
    let point = |p: [f64; 3]| to.point(p.map(|c| c as f32)).map(f64::from);
    let direction = |d: [f64; 3]| to.direction(d.map(|c| c as f32)).map(f64::from);
    let points: Vec<[f64; 3]> = grown
        .vertices
        .iter()
        .filter_map(|v| mesh.positions.get(*v as usize))
        .map(|p| point(p.map(f64::from)))
        .collect();
    let range = |origin: [f64; 3], dir: [f64; 3]| {
        points
            .iter()
            .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], p| {
                let t = (0..3).map(|k| (p[k] - origin[k]) * dir[k]).sum::<f64>();
                [lo.min(t), hi.max(t)]
            })
    };
    let sheet = match grown.fit.shape {
        FittedShape::Plane { origin, normal } => {
            let (origin, normal) = (point(origin), direction(normal));
            let other = if normal[0].abs() < 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            let u = {
                let c = [
                    normal[1] * other[2] - normal[2] * other[1],
                    normal[2] * other[0] - normal[0] * other[2],
                    normal[0] * other[1] - normal[1] * other[0],
                ];
                let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
                c.map(|x| x / len)
            };
            let v = [
                normal[1] * u[2] - normal[2] * u[1],
                normal[2] * u[0] - normal[0] * u[2],
                normal[0] * u[1] - normal[1] * u[0],
            ];
            FitSheet::Plane {
                origin,
                normal,
                u,
                across: [range(origin, u), range(origin, v)],
            }
        }
        FittedShape::Cylinder {
            point: p,
            axis,
            radius,
        } => {
            let (p, axis) = (point(p), direction(axis));
            FitSheet::Cylinder {
                point: p,
                axis,
                radius,
                along: range(p, axis),
            }
        }
        _ => return Err(CommandError::failed("the fit is not a plane or cylinder")),
    };
    let mut out = args.clone();
    out.remove("at");
    out.remove("shape");
    out.insert(
        "sheet".into(),
        serde_json::to_value(sheet).unwrap_or(Value::Null),
    );
    out.insert("sigma".into(), serde_json::json!(grown.fit.sigma));
    out.insert("scan".into(), serde_json::json!(scan.0.to_string()));
    Ok(out)
}

pub(super) fn merge_fields(data: &mut Value, args: &CommandArgs) -> Result<(), CommandError> {
    let Value::Object(outer) = data else {
        // A step with no fields takes none.
        return Ok(());
    };
    let Some(Value::Object(fields)) = outer.values_mut().next() else {
        return Ok(());
    };
    for (name, value) in args {
        if matches!(name.as_str(), "body" | "sketches" | "name") {
            continue;
        }
        if !fields.contains_key(name) {
            return Err(CommandError::bad(name, "is not a field of this surface"));
        }
        fields.insert(name.clone(), value.clone());
    }
    Ok(())
}

/// What `sketches` is to a kind of step, for the kinds that read it.
fn sketches(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "surface.extrude" | "surface.revolve" => {
            "The sketches it is built from: every chain of each, open or closed"
        }
        "surface.planar" => "The sketches whose closed loops it fills",
        "surface.fill" => "The sketches whose curves close the hole, end to end",
        "surface.ruled" => "Two sketches: the first curve, then the second",
        "surface.loft" => "The sections, a sketch each, in order: two or more",
        "surface.sweep" => "The profile's sketch, then the path's",
        "surface.split" => "The sketches whose curves cut the faces",
        _ => return None,
    })
}

/// The fields of a kind of step, as its command takes them.
fn fields(tool: &str) -> &'static [(&'static str, ParamKind, &'static str)] {
    use ParamKind::{Any, Bool, Id, Integer, List, Number, String};
    match tool {
        "surface.extrude" => &[
            ("length", Number, "mm (10)"),
            (
                "direction",
                Any,
                "SketchNormal (the default), X, Y, Z or {Custom = {x, y, z}}",
            ),
            ("symmetric", Bool, "Half each way"),
            ("reversed", Bool, "The other way"),
            ("curves", List, BY_CURVES),
        ],
        "surface.revolve" => &[
            ("angle_deg", Number, "Degrees (360)"),
            (
                "axis",
                Any,
                "SketchVertical (the default), SketchHorizontal, X, Y, Z or \
                 {Custom = {origin = {x, y, z}, direction = {x, y, z}}}",
            ),
            ("curves", List, BY_CURVES),
        ],
        "surface.planar" => &[("curves", List, BY_CURVES)],
        "surface.fill" => &[
            (
                "continuity",
                String,
                "G0 (the default), G1 or G2: how it meets the faces of edges in `boundary`",
            ),
            ("boundary", List, BY_CURVES),
        ],
        "surface.ruled" => &[("first", Any, CURVE), ("second", Any, CURVE)],
        "surface.loft" => &[
            (
                "closed",
                Bool,
                "Run on from the last section back to the first",
            ),
            (
                "sections",
                List,
                "In place of `sketches`: each {Sketch = id}",
            ),
            (
                "guides",
                List,
                "Curves the surface follows between the sections, each {Sketch = id} or \
                 {Edge = {point, direction}}, crossing every section once",
            ),
        ],
        "surface.sweep" => &[
            (
                "profile",
                List,
                "In place of `sketches`: the profile, each {Sketch = id}",
            ),
            (
                "path",
                List,
                "In place of `sketches`: the path, each {Sketch = id}",
            ),
            (
                "second_rail",
                List,
                "A second rail, each {Sketch = id}: the profile runs from the path to it and \
                 rides both",
            ),
        ],
        "surface.offset" => &[
            (
                "faces",
                List,
                "The faces to copy, each {point, normal} as pc.doc.faces lists a face",
            ),
            (
                "distance",
                Number,
                "mm along the faces' normals (1); negative the other way",
            ),
        ],
        "surface.extend" => &[
            (
                "edges",
                List,
                "The free edges to grow past, each {point, direction}, as pc.doc.edges lists an edge",
            ),
            ("length", Number, "mm (5)"),
            ("continuity", String, "G1 (the default), G0 or G2"),
        ],
        "surface.blend" => &[
            ("first", Any, EDGE),
            ("second", Any, EDGE),
            ("continuity", String, "G1 (the default), G0 or G2"),
        ],
        "surface.split" => &[
            (
                "faces",
                List,
                "The faces to cut, each {point, normal} as pc.doc.faces lists a face",
            ),
            ("curves", List, "In place of `sketches`: each {Sketch = id}"),
        ],
        "surface.sew" => &[(
            "gap",
            Number,
            "mm: edges this far apart are joined too (0, only edges that meet)",
        )],
        "surface.fillet" => &[
            (
                "edges",
                List,
                "The edges to round, each {point, direction}, as pc.doc.edges lists an edge",
            ),
            ("radius", Number, "mm (2)"),
        ],
        "surface.fillet_faces" => &[
            (
                "first",
                Any,
                "A face of the body, {point, normal} as pc.doc.faces lists a face",
            ),
            ("second", Any, "Another face of the body, the same way"),
            ("radius", Number, "mm (2)"),
            (
                "flip_first",
                Bool,
                "Roll the round on the first face's other side",
            ),
            (
                "flip_second",
                Bool,
                "Roll the round on the second face's other side",
            ),
        ],
        "surface.thicken" => &[
            (
                "thickness",
                Number,
                "mm along the faces' normals (2); negative the other way",
            ),
            ("both_sides", Bool, "Half each side"),
        ],
        "surface.trim" => &[
            (
                "plane",
                Any,
                "YZ (the default), XZ, XY or {Custom = {origin = {x, y, z}, normal = {x, y, z}}}, \
                 in the body's frame",
            ),
            ("offset", Number, "mm along the plane's normal"),
            ("flip", Bool, "Keep the side the normal points away from"),
        ],
        "surface.fit_network" => &[
            ("scan", Id, "The scan to fit"),
            ("curves", List, BY_CURVES),
            (
                "polylines",
                List,
                "Further boundaries after the sketches' and `curves`, each a list of \
                 {x, y, z} points in the body's frame",
            ),
            (
                "patches",
                List,
                "Each patch's four sides in order round it, as boundary numbers from 1: \
                 negative where one runs against the walk, 0 for a side shrunk to a point",
            ),
            (
                "chained",
                List,
                "Additional patches: four sides, each a list of nonzero signed boundary \
                 numbers in walking order; an empty list is a side shrunk to a point",
            ),
            (
                "corrections",
                Integer,
                "Parameter-correction passes after refinement (0); RMS distance never increases",
            ),
            (
                "tolerance",
                Number,
                "How near the points the patches keep, mm (0.05)",
            ),
            (
                "smoothing",
                Number,
                "The fairing weight that settles what the points leave free (0.0001)",
            ),
            (
                "creases",
                List,
                "Boundary numbers the patches meet across creased",
            ),
        ],
        "surface.fit_scan_spline" => &[
            ("scan", Id, "The scan to fit"),
            (
                "at",
                Any,
                "{x, y, z}: a point on the region to fit, in the document's frame",
            ),
            (
                "turn_deg",
                Number,
                "The region spreads across triangles turning less than this, degrees (8)",
            ),
            (
                "tolerance",
                Number,
                "How near the points the sheet keeps, mm (0.05)",
            ),
            (
                "points",
                Any,
                "The region's points, in the body's frame; worked out from `scan` and `at`",
            ),
        ],
        "surface.trim_scan" => &[
            ("scan", Id, "The scan whose surface the faces are kept on"),
            (
                "tolerance",
                Number,
                "How near the scan a face must lie, mm; three times the largest fit's σ (at least 0.05)",
            ),
            (
                "share",
                Number,
                "How much of a face's area must lie that near, 0 to 1 (0.5)",
            ),
        ],
        "surface.fit_scan" => &[
            ("scan", Id, "The scan to fit"),
            (
                "at",
                Any,
                "{x, y, z}: a point on the region to fit, in the document's frame",
            ),
            ("shape", String, "plane (the default) or cylinder"),
            (
                "extend",
                Number,
                "How far past the fitted region it reaches, a share of its size (0.35)",
            ),
            (
                "sheet",
                Any,
                "The fitted sheet, in the body's frame; worked out from `scan` and `at`",
            ),
            (
                "sigma",
                Number,
                "The fit's RMS distance, mm; worked out with `sheet`",
            ),
        ],
        "surface.mirror" => &[
            (
                "plane",
                Any,
                "YZ (the default), XZ, XY or {Custom = {origin = {x, y, z}, normal = {x, y, z}}}, \
                 in the body's frame",
            ),
            ("offset", Number, "mm along the plane's normal"),
        ],
        _ => &[],
    }
}

/// A step's notes, related commands and example: the ones every step that
/// works on a body, or every one that starts its own, shares, then
/// `steps.md`'s.
fn explained(tool: &str, spec: CommandSpec) -> CommandSpec {
    let spec = if SurfaceFeature::for_tool(tool).is_some_and(|f| f.needs_body()) {
        spec.note(
            "`body` is required, a body or any feature in it: the step works on the \
             surfaces the body holds before it.",
        )
    } else {
        spec.note(
            "It goes in `body` (a body, or any feature in it); without one, in its first \
             sketch's body when that holds only sketches, datums and surfaces, else in a new \
             body named Surface. A body Design builds is refused: surfaces go in a body of \
             their own, and `pc.sketch.new{body = id}` starts a sketch in one.",
        )
        .note(
            "An open sheet has an area and no volume: `pc.doc.measure` gives `volume` nil \
             until the body is sewn closed or thickened.",
        )
        .see_also("sketch.new")
    };
    prose().explain(spec)
}

/// The notes, related commands and examples of each step's command.
fn prose() -> &'static CommandProse {
    static PROSE: OnceLock<CommandProse> = OnceLock::new();
    CommandProse::once(&PROSE, "steps.md", include_str!("steps.md"), &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::KINDS;
    use core_document::WorkbenchFeature;

    /// A step a tool makes records its command with every field by name,
    /// so each field is an argument its command takes.
    #[test]
    fn every_field_of_a_step_is_an_argument_of_its_command() {
        for kind in KINDS.iter().filter(|k| k.waits.is_none()) {
            let spec = step(kind);
            let feature = SurfaceFeature::for_tool(kind.tool).unwrap();
            let serde_json::Value::Object(outer) = feature.to_json() else {
                panic!("{} is an object", kind.tool);
            };
            if let Some(serde_json::Value::Object(fields)) = outer.values().next() {
                for name in fields.keys() {
                    assert!(
                        spec.params.iter().any(|p| &p.name == name),
                        "{} takes `{name}`",
                        kind.tool
                    );
                }
            }
            assert!(!spec.notes.is_empty(), "{} has notes", kind.tool);
            assert_eq!(spec.examples.len(), 1, "{} has an example", kind.tool);
        }
    }
    #[test]
    fn every_section_of_the_steps_prose_names_a_command() {
        let bench = &mut crate::SurfaceWorkbench::default();
        core_document::testing::assert_prose_names_commands(bench, prose());
    }
}
