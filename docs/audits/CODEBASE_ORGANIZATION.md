# Codebase organization and reuse audit

**Baseline:** commit `b9b815e8`. This is a development backlog, not a user guide or a claim that every source line has been reviewed.

**Start here:** address F1's ordering inconsistency, then take F3's small save-helper extraction. Treat the larger module splits as incremental work alongside changes to those areas, not a rewrite.

## Overall assessment

**The project has sound crate boundaries, but some important behavior is repeated inside those boundaries and several modules have accumulated too many responsibilities.** Keep the workspace architecture. Concentrate refactoring on shared policy, deterministic queries, and smaller internal modules.

The strongest positive evidence is the executable layer gate: `node scripts/check-layers.mjs` passes for **31 crates**. Geometry contracts, the kernel adapter, document operations, workbench registration, rendering, and application orchestration have distinct owners. The existing seams are a better starting point than a new framework or a general-purpose `utils` crate.

The highest-value findings are:

1. Feature-history ordering is independently implemented and already differs between native planners and package enumeration.
2. Desktop and headless rebuilds independently encode failure recovery and mesh-history orchestration.
3. Identical document-save logic and repeated triangle queries can have one owner.
4. Large host and workbench modules mix coordination, domain logic, and command transport.
5. The host/workbench boundary checks cover fewer workbenches than their stated invariant.

### Scope and exclusions

Reviewed workspace manifests and architectural gates; indexed tracked Rust source; inspected representative document, host, kernel-worker, native-workbench, package-host, persistence, and geometry paths. A simple exact-block scan helped locate candidates; findings below were checked in source rather than inferred from that scan alone. It does not establish a codebase-wide duplication percentage.

**Excluded:** right-sidebar inputs, task/property panel drawing, native versus SDK widget implementations, and the ongoing unified panel migration. Large panel/editor files are not findings. Viewport handles are included because their duplicated code is hit-testing and on-canvas interaction, not sidebar layout.

This is a static organization/reuse audit. No application code changed, no full build or test suite ran, and no runtime defect is claimed unless explicitly demonstrated. Security, dependency freshness, numerical accuracy, and performance are not comprehensively audited here. The website, packaging, and SDK examples received structural inspection rather than a line-by-line implementation review.

## Prioritized backlog

Estimates are engineering days for one person familiar with the subsystem, including focused tests and review; full platform validation may add time. Priority reflects correctness exposure and maintenance value, not just lines removed.

### First batch

| ID | Priority | Work | Estimate | Evidence level |
| --- | --- | --- | --- | --- |
| F1 | P1 | Centralize deterministic feature ordering; fix package tie-breaking | 1–2 days | Confirmed policy duplication and ordering difference |
| F2 | P1 | Share rebuild failure/prefix policy between desktop and headless | 3–5 days | Confirmed duplicated policy; no runtime failure reproduced |
| F3 | P2 | Give document-server atomic saves one implementation | 0.5–1 day | Confirmed near-identical implementation |
| F4 | P2 | Consolidate application/document triangle queries | 1–2 days | Confirmed repeated algorithms and differing edge handling |
| F5 | P2 | Make the workbench seam gate cover its full invariant | 0.5–1 day | Confirmed incomplete and independently maintained deny lists |

### Subsequent work

| ID | Priority | Work | Estimate | Evidence level |
| --- | --- | --- | --- | --- |
| F6 | P2 | Split host command/domain logic from script execution | 2–4 days | Confirmed responsibility concentration |
| F7 | P2 | Split large native-workbench implementation modules | 2–4 days per selected workbench | Confirmed responsibility concentration; not proof of duplication |
| F8 | P3 | Reuse the viewport scalar-arrow handle mechanics | 0.5–1 day | Confirmed duplication |
| F9 | P3 | Move document archive implementation out of the root module | 1–2 days | Confirmed responsibility concentration |

P1 means prioritize for follow-up, not an emergency or a verified user-visible failure. Do not combine these into one broad refactor PR.

## Findings

### F1: Repeated history queries already disagree on tie-breaking

**Evidence**

- `crates/core_document/src/lib.rs:2968`, `Document::body_history_of`, collects nodes and sorts by `(seq, FeatureId)`.
- `crates/workbenches/wb_design/src/build.rs:25`, `design_features_of_body`, independently filters, decodes evaluated data, and sorts by `(seq, id)`.
- `crates/workbenches/wb_surface/src/build.rs:21`, `surface_features_of_body`, repeats that collection pattern.
- `crates/workbenches/wb_mesh/src/lib.rs:551`, `mesh_plans`, independently groups, sorts, and restricts steps to the body's tip.
- `crates/workbenches/wb_wasm/src/host.rs:395`, `Reach::features`, sorts converted nodes **only by `seq`**.
- The underlying feature storage is a `HashMap` (`crates/core_document/src/feature.rs:183`), exposed through `all_nodes`.

**Why it matters**

The native Design planner explicitly documents ties from concurrent replica insertions. Sorting only by sequence retains the source iteration order for equal sequences; that is not a deterministic order across independently constructed hash maps. Package feature enumeration therefore does not provide the same tie-breaking rule as the native history helpers. This is a source-level inconsistency; its effect on a particular package was not reproduced.

Repeated full-tree filtering also makes it easy to disagree about tip limits, suppressed nodes, and raw versus evaluated feature data. It may be costly when repeated, but no performance gain is claimed without measurement.

**Action**

1. Add a core-owned ordered feature query, with `(seq, id)` as the documented total order. Build the body-specific query on it rather than adding another sorting convention.
2. Use it in package enumeration and migrate the native planners incrementally. Keep feature decoding in its owning workbench.
3. Make tip and suppression filtering explicit options or separate helpers. Do not accidentally exclude suppressed or later nodes from an inspection/listing API that must show them.

**Done when:** equal-sequence nodes enumerate identically regardless of insertion order; package and native paths share that ordering rule; body-less nodes, suppressed steps, and tip navigation retain their intended semantics. Use an existing document/package test harness, not a new test binary.

### F2: Desktop/headless execution shares primitives but duplicates rebuild policy

**Evidence**

- `crates/app_shell/src/headless.rs:461`, `Headless::rebuild`, collects probe/projection arrays and topology tags, executes chains, identifies the failing feature, truncates to that feature's start, and retries the prefix.
- `crates/app_shell/src/kernel_worker.rs:820–977` independently computes feature ranges/tags and constructs a prefix build after a failure, retaining probes and projections only through the prefix.
- `crates/core_document/src/rebuild.rs`, `BuildPlan::stop_at_failing_formulas`, also truncates operations, feature mappings, probes, and projections together.
- `crates/app_shell/src/headless.rs:563`, `rebuild_meshes`, and `crates/app_shell/src/app/mesh_histories.rs:51`, `drive_mesh_histories`, both enumerate mesh plans, build from mesh starts, restore unplanned bodies, and attribute outer failures to a feature.

There is already valuable reuse: `store_built_solid`, `mark_unbuilt`, `store_mesh_build`, and shape landing are shared. Preserve it.

**Why it matters**

Any new build metadata or change to “what remains visible after a failing feature” needs coordinated edits in several paths. Headless currently retries in a loop; the worker makes a prefix attempt through its own result/cancellation machinery. Different scheduling is necessary, but the rules for feature boundaries and retained metadata should not have separate owners.

**Action**

Extract a pure `BuildPlan` operation for finding a failing feature's start and producing the corresponding prefix plus unbuilt features. Reuse it for formula failures and execution failures where their semantics agree. Separately share mesh-plan collection/restoration decisions and result attribution, leaving threads, stale-result rejection, caches, preview ranges, progress, and cancellation in their respective runners.

Do **not** replace both paths with one async engine, or put the concrete kernel in `core_document`.

**Done when:** a focused parity harness checks first-feature failure, a multi-op feature failing part-way through, later-feature failure, probe/projection cutoffs, and removing the final mesh step. Desktop-only cancellation and preview behavior remain covered by their existing owner tests. Test the policy directly before moving scheduling code.

### F3: Atomic document saving has two implementations in the same crate

**Evidence**

- `crates/doc_server/src/direct.rs:132`, `write_atomically`.
- `crates/doc_server/src/daemon.rs:231`, `write_atomically`.

Both create the parent directory, choose a `.prtcad.writing` neighbor, write the bytes, synchronize the file, and rename it over the target. Apart from imports/signature spelling, the algorithm is the same.

**Why it matters**

Changes to temporary naming, cleanup, replacement behavior, or durability could reach one backend but not the other. This is a small, high-confidence reuse opportunity at the correct existing ownership boundary.

**Action**

Move the helper into a private `doc_server` file-I/O module and call it from both backends. First preserve current behavior. Treat any concurrency or crash-durability strengthening as a separately specified change; the duplication alone is not proof of data loss.

**Done when:** both paths use the same helper; existing daemon/direct tests pass; shared tests cover creation, replacement, and failed-write behavior. Validate platform-specific replacement semantics on supported platforms. Do not broaden this into a project-wide persistence framework for settings and caches.

### F4: Picking, notes, and package geometry repeat triangle math

**Evidence**

- `crates/app_shell/src/app/input.rs:1658`, `ray_triangle`, and `crates/app_shell/src/app/note_tool.rs:267`, `ray_hit`, implement the same ray/triangle test with the same determinant threshold and positive-distance requirement.
- `crates/app_shell/src/app/input.rs:2048`, `point_triangle_distance_sq`, and `crates/workbenches/wb_wasm/src/geometry.rs:290`, `triangle_distance_sq`, implement the same region-based nearest-triangle calculation.
- `crates/core_document/src/notes.rs:377`, `closest_on_triangle`, calculates the nearest point using the same regions, with a near-zero final-denominator fallback. The input implementation's final division has no corresponding check.

**Why it matters**

These consumers answer closely related questions about the same display mesh, but robustness fixes can diverge. The denominator difference is evidence of independently maintained behavior, not a demonstrated crash or a complete degeneracy analysis.

**Action**

Start with a private host helper for the two identical ray tests. For the cross-crate closest-point calculation, place a small display-mesh query module in `core_document`, which both consumers already depend on. Derive squared distance from the shared closest-point result. Agree on degenerate-triangle behavior before changing callers.

Do not put algorithms into the data-only `kernel_api`. Do not force the kernel adapter's separate double-precision geometry or standalone numerical crates through a document-level helper.

**Done when:** one test table covers triangle interior, edges, vertices, zero-area triangles, repeated vertices, rays behind the origin, and parallel rays. Existing note-placement and picking tests still pass. Keep mesh traversal policy, face identity, clipping, and occlusion distance limits with callers.

### F5: The host/workbench seam checks are narrower than the stated rule

**Evidence**

- `crates/app_shell/src/app/seam_lint.rs:10`, `BYPASSES`, names the Design and Sketch crate names, selected IDs/types, and the datum kind.
- `.github/workflows/ci.yml:88` maintains a second grep list. It is narrower still: it does not include the Rust feature-type names covered by the test.
- Other built-in workbenches exist, but their names/types are not in these checks.

**Why it matters**

A host special-case for another workbench could pass both checks even though it violates the documented architectural boundary. This is a guardrail coverage gap, not evidence that such a bypass currently exists. The crate-layer gate cannot catch it: the host is above all workbenches in the permitted dependency graph.

**Action**

Maintain one authoritative seam checker covering the built-in workbench identities and prohibited direct dependencies. Invoke the same checker from local validation and CI. Distinguish legitimate facade/package-service types from concrete workbench feature knowledge so widening the check does not create arbitrary exemptions.

**Done when:** fixtures prove that references to each built-in workbench are rejected in host source, while permitted facade access passes. Adding a built-in workbench must update or automatically extend the check.

### F6: Host scripts are also a domain command library and execution controller

**Evidence**

`crates/app_shell/src/app/scripts.rs` contains command specifications, file/keymap command adaptation, agent-access handling, script execution, argument decoding, document commands, arrays/notes/textures helpers, pictures, and reports. Specific seams already exist:

- `with_file_args` at line 2349 and `all_commands` at line 2502.
- Script/agent answer conversion around lines 3587–3632.
- Body/texture/array parsing around lines 4636–5075.
- Report assembly at line 5483.

`crates/app_shell/src/headless.rs` imports `doc_commands`, `document_command`, and `tree_item` from this module. It also maintains `DOC_COMMANDS` and its own `file_commands` declarations. Headless sharing is good, but reusable domain operations are housed inside the desktop script-controller module.

**Action**

Split within `app_shell` first: command catalog, document command handlers, script runner/approval orchestration, and report/picture adapters. Move shared file argument specifications into the catalog while allowing intentional headless/desktop differences. Consider an explicit availability property only if it makes the existing headless whitelist unnecessary without coupling core command types to window details.

Do not introduce a new crate solely to shorten this file, and do not move agent authorization out of the execution path that enforces it.

**Done when:** headless and desktop depend on the same domain-command module, common argument contracts cannot drift unnoticed, unavailable window commands remain unavailable headlessly, and existing hostile-argument/command-example/agent-approval tests pass.

### F7: Large workbench root modules mix domain operations and trait orchestration

**Evidence, excluding panel rendering**

- `crates/workbenches/wb_sketch/src/lib.rs`: the `Workbench` implementation begins at line 3223, with selection geometry (`connected_curves`, rectangle intersection helpers) around lines 7663–7815 and external-projection/reference following around lines 7854–8152. Dedicated `selection.rs`, `external.rs`, and `external_ref.rs` modules already exist.
- `crates/workbenches/wb_inspect/src/lib.rs`: comparison/thickness creation, tolerance construction, report construction, shape conversion, fitting/growing, and command response serialization precede and accompany the large `Workbench` implementation. Domain modules such as `compare.rs`, `dimension.rs`, and `construction.rs` already exist.
- `crates/workbenches/wb_design/src/build.rs`: rebuild scheduling/invalidation, cross-body inputs, plan assembly, and operation generation share one module. Examples include `rebuild_jobs` at line 82, `plan_feature` at line 653, `extrude_op` at line 1661, and hole/thread construction from line 2253 onward.

**Why it matters**

Unrelated changes meet in the same files, and reusable functions are harder to discover. File size alone does not prove poor design: these files also contain tests, and splitting a match over feature variants does not automatically improve it.

**Action**

Move cohesive groups, preserving signatures and visibility: sketch reference following into its reference module; selection math into selection; Inspection feature creation and command-answer construction into their domain modules; Design operation generation into feature-family submodules under `build/`. Keep the trait implementation as dispatch and lifecycle coordination, not a second domain implementation.

**Done when:** each moved group has a clear domain owner; no new cyclic dependency or broadly public helper API is introduced; existing interaction, command, and solid-chain tests pass. Make each move separately from behavioral changes. Avoid a generic base-workbench framework: these workbenches have materially different lifecycles.

### F8: Two viewport scalar handles repeat drawing, hit-testing, and drag math

**Evidence**

- `crates/app_shell/src/app/array_handle.rs:49–153`, `SpacingDrag` and `Handle`.
- `crates/app_shell/src/app/section.rs:41–133`, `OffsetDrag` and `Handle`.

Both project a scalar displacement onto a screen vector, snap to 0.1 mm, hit-test the arrow shaft/tip, and generate the same arrow/dot/value-label geometry. The constants for arrow length and reach also match.

**Action**

Extract a small host-local scalar-arrow representation and press-relative drag helper. Pass the label/value, step, color, and screen geometry in. Keep each domain's projection and event lifecycle separate: the section handle has an end-on fallback and flip behavior, while the array scales displacement by occurrence index.

This is viewport reuse, not part of the excluded sidebar migration. Do not change the independently shared `transform_gizmo` contract merely to eliminate two local implementations.

**Done when:** both callers use the shared primitive; tests retain snapping, end-on handling, and cancellation semantics. If appearance changes, capture both handles before/after with the same view and DPI.

### F9: Document archive mechanics sit inside the document root module

**Evidence**

`crates/core_document/src/lib.rs:3982–4606` contains save/load entry points, watched serialization, thumbnail/stamp extraction, compression detection, archive reading/writing, shape-path synchronization, and face-color blob encoding. The same root module also owns the document state and many domain mutators. Neighboring domain modules already use separate files for operations, history, assets, placement, and notes.

**Action**

Move archive implementation and encoding helpers into a private persistence module, retaining the public `Document` methods and format behavior. Let it access private state through normal Rust module privacy; do not add mutable document escape hatches to make the split easier.

**Done when:** existing persistence/compatibility tests pass with unchanged public APIs and serialized semantics. This is a code organization change, not grounds for raising the container format or rewriting saved files.

## Existing reuse and boundaries worth preserving

### Keep these architectural choices

1. **Contracts separate from implementations:** `kernel_api`/`bench_api`, the kernel adapter, and the renderer have distinct purposes. Avoid moving geometry algorithms into contracts to make imports convenient.
2. **One document mutation path:** operation application, inverse history, and derived geometry are already separated. Refactors must not bypass recorded document mutators.
3. **Workbench registry as the host seam:** reuse belongs behind this seam, not in host branches that recognize concrete feature kinds.
4. **Independent mathematical libraries:** solver, image, metrology, camera, and gizmo code have portability goals. Similar vector operations alone do not justify coupling them to document types.
5. **Shared service entry points:** headless already reuses import landing/export helpers; package-store installation reuses remote download validation. Extend these patterns instead of replacing them.

### Similar code that is not automatically redundancy

- Native/package conversion is a boundary adapter. Converting IDs, coordinate frames, and serializable values is expected; only shared policy should be extracted.
- The SDK has its own WIT copy for separate builds. `crates/workbenches/wb_wasm/tests/packages.rs:169–178` checks agreement. A guarded copy serving packaging independence is different from two implementations silently drifting.
- Desktop and headless need different scheduling, settings, and window availability. Share their decisions, not their entire runtime.
- CPU pictures, GPU rendering, and website rendering target different environments. No evidence from this audit supports replacing them with one renderer.
- Repeated tiny vector/segment calculations in independent numerical crates may be cheaper and safer than a new shared dependency. Prefer measured maintenance problems over blanket deduplication.

## Suggested execution sequence

### Batch 1: Establish shared rules

1. **F1:** add an equal-sequence ordering regression case and fix the package path; extract the common query without changing filtering semantics.
2. **F3:** consolidate save helpers in a small behavior-preserving PR.
3. **F5:** make the seam check authoritative for all built-in workbenches.

### Batch 2: Reduce behavior drift

1. **F4:** specify degenerate mesh-query behavior and consolidate the matching algorithms.
2. **F2:** extract pure prefix/failure policy and exercise desktop/headless parity; leave scheduling alone.

### Batch 3: Improve navigation without changing behavior

1. **F6:** split host command catalog/domain handlers from execution orchestration.
2. **F7:** move one workbench's cohesive non-panel logic at a time.
3. **F8/F9:** take the small viewport helper and persistence module splits when those areas are next touched.

For every implementation PR: run the narrow affected crate tests first, then the repository-required gates before committing. Keep moves separate from behavioral fixes, preserve existing test ownership, and do not duplicate full end-to-end test suites merely to validate extraction.

## Audit checks and limitations

Commands ran from the repository worktree. Results describe the inspected baseline, not a validated refactor:

| Check | Result |
| --- | --- |
| `node scripts/check-layers.mjs` | **Passed:** 31 crates, each depending only on lower layers |
| `node scripts/check-solver-deps.mjs` | **Did not reach dependency checks:** `ENOENT` creating a temporary directory under `/tmp/claude` |
| `node scripts/check-archive-deps.mjs` | **Did not reach dependency checks:** same missing temporary-directory parent |
| Source searches and exact-block candidate scan | Completed; candidates manually inspected; no comprehensive duplication metric |
| Full tests, clippy, runtime smoke/captures | **Not run:** documentation-only static audit |

The standalone-check failures are environment prerequisites, not evidence of forbidden dependencies. No workaround or dependency installation was attempted. Existing untracked worktree files were left untouched.

**Next action:** turn F1 into a small issue with the equal-sequence test and ordering helper as its acceptance criteria.
