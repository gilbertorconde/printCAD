# Rebuilds, previews and imports

**Feature preview.** While a bench's `editing_feature` is one that builds
solid (Design's open task), `recompute.rs` asks its body's builds for a
preview (`request_build_solid`'s `preview`, `ChainRequest::preview`
over the feature's op range): the chain keeps the feature's tool and the
body before it (an adding feature) or after it (a cutting one) in
`SolidBuildResult::preview`. The body's geometry is then that solid, so
picks match what is drawn; `session.builds.previews` keeps the whole solid, put
back when the task closes (and in the save snapshot), and the tool, drawn
`front_only` in `rendering.preview_color`/`preview_opacity` with its edges
over its faces (the renderer's `TranslucentFront` pass, back faces culled).
A previewed fillet or chamfer also has the chain work out the largest size
that builds (`ops::dressup::size_limit`, kept in the `ChainCache` by the
history before it and `SolidOp::size_limit_key`), answered as
`SolidBuildResult::size_limit` and kept by `Document::store_size_limit`;
Design's `size_limit.rs` reads it while the key holds, and its handle
(`Handle::stop`) and fields (`Quantity::drag_max`) stop a drag there.

Recompute loop: workbench edits document → features marked dirty via the
dependency DAG → `drive_part_recompute` (each frame) asks every bench for
its `rebuild_jobs` (a `BuildPlan` of `SolidOp`s per body, the bench settling
the dirty flags of the plan's features and inputs itself) → kernel worker
thread → results land in the document's imported-geometry sidecar →
rendered/picked like any body. A failure leaves the body its history
before the failing feature: a plan that cannot plan a feature stops there
(`BuildPlan::failed`, the features after it in `unbuilt`; Design's
`body_plan` and Surface's `plan_until_failure`, beside the strict
`body_build_ops`/`body_plan` that answer an error), and a kernel failure
at op N has the worker build the ops before the failing feature's first
(`run_chain`, `KernelResponse::SolidBuilt::failed`); the failing feature
carries the error and every feature after it "not built"
(`mark_unbuilt`), and a first feature failing leaves no built shape
(`SolidFailed::nothing_built`). A body has one build out at a time
(`session.builds.out`, `recompute::BuildsOut`): a plan made while it builds waits in its
place, a newer one replacing it, and goes when the build lands
(`build_landed`), so a drag that changes a feature every frame builds only
the latest shape rather than replaying each step. A history that changed shape goes through
`invalidate_body`; a history jump or Recompute All through `invalidate_all`.
Builds skip what an edit did not change (what is left is in `docs/ROADMAP.md`):
`kernel_ogeom::ChainCache` (one per body on the worker) keeps the chain's
state (model clone, solid, face names, pattern tools, probe answers) at the
start of the op that differed from the last build and at the end, and the
next build whose ops agree up to there resumes from it; an edited op that
makes the same solid as before (`fingerprint`: geometry, not snapshot text)
returns the last result; faces and edges whose geometry, deflection and
chords are unchanged keep their triangulation, outline and bounds
(`reuse.rs`, keyed by geometry since every op renumbers its faces). The
app drops a build a newer plan replaced (`KernelWorker::drop_build`: a
queued one never runs, a running one stops unless past half its body's
usual time), meshes a body coarse while plans outrun builds (`moving`,
`coarse`) and finely 300 ms after they stop, builds an open task's body only
up to the edited feature (`QueuedBuild::up_to_edited`, the rest once
settled, `preview_rest`), and builds bodies on a pool of 2 to 4 threads
(`build_loop`) beside the worker thread for everything else.
`examples/rebuild_bench.rs` (`--ops`, `--before`) measures it.

Import performance: the per-solid work and each mesh's face pass go through
`ogeom_core::parallel::map_ordered` (order-preserving, so output is identical
at any thread count, which `tests/kernel/step_import.rs` asserts). Never nest two
`map_ordered` passes: `tess::Faces::{Wide, Inline}` says which level owns the
threads. Import meshes inline from the model already in memory; a deferred
pass would have to parse every snapshot back, which cost more than the
meshing. `crates/kernel_ogeom/examples/import_bench.rs` reports the phase
breakdown; measure with it before optimizing. STEP text is decoded lossily
(exporters emit Latin-1 in string literals).

Progress/cancel: the worker installs one `Watch` per job
(`kernel_ogeom::{Watch, watched, Canceller}`, re-exported so app code never
depends on `ogeom` directly). `kernel_ogeom::progress::context` announces our
own labels prefixed with `CONTEXT_PREFIX`; ogeom announces its own stages on
the same thread-local channel. The worker's sink files them into a shared
`Activity` slot (not a channel: `mpsc::Sender` is `Send` but not `Sync`), the
status bar reads it each frame, and `Canceller` backs the Cancel button. Our
op/face/solid loops call `progress::checkpoint()` themselves, since
`triangulate_face` has no checkpoints of its own. Stages announcing
`(done, total)` (kernel-side, or ours via `progress::stage_at` fed by a
shared monotone counter in the parallel import loop) draw as a determinate
bar instead of the spinner; a new context resets counts to unknown.
The reader's warnings never reach the terminal one by one: `ImportedModel.report`
carries them (by kind with counts, the full prose, untrimmed face ids, skipped
keywords). With `diagnostics.import_report` on (Preferences › General, off by
default) `app/import_report.rs` writes them to
`$TMPDIR/printcad/import-reports/<stem>-<stamp>.txt` (temp, so the system
clears them) and logs the path; off, one line gives the counts and names the
switch. That file is what goes to the kernel's maintainer.
**Shape health.** Every imported body is run through the kernel's checker as
it is read (`kernel_ogeom/src/health.rs`, a few ms a body) and carries a
`ShapeHealth` on its `ImportedGeometry`: broken findings (an algorithm
reading the shape answers wrongly) versus suspect (harmless). A broken body's
tree row, and every row above it, draws in the danger colour with the
findings as its tooltip, and the tree and viewport menus offer "Repair
shape". The repair is an op (`RequestBodyRepair`, a history barrier like an
import, since the unrepaired shape would have to be re-derived from the
file); the repaired geometry is derived from it by `drive_shape_repairs`
(`recompute.rs`), so a peer's request and a reopened document repair the
same way. The property panel's Physical group (volume, surface area,
centre of mass) is measured by the kernel worker on demand for the body the
panel shows, once per geometry revision (`drive_measurement`), never during
an import, where it would cost ~15 ms a body. **Announcement discipline:** `progress::
context` marks a *phase* and resets the display: call it once per phase,
never per body or per face. Anything emitted inside a loop is
`progress::detail` (a kernel-style sub-stage: shown under a sequential
phase, ignored while our counted `stage_at` loop owns the display) or
`stage_at`. Our counted stage speaks alone; the kernel's per-body stages
from twenty threads are noise, not information; the 1 s frame log's
`status_changes` counts status-text changes per second (a readable bar
changes ~1/s; a slot overwritten by threads changes every frame).
