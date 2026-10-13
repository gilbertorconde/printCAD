//! Time the rebuilds an edit makes: a plate with pockets, bosses, a
//! patterned slot and a fillet, built through the workbenches' commands as
//! a script would, then built by the kernel as the application builds it:
//! from scratch, after an edit to its last feature, after one in the middle
//! of its history, and again unchanged.
//!
//! ```text
//! cargo run --release -p kernel_ogeom --example rebuild_bench [-- --ops]
//! ```
//!
//! `--ops` prints each feature's time and the meshing as the chain logs
//! them (target `printcad.chain`). `--before` is the baseline that keeps
//! nothing between builds: every build from the first feature, the whole
//! history while a task is open, at full detail.
//!
//! Then an imported part (the bundled `drive_frame_upper.step`) as a
//! body's base, a boss beside it whose height is edited: the history is short,
//! the solid large and curved, so meshing is most of a build. Then a pocket
//! cut down through that part's middle and widened: a small edit the
//! boolean must still make against the whole solid. Last, plates with a
//! growing grid of holes, one more added at a corner: what a small edit
//! costs as the solid grows.

use core_document::{
    CommandArgs, CommandError, CommandResult, CommandSpec, Document, DocumentService, FeatureId,
    WorkbenchId, WorkbenchRuntimeContext,
};
use kernel_api::{SolidBuildResult, TessellationSettings};
use kernel_ogeom::{ChainCache, OgeomKernel};
use scripting::ScriptEngine;

struct Benches {
    registry: DocumentService,
    document: Document,
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
        wb.run_command(&id, &args, &mut ctx)
    }
}

const PLATE: &str = r#"
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 160, height = 100}
pc.design.pad{sketch = s, length = 10}
"#;

/// The rest, on the plate's body (`BODY` stands for its id).
const PART: &str = r#"
for i = 0, 11 do
    local h = pc.sketch.new{body = "BODY", plane = "XY", offset = 10}
    pc.sketch.circle{sketch = h, x = 15 + (i % 6) * 26, y = 15 + math.floor(i / 6) * 70, radius = 4}
    local pocket = pc.design.pocket{sketch = h, depth = 6}
    if i == 5 then hole = pocket end
end
middle = nil
for i = 0, 5 do
    local b = pc.sketch.new{body = "BODY", plane = "XY", offset = 10}
    pc.sketch.circle{sketch = b, x = 25 + i * 22, y = 50, radius = 6}
    local boss = pc.design.pad{sketch = b, length = 8}
    if i == 2 then middle = boss end
end
local p = pc.sketch.new{body = "BODY", plane = "XY", offset = 10}
pc.sketch.rect{sketch = p, x = 10, y = 30, width = 6, height = 8}
pc.design.pocket{sketch = p, depth = 4}
pc.design.linear_pattern{body = "BODY", length = 130, occurrences = 6}
pc.design.fillet{body = "BODY", radius = 1, face_point = {80, 50, 0}, face_normal = {0, 0, -1}}
local t = pc.sketch.new{body = "BODY", plane = "XY", offset = 18}
pc.sketch.circle{sketch = t, x = 80, y = 50, radius = 3}
last = pc.design.pad{sketch = t, length = 4}
"#;

fn main() {
    let before = std::env::args().any(|a| a == "--before");
    if std::env::args().any(|a| a == "--ops") {
        tracing_subscriber::fmt()
            .with_env_filter("printcad.chain=debug")
            .with_target(false)
            .without_time()
            .init();
    }
    let mut registry = DocumentService::default();
    registry
        .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
        .unwrap();
    registry
        .register_workbench(Box::new(wb_design::DesignWorkbench::default()))
        .unwrap();
    let mut host = Benches {
        registry,
        document: Document::new("bench"),
    };
    let mut engine = ScriptEngine::new();
    let out = engine.run_script(PLATE, "plate.lua", &mut host);
    assert_eq!(out.error, None, "the plate builds");
    let body = host.document.bodies()[0].id;
    let part = PART.replace("BODY", &body.0.to_string());
    let out = engine.run_script(&part, "part.lua", &mut host);
    assert_eq!(out.error, None, "the part builds");
    let ids = engine.run_script(
        "print(last) print(middle) print(hole)",
        "ids.lua",
        &mut host,
    );
    let feature = |i: usize| FeatureId(ids.printed[i].trim().parse().expect("a feature id"));
    let (last, middle, hole) = (feature(0), feature(1), feature(2));
    let named = |name: &str| match name {
        "middle" => middle,
        _ => last,
    };
    let detail = TessellationSettings::default();
    let mut cache = ChainCache::default();
    let full = detail.clone();

    let build =
        |host: &mut Benches, cache: &mut ChainCache, label: &str, detail: &TessellationSettings| {
            host.registry.evaluate(&mut host.document);
            let mut plan = wb_design::body_build_ops(&host.document, body).unwrap();
            // "…, up to <feature>" builds as an open task does: up to the end
            // of the feature it edits.
            if let Some(name) = label.split(", up to ").nth(1).filter(|_| !before) {
                let feature = named(name);
                let end = plan
                    .op_features
                    .iter()
                    .rposition(|f| *f == feature)
                    .unwrap()
                    + 1;
                plan.ops.truncate(end);
                plan.op_features.truncate(end);
            }
            let tags: Vec<_> = plan
                .op_features
                .iter()
                .map(|f| kernel_api::naming::name_of_id(f.0.as_bytes()))
                .collect();
            let started = std::time::Instant::now();
            let built: SolidBuildResult = OgeomKernel::new()
                .execute_solid_chain_cached(
                    &plan.ops,
                    &tags,
                    if before { &full } else { detail },
                    None,
                    &[],
                    &[],
                    (!before).then_some(&mut *cache),
                )
                .unwrap();
            println!(
                "{label:<28} {:>4} ops, {:>2} kept{:<13}  {:>8.1} ms  {} triangles",
                plan.ops.len(),
                cache.resumed(),
                if cache.reused() { ", rest reused" } else { "" },
                started.elapsed().as_secs_f64() * 1000.0,
                built.mesh.indices.len() / 3
            );
        };
    let mut set_field = |host: &mut Benches, feature: FeatureId, field: &str, value: f64| {
        let out = engine.run_script(
            &format!(
                "pc.design.set{{feature = \"{}\", {field} = {value}}}",
                feature.0
            ),
            "set.lua",
            host,
        );
        assert_eq!(out.error, None);
    };

    build(&mut host, &mut cache, "from scratch", &detail);
    set_field(&mut host, last, "length", 5.0);
    build(&mut host, &mut cache, "last feature edited", &detail);
    set_field(&mut host, last, "length", 6.0);
    build(&mut host, &mut cache, "last feature edited again", &detail);
    set_field(&mut host, middle, "length", 9.0);
    build(&mut host, &mut cache, "a middle feature edited", &detail);
    set_field(&mut host, middle, "length", 10.0);
    build(&mut host, &mut cache, "the same one again", &detail);
    build(&mut host, &mut cache, "nothing changed", &detail);
    // As the application builds a body being dragged, then settled.
    let coarse = TessellationSettings {
        mesh_deviation: detail.mesh_deviation * 4.0,
        chord_tolerance: detail.chord_tolerance * 4.0,
        angular_tolerance_deg: (detail.angular_tolerance_deg * 3.0).min(45.0),
        ..detail.clone()
    };
    set_field(&mut host, last, "length", 7.0);
    build(&mut host, &mut cache, "dragged, coarse", &coarse);
    build(&mut host, &mut cache, "settled, full detail", &detail);
    // A task open on a middle feature: built up to it, then the rest.
    set_field(&mut host, middle, "length", 11.0);
    build(&mut host, &mut cache, "task edit, up to middle", &detail);
    set_field(&mut host, middle, "length", 12.0);
    build(&mut host, &mut cache, "next edit, up to middle", &detail);
    build(&mut host, &mut cache, "settled, the rest", &detail);
    // A pocket cut through the plate, then deeper: the same hole.
    set_field(&mut host, hole, "depth", 12.0);
    build(&mut host, &mut cache, "a pocket made through", &detail);
    set_field(&mut host, hole, "depth", 14.0);
    build(&mut host, &mut cache, "deeper, the same hole", &detail);

    imported_part(&detail, before);
    holed_plate(&detail, before);
}

/// A plate with a grid of holes, one more hole added at a corner: the edit
/// touches a few faces of a solid that holds hundreds.
fn holed_plate(detail: &TessellationSettings, before: bool) {
    use kernel_api::{BooleanOp, Placement, PrimitiveKind, SolidOp};
    let hole = |x: f64, y: f64| SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius: 1.5,
            height: 20.0,
            angle_deg: 360.0,
        },
        placement: Placement {
            origin: [x, y, -5.0],
            ..Placement::default()
        },
        op: BooleanOp::Cut,
    };
    println!();
    for n in [8usize, 16, 24] {
        let size = 10.0 * n as f64 + 10.0;
        let mut ops = vec![SolidOp::Primitive {
            kind: PrimitiveKind::Box {
                length: size,
                width: size,
                height: 10.0,
            },
            placement: Placement::default(),
            op: BooleanOp::NewSolid,
        }];
        for i in 0..n {
            for j in 0..n {
                ops.push(hole(10.0 + 10.0 * i as f64, 10.0 + 10.0 * j as f64));
            }
        }
        let tags: Vec<u64> = (1..=ops.len() as u64 + 1).collect();
        let mut cache = ChainCache::default();
        let built = OgeomKernel::new()
            .execute_solid_chain_cached(
                &ops,
                &tags[..ops.len()],
                detail,
                None,
                &[],
                &[],
                Some(&mut cache),
            )
            .unwrap();
        let mut faces = built.mesh.faces.clone();
        faces.sort_unstable();
        faces.dedup();
        ops.push(hole(5.0, 5.0));
        let started = std::time::Instant::now();
        OgeomKernel::new()
            .execute_solid_chain_cached(
                &ops,
                &tags,
                detail,
                None,
                &[],
                &[],
                (!before).then_some(&mut cache),
            )
            .unwrap();
        println!(
            "{:<31} {:>4} ops, {:>3} kept  {:>8.1} ms  {} bytes of snapshot",
            format!("{} faces, a corner hole added", faces.len()),
            ops.len(),
            cache.resumed(),
            started.elapsed().as_secs_f64() * 1000.0,
            built.brep_blob.len()
        );
    }
}

/// An imported part as the base, a boss beside it, edited.
fn imported_part(detail: &TessellationSettings, before: bool) {
    use kernel_api::{BooleanOp, Kernel, Placement, PrimitiveKind, SolidOp};
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/drive_frame_upper.step");
    let model = OgeomKernel::new().import_step(&path, detail).unwrap();
    let body = &model.bodies[0];
    let (lo, hi) = body.bounds_mm.unwrap();
    let boss = |height: f64| SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius: 3.0,
            height,
            angle_deg: 360.0,
        },
        placement: Placement {
            // Beside the part: the whole of it is meshed either way.
            origin: [f64::from(hi[0]) + 10.0, f64::from(lo[1]), f64::from(lo[2])],
            ..Placement::default()
        },
        op: BooleanOp::Fuse,
    };
    let base = SolidOp::Shape {
        brep: body.brep_blob.clone(),
    };
    // A pocket cut into the part's middle, straight down through it: the
    // boolean meets the large solid, though it touches little of it.
    let pocket = |radius: f64| SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius,
            height: f64::from(hi[2] - lo[2]) + 2.0,
            angle_deg: 360.0,
        },
        placement: Placement {
            origin: [
                f64::from(lo[0] + hi[0]) / 2.0,
                f64::from(lo[1] + hi[1]) / 2.0,
                f64::from(lo[2]) - 1.0,
            ],
            ..Placement::default()
        },
        op: BooleanOp::Cut,
    };
    let tags = [1, 2];
    let mut cache = ChainCache::default();
    println!();
    for (label, edit) in [
        ("imported, from scratch", boss(4.0)),
        ("imported, boss edited", boss(5.0)),
        ("imported, boss edited again", boss(6.0)),
        ("imported, pocket cut in", pocket(2.0)),
        ("imported, pocket widened", pocket(2.5)),
        ("imported, pocket widened again", pocket(3.0)),
    ] {
        let started = std::time::Instant::now();
        let built = match OgeomKernel::new().execute_solid_chain_cached(
            &[base.clone(), edit],
            &tags,
            detail,
            None,
            &[],
            &[],
            (!before).then_some(&mut cache),
        ) {
            Ok(built) => built,
            Err(e) => {
                println!("{label:<31} failed: {}", e.message);
                continue;
            }
        };
        println!(
            "{label:<31} {:>4} ops, {:>2} kept  {:>8.1} ms  {} triangles",
            2,
            cache.resumed(),
            started.elapsed().as_secs_f64() * 1000.0,
            built.mesh.indices.len() / 3
        );
    }
}
