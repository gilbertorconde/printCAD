# Solver problem and migration contract

This specifies the public boundary of [RFC 0003](0003-standalone-sketch-solver.md).
The first copy of the library is `crates/sketch_solver`. Its modules are
`problem`, `compile`, `curves`, `spline`, `residual`, `solve`, `freedom`, and
`diagnosis`, `input`, `contact`, `trace`, and optional `replay`. The module
paths, rather than re-exports, identify public types.

## Identity and ordering

`PointId(u128)`, `CurveId(u128)` and `ConstraintId(u128)` are distinct
caller-supplied identities. `VariableIndex(usize)` and `EquationIndex(usize)`
address numerical arrays. None implements `Deref` or generates identities.
The adapter maps a UUID through `as_u128()` and reconstructs it through
`Uuid::from_u128()`. The inverse is exact, including UUID ordering used to
order ellipse focus rules. Point and curve identities remain separate even
if they carry the same number; duplicate identities within a domain fail
validation. `ItemReference` explicitly distinguishes a point from a curve.

Origin and coordinate axes use `PointReference::Origin` and
`CurveReference::{XAxis,YAxis}`; ordinary references carry a typed ID.
They are immutable, contribute no free variables, and are inserted only
when an active relation uses any reference. All three reference points
precede shape variables, as in the application compiler.

`Problem.geometry: Vec<Geometry>` preserves the application's interleaving
of points and curves. The compiler first visits shape-bearing curves,
then geometry again for points and radii, then constraints for contact
parameters. It never sorts geometry, constraints, residuals or summations
through a lookup map. Maps provide lookup only. Focus rules retain their
explicit curve-ID sort. IDs in the result always name the caller's input.

## Public records

`problem::Problem` owns:

- `geometry: Vec<Geometry>`: points with `Vector { x, y }` coordinates; lines
  with typed endpoints; circles and arcs with centre, radius and arc
  endpoints; ellipses with centre, major-vector and initial minor radius,
  optional arc endpoints; hyperbolas/parabolas with centre or vertex,
  axis, minor and endpoints; splines with typed control points, degree,
  knots, rational weights and periodicity.
- `constraints: Vec<Constraint>`: ordered `ConstraintId` plus one of the
  42 semantic `Relation` variants in the equation inventory. Only active,
  driving application relations enter this vector. Internal alignment
  includes a typed role and item. Generic gap/contact operands use typed
  item references, rather than an untyped ID with guessed interpretation.
- `external: Vec<ItemReference>`: explicitly fixed geometry and its points;
  external shape degrees of freedom never count as free.
- `held_points: Vec<PointReference>`: the current gesture's held points, used in
  focus classification as well as variable pinning.
- `application_held_points: Vec<PointReference>`: fit-spline controls and text
  outlines that the application updates after applying a result. They
  have no font, image, fit-point or document dependencies in the core.
- `settings: Settings`: effective positive finite tolerance, positive
  iteration limit, LM retry limit, damping limits and floor, finite
  difference step, direction floor, step cap, rank tolerance and diagnosis
  limit. `Settings::default()` uses the current numerical constants;
  replay records every value. The adapter resolves the application's
  zero-iteration and nonpositive-tolerance fallback before construction.

The validated entry points are:

- `compile::compile(&Problem) -> Result<System, input::InputError>`.
- `solve::solve(&Problem) -> Result<Iteration, input::InputError>`.
- `freedom::analyze(&Problem) -> Result<Freedom, input::InputError>`.
- `diagnosis::analyze(&Problem) -> Result<Diagnosis, input::InputError>`.

`Iteration` returns ordered `f64` values, `SolveOutcome`, iterations even
for unsuccessful attempts, residual infinity norm and effective threshold.
`System` provides typed point/radius/shape accessors and variable/equation
indices into numerical arrays. Held values stay unchanged. `Length`,
`Radians` and `Ratio` distinguish quantities; `Direction` is dimensionless
and `ControlEdgeIndex` identifies a spline control edge.

`trace::snapshot` records initial values, free indices, contact parameters,
selected branches, ray order, residual specifications and equation provenance.
Every constraint has a `ConstraintRows` record, including valid zero-row
relations. `replay::run`, enabled with serde, combines this trace, the answer,
ordered residual values with origins, freedom and diagnosis into one versioned
result. Equation units and mixed scaling are specified in the equation inventory;
the numerical result does not assign a single physical unit to all residuals.

`Freedom` reports free variables, Jacobian rank, uncompiled free shapes and
their difference. Rank compilation includes arc endpoint rules even without
user residuals. A hidden parabola minor slot is pinned. This analysis does not
replace the application's nudge-based `free_points` policy.

`Diagnosis` reports DoF, ordered redundant/conflicting IDs and `analyzed`.
The default limit is 60 semantic constraints, including valid zero-row relations.
Each probe recompiles geometry, shape variables and derived foci. Standalone
diagnosis applies a solved `f64` configuration. `diagnose_at` allows the
application to supply its configuration after f32 write-back, spline refit and
text following, without callbacks into the application.

`input::InputError` distinguishes duplicate IDs, missing references,
nonfinite geometry, invalid relation domains/settings and invalid splines.
Finite degenerate geometry remains admissible; failure to converge is a
numerical outcome. Replay rejects unsupported schema versions separately.

## Adapter contract

`prepare` builds the ordered problem with reversible UUID-to-typed-ID
conversion. An exhaustive match enforces all 42 relation mappings. Inactive and
reference dimensions are filtered here; unresolved stored references retain
the application's zero-row compatibility through trusted `compile_system`.

`wb_sketch::solver_report::compilation_reports` inspects every stored relation:
inactive/reference/compiled/no-equations disposition, typed equation indices,
strict semantic input errors, and omitted pair indices for partial Offset
relations. Valid zero-row relations and malformed saved relations are observable
separately. This inspection does not alter numerical solve or diagnosis policy.

`apply` writes f32 positions and radii, normalizes ellipse axes in the
current f32 operation order, renames major/minor dimensions and internal
roles, reverses affected axis line endpoints, and places derived foci.
It updates `unsolved` and fully constrained flags, refits splines, then
follows text outlines. A failed held attempt restores geometry and runs
an ordinary solve from the pre-attempt gesture position. Constraint role
changes and the existing retry order are checked by application fixtures.

The adapter promotes stored numbers without changing their precision.
In particular, ellipse minor radii are computed with the existing f32
length and multiplication before promotion, and pitch directions are
normalized in f32 before promotion. The core receives these initial
values explicitly; it does not know f32 storage or normalize them again.

## Domains and compatibility

Lengths share the caller's unit; angular quantities are radians.
`DistanceX/Y >= 0` measure absolute separation, negative values signed
separation. A missing second point measures the first from zero.
`Angle`, `AngleToAxis`, `AngleAtPoint`, `AngleThreePoints` and `PolarPitch`
are periodic directions with current wrapped errors. `ArcAngle` and
`ArcLength` use a CCW sweep in `(0, 2π]`, with equal endpoints denoting a
full turn. New standalone inputs outside this sweep domain fail explicitly;
the application's explicit compilation report identifies unsupported legacy
domains; its trusted path retains saved-document behaviour. This does not introduce a signed-sweep or multi-turn arc model.

Tangency selects the initial external/internal branch once; equal errors
select external. A shared line/arc or arc/arc endpoint uses its endpoint
tangent relation. Gap inside/nested selection and the refraction ray's
near/far endpoints are also fixed at compilation. Curve contacts add
nearest parameters from the current coarse/fine searches. Curve lengths
keep 128 chord intervals; spline basis normalization retains its current
degree, knots and rational/periodic evaluation contracts.

## Captured corpus and checks

`wb_sketch/src/solver/migration.rs` builds 24 deterministic input scenes;
`migration.json` stores the complete inputs and application answers.
It records ordinary/held/freedom compilation, held-attempt outcome,
final outcome, iterations when available, stored geometry and constraint
roles, DoF and diagnosis coverage/IDs. A fixed floating tolerance of
`1e-6 * max(1, |expected|)` is declared before refactoring. IDs, topology,
enum tags, order, integer metadata and iteration counts compare exactly.
This is migration equivalence, not a mathematical oracle.

Separate assertions check line lengths, fixed and held points, ordinary
fallback, contact branch distances and translated text outlines. Controlled
changes of a coordinate, DoF and iteration count must fail comparison.
Existing solver and internal-geometry tests supply the broader independent
geometric assertions listed in the equation inventory.

Capture is an explicit test-only maintenance action, never a production
fallback. Capture once before changing the solver and review the diff;
do not recapture to make a changed solver pass.

```sh
PRINTCAD_CAPTURE_SOLVER_CORPUS=1 cargo test -p wb_sketch --release \
  --no-default-features --locked --lib \
  solver::migration::captured_application_corpus -- --exact
cargo test -p wb_sketch --release --no-default-features --locked --lib solver::
cargo test -p wb_sketch --release --no-default-features --locked --lib \
  solver::migration::measure_migration_baseline -- --ignored --exact --nocapture
```

The timing test separates compilation, pure iteration and diagnosis. Its
solve phase includes input cloning, write-back, refit and text following. Clean and incremental library-test
builds use an isolated target directory and the same toolchain, profile,
features and lockfile. No speedup is inferred from cached workspace builds.

Captured before numerical refactoring, Rust 1.98, release, locked,
`wb_sketch --no-default-features --lib`: a clean library-test build took
52.151 s; a rebuild triggered by touching only `solver.rs` took 20.845 s.
Five timing trials, each 50 repetitions of the 24-case corpus, gave these
medians (microseconds per whole corpus): compilation 77.076, solve including
input clone/application updates 226.028, diagnosis 1036.967. Ranges were
69.222–93.797, 221.468–241.323 and 1014.077–1066.131 respectively. These
measurements describe this machine and test harness, not a CI budget or
a standalone-core speedup. The engine is the solver at `566bf70` with
test-only observation and corpus modules attached; its numerical code is
unchanged.

## Extraction acceptance

The extraction is complete as of 2026-10-10. Acceptance includes the frozen
application and semantic corpora, independent geometric checks, standalone
dependency and feature gates, browser/WASI compilation, the full workspace
release suite (1843 passed, 0 failed, 2 ignored across 58 suites), and the
CI budget (1843 tests across 35 programs in 21.4 s; slowest program 8.7 s,
within the 150 s total / 60 s per-program limits). The constrained-sketch
startup smoke passed with zero GPU errors; it does not exercise pointer or
keyboard interaction.

At the user's request, final post-extraction build and runtime measurements
are deferred and do not block acceptance. No speedup or absence of a
performance regression is claimed. The baseline above remains a reference;
there are no comparable final measurements. Optimization, if needed, is
separate work. This acceptance update changes documentation only.

For a future comparison, use the same Rust 1.98 toolchain, release profile,
features and locked dependency versions on an idle machine. Build the
`wb_sketch --no-default-features --lib` test target without running it in a
fresh target directory, touch only `crates/sketch_solver/src/compile.rs`,
then rebuild that same target. The baseline touch changed the workbench's
`solver.rs`; report this difference in the compilation boundary explicitly.
Run the timing test above five times (50 repetitions of all 24 cases each),
reporting medians and ranges for compilation, pure iteration,
solve including clone/application updates, and diagnosis. Pure iteration
has no pre-extraction baseline. The extraction adds the local solver package
to Cargo.lock; existing dependency versions are unchanged.
