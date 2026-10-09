# RFC 0003 Standalone 2D sketch solver

- Status: proposed
- Date: 2026-10-09
- Scope: a new `sketch_solver` crate, the `wb_sketch` solver adapter,
  numerical fixtures, a replay example and CI checks

## Summary

Extract the 2D constraint solver into a library that accepts a complete
numerical problem and returns a result without reading or mutating a
workbench `Sketch`. Keep sketch editing, document persistence and derived
geometry updates in `wb_sketch`, behind an adapter to that library.

The purpose is to make solver regressions reproducible from small inputs,
test the mathematics independently of sketch editing, and compare the same
problem across revisions or implementations. The extraction preserves the
current algorithm and sketch behaviour. Numerical improvements can then
be reviewed against the captured corpus in separate changes.

## Why extract a solver that already runs headless

The solver already has numerical tests and runs without a window:

```sh
cargo test -p wb_sketch --release --no-default-features --lib solver::
```

The remaining coupling is at the library boundary. `solver` is a private
module of `wb_sketch`; its public functions accept the workbench's `Sketch`.
Even with the `egui` feature disabled, the package depends on
`core_document`, `kernel_api`, image decoding and font parsing. A caller
cannot depend on the numerical solver alone.

`solve` also refits splines and makes text outlines follow their anchors.
`write_back` rounds values to the sketch's `f32` storage and can rename
ellipse axes and foci. A failed regression may therefore involve building
the equations, the numerical iteration, or updating the sketch. Giving
each boundary a test makes that distinction observable.

| Test need | What an independent library provides |
| --- | --- |
| Reproduce a failure | Geometry, constraints, settings and initial values in one portable fixture |
| Check convergence | Residuals and the effective tolerance beside the outcome |
| Test random cases | Generate and shrink numerical problems without a document or tool harness |
| Compare revisions | Run the same input through a small executable with a stable output format |
| Locate a regression | Separate equation compilation, iteration and sketch write-back tests |
| Measure cost | Time problem compilation, solve and diagnosis separately from workbench work |

The smaller dependency graph should reduce the work needed to build and
link numerical tests. Measure clean build time, incremental build time and
test time before and after extraction; no speedup is assumed as an
acceptance condition. The existing interaction and recording tests remain
necessary to prove that the application uses the library correctly.

## Library boundary

Proposed dependency direction:

```text
wb_sketch tools, commands and document integration
                    |
             solver adapter
                    |
              sketch_solver
       problem compilation, curve math,
       residuals, iteration, rank and diagnosis
```

`sketch_solver` has no dependency, including a dev-dependency, on
`wb_sketch`, `core_document`, `bench_api`, `kernel_api`, the solid kernel,
UI or rendering crates, scripting, fonts or image decoding. Its public
types belong to the numerical problem. Serialization is an optional feature;
JSON file handling belongs to a replay example. Production callers use the
library directly.

The core owns the complete set of mathematical relations the sketcher
currently supports, including curve parameters introduced by contacts,
internal alignment, linked offsets and array dimensions. Moving only the
linear algebra would leave equation construction untestable outside the
workbench and would not complete this extraction.

Keep one implementation of curve evaluation, spline basis evaluation and
any numerical helper shared with sketch operations. Move the required pure
parts below `wb_sketch` and call them from both consumers. Helpers that take
a `Sketch`, sample for display, or edit document metadata remain in the
workbench. There must be no callback from the core into workbench code.

## Problem and result contract

The following names describe the proposed API; the first implementation
settles their exact fields against the complete relation inventory:

```rust
pub fn solve(problem: &Problem) -> Result<Solution, InputError>;
pub fn degrees_of_freedom(problem: &Problem) -> Result<Freedom, InputError>;
pub fn diagnose(problem: &Problem) -> Result<Diagnosis, InputError>;
```

`Problem` contains ordered records for points and curves, all initial
values, mathematical constraints, held variables and solver settings.
Curves include their topology, radii, conic shape parameters and rational
spline degree, knots, weights and control-point references. References and
externally fixed geometry arrive as explicit numerical geometry and held
variables; the library never queries a solid or an application document.

Use distinct caller-supplied ID types for points, curves and constraints,
with a reversible mapping to the sketch's UUIDs. IDs are data, never
generated during a solve. Keep internal variable and equation indices
separate from those IDs. Ordered records preserve the current traversal
order during migration; maps serve lookup, not floating-point accumulation
order.

Numerical values use `f64`; angles use radians and lengths use the caller's
consistent sketch unit. The adapter preserves existing `f32` input
promotion, intermediate rounding where it affects behaviour, and final
storage rounding. Extracting the library does not change the persisted
sketch format or introduce a precision migration.

`Solution` returns solved values, the outcome, iterations, the effective
convergence threshold and residual rows associated with constraint IDs.
Internal geometric rules have an explicit origin too. Report a constraint
with no rows explicitly, rather than making it indistinguishable from one
that was satisfied. Auxiliary contact parameters are accounted for in
degrees of freedom, and free shape variables absent from the solve receive
the same accounting as today.

Input validation checks finite values, unique IDs, typed references and
the existing admissible geometry and settings domains. An invalid input is
distinct from a valid problem that does not converge. The current sketcher
can skip constraints on missing or unsuitable geometry; the adapter records
those omissions explicitly and preserves that application behaviour during
extraction. The standalone API must not silently reduce an invalid problem.

Preserve the current outcome and diagnosis semantics, including the
diagnosis size limit and its `analyzed` flag. Failure to converge is not
proof of contradictory constraints. Improving conflict analysis belongs
to a later numerical change.

## Workbench adapter

`wb_sketch::solver` becomes a compatibility layer. Existing callers keep
their entry points while the adapter builds a `Problem`, invokes the core
and applies the returned values and metadata changes.

| Responsibility | Owner after extraction |
| --- | --- |
| Resolve mathematical relations into residuals and variables | Core |
| Evaluate curves and their auxiliary contact parameters | Core |
| Iteration, damping, step acceptance, rank and diagnosis | Core |
| Decide whether a sketch constraint is active and driving | Adapter; excluded IDs remain observable |
| Translate UUIDs, origin and axis references | Adapter |
| Mark external, held and application-derived variables | Adapter, with explicit input to the core |
| Apply solved values and update `unsolved` and fully constrained flags | Adapter |
| Apply ellipse axis, internal-role and focus changes | Adapter using explicit results from the numerical helpers |
| Refit fit-point splines and move text outlines | Workbench after write-back |
| Restore failed held-point attempts and retry an ordinary solve | Adapter |
| Decide which older constraint a new one supersedes | Workbench using core diagnosis |

Some derived foci depend on which constraints are excluded during diagnosis
and which points are held. Their classification cannot be frozen once and
reused for all probes. Preserve that dependency in pure problem compilation
and test the probe-specific held set. Application-owned spline controls and
text outlines remain a separate classification supplied by the adapter.

Separate numerical residuals before write-back from residuals measured on
the stored geometry after rounding, ellipse normalization, refitting and
text following. Tests cover both; moving a post-processing step must not
silently change the meaning of the convergence flag.

## Replay and fixture format

Add optional serialization for the problem and result and a small example:

```sh
cargo run -p sketch_solver --release --features serde \
  --example solve_problem -- problem.json answer.json
```

A fixture includes a schema version, ordered geometry and constraints,
initial values, held variables and all solver settings. Preserve `f64`
round trips. An answer identifies the engine revision and reports outcome,
degrees of freedom, diagnosis coverage, residuals and solved geometry.
Library code does no file I/O and never reads environment variables for
numerical settings.

Tangent and gap branches, nearest contact parameters and ray endpoint
ordering are selected from the starting geometry today. Preserve that
policy during extraction. Retain the selected branches and initial
auxiliary parameters in a compiled-problem trace so a failure can be
replayed without reselecting them. The semantic fixture tests equation
construction; the compiled trace helps isolate iteration failures.

Fixtures contain numerical data rather than `.prtcad` containers, images,
font paths or machine-specific files. Generated failures print their seed
and a minimized problem that becomes an ordinary regression fixture.

## Test strategy

### Numerical tests

Move the existing numerical assertions with their implementation. Separate
tests that depend on tools, dimension picking or workbench write-back into
adapter tests. An exhaustive inventory maps every current `ConstraintKind`
to core relations and identifies its residual count, units, variable count
and branch conventions. No relation may disappear during the move.

Use independent geometric assertions as well as residuals: measured
distance, incidence, tangent direction, radius and curve membership. For
known feasible cases, require convergence. A test that checks residuals only
when the solver happens to converge does not prove that case works.

### Generated problems and boundary cases

Start from planted feasible configurations, perturb their initial values,
and require recovery. Generate invalid and contradictory cases separately,
with outcomes appropriate to each. Bound problem size and case count to
keep ordinary tests inside the existing suite budget.

Include deterministic cases for zero and near-zero lengths, coincident
centres, unconstrained and fully held geometry, duplicate dimensions,
multiple simultaneous conflicts, rational and periodic splines, contact
parameters, and ellipses crossing equal axes with visible internal geometry.
Angles cover both signs, 180, 270, 360, 450 and 720 degrees and neighbours
of branch boundaries. Require an explicit refusal for values outside a
relation's domain; equivalent directions and arc sweeps have different
contracts and cannot share a blanket periodicity assertion.

Test repeated solves for drift, finite output and consistent status and
residuals. Test supported changes of unit, translations and rotations with
the appropriate tolerance and semantic transformations. Current finite
differences and scale-dependent tolerances can expose existing defects;
record their minimal cases and track fixes separately rather than widening
tolerances to make the extraction pass.

### Adapter and migration tests

Capture a corpus before redirecting the workbench to the core. Compare the
original and extracted paths on the same inputs for outcome, degrees of
freedom, analyzed status, diagnostic IDs, stored geometry and rewritten
constraint metadata. Use declared numerical tolerances for floating-point
values and exact checks for IDs and topology. Keep the original path only
as a temporary migration oracle and remove it when the corpus agrees.

Test active and driving filters, axis and external references, held-point
fallback, derived spline and text points, ellipse role swaps, and old saved
sketches. Retain the existing interaction, command recording and
sketch-to-solid tests to catch integration failures.

Cross-engine comparisons require a shared semantic subset. Positive
`DistanceX/Y` are absolute in the current sketcher, while negative values
are signed; arc dimensions and tangent branches also need explicit domain
contracts. With remaining degrees of freedom, compare constraint
satisfaction and fixed geometry rather than arbitrary free coordinates.
Even zero degrees of freedom can have multiple discrete solutions: compare
coordinates only when the fixture fixes the branch. Diagnosis sets are
compared only where both engines define them the same way.

## Implementation milestones

1. **Capture behaviour and agree on the contract.** Inventory all relations,
   hidden arc rules, references, derived variables and post-processing.
   Capture the corpus and the numerical, adapter and build-time baselines.
   Decide the crate name and problem/result records before moving code.
2. **Separate pure mathematics from sketch updates.** Introduce explicit
   preparation and application boundaries within `wb_sketch`. Move shared
   spline and curve primitives below application-specific helpers. Keep
   iteration order, defaults, branch choices and rounding unchanged; prove
   equivalence with the corpus after each boundary change.
3. **Create the independent crate.** Move problem compilation, residuals,
   numerical iteration, freedom analysis and diagnosis to `sketch_solver`.
   Redirect production calls through the adapter. Move numerical tests and
   keep adapter assertions with `wb_sketch`.
4. **Add replay and generated tests.** Implement the optional fixture format,
   compiled trace and replay example. Add bounded generation and shrinking,
   deterministic boundary matrices and per-constraint residual checks.
5. **Enforce independence and complete migration.** Add a dependency-graph
   check for every supported feature set, default and no-default-feature
   tests, and compilation for the browser and workbench-package targets.
   Verify a tiny external crate can depend on the library without linking
   application code. Remove the temporary original solver and document the
   public contract and replay commands.

Each milestone is a separate reviewable change. The extraction retains
the existing LM implementation, finite-difference Jacobian, rank threshold,
diagnosis limit and held-point retry policy. Algorithm changes, new
constraints, a sketch precision migration and publishing to crates.io are
separate decisions.

## Acceptance

- A standalone caller constructs and solves a problem through public types;
  its dependency graph contains none of the application packages listed
  above, including through optional features or dev-dependencies.
- Every current relation and hidden geometric rule has a tested mapping;
  every omission is explicitly accounted for.
- Replay uses the same core implementation as production. Serialization
  preserves the input and the numerical result within declared tolerances.
- The migration corpus preserves numerical outcomes, sketch write-back,
  topology, constraint roles, diagnosis semantics and drag fallback.
- Core tests run without the workbench, a window, a GPU, fonts or images.
  Generated cases have reproducible seeds and shrink to replayable fixtures.
- Existing workspace tests, strict clippy, formatting, browser checks and
  the sketch interaction smoke test pass within the project's test budget.
- Production has one numerical solver implementation and one source of each
  shared mathematical primitive.

## Alternatives and tradeoffs

Making `wb_sketch::solver` public would allow direct external calls, but
would retain the workbench model, dependencies and write-back side effects.
It is useful for a small runner, but does not meet this library boundary.

Moving the whole sketch workbench into a shared crate would preserve a
single model, but would also move fonts, image handling, editing and
document integration into the numerical dependency graph. A small typed
problem plus an exhaustive adapter gives the solver its own contract while
the saved sketch format stays owned by the workbench.

The main cost is maintaining the adapter as the relation model grows.
Exhaustive conversion, coverage checks and adapter fixtures make a missing
translation fail at compile time or in the suite. Keep the mathematical
definition in the core and conversion in the adapter, without a second
residual implementation there.

The replay schema adds a compatibility obligation. Version it independently
of `.prtcad`; reject unsupported versions clearly. Reproducibility means
stable problem construction and numerical results within documented
tolerances, rather than promising identical floating-point bits on every
architecture.

## Code map

- [Current solver and diagnostics](../../crates/workbenches/wb_sketch/src/solver.rs)
- [Sketch geometry and constraint records](../../crates/workbenches/wb_sketch/src/sketch.rs)
- [Spline basis and fit-point updates](../../crates/workbenches/wb_sketch/src/spline.rs)
- [Geometry helpers](../../crates/workbenches/wb_sketch/src/geom2d.rs)
- [Gap and dimension measurements](../../crates/workbenches/wb_sketch/src/measure.rs)
- [Text outline updates](../../crates/workbenches/wb_sketch/src/text.rs)
- [Interaction tests](../../crates/workbenches/wb_sketch/tests/sketcher/interaction.rs)
- [Recording tests](../../crates/workbenches/wb_sketch/tests/sketcher/recording.rs)
