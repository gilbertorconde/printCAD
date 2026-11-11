//! One command per kind of step, its fields as arguments, with notes,
//! related commands and a working example; and the scan fits that work
//! out a step's arguments from a point on a scan.

use core_document::{
    BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, Document, FeatureId, ParamKind,
    WorkbenchContext, WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use crate::feature::{CurveRef, FitSheet, KINDS, Kind, SurfaceFeature};
use crate::{SurfaceWorkbench, panel_free};

const EDGE: &str = "{point, direction}, as pc.doc.edges lists an edge";

const CURVE: &str = "{Sketch = id}, {Edge = {point, direction}} for an edge of the body, or \
     {BodyEdge = {body = id, edge = {point, direction}}} for an edge of another body, in its frame";

const BY_CURVES: &str = "In place of `sketches`: each {Sketch = id}, {Edge = {point, direction}} for \
     an edge of the body, or {BodyEdge = {body = id, edge = {point, direction}}} for an edge of \
     another body, in its frame";

/// The examples' setup: an STL file of a wave sheet begun, `n` steps a side.
const WAVE_START: &str = r#"
                local path = os.tmpname() .. ".stl"
                local f = io.open(path, "w")
                f:write("solid wave\n")
                local n = 30"#;

/// The examples' setup: the wave's facets through `at(i, j)` written and
/// imported as a scan, `scan` its body.
const WAVE_SCAN: &str = r#"
                local function facet(p, q, s)
                  f:write("facet normal 0 0 0\nouter loop\n")
                  for _, v in ipairs({p, q, s}) do
                    f:write(string.format("vertex %.6f %.6f %.6f\n", v[1], v[2], v[3]))
                  end
                  f:write("endloop\nendfacet\n")
                end
                for i = 0, n - 1 do
                  for j = 0, n - 1 do
                    facet(at(i, j), at(i + 1, j), at(i + 1, j + 1))
                    facet(at(i, j), at(i + 1, j + 1), at(i, j + 1))
                  end
                end
                f:write("endsolid wave\n")
                f:close()
                pc.file.import{path = path, scan = true}
                pc.doc.rebuild()
                os.remove(path)
                local scan
                for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end"#;

/// The examples' setup: a 20 by 10 by 6 box exported and imported as a
/// scan, `scan` its body, the box hidden.
const BOX_SCAN: &str = r#"
                local box = pc.doc.new_body{}
                pc.design.primitive{body = box, kind = {Box = {length = 20, width = 10, height = 6}}}
                assert(#pc.doc.rebuild() == 0)
                local path = os.tmpname() .. ".stl"
                pc.file.export{path = path, bodies = {box}}
                pc.doc.set_visible{id = box, visible = false}
                pc.file.import{path = path, scan = true}
                pc.doc.rebuild()
                os.remove(path)
                local scan
                for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end"#;

pub(super) fn configure(context: &mut WorkbenchContext) {
    for kind in KINDS {
        if kind.waits.is_none() {
            context.register_command(step(kind));
        }
    }
}

pub(super) fn run(
    bench: &SurfaceWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    match id {
        FIT_FREE if args.contains_key("at") => {
            let args = fit_free_args(args, ctx)?;
            bench.create_by_command(id, &args, ctx)
        }
        FIT_SCAN if args.contains_key("at") => {
            let args = fit_scan_args(args, ctx)?;
            bench.create_by_command(id, &args, ctx)
        }
        _ => bench.create_by_command(id, args, ctx),
    }
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

/// A step's notes, related commands and example.
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
    match tool {
        "surface.extrude" => spec
            .note(
                "`length` is 10 mm when left out. `direction` is SketchNormal (square to the \
                 first sketch), X, Y, Z or {Custom = {x, y, z}}; \"Custom\" without its \
                 numbers is refused with the form it takes. `symmetric = true` runs half \
                 each way, `reversed = true` the other way.",
            )
            .note(
                "Built from edges (`curves`) it needs a direction of its own: SketchNormal \
                 fails at rebuild (\"the direction follows a sketch's plane\").",
            )
            .note(
                "An edge of another body (`BodyEdge`, the edge as `pc.doc.edges{body = id}` \
                 lists it) is taken where that body sits, and the step builds again when that \
                 body changes or moves.",
            )
            .see_also("surface.check")
            .example(
                "Two walls from an open outline",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.polyline{sketch = s, points = {{10, 0}, {0, 0}, {0, 10}}}
                local walls = pc.surface.extrude{sketches = {s}, length = 5}
                assert(#pc.doc.rebuild() == 0)
                local body = pc.doc.feature{id = walls}.body
                assert(#pc.doc.faces{body = body} == 2, "a face per line")
                local m = pc.doc.measure{body = body}
                assert(math.abs(m.area - 100) < 1e-6 and m.volume == nil, "a sheet, open")
                assert(math.abs(m.max[3] - 5) < 1e-6)

                -- A wall standing on the top edge of the first, in a body of its own.
                local top
                for _, e in ipairs(pc.doc.edges{body = body}) do
                  if math.abs(e.point[3] - 5) < 1e-6 and math.abs(e.point[2]) < 1e-6 then top = e end
                end
                local on = pc.surface.extrude{direction = "Z", length = 2, curves = {
                  {BodyEdge = {body = body, edge = {point = top.point, direction = top.direction}}}}}
                assert(#pc.doc.rebuild() == 0)
                local other = pc.doc.measure{body = pc.doc.feature{id = on}.body}
                assert(math.abs(other.min[3] - 5) < 1e-6 and math.abs(other.max[3] - 7) < 1e-6)
                "#,
            ),
        "surface.revolve" => spec
            .note(
                "`angle_deg` is 360 when left out. `axis` is SketchVertical (the first \
                 sketch's vertical axis through its origin), SketchHorizontal, X, Y, Z or \
                 {Custom = {origin = {x, y, z}, direction = {x, y, z}}}.",
            )
            .note(
                "A curve lying along the axis fails at rebuild (\"the whole wire lies along \
                 the axis, so it revolves out nothing\").",
            )
            .see_also("surface.check")
            .example(
                "A cylinder's side from a line",
                r#"
                local s = pc.sketch.new{plane = "XZ"}
                pc.sketch.line{sketch = s, x1 = 5, y1 = 0, x2 = 5, y2 = 10}
                local tube = pc.surface.revolve{sketches = {s}}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = pc.doc.feature{id = tube}.body}
                assert(math.abs(m.area - 2 * math.pi * 5 * 10) < 1e-3, m.area)
                assert(math.abs(m.min[1] + 5) < 1e-6 and math.abs(m.max[3] - 10) < 1e-6)
                "#,
            ),
        "surface.planar" => spec
            .note(
                "It fills closed flat loops, a loop inside another a hole. An open chain \
                 fails at rebuild (\"profile wire is not closed\").",
            )
            .see_also("surface.sew")
            .example(
                "A plate with a hole",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
                pc.sketch.circle{sketch = s, x = 10, y = 5, radius = 2}
                local plate = pc.surface.planar{sketches = {s}}
                assert(#pc.doc.rebuild() == 0)
                local body = pc.doc.feature{id = plate}.body
                assert(#pc.doc.faces{body = body} == 1)
                local area = pc.doc.measure{body = body}.area
                assert(math.abs(area - (200 - math.pi * 4)) < 1e-3, area)
                "#,
            ),
        "surface.fill" => spec
            .note(
                "The curves must meet end to end and close; ones that do not fail at rebuild \
                 (\"the boundary does not close\"). The loop may rise out of a plane, which \
                 `surface.planar` cannot fill.",
            )
            .note(
                "`continuity` (G0, G1 or G2) is how it meets the face of each edge given in \
                 `boundary`; a sketch's curve is only touched.",
            )
            .see_also("surface.planar")
            .example(
                "A disc filling a circle",
                r#"
                local s = pc.sketch.new{plane = "XY", offset = 20}
                pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
                local cap = pc.surface.fill{sketches = {s}}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = pc.doc.feature{id = cap}.body}
                assert(math.abs(m.area - math.pi * 25) < 1e-3, m.area)
                assert(math.abs(m.min[3] - 20) < 1e-6 and math.abs(m.max[3] - 20) < 1e-6)
                "#,
            ),
        "surface.ruled" => spec
            .note(
                "Exactly two curves, the first sketch's and the second's; a third is \
                 refused. Their ends pair start to start, as each was drawn: draw both the \
                 same way round, or the sheet twists.",
            )
            .see_also("surface.loft")
            .example(
                "A slanted strip between two lines",
                r#"
                local a = pc.sketch.new{plane = "XY"}
                pc.sketch.line{sketch = a, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
                local b = pc.sketch.new{plane = "XY", offset = 5}
                pc.sketch.line{sketch = b, x1 = 0, y1 = 3, x2 = 10, y2 = 3}
                local strip = pc.surface.ruled{sketches = {a, b}}
                assert(#pc.doc.rebuild() == 0)
                local area = pc.doc.measure{body = pc.doc.feature{id = strip}.body}.area
                assert(math.abs(area - 10 * math.sqrt(34)) < 1e-3, area)
                "#,
            ),
        "surface.loft" => spec
            .note(
                "Two sections or more, a sketch each, passed through in order; `closed = true` \
                 runs on from the last back to the first. Sections may differ: a square to a \
                 circle gives a face per side.",
            )
            .note(
                "`guides` shape the surface between the sections: each crosses every section \
                 once, all in the same order, and the sections' ends or middles alike. A \
                 guided loft is one face through sections of one curve each; it does not \
                 close (\"a loft that follows guides does not close\"), and a guide missing a \
                 section fails at rebuild.",
            )
            .see_also("surface.ruled")
            .example(
                "A bulge through three circles",
                r#"
                local function ring(z, r)
                  local s = pc.sketch.new{plane = "XY", offset = z}
                  pc.sketch.circle{sketch = s, x = 0, y = 0, radius = r}
                  return s
                end
                local loft = pc.surface.loft{sketches = {ring(0, 5), ring(10, 8), ring(20, 5)}}
                assert(#pc.doc.rebuild() == 0)
                local body = pc.doc.feature{id = loft}.body
                assert(#pc.doc.faces{body = body} == 1)
                local m = pc.doc.measure{body = body}
                assert(math.abs(m.max[1] - 8) < 1e-3 and math.abs(m.max[3] - 20) < 1e-6)

                -- Two lines, bowed between them by two arched guides.
                local function edge(z)
                  local s = pc.sketch.new{body = body, plane = "XY", offset = z}
                  pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
                  return s
                end
                local function guide(x)
                  -- Through (0, 0), (3, 5) and (0, 10) on YZ: y, then z.
                  local s = pc.sketch.new{body = body, plane = "YZ", offset = x}
                  local r, a = 17 / 3, math.deg(math.atan(5, 8 / 3))
                  pc.sketch.arc{sketch = s, x = 3 - r, y = 5, radius = r, start = -a, ["end"] = a}
                  return {Sketch = s}
                end
                local bowed = pc.surface.loft{body = body, sketches = {edge(0), edge(10)},
                  guides = {guide(0), guide(10)}}
                assert(#pc.doc.rebuild() == 0)
                local faces = pc.doc.faces{body = body}
                assert(#faces == 2, "the bulge and the bowed sheet")
                assert(math.abs(pc.doc.measure{body = body}.max[2] - 8) < 1e-3)
                "#,
            ),
        "surface.sweep" => spec
            .note(
                "`sketches` is the profile's sketch, then the path's. A closed path fails at \
                 rebuild (\"a sweep surface along a closed spine is not built\"), which is \
                 what a circle profile and its path given the other way round meet.",
            )
            .note(
                "With `second_rail` the path is the first rail: the profile, an open chain, \
                 runs from the path's start to the second rail's, and at every point along \
                 them is turned and scaled so its ends ride the rails. Ends off the rails' \
                 starts fail at rebuild (\"the profile's ends must sit on the rails' \
                 starts\").",
            )
            .see_also("surface.loft")
            .example(
                "A tube along a line",
                r#"
                local profile = pc.sketch.new{plane = "XY"}
                pc.sketch.circle{sketch = profile, x = 0, y = 0, radius = 2}
                local path = pc.sketch.new{plane = "XZ"}
                pc.sketch.line{sketch = path, x1 = 0, y1 = 0, x2 = 0, y2 = 20}
                local tube = pc.surface.sweep{sketches = {profile, path}}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = pc.doc.feature{id = tube}.body}
                assert(math.abs(m.area - 2 * math.pi * 2 * 20) < 1e-3, m.area)
                assert(math.abs(m.max[3] - 20) < 1e-6)

                -- An arch between two rails that spread from 10 apart to 14.
                local arch = pc.sketch.new{plane = "XZ"}
                pc.sketch.arc{sketch = arch, x = 5, y = 0, radius = 5, start = 0, ["end"] = 180}
                local body = pc.doc.feature{id = arch}.body
                local left = pc.sketch.new{body = body, plane = "XY"}
                pc.sketch.line{sketch = left, x1 = 10, y1 = 0, x2 = 14, y2 = 20}
                local right = pc.sketch.new{body = body, plane = "XY"}
                pc.sketch.line{sketch = right, x1 = 0, y1 = 0, x2 = 0, y2 = 20}
                pc.surface.sweep{body = body, sketches = {arch, left},
                  second_rail = {{Sketch = right}}}
                assert(#pc.doc.rebuild() == 0)
                m = pc.doc.measure{body = body}
                assert(math.abs(m.max[1] - 14) < 1e-2 and math.abs(m.max[3] - 7) < 2e-2,
                  "wider and taller at the far end")
                "#,
            ),
        "surface.offset" => spec
            .note(
                "`faces` are faces of the body as `pc.doc.faces` lists them (the entry \
                 itself will do; a curved face has no `normal` and needs none). The copy is \
                 added beside them, the faces kept. `distance` is 1 mm when left out.",
            )
            .see_also("doc.faces")
            .see_also("surface.thicken")
            .example(
                "A sheet copied 3 mm up",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
                local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
                assert(#pc.doc.rebuild() == 0)
                local face = pc.doc.faces{body = body}[1]
                pc.surface.offset{body = body, faces = {face}, distance = 3}
                assert(#pc.doc.rebuild() == 0)
                assert(#pc.doc.faces{body = body} == 2, "the sheet and its copy")
                assert(math.abs(pc.doc.measure{body = body}.max[3] - 3) < 1e-6)
                "#,
            ),
        "surface.extend" => spec
            .note(
                "`edges` are free edges of the body's sheets, each {point = e.point, \
                 direction = e.direction} from `pc.doc.edges`; each face grows past them by \
                 `length` (5 mm) and stays one face.",
            )
            .note(
                "`continuity` G1 (the default) and G0 run straight on, G2 along the face's \
                 own surface. Past a cylinder's straight side, a curved direction, only G2 \
                 builds; G1 and G0 fail at rebuild (\"a linear extension of an analytic \
                 surface across a curved parameter line would leave the surface\").",
            )
            .see_also("doc.edges")
            .example(
                "A sheet grown 5 mm past one side",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
                local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
                assert(#pc.doc.rebuild() == 0)
                local side
                for _, e in ipairs(pc.doc.edges{body = body}) do
                  if math.abs(e.point[1] - 10) < 1e-6 then side = e end
                end
                pc.surface.extend{body = body, length = 5,
                  edges = {{point = side.point, direction = side.direction}}}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = body}
                assert(math.abs(m.area - 150) < 1e-6 and math.abs(m.max[1] - 15) < 1e-6)
                assert(#pc.doc.faces{body = body} == 1, "still one face")
                "#,
            ),
        "surface.blend" => spec
            .note(
                "`first` and `second` are edges of the body's sheets, each {point, direction} \
                 from `pc.doc.edges`; the blend is a new face between them, meeting each \
                 edge's face with `continuity` (G1 by default, G0 or G2).",
            )
            .see_also("doc.edges")
            .see_also("surface.fill")
            .example(
                "A bridge between two strips",
                r#"
                local a = pc.sketch.new{plane = "XY", offset = 0}
                pc.sketch.line{sketch = a, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
                local low = pc.surface.extrude{sketches = {a}, length = 5, direction = "Y"}
                local body = pc.doc.feature{id = low}.body
                local b = pc.sketch.new{body = body, plane = "XY", offset = 10}
                pc.sketch.line{sketch = b, x1 = 0, y1 = 10, x2 = 10, y2 = 10}
                pc.surface.extrude{body = body, sketches = {b}, length = 5, direction = "Y"}
                assert(#pc.doc.rebuild() == 0)
                local first, second
                for _, e in ipairs(pc.doc.edges{body = body}) do
                  if math.abs(e.point[2] - 5) < 1e-6 and math.abs(e.point[3]) < 1e-6 then first = e end
                  if math.abs(e.point[2] - 10) < 1e-6 and math.abs(e.point[3] - 10) < 1e-6 then second = e end
                end
                pc.surface.blend{body = body,
                  first = {point = first.point, direction = first.direction},
                  second = {point = second.point, direction = second.direction}}
                assert(#pc.doc.rebuild() == 0)
                assert(#pc.doc.faces{body = body} == 3, "two strips and the bridge")
                assert(pc.doc.measure{body = body}.area > 200)
                "#,
            ),
        "surface.split" => spec
            .note(
                "`faces` are faces of the body as `pc.doc.faces` lists them. A sketch's curve \
                 lands on the face as seen square to the sketch's plane, so a sketch above \
                 the face cuts it too.",
            )
            .note(
                "Each curve must cross the face from edge to edge; one ending inside fails at \
                 rebuild (\"it must run from boundary to boundary\").",
            )
            .see_also("doc.faces")
            .see_also("surface.trim")
            .example(
                "A sheet cut in two along a line",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
                local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
                assert(#pc.doc.rebuild() == 0)
                local face = pc.doc.faces{body = body}[1]
                local cut = pc.sketch.new{body = body, plane = "XY", offset = 5}
                pc.sketch.line{sketch = cut, x1 = 4, y1 = -1, x2 = 4, y2 = 11}
                pc.surface.split{body = body, faces = {face}, sketches = {cut}}
                assert(#pc.doc.rebuild() == 0)
                local areas = {}
                for _, f in ipairs(pc.doc.faces{body = body}) do areas[#areas + 1] = f.area end
                table.sort(areas)
                assert(#areas == 2 and math.abs(areas[1] - 40) < 1e-3 and math.abs(areas[2] - 60) < 1e-3)
                "#,
            ),
        "surface.sew" => spec
            .note(
                "It joins every face of the body where their edges meet; `gap` (mm) joins \
                 edges up to that far apart too.",
            )
            .note(
                "A shell that closes becomes a solid, with a volume. One that does not stays \
                 a sheet, its faces now sharing edges: `surface.check` lists those joins and \
                 `surface.fillet` can round them.",
            )
            .see_also("surface.check")
            .see_also("surface.fillet")
            .see_also("surface.planar")
            .example(
                "Two walls joined at their corner",
                r#"
                local a = pc.sketch.new{plane = "XY"}
                pc.sketch.line{sketch = a, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
                local wall = pc.surface.extrude{sketches = {a}, length = 5}
                local body = pc.doc.feature{id = wall}.body
                local b = pc.sketch.new{body = body, plane = "XY"}
                pc.sketch.line{sketch = b, x1 = 0, y1 = 10, x2 = 0, y2 = 0}
                pc.surface.extrude{body = body, sketches = {b}, length = 5}
                assert(#pc.doc.rebuild() == 0)
                assert(#pc.surface.check{body = body} == 0, "touching, not joined")
                pc.surface.sew{body = body}
                assert(#pc.doc.rebuild() == 0)
                assert(#pc.surface.check{body = body} == 1, "one shared edge")
                "#,
            ),
        "surface.fillet" => spec
            .note(
                "`edges` are edges two faces of the body share, each {point, direction} from \
                 `pc.doc.edges`; `radius` is 2 mm when left out.",
            )
            .note(
                "Separate sheets that only touch share no edge: the round fails at rebuild \
                 (\"Sew them first so they share it\"). Sew them, or draw the faces from one \
                 sketch.",
            )
            .see_also("surface.sew")
            .see_also("doc.edges")
            .example(
                "Two walls rounded at their corner",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.polyline{sketch = s, points = {{10, 0}, {0, 0}, {0, 10}}}
                local walls = pc.surface.extrude{sketches = {s}, length = 5}
                local body = pc.doc.feature{id = walls}.body
                assert(#pc.doc.rebuild() == 0)
                local corner
                for _, e in ipairs(pc.doc.edges{body = body}) do
                  if #e.faces == 2 then corner = e end
                end
                pc.surface.fillet{body = body, radius = 2,
                  edges = {{point = corner.point, direction = corner.direction}}}
                assert(#pc.doc.rebuild() == 0)
                assert(#pc.doc.faces{body = body} == 3, "two walls and the round")
                local area = pc.doc.measure{body = body}.area
                assert(math.abs(area - (100 - 4 * 5 + math.pi * 5)) < 1e-3, area)
                "#,
            ),
        "surface.fillet_faces" => spec
            .note(
                "`first` and `second` are faces of the body as `pc.doc.faces` lists them, that \
                 need share no edge: two separate surfaces. Each is cut back to where the \
                 round touches it and the three are sewn into one sheet; `radius` is 2 mm \
                 when left out.",
            )
            .note(
                "The round rolls on the side each face's normal points to; `flip_first` or \
                 `flip_second` rolls it on that face's other side. A side the ball cannot \
                 reach both faces from fails at rebuild (\"rounding between the faces \
                 failed\").",
            )
            .see_also("surface.fillet")
            .see_also("doc.faces")
            .example(
                "Two walls of an L that stop short of the corner, rounded together",
                r#"
                local a = pc.sketch.new{plane = "XY"}
                pc.sketch.line{sketch = a, x1 = 2, y1 = 0, x2 = 10, y2 = 0}
                local body = pc.doc.feature{id = pc.surface.extrude{sketches = {a}, length = 5}}.body
                local b = pc.sketch.new{body = body, plane = "XY"}
                pc.sketch.line{sketch = b, x1 = 0, y1 = 10, x2 = 0, y2 = 2}
                pc.surface.extrude{body = body, sketches = {b}, length = 5}
                assert(#pc.doc.rebuild() == 0)
                local first, second
                for _, f in ipairs(pc.doc.faces{body = body}) do
                  if math.abs(f.point[2]) < 1e-6 then first = f else second = f end
                end
                pc.surface.fillet_faces{body = body, first = first, second = second, radius = 3,
                  flip_first = true, flip_second = true}
                assert(#pc.doc.rebuild() == 0)
                assert(#pc.doc.faces{body = body} == 3, "two walls and the round")
                assert(#pc.surface.check{body = body} == 2, "one sheet")
                local area = pc.doc.measure{body = body}.area
                assert(math.abs(area - (2 * 7 * 5 + math.pi * 3 / 2 * 5)) < 1e-2, area)
                "#,
            ),
        "surface.thicken" => spec
            .note(
                "Each sheet becomes a solid: `thickness` (2 mm) along the faces' normals, \
                 negative the other way, or half each way with `both_sides = true`.",
            )
            .note(
                "A body already sewn into a solid is refused at rebuild (\"the body has no \
                 sheet to thicken\"): Design's Thickness hollows a solid.",
            )
            .see_also("surface.sew")
            .see_also("surface.offset")
            .example(
                "A sheet given 2 mm",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
                local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
                pc.surface.thicken{body = body, thickness = 2}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = body}
                assert(math.abs(m.volume - 200) < 1e-6, m.volume)
                assert(#pc.doc.faces{body = body} == 6, "a box")
                "#,
            ),
        "surface.trim" => spec
            .note(
                "It keeps what lies on the side the plane's normal points to: YZ keeps +X. \
                 `flip = true` keeps the other side; `offset` moves the plane along its \
                 normal, YZ with 4 being x = 4.",
            )
            .note(
                "A plane that leaves nothing fails at rebuild (\"the plane leaves nothing of \
                 the body on the side kept; flip it or move it\"). It cuts a sewn solid too, \
                 which stays solid.",
            )
            .see_also("surface.split")
            .see_also("surface.mirror")
            .example(
                "A sheet cut at x = 4, either side kept",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
                local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
                local trim = pc.surface.trim{body = body, plane = "YZ", offset = 4}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = body}
                assert(math.abs(m.area - 60) < 1e-6 and math.abs(m.min[1] - 4) < 1e-6)
                pc.surface.set{feature = trim, flip = true}
                assert(#pc.doc.rebuild() == 0)
                assert(math.abs(pc.doc.measure{body = body}.area - 40) < 1e-6)
                "#,
            ),
        "surface.fit_network" => spec
            .note(
                "Its boundaries are numbered from 1 in order: every chain of each sketch in \
                 `sketches`, then `curves`, then `polylines`. Each bounds one patch or two, \
                 and two patches walk the one they share opposite ways. A patch's sides go \
                 round it counter-clockwise seen from the side it faces: along u, up v, back \
                 along u, back down v. A side may be 0, shrunk to the point its neighbours \
                 meet at: a three-sided patch.",
            )
            .note(
                "Every patch is fitted to the scan's points at once, those within the \
                 boundaries' box, and the patches are sewn together: no gap between them, \
                 tangent across every boundary but the creases, exactly where four patches meet \
                 at a corner and nearly where three or five do. A patch gains spans where it \
                 misses its points by more than `tolerance`.",
            )
            .note(
                "Use `chained` for additional patches whose sides are lists of signed curve \
                 numbers, in walking order; an empty list shrinks a side to a point. A side \
                 may join several curves at T-junctions. Its opposite must have the same \
                 number of curves, or one curve carrying the junctions to the next patch. \
                 The patch is held C1 across each junction. `corrections` requests parameter \
                 correction after refinement: each pass is kept only if RMS distance decreases.",
            )
            .see_also("surface.fit_scan_spline")
            .see_also("surface.check")
            .example(
                "A wavy scan fitted as three patches meeting tangent at a T-junction",
                &format!("{WAVE_START}{}{WAVE_SCAN}{}", r#"
                local function z(x) return 2 * math.sin(x / 4) end
                local function at(i, j)
                  local x, y = 20 * i / n, 20 * j / n
                  return {x, y, z(x)}
                end"#, r#"
                local function line(x1, y1, x2, y2)
                  local pts = {}
                  for k = 0, 20 do
                    local x, y = x1 + (x2 - x1) * k / 20, y1 + (y2 - y1) * k / 20
                    pts[#pts + 1] = {x, y, z(x)}
                  end
                  return pts
                end
                local sheet = pc.surface.fit_network{scan = scan,
                  polylines = {line(0, 0, 10, 0), line(10, 0, 20, 0), line(20, 0, 20, 8),
                    line(20, 20, 10, 20), line(10, 20, 0, 20), line(0, 20, 0, 0),
                    line(10, 0, 10, 8), line(20, 8, 20, 20), line(10, 8, 10, 20),
                    line(10, 8, 20, 8)},
                  patches = {{2, 3, -10, -7}, {10, 8, 4, -9}},
                  chained = {{{1}, {7, 9}, {5}, {6}}}, corrections = 2}
                assert(#pc.doc.rebuild() == 0)
                local body = pc.doc.feature{id = sheet}.body
                local joins = pc.surface.check{body = body}
                assert(#joins == 3)
                for _, join in ipairs(joins) do
                  assert(join.gap < 0.00001 and join.angle_deg < 0.01, "tangent at the junction")
                end
                local m = pc.doc.measure{body = body}
                assert(math.abs(m.max[3] - 2) < 0.05 and math.abs(m.max[1] - 20) < 0.01, m.max[1] .. " " .. m.max[3])
                "#),
            ),
        "surface.fit_scan_spline" => spec
            .note(
                "The region spreads over the scan from the triangle at `at` across every edge \
                 whose two triangles turn less than `turn_deg`; its points (up to 6000) are kept \
                 with the step. A B-spline sheet is fitted to them, seeded on the plane or \
                 cylinder they lie nearest and refined where it misses them by more than \
                 `tolerance`, until it meets it or only noise is left.",
            )
            .see_also("surface.fit_scan")
            .see_also("surface.trim_scan")
            .example(
                "A wavy sheet, scanned, fitted as one free-form surface",
                &format!("{WAVE_START}{}{WAVE_SCAN}{}", r#"
                local function at(i, j)
                  local x, y = 20 * i / n, 20 * j / n
                  return {x, y, 2 * math.sin(x / 4)}
                end"#, r#"
                local sheet = pc.surface.fit_scan_spline{scan = scan, at = {10, 10, 2 * math.sin(2.5)}}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = pc.doc.feature{id = sheet}.body}
                assert(math.abs(m.max[3] - 2) < 0.1 and math.abs(m.max[1] - 20) < 0.1, m.max[3])
                "#),
            ),
        "surface.trim_scan" => spec
            .note(
                "Each of the body's sheets is cut along the curves where the others cross it; \
                 every face it is cut into is then kept when `share` of its area lies within \
                 `tolerance` of the scan's triangles, and what is kept is sewn. A shell that \
                 closes becomes a solid. Fitted surfaces reach past their regions so that this \
                 cutting meets them up.",
            )
            .see_also("surface.fit_scan")
            .see_also("surface.sew")
            .example(
                "A scanned box's six sides fitted, trimmed by the scan into the box again",
                &format!("{BOX_SCAN}{}", r#"
                local body
                for _, at in ipairs({{10, 5, 6}, {10, 5, 0}, {0, 5, 3}, {20, 5, 3}, {10, 0, 3}, {10, 10, 3}}) do
                  local f = pc.surface.fit_scan{scan = scan, at = at, body = body}
                  body = pc.doc.feature{id = f}.body
                end
                pc.surface.trim_scan{body = body, scan = scan}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = body}
                assert(math.abs(m.volume - 1200) < 0.01, m.volume)
                "#),
            ),
        "surface.fit_scan" => spec
            .note(
                "A plane or cylinder is grown over the scan from `at` (the region its points \
                 lie on within the fit's scatter) and fitted to it; the sheet covers what its \
                 points cover, extended by `extend` of its size so the fits of neighbouring \
                 faces cross and a trim between them can cut each to the other. A cylinder is \
                 a whole turn. `sigma` is the fit's RMS distance from the points.",
            )
            .note(
                "A free-form region is fitted with a B-spline by `pc.surface.fit_scan_spline`.",
            )
            .see_also("surface.fit_scan_spline")
            .see_also("inspect.grow")
            .see_also("surface.sew")
            .example(
                "A scanned box's top and side fitted as planes that cross past the edge",
                &format!("{BOX_SCAN}{}", r#"
                local top = pc.surface.fit_scan{scan = scan, at = {10, 5, 6}}
                local body = pc.doc.feature{id = top}.body
                pc.surface.fit_scan{scan = scan, at = {20, 5, 3}, body = body, extend = 0.5}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = body}
                -- The top reaches past x = 20, the side above z = 6.
                assert(m.max[1] > 22 and m.max[3] > 7, m.max[1])
                "#),
            ),
        "surface.mirror" => spec
            .note(
                "It adds the body's reflection beside what the body holds, the original kept; \
                 the two are separate pieces of the one body.",
            )
            .note(
                "`plane` YZ (the default), XZ, XY or {Custom = {origin, normal}}, in the \
                 body's frame; `offset` moves it along its normal: YZ with 10 reflects in \
                 x = 10.",
            )
            .see_also("surface.trim")
            .example(
                "A sheet and its reflection",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
                local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
                pc.surface.mirror{body = body, plane = "YZ"}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = body}
                assert(math.abs(m.area - 200) < 1e-6 and math.abs(m.min[1] + 10) < 1e-6)
                assert(#pc.doc.faces{body = body} == 2)
                "#,
            ),
        _ => spec,
    }
}

#[cfg(test)]
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
}
