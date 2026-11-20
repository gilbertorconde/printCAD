//! Building everything that changed, here and now.

use std::collections::HashSet;

use core_document::{BodyId, BuildError, Document, DocumentService};
use kernel_api::{KernelQueries, MeshSolidResult, TessellationSettings};
use kernel_ogeom::OgeomKernel;

use crate::{BuildFailure, Built, Chain, Next};

/// Reads the shapes a document takes from files: what a body's shape was
/// replaced by, and the stand-in of a body whose history does not build.
pub trait ShapeReader {
    /// The shape the file `asset` of `document` holds, or `None` when it
    /// could not be read (the reader says why where its caller looks).
    fn read(
        &mut self,
        kernel: &mut OgeomKernel,
        document: &Document,
        asset: uuid::Uuid,
        detail: &TessellationSettings,
    ) -> Option<MeshSolidResult>;
}

/// No shapes are read from files.
impl ShapeReader for () {
    fn read(
        &mut self,
        _: &mut OgeomKernel,
        _: &Document,
        _: uuid::Uuid,
        _: &TessellationSettings,
    ) -> Option<MeshSolidResult> {
        None
    }
}

/// What the kernel made of a body's plan.
#[derive(Debug)]
pub enum KernelStep {
    /// Nothing was asked of it: the plan was empty or failed outright.
    Nothing,
    /// The whole plan built.
    Built,
    /// An op failed. With `built_before`, the body shows the history
    /// before the failing feature; else it shows nothing built.
    Failed {
        failure: BuildFailure,
        built_before: bool,
    },
    /// The build was cancelled; the body keeps what it showed.
    Cancelled,
    /// The kernel panicked, with this message; the body keeps what it
    /// showed.
    Panicked(String),
}

/// How a body's last build went.
#[derive(Debug)]
pub struct BodyBuild {
    pub body: BodyId,
    /// The feature its plan could not plan.
    pub refused: Option<BuildError>,
    pub kernel: KernelStep,
}

/// What [`build_all`] did.
#[derive(Debug, Default)]
pub struct Build {
    /// Each body built, with how its last build went, in the order they
    /// were first built.
    pub bodies: Vec<BodyBuild>,
    /// What went wrong outside the bodies' histories (a mirrored copy the
    /// kernel could not mirror), for the caller to show.
    pub notes: Vec<String>,
}

/// Build every solid that changed, on this thread, until nothing is left
/// to build: each round reads the shapes taken from files, works out what
/// stands on the solids just built (`DocumentService::evaluate`), builds
/// the mesh histories, and builds the bodies the benches plan. Mirrored
/// copies take their shapes at the end.
pub fn build_all(
    document: &mut Document,
    registry: &DocumentService,
    kernel: &mut OgeomKernel,
    detail: &TessellationSettings,
    shapes: &mut dyn ShapeReader,
) -> Build {
    let mut build = Build::default();
    let mut read = HashSet::new();
    // A plan that fails keeps its body's features clean, so the loop ends;
    // the bound is a guard, not the expected way out.
    for _ in 0..32 {
        read_shapes(document, registry, kernel, detail, shapes, &mut read);
        registry.evaluate(document);
        build_meshes(document, registry);
        let jobs = registry.rebuild_jobs(document);
        if jobs.is_empty() {
            break;
        }
        for job in jobs {
            let begun = crate::begin(document, job);
            let kernel = match begun.next {
                Next::Build(plan) => build_body(document, kernel, begun.body, &plan, detail),
                Next::Emptied | Next::Unplanned => KernelStep::Nothing,
            };
            let body = BodyBuild {
                body: begun.body,
                refused: begun.refused,
                kernel,
            };
            match build.bodies.iter_mut().find(|b| b.body == body.body) {
                Some(last) => *last = body,
                None => build.bodies.push(body),
            }
        }
    }
    read_shapes(document, registry, kernel, detail, shapes, &mut read);
    mirror_copies(document, &mut build.notes);
    build
}

/// Build one body's plan and land what it built.
fn build_body(
    document: &mut Document,
    kernel: &mut OgeomKernel,
    body: BodyId,
    plan: &core_document::BuildPlan,
    detail: &TessellationSettings,
) -> KernelStep {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::build_until_failure(Chain::of_plan(plan, detail), |chain| {
            crate::execute(kernel, &chain, None)
        })
    }));
    match built {
        Ok(Built::Whole(result)) => {
            crate::store_answers(
                document,
                &plan.op_features,
                &plan.probes,
                &plan.projections,
                &result,
            );
            crate::store_solid(document, body, result);
            KernelStep::Built
        }
        Ok(Built::Before {
            result,
            ops,
            probes,
            projections,
            failure,
        }) => {
            crate::store_answers(
                document,
                &plan.op_features[..ops],
                &probes,
                &projections,
                &result,
            );
            crate::store_solid(document, body, result);
            crate::record_failure(document, &failure);
            KernelStep::Failed {
                failure,
                built_before: true,
            }
        }
        Ok(Built::Failed { failure, .. }) if failure.cancelled() => KernelStep::Cancelled,
        Ok(Built::Failed {
            failure,
            nothing_built,
        }) => {
            crate::record_failure(document, &failure);
            if nothing_built {
                crate::clear_unbuilt(document, body);
            }
            KernelStep::Failed {
                failure,
                built_before: false,
            }
        }
        Err(panic) => KernelStep::Panicked(panic_message(&panic)),
    }
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Read every shape the document takes from a file and has not yet
/// (`Document::bodies_awaiting_shape`), each once a build.
fn read_shapes(
    document: &mut Document,
    registry: &DocumentService,
    kernel: &mut OgeomKernel,
    detail: &TessellationSettings,
    shapes: &mut dyn ShapeReader,
    read: &mut HashSet<(BodyId, uuid::Uuid)>,
) {
    for (body, asset) in document.bodies_awaiting_shape() {
        if !read.insert((body, asset)) {
            continue;
        }
        if let Some(shape) = shapes.read(kernel, document, asset, detail) {
            crate::land_shape(document, registry, body, asset, shape);
        }
    }
}

/// Build every mesh body's history that changed.
fn build_meshes(document: &mut Document, registry: &DocumentService) {
    for plan in registry.mesh_build_plans(document) {
        if document.derived().mesh_steps(&plan).is_some() {
            continue;
        }
        let Some(start) = document.mesh_start(plan.body) else {
            continue;
        };
        let answer = match plan.restored_start(&start.mesh) {
            Some(answer) => Ok(answer),
            None => kernel_ogeom::QUERIES
                .build_mesh_chain(&start.mesh, &plan.ops)
                .map_err(|e| e.to_string()),
        };
        plan.land(document, answer);
    }
}

/// Give every mirrored copy without one its snapshot: its source's,
/// mirrored by the kernel.
fn mirror_copies(document: &mut Document, notes: &mut Vec<String>) {
    for (body, from, plane) in document.copies_awaiting_shape() {
        match kernel_ogeom::QUERIES.mirror(
            &from,
            plane.point.map(f64::from),
            plane.normal.map(f64::from),
        ) {
            Ok(blob) => {
                document.set_mirrored_shape(body, &from, blob);
            }
            Err(error) => {
                let name = document
                    .body(body)
                    .map_or("a mirrored copy", |b| b.name.as_str());
                notes.push(format!("`{name}` could not be mirrored: {error}"));
            }
        }
    }
}
