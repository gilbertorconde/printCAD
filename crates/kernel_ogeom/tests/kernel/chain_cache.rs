//! A build that resumes from a state the body's last builds kept makes
//! exactly what a build from the first feature makes: after an edit at the
//! end of the history, in its middle, and before a pattern that repeats an
//! earlier feature.

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

struct Part {
    host: Benches,
    engine: ScriptEngine,
    body: core_document::BodyId,
}

impl Part {
    /// A plate, three pockets, a boss, a pocket patterned and a last boss.
    fn new() -> Self {
        let mut registry = DocumentService::default();
        registry
            .register_workbench(Box::new(wb_sketch::SketchWorkbench::default()))
            .unwrap();
        registry
            .register_workbench(Box::new(wb_design::DesignWorkbench::default()))
            .unwrap();
        let mut host = Benches {
            registry,
            document: Document::new("cache"),
        };
        let mut engine = ScriptEngine::new();
        let out = engine.run_script(
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 60, height = 40}
            pc.design.pad{sketch = s, length = 6}
            "#,
            "plate.lua",
            &mut host,
        );
        assert_eq!(out.error, None);
        let body = host.document.bodies()[0].id;
        let out = engine.run_script(
            &r#"
            for i = 0, 2 do
                local h = pc.sketch.new{body = "BODY", plane = "XY", offset = 6}
                pc.sketch.circle{sketch = h, x = 10 + i * 15, y = 10, radius = 3}
                pocket = pc.design.pocket{sketch = h, depth = 4}
            end
            local b = pc.sketch.new{body = "BODY", plane = "XY", offset = 6}
            pc.sketch.circle{sketch = b, x = 30, y = 28, radius = 5}
            boss = pc.design.pad{sketch = b, length = 5}
            local p = pc.sketch.new{body = "BODY", plane = "XY", offset = 6}
            pc.sketch.rect{sketch = p, x = 4, y = 30, width = 3, height = 4}
            pc.design.pocket{sketch = p, depth = 2}
            pc.design.linear_pattern{body = "BODY", length = 40, occurrences = 4}
            local t = pc.sketch.new{body = "BODY", plane = "XY", offset = 11}
            pc.sketch.circle{sketch = t, x = 30, y = 28, radius = 2}
            last = pc.design.pad{sketch = t, length = 3}
            print(pocket) print(boss) print(last)
            "#
            .replace("BODY", &body.0.to_string()),
            "part.lua",
            &mut host,
        );
        assert_eq!(out.error, None);
        Part { host, engine, body }
    }

    fn id(&mut self, name: &str) -> FeatureId {
        let out = self
            .engine
            .run_script(&format!("print({name})"), "id.lua", &mut self.host);
        FeatureId(out.printed[0].trim().parse().unwrap())
    }

    fn set(&mut self, feature: FeatureId, field: &str, value: f64) {
        let out = self.engine.run_script(
            &format!(
                "pc.design.set{{feature = \"{}\", {field} = {value}}}",
                feature.0
            ),
            "set.lua",
            &mut self.host,
        );
        assert_eq!(out.error, None);
    }

    /// The body built from scratch, and through `cache`.
    fn build(&mut self, cache: &mut ChainCache) -> (SolidBuildResult, SolidBuildResult) {
        self.host.registry.evaluate(&mut self.host.document);
        let plan = wb_design::body_build_ops(&self.host.document, self.body).unwrap();
        let tags: Vec<_> = plan
            .op_features
            .iter()
            .map(|f| kernel_api::naming::name_of_id(f.0.as_bytes()))
            .collect();
        let detail = TessellationSettings::default();
        let fresh = OgeomKernel::new()
            .execute_solid_chain_named(&plan.ops, &tags, &detail, None, &[])
            .unwrap();
        let cached = OgeomKernel::new()
            .execute_solid_chain_cached(&plan.ops, &tags, &detail, None, &[], Some(cache))
            .unwrap();
        (fresh, cached)
    }
}

/// How many ops the part's history makes.
fn cache_len(part: &mut Part) -> usize {
    part.host.registry.evaluate(&mut part.host.document);
    wb_design::body_build_ops(&part.host.document, part.body)
        .unwrap()
        .ops
        .len()
}

fn same(fresh: &SolidBuildResult, cached: &SolidBuildResult, what: &str) {
    // The geometry, not the snapshot's text: a curve can record a longer
    // parameter range for the same edge.
    assert!(
        fresh.mesh.positions == cached.mesh.positions,
        "{what}: the same mesh"
    );
    assert!(
        fresh.mesh.indices == cached.mesh.indices,
        "{what}: the same triangles"
    );
    assert!(
        fresh.mesh.face_names == cached.mesh.face_names,
        "{what}: the same names"
    );
    let volume = |r: &SolidBuildResult| {
        OgeomKernel::new()
            .physical_properties(&r.brep_blob)
            .unwrap()
            .volume_mm3
            .unwrap()
    };
    let (a, b) = (volume(fresh), volume(cached));
    assert!((a - b).abs() < 1e-9 * a.max(1.0), "{what}: {a} against {b}");
    assert_eq!(fresh.bounds_mm, cached.bounds_mm, "{what}: the same bounds");
    assert_eq!(
        fresh.mesh.edges, cached.mesh.edges,
        "{what}: the same outline"
    );
}

#[test]
fn a_resumed_build_is_the_build_from_scratch() {
    let mut part = Part::new();
    let mut cache = ChainCache::default();
    let (fresh, cached) = part.build(&mut cache);
    same(&fresh, &cached, "the first build");

    assert_eq!(cache.resumed(), 0, "nothing kept yet");
    let ops = cache_len(&mut part);

    // The first edit of a feature builds from where the body's last build
    // ended at most; the next edit of it starts at the feature itself.
    let last = part.id("last");
    part.set(last, "length", 4.0);
    let (fresh, cached) = part.build(&mut cache);
    same(&fresh, &cached, "the last feature edited");
    part.set(last, "length", 5.0);
    let (fresh, cached) = part.build(&mut cache);
    same(&fresh, &cached, "the last feature edited again");
    assert!(
        cache.resumed() >= ops - 2,
        "resumed after {} of {ops} ops",
        cache.resumed()
    );

    let boss = part.id("boss");
    part.set(boss, "length", 6.0);
    let (fresh, cached) = part.build(&mut cache);
    same(&fresh, &cached, "a feature before the pattern edited");
    part.set(boss, "length", 7.0);
    let (fresh, cached) = part.build(&mut cache);
    same(&fresh, &cached, "the same feature again");
    let at_boss = cache.resumed();
    assert!(
        at_boss > 0 && at_boss < ops - 4,
        "resumed after {at_boss} ops"
    );

    let pocket = part.id("pocket");
    part.set(pocket, "depth", 3.0);
    let (fresh, cached) = part.build(&mut cache);
    same(&fresh, &cached, "an earlier feature edited");
    assert!(
        cache.resumed() < at_boss,
        "an earlier edit goes back further"
    );

    let (fresh, cached) = part.build(&mut cache);
    same(&fresh, &cached, "nothing changed");
    assert_eq!(cache.resumed(), ops, "nothing to build again");
}

/// A pocket already through the plate, made deeper, cuts the same hole:
/// the build stops there and takes the rest of the last build's result,
/// which is what a build from scratch makes. A change that does alter the
/// solid builds on.
#[test]
fn an_edit_that_makes_the_same_solid_takes_the_rest_as_built() {
    let mut part = Part::new();
    // Every edit kept: whether one is worth comparing is timed, and a busy
    // machine times it differently.
    let mut cache = ChainCache::keeping_every_edit();
    part.build(&mut cache);
    let pocket = part.id("pocket");
    part.set(pocket, "depth", 10.0);
    let (fresh, cached) = part.build(&mut cache);
    same(&fresh, &cached, "through the plate");
    assert!(!cache.reused(), "a new hole: built on");

    part.set(pocket, "depth", 12.0);
    let (fresh, cached) = part.build(&mut cache);
    same(&fresh, &cached, "deeper, still through");
    assert!(cache.reused(), "the same hole: the rest as built");

    part.set(pocket, "depth", 2.0);
    let (fresh, cached) = part.build(&mut cache);
    same(&fresh, &cached, "shallower");
    assert!(!cache.reused(), "a different hole: built on");
}
