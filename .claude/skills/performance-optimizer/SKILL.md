---
name: performance-optimizer
description: "Measure a performance problem, profile it, and keep only changes that beat a predeclared threshold in paired runs. Use for a slow import, a slow rebuild after an edit, a stuttering view, a slow frame, a long startup or exit, or a slow test. Not for correctness bugs (use systematic-debugging) or style cleanup (use comment-cleanup)."
---

# Performance optimizer

Optimize only a measured problem. Keep a change only when comparable
evidence shows it improves the agreed metric with no broken constraint.
`printcad-kernel` (imports) and `printcad-render-loop` (edges) say it too: measure before optimizing.

## 1. Define

Before any edit, write down:

- the user-visible symptom and the workload that shows it (which file,
  which gesture, how many bodies);
- one primary metric of the same type as the symptom, for example:

| Symptom | Metric and source |
|---|---|
| An import is slow | the phase breakdown of `cargo run --release -p kernel_ogeom --example import_bench -- <file>`; reference timings live in the import-performance memory |
| A rebuild after an edit is slow | `cargo run --release -p kernel_ogeom --example rebuild_bench -- --ops … --before …` |
| The view stutters while orbiting | the 1 s `printcad.frame` log (fps, phase costs) under `PRINTCAD_BENCH_ORBIT`, which expects one scene redraw per frame |
| A still view keeps redrawing | `scene: N/s` in the status bar and the frame log under `PRINTCAD_BENCH_SPIN`, which expects zero; a redraw means the fingerprint changed |
| A UI-only frame is slow | the frame log; a UI-only frame costs about 2 ms on any model |
| The edge pass is suspected | the same orbit with `PRINTCAD_NO_EDGES=1`; `PRINTCAD_EDGE_MIN_PX` culls edges of small bodies |
| The status bar flickers | `status_changes` in the frame log (a readable bar changes about once a second) |
| Exit or teardown is slow | `PRINTCAD_EXIT_AFTER_MS` on the loaded document, with validation layers in a debug build |
| A package call is slow | the wasmtime epoch deadline (25 ms for frame and input calls); long work belongs in a job (`jobs.rs`) |
| A test or the suite is slow | `time cargo test -p <crate> <filter>`; `cargo test -p wb_wasm` builds the SDK and is slow by design |

- the target, and the minimum gain that keeps an experiment (beyond
  noise);
- the hard constraints: output identical at any thread count
  (`tests/step_import.rs` asserts it), `scene_fingerprint` completeness,
  no `queue_wait_idle` or `device_wait_idle` on the hot path, a
  validation-clean app, the per-body build and cache invariants, undo
  and ops unchanged, bounded memory (packages run under a memory cap).

Stop with `BLOCKED` when the problem does not reproduce and no
trustworthy evidence defines a safe proxy.

## 2. Baseline

- Always `--release`: the dev profile optimizes dependencies, including
  the kernel, but not our own crates, so a debug build is slower in ways
  that do not reflect release.
- Run the relevant correctness tests first, so a baseline failure is not
  blamed on an experiment.
- Fix the build mode, the file, the thread count, cache state and warm-up
  (`ChainCache` makes the second build of a body cheap; say which build
  you time). Cover the reported case and the boundary that could reverse
  the conclusion (a small STEP and a large assembly; the kernel scales
  with faces, the renderer with triangles).
- A user's file is copied to `/tmp` before anything reads it. Never
  measure on the huge assembly files when a small one shows the effect.
- Record raw results, the median or the relevant percentile, the spread
  and the environment. Interleave baseline and candidate runs (A, B, A,
  B) instead of all-before then all-after.
- Report an inconclusive result as inconclusive. Never rerun until a gain
  appears.

## 3. Profile and hypothesize

Profile the whole path before one function: the import bench's phases,
`cargo flamegraph`, the frame log's phase costs, `tracing` spans.
Separate root cost from cold start, debug builds and instrumentation
overhead. Write a short ordered hypothesis list: expected gain,
mechanism, files, risk, how to verify. Drop a hypothesis with no
measurable mechanism or that needs speculative scale.

Known shapes: a parallel pass is `ogeom_core::parallel::map_ordered`,
never nested (`tess::Faces::{Wide, Inline}` says which level owns the
threads); a deferred mesh pass that re-parses snapshots costs more than
meshing inline; progress announcements inside a loop are noise
(`progress::detail` or `stage_at`, never `context`).

## 4. One experiment at a time

- Apply the smallest change that tests one mechanism.
- For caching, batching or parallelism, protect invalidation, ordering,
  idempotency, cancellation (`Canceller`, `drop_build`) and bounded
  resources. A cache keyed by less than what the result depends on is a
  stale-geometry bug, not a gain.
- Run the focused tests, then repeat the exact baseline benchmark.
- `KEEP` only when the gain beats the predeclared threshold and every
  constraint passes. Otherwise `DISCARD` and revert only that
  experiment. Never lower the threshold after you see the result.
- After a kept change, take a new baseline before the next hypothesis.
- Local kernel builds go through `CARGO_TARGET_DIR=/var/tmp/pc-local`,
  never `/tmp` (RAM) or the home partition.

## 5. Report

Metric, workload and environment; the baseline and final numbers with
spread; the hypothesis ledger with each `KEEP` or `DISCARD` and its
evidence; the checks run; and the verdict: `IMPROVED`, `NO_CHANGE` or
`BLOCKED`. A benchmark gain is not proven impact in the app; say which
it is. Update the import-performance memory when reference timings move.
