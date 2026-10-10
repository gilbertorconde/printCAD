# sketch_solver

Standalone two-dimensional constraint solving: ordered semantic geometry and
relations are compiled into double-precision equations, solved with damped
Gauss–Newton iteration, and analyzed for numerical rank and constraint health.
The default build uses only the Rust standard library. `serde` is optional.

The crate contains its own `Cargo.toml`, sources and tests. It has no workspace
package or dependency inheritance. To use the same implementation elsewhere,
copy this entire directory, including tests and fixtures, then add a path
dependency on that copy. Keep changes to formulas and regression cases together.
There is no application, document, UUID, kernel, image, font or UI dependency.
Browser and WASI library compilation do not imply an SDK or WIT interface.

```rust
use sketch_solver::problem::*;

let point = PointId(1);
let problem = Problem {
    geometry: vec![Geometry::Point(Point {
        id: point,
        position: Vector { x: 4.0, y: 5.0 },
    })],
    constraints: vec![Constraint {
        id: ConstraintId(1),
        kind: Relation::Coincident {
            point1: point.into(), point2: PointReference::Origin,
        },
    }],
    ..Problem::default()
};
let compiled = sketch_solver::compile::compile(&problem)?;
let answer = sketch_solver::solve::solve(&problem)?;
let index = compiled.point_variable(point.into()).unwrap().0;
assert!(answer.values[index].abs() < 1e-8);
# Ok::<(), sketch_solver::input::InputError>(())
```

Use `compile::compile`, `solve::solve`, `freedom::analyze` and
`diagnosis::analyze` for validated semantic input. `InputError` is distinct from
`SolveOutcome::NotConverged`. IDs are supplied by the caller and never generated;
point, curve and constraint IDs are distinct types. The origin and axes are
explicit references. Geometry and relation order determine variable and row
order. Index newtypes expose offsets into numerical answer vectors.

Coordinates and lengths use one caller-chosen unit; `Radians` identifies angles.
`Direction` is dimensionless, and `ControlEdgeIndex` identifies a spline edge.
All numerical inputs must be finite, geometry and constraint IDs unique, and
references resolvable to the required geometry kind. Degenerate finite geometry
is allowed and may fail to converge. Directional angles are periodic. Arc sweep
targets must be greater than zero and at most one full turn; equal endpoints
represent a full counter-clockwise turn. Positive coordinate dimensions are
absolute distances; negative ones choose the signed branch. Initial ellipse
minor radii and pitch directions are supplied explicitly so callers can preserve
their own storage rounding and normalization. A pitch direction must already be
normalized, or be the zero vector.

`Settings` records damping, iteration, finite-difference, step, convergence,
minimum-length, rank and diagnosis policies. Default convergence compares the
residual infinity norm with `1e-9 * max(1, |values|_infinity)`. Mixed equation
scales and central differences limit the meaning of this numerical threshold;
callers should independently measure their stored geometry after rounding.
Answers include failed-attempt iteration counts, residual norms and thresholds.
The solver does not mutate a problem, refit splines or move text outlines.

`held_points`, `application_held_points` and `external` are separate pin sets.
External curves' referenced points must also be present in `external` when the
caller intends to hold them. Focus classification is repeated for every
constraint exclusion. Rank includes endpoint rules for otherwise unconstrained
arcs and free shape parameters absent from solve compilation. Diagnosis defaults
to 60 relations; larger problems return `analyzed = false` with their DoF.
`diagnosis::diagnose_at` accepts a caller's post-processing configuration for
rank analysis without callbacks. The independent path uses `f64` configuration.

The low-level `compile_system`, `iterate`, `estimate` and `diagnose_at` functions
also serve the application adapter. They retain its explicit compatibility with
unresolved stored constraints (zero rows) and require trusted finite geometry
and settings. Untrusted consumers should use the validated entry points above.
Numeric compiled equations expose raw offsets only at this low-level boundary.

With `serde`, `replay::Fixture` carries schema version 1 and a semantic problem.
`replay::run` returns a deterministic compiled trace, answer, residuals with
typed equation indices and provenance, DoF and diagnosis. The trace records
initial auxiliary contact parameters, tangent/gap branches and refraction ray
order. Every input constraint has a row report, including zero-row relations.
Unsupported schemas and invalid semantic inputs produce separate replay errors.
Serde JSON preserves `f64` round trips and the full `u128` ID domain. IDs within
`u64` serialize as integers; larger IDs serialize as decimal strings. The reader
accepts both without enabling arbitrary-precision JSON numbers in consumers.

The example runner accepts a single fixture, the saved regression corpus, or
standard input (`-`). An optional second argument records the caller's revision;
neither the numerical library nor its replay functions read the environment.

```sh
cargo run -p sketch_solver --features serde --example solve_problem -- \
  crates/sketch_solver/fixtures/migration.json
cargo test -p sketch_solver --features serde --test replay
cargo test -p sketch_solver --test generated -- --nocapture
cargo test -p sketch_solver --test boundaries
```

`fixtures/migration.json` contains 24 frozen semantic cases: 23 numerical runs
and one invalid stored-reference case rejected by the strict interface. The
application's compatibility path also checks its original stored-geometry
baseline without overwriting it. External and application-held pin lists are
sets; their ordering does not change equations. Geometry and relation ordering
remains significant. Regression traces and floating-point answers are fixed;
recapture is not a way to make a changed implementation pass.

The bounded generators run 96 fixed seeds across three planted-feasible
families (288 problems), with independent coordinate/distance/direction oracles.
Failure output includes seed, family and a structurally reduced problem; with
serde enabled it also prints a versioned fixture for replay. The reducer removes
irrelevant constraints and geometry while preserving validated numerical failure.
An answer mutation must fail the geometric oracle. Boundary tests include 90
repeated directional-angle transitions, the separate arc-sweep domain,
degenerate finite geometry, fully held inputs, ellipse equal axes on both sides,
both gap branches, multiple conflicts and the 60/61 diagnosis limit.

Run the default and independent feature checks with:

```sh
cargo test -p sketch_solver
cargo test -p sketch_solver --no-default-features
cargo test -p sketch_solver --features serde
cargo clippy -p sketch_solver --all-targets --all-features -- -D warnings
cargo check -p sketch_solver --all-features --target wasm32-unknown-unknown
cargo check -p sketch_solver --all-features --target wasm32-wasip2
node scripts/check-solver-deps.mjs
```

The repository dependency gate copies the crate outside the workspace and
resolves default, no-default, serde and all-feature graphs with `cargo metadata`
and `cargo tree`. It checks normal, build, development and target dependencies;
default builds permit only this crate, while serialization permits only serde's
JSON and derive dependencies. CI runs this gate alongside feature tests, strict
clippy and browser/WASI compilation. In a copied crate, run the same Cargo
commands with `--manifest-path /path/to/sketch_solver/Cargo.toml`.
