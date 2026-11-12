//! Reading a scan: a hole's or a thread's size, a face snapped onto it, its
//! symmetry plane and how a body lies against it.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Map, json};

use super::features::datum;
use crate::DesignWorkbench;

/// The largest RMS mirror residual, mm, a scan's symmetry plane is taken
/// at.
pub(crate) const SYMMETRY_ACCEPT: f64 = 0.1;

/// The name a scan's symmetry plane is given, by which Mirror finds it.
pub(crate) const SYMMETRY_NAME: &str = "Scan symmetry";

/// The plane `scan` is most nearly symmetric about, kept as a datum plane
/// of `body` when its RMS mirror residual is within `accept`.
pub(crate) fn scan_symmetry(
    ctx: &mut WorkbenchRuntimeContext,
    scan: BodyId,
    body: BodyId,
    accept: f64,
) -> CommandResult {
    if ctx.document.body_scan(scan).is_none() {
        return Err(CommandError::bad("scan", "is not a scan"));
    }
    let (mesh, _) = ctx
        .document
        .local_geometry(scan)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to read the scan with"))?;
    let found = kernel
        .symmetry_plane(&mesh)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    if found.rms > accept {
        return Err(CommandError::failed(format!(
            "the scan is not symmetric: its best plane mirrors it to {:.3} mm RMS, over {accept}",
            found.rms
        )));
    }
    // From the scan's frame to the body's.
    let to = ctx
        .document
        .body_placement(body)
        .inverse()
        .after(&ctx.document.body_placement(scan));
    let point = to.point(found.point.map(|c| c as f32)).map(f64::from);
    let normal = to.direction(found.normal.map(|c| c as f32)).map(f64::from);
    let mut args = Map::new();
    args.insert("kind".into(), json!("plane"));
    args.insert("body".into(), json!(body.0.to_string()));
    args.insert("face_point".into(), json!(point));
    args.insert("face_normal".into(), json!(normal));
    args.insert("name".into(), json!(SYMMETRY_NAME));
    let datum_id = datum(&Args(&args), ctx)?;
    Ok(json!({
        "datum": datum_id,
        "rms": found.rms,
        "point": {"x": point[0], "y": point[1], "z": point[2]},
        "normal": {"x": normal[0], "y": normal[1], "z": normal[2]},
    }))
}

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<DesignWorkbench>) {
    t.add(
        CommandSpec::new(
            "design.read_hole",
            "A round's size off a scan, and the nearest standard",
        )
        .param("scan", ParamKind::Id, "The scan")
        .param("at", ParamKind::Any, "{x, y, z}: a point on the round's wall, in the document's frame")
        .returns("{diameter, depth, hole, sigma, start, axis, sweep, fillet, design, nearest}")
        .read_only()
        .note(
            "A cylinder is grown over the scan from `at`: `diameter`, `depth` (how far its \
             points run along the axis), `hole` (true when the wall faces its axis, false for \
             a boss or pin), `sigma` its fit's RMS, `start` and `axis` where it begins and \
             which way it runs. `design` is the diameter rounded to the value it was likely \
             drawn at, and `nearest` the three nearest standard sizes, each {standard, size, \
             as, diameter, off}: for a hole the tap drills and clearance holes of every thread \
             standard, for a boss the threads' major diameters.",
        )
        .note(
            "`sweep` is how far round its axis the round's points reach, in degrees. Under \
             300° it is a fillet: `fillet` is then {radius, design, concave}, the radius it \
             was likely drawn at in `design`, `concave` for an inside round (the wall faces \
             its axis), and `nearest` is empty.",
        )
        .see_also("design.hole")
        .see_also("design.fillet")
        .example(
            "A clearance hole scanned and read as an M6's",
            r#"
            local plate = pc.doc.new_body{}
            pc.design.primitive{body = plate, kind = {Box = {length = 30, width = 20, height = 5}}}
            pc.design.subtractive_primitive{body = plate, variant = "cylinder",
              kind = {Cylinder = {radius = 3.3, height = 7, angle_deg = 360}},
              placement = {origin = {12, 10, -1}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
            assert(#pc.doc.rebuild() == 0)
            local path = os.tmpname() .. ".stl"
            pc.file.export{path = path, bodies = {plate}, tolerance = 0.002}
            pc.doc.set_visible{id = plate, visible = false}
            pc.file.import{path = path, scan = true}
            pc.doc.rebuild()
            os.remove(path)
            local scan
            for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
            local r = pc.design.read_hole{scan = scan, at = {15.3, 10, 2.5}}
            assert(r.hole and math.abs(r.diameter - 6.6) < 0.01 and math.abs(r.depth - 5) < 0.01, r.diameter)
            assert(r.nearest[1].size == "M6" and r.nearest[1].as == "normal clearance", r.nearest[1].size)
            assert(r.fillet == nil and r.sweep > 300, r.sweep)
            "#,
        )
        .example(
            "A rounded edge scanned and read as an R2 fillet",
            r#"
            local block = pc.doc.new_body{}
            pc.design.primitive{body = block}
            pc.design.fillet{body = block, radius = 2,
              edges = {Edges = {{point = {5, 10, 10}, direction = {1, 0, 0}}}}}
            assert(#pc.doc.rebuild() == 0)
            local path = os.tmpname() .. ".stl"
            pc.file.export{path = path, bodies = {block}, tolerance = 0.002}
            pc.doc.set_visible{id = block, visible = false}
            pc.file.import{path = path, scan = true}
            pc.doc.rebuild()
            os.remove(path)
            local scan
            for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
            local h = 2 * math.sqrt(0.5)
            local r = pc.design.read_hole{scan = scan, at = {5, 8 + h, 8 + h}}
            assert(r.fillet and math.abs(r.fillet.radius - 2) < 0.01 and r.fillet.design == 2, r.sweep)
            assert(not r.fillet.concave and math.abs(r.sweep - 90) < 2 and #r.nearest == 0, r.sweep)
            "#,
        ),
        read_hole,
    );
    t.add_on(
        CommandSpec::new(
            "design.snap_to_scan",
            "Move a face of the body onto a scan",
        )
        .param("body", ParamKind::Id, "The body")
        .param("face_point", ParamKind::Any, "{x, y, z}: a point of the face, in the body's own frame")
        .param("face_normal", ParamKind::Any, "{x, y, z}: the face's outward normal there")
        .param("scan", ParamKind::Id, "The scan")
        .param("at", ParamKind::Any, "{x, y, z}: where on the scan the face should lie, in the document's frame")
        .optional("name", ParamKind::String, "The feature's name in the tree")
        .returns("{feature, moved, angle, sigma} for a flat face; {feature, radius, fitted, design, angle, axis_offset, sigma} for a round one")
        .note(
            "A plane or cylinder, as the face is, is grown over the scan from `at` and \
             fitted. A flat face is moved along its normal onto the fitted plane by a Move \
             Faces feature (`moved`, mm, and `angle`, how far the scan's plane is turned from \
             the face); a round face is offset to the fitted radius by an Offset Faces feature \
             (`design` is the radius it was likely drawn at, `axis_offset` how far the two \
             axes lie apart). A scan surface turned more than 20° from the face is refused, \
             and the face is never turned: a draft or a tilt is a feature of its own.",
        )
        .see_also("design.move_faces")
        .see_also("design.offset_faces")
        .example(
            "A block's top snapped 0.4 mm up and its bore widened to the scan's",
            r#"
            local function block(height, radius)
              local b = pc.doc.new_body{}
              pc.design.primitive{body = b, kind = {Box = {length = 20, width = 10, height = height}}}
              pc.design.subtractive_primitive{body = b, variant = "cylinder",
                kind = {Cylinder = {radius = radius, height = 20, angle_deg = 360}},
                placement = {origin = {10, 5, -5}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
              return b
            end
            local printed = block(5.4, 3.2)
            assert(#pc.doc.rebuild() == 0)
            local path = os.tmpname() .. ".stl"
            pc.file.export{path = path, bodies = {printed}, tolerance = 0.002}
            pc.doc.set_visible{id = printed, visible = false}
            pc.file.import{path = path, scan = true}
            pc.doc.rebuild()
            os.remove(path)
            local scan
            for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
            local model = block(5, 3)
            assert(#pc.doc.rebuild() == 0)
            local top = pc.design.snap_to_scan{body = model, face_point = {2, 2, 5},
              face_normal = {0, 0, 1}, scan = scan, at = {2, 2, 5.4}}
            assert(math.abs(top.moved - 0.4) < 1e-3 and top.angle < 0.01, top.moved)
            local bore = pc.design.snap_to_scan{body = model, face_point = {13, 5, 2},
              face_normal = {-1, 0, 0}, scan = scan, at = {13.2, 5, 2.5}}
            assert(math.abs(bore.fitted - 3.2) < 0.01 and bore.design == 3, bore.fitted)
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = model}
            local want = 20 * 10 * 5.4 - math.pi * 3.2 ^ 2 * 5.4
            assert(math.abs(m.volume - want) < 0.5, m.volume)
            "#,
        ),
        snap_to_scan,
    );
    t.add(
        CommandSpec::new(
            "design.scan_symmetry",
            "A scan's symmetry plane as a datum, for Mirror",
        )
        .param("scan", ParamKind::Id, "The scan")
        .param("body", ParamKind::Id, "The body the datum plane goes in")
        .optional(
            "accept",
            ParamKind::Number,
            "The largest RMS mirror residual taken, mm; 0.1",
        )
        .returns("{datum, rms, point, normal}: the plane in the body's frame")
        .note(
            "The kernel searches for the plane that mirrors the scan onto itself best; it is \
             refused when its RMS residual is over `accept`. The datum plane is named Scan \
             symmetry, and Mirror then mirrors about it by default in that body (a face \
             selected still wins), so half a part modelled from a symmetric scan is \
             mirrored whole.",
        )
        .see_also("design.mirror")
        .see_also("inspect.use_symmetry")
        .example(
            "A scan of a block with a blind hole on its middle line, symmetric about x = 10 alone",
            r#"
            local block = pc.doc.new_body{}
            pc.design.primitive{body = block, kind = {Box = {length = 20, width = 12, height = 5}}}
            pc.design.subtractive_primitive{body = block, variant = "cylinder",
              kind = {Cylinder = {radius = 2, height = 7, angle_deg = 360}},
              placement = {origin = {10, 3, 2}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
            assert(#pc.doc.rebuild() == 0)
            local path = os.tmpname() .. ".stl"
            pc.file.export{path = path, bodies = {block}, tolerance = 0.002}
            pc.doc.set_visible{id = block, visible = false}
            pc.file.import{path = path, scan = true}
            pc.doc.rebuild()
            os.remove(path)
            local scan
            for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
            local model = pc.doc.new_body{}
            local s = pc.design.scan_symmetry{scan = scan, body = model}
            assert(s.rms < 0.01 and math.abs(math.abs(s.normal.x) - 1) < 1e-3, s.rms)
            assert(math.abs(s.point.x - 10) < 0.01, s.point.x)
            -- Half the block modelled, then mirrored about the scan's plane.
            pc.design.primitive{body = model, kind = {Box = {length = 10, width = 12, height = 5}}}
            pc.design.mirror{body = model}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = model}
            assert(math.abs(m.min[1]) < 1e-6 and math.abs(m.max[1] - 20) < 0.01, m.max[1])
            "#,
        ),
        scan_symmetry_command,
    );
    t.add(
        CommandSpec::new(
            "design.read_thread",
            "A thread's pitch, size and hand off a scan",
        )
        .param("scan", ParamKind::Id, "The scan")
        .param(
            "at",
            ParamKind::Any,
            "{x, y, z}: a point on the thread, in the document's frame",
        )
        .returns(
            "{pitch, major, minor, right_handed, internal, clarity, length, start, axis, nearest}",
        )
        .read_only()
        .note(
            "The thread's axis comes from a cylinder grown over its crests and flanks from \
             `at`; every scan point within a third of its radius of it, along the grown \
             stretch, is then read for the helix it follows: the pitch (tried every 0.02 mm \
             from 0.2 to 6.5, then to the thousandth about the best) and hand whose turns the points' radii follow most strongly. \
             `clarity` is that helix's share of the radii's spread, near 1 on a clean scan \
             of a thread and low where there is none. `major` and `minor` are twice the \
             radius the outermost and innermost 3 % of points reach, `internal` true in a \
             hole, and `nearest` the three standard threads nearest by diameter and pitch, \
             each {standard, size, major, pitch}.",
        )
        .see_also("design.read_hole")
        .see_also("design.hole")
        .example(
            "A scanned M8 thread read back: a synthetic scan of a bolt's thread, 1.25 \
             pitch, right-handed, written as an STL and imported",
            r#"
            local path = os.tmpname() .. ".stl"
            local f = io.open(path, "w")
            f:write("solid bolt\n")
            local pitch, turns, around, along = 1.25, 5, 48, 120
            local function at(i, j)
              local a = 2 * math.pi * i / around
              local t = turns * pitch * j / along
              local phase = (t / pitch - a / (2 * math.pi)) % 1
              local r = 4 - 0.68 * math.abs(2 * phase - 1)
              return {r * math.cos(a), r * math.sin(a), t}
            end
            local function facet(p, q, s)
              f:write("facet normal 0 0 0\nouter loop\n")
              for _, v in ipairs({p, q, s}) do
                f:write(string.format("vertex %.6f %.6f %.6f\n", v[1], v[2], v[3]))
              end
              f:write("endloop\nendfacet\n")
            end
            for j = 0, along - 1 do
              for i = 0, around - 1 do
                local a, b = at(i, j), at(i + 1, j)
                local c, d = at(i + 1, j + 1), at(i, j + 1)
                facet(a, b, c)
                facet(a, c, d)
              end
            end
            f:write("endsolid bolt\n")
            f:close()
            pc.file.import{path = path, scan = true}
            pc.doc.rebuild()
            os.remove(path)
            local scan
            for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
            local p = at(10, 60)
            local t = pc.design.read_thread{scan = scan, at = {p[1], p[2], p[3]}}
            assert(math.abs(t.pitch - 1.25) < 0.01 and t.right_handed and not t.internal, t.pitch)
            assert(t.nearest[1].size == "M8", t.nearest[1].size)
            "#,
        ),
        read_thread,
    );
    t.add(
        CommandSpec::new("design.scan_deviation", "How a body lies against a scan")
            .param("body", ParamKind::Id, "The body being modelled")
            .param("scan", ParamKind::Id, "The scan it is modelled from")
            .optional(
                "range",
                ParamKind::Number,
                "On the surface within this, mm; 0.1",
            )
            .returns("{within, inside, outside, rms, measured}")
            .read_only()
            .note(
                "Each scan point's signed distance to the body's solid, where both sit: \
             `within` is the share within ±range, `inside` the share deeper inside the body \
             (material the scan does not have: a hole or pocket still to cut), `outside` \
             the share farther out (the scan's other parts, or material still to add), \
             and `rms` the RMS distance of those within. The Deviation from scan tool \
             paints the scans so while modelling, against the body being edited or the \
             one selected.",
            )
            .see_also("inspect.compare")
            .example(
                "A scan of a 10 mm cube against a body 2 mm short of it",
                r#"
            local cube = pc.doc.new_body{}
            pc.design.primitive{body = cube}
            assert(#pc.doc.rebuild() == 0)
            local path = os.tmpname() .. ".stl"
            pc.file.export{path = path, bodies = {cube}}
            pc.doc.set_visible{id = cube, visible = false}
            pc.file.import{path = path, scan = true}
            pc.doc.rebuild()
            os.remove(path)
            local scan
            for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
            local short = pc.doc.new_body{}
            pc.design.primitive{body = short, kind = {Box = {length = 10, width = 10, height = 8}}}
            assert(#pc.doc.rebuild() == 0)
            local d = pc.design.scan_deviation{body = short, scan = scan}
            assert(d.within == 0.5 and d.outside == 0.5 and d.inside == 0, d.within)
            "#,
            ),
        scan_deviation,
    );
}

fn scan_symmetry_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let scan = BodyId(a.id("scan")?);
    let body = BodyId(a.id("body")?);
    let accept = a.opt_number("accept")?.unwrap_or(SYMMETRY_ACCEPT);
    scan_symmetry(ctx, scan, body, accept)
}

fn read_hole(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let scan = BodyId(a.id("scan")?);
    let at = a.vec3("at")?;
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to fit with"))?;
    let r = crate::read_scan::read_round(ctx.document, kernel, scan, at)
        .map_err(CommandError::failed)?;
    let xyz = |v: [f64; 3]| json!({"x": v[0], "y": v[1], "z": v[2]});
    Ok(json!({
        "diameter": r.diameter,
        "depth": r.depth,
        "hole": r.hole,
        "sigma": r.sigma,
        "start": xyz(r.start),
        "axis": xyz(r.axis),
        "sweep": r.sweep_deg,
        "fillet": r.fillet().then(|| json!({
            "radius": r.diameter / 2.0,
            "design": metrology::design::radius_mm(r.diameter / 2.0),
            "concave": r.hole,
        })),
        "design": metrology::design::length_mm(r.diameter),
        "nearest": if r.fillet() {
            Vec::new()
        } else {
            crate::read_scan::nearest_sizes(r.diameter, r.hole, 3)
        },
    }))
}

fn snap_to_scan(
    bench: &mut DesignWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let body = BodyId(a.id("body")?);
    let scan = BodyId(a.id("scan")?);
    let (point, normal, at) = (a.vec3("face_point")?, a.vec3("face_normal")?, a.vec3("at")?);
    let surface = crate::snap::face_surface(ctx.document, body, point)
        .ok_or_else(|| CommandError::failed("the body has no face there"))?;
    let kind = match surface {
        kernel_api::FaceSurface::Plane { .. } => kernel_api::FitKind::Plane,
        kernel_api::FaceSurface::Cylinder { .. } => kernel_api::FitKind::Cylinder,
        _ => {
            return Err(CommandError::failed(
                "only a flat or round face snaps to a scan",
            ));
        }
    };
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to fit with"))?;
    let (fitted, sigma) = crate::snap::fit_on_scan(ctx.document, kernel, scan, at, kind, body)
        .map_err(CommandError::failed)?;
    let snap =
        crate::snap::snap_of(&surface, point, normal, &fitted).map_err(CommandError::failed)?;
    let mut call = Map::new();
    call.insert("body".into(), json!(body.0.to_string()));
    call.insert("face_point".into(), json!(point));
    call.insert("face_normal".into(), json!(normal));
    if let Some(name) = a.opt_string("name")? {
        call.insert("name".into(), json!(name));
    }
    Ok(match snap {
        crate::snap::Snap::Move {
            distance,
            angle_deg,
        } => {
            let n = {
                let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2])
                    .sqrt()
                    .max(1e-300);
                normal.map(|c| c / len * distance)
            };
            call.insert("translation".into(), json!(n));
            let feature = super::run(bench, "design.move_faces", &call, ctx)?;
            json!({"feature": feature, "moved": distance, "angle": angle_deg, "sigma": sigma})
        }
        crate::snap::Snap::Offset {
            distance,
            radius,
            fitted,
            angle_deg,
            axis_offset,
        } => {
            call.insert("distance".into(), json!(distance));
            let feature = super::run(bench, "design.offset_faces", &call, ctx)?;
            json!({
                "feature": feature,
                "radius": radius,
                "fitted": fitted,
                "design": metrology::design::radius_mm(fitted),
                "angle": angle_deg,
                "axis_offset": axis_offset,
                "sigma": sigma,
            })
        }
    })
}

fn read_thread(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let scan = BodyId(a.id("scan")?);
    let at = a.vec3("at")?;
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to fit with"))?;
    let t = crate::read_scan::read_thread(ctx.document, kernel, scan, at)
        .map_err(CommandError::failed)?;
    let xyz = |v: [f64; 3]| json!({"x": v[0], "y": v[1], "z": v[2]});
    Ok(json!({
        "pitch": t.pitch,
        "major": t.major,
        "minor": t.minor,
        "right_handed": t.right_handed,
        "internal": t.internal,
        "clarity": t.clarity,
        "length": t.length,
        "start": xyz(t.start),
        "axis": xyz(t.axis),
        "nearest": crate::read_scan::nearest_threads(t.major, t.pitch, 3),
    }))
}

fn scan_deviation(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let body = BodyId(a.id("body")?);
    let scan = BodyId(a.id("scan")?);
    let range = a
        .opt_number("range")?
        .unwrap_or(crate::scan_deviation::RANGE);
    if range <= 0.0 {
        return Err(CommandError::bad("range", "is a distance above zero"));
    }
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to measure with"))?;
    let inputs =
        crate::scan_deviation::inputs(ctx.document, scan, body).map_err(CommandError::failed)?;
    let values = crate::scan_deviation::measure(kernel, &inputs).map_err(CommandError::failed)?;
    Ok(crate::scan_deviation::reading(&values, range))
}
