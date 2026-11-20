---
name: printcad-testing
description: "printCAD's testing conventions: one integration-test program per crate (tests/<name>/main.rs plus modules), the suite's time budget and --changed runs, CI scope on Linux, Windows and macOS, the test-lines report and shared harnesses, where sketcher end-to-end and full-stack design tests live, STEP fixtures, and before/after pictures or measures in pull requests. Use when adding, moving or speeding up a test, choosing where a test goes, reasoning about what CI runs, or preparing a pull request's evidence. Not for judging whether a test earns its place (use test-audit) or a red CI run (use ci-iterate)."
---

# printCAD: testing conventions

The before-committing checks and the comment rule are in `CLAUDE.md`. Pictures for a pull request use the hooks in `printcad-bench-hooks`.

## Testing conventions

- Each crate's integration tests are one program (`tests/<name>/main.rs`
  with a module per file: `kernel_ogeom/tests/kernel/`,
  `core_document/tests/document/`, `wb_assembly/tests/assembly/`,
  `wb_sketch/tests/sketcher/`): every program links the crate and its
  dependencies again, which costs far more than running tests. Add a test
  file as a module there, never as a new file beside `main.rs`'s
  directory. Dependencies build with line tables only
  (`[profile.dev.package."*"]`) for the same reason.
  Our own crates build with line tables too (`[profile.dev] debug =
  "line-tables-only"`, about a fifth off a test build); a debugger session
  sets `CARGO_PROFILE_DEV_DEBUG=full`.
- The suite has a time budget (`scripts/test-budget.mjs`: 60 s a program,
  150 s in all, on CI, two programs running at a time). While
  working, test what a change reaches: `node scripts/test-budget.mjs
  --changed` (`scripts/affected-crates.mjs` against the merge base with
  origin/master: changed crates and everything depending on them,
  dev-dependencies included; the changed crates' programs run first).
- Heavy tests: a test that needs a big model (the real part, the
  carriage, a package's fixture) or a long build goes into a `mod heavy`
  at the end of its module file, under `#[cfg(feature = "heavy-tests")]`
  with `use super::*;`, keeping its name and doc comment; the doc says
  when it needs a big model. The package declares `heavy-tests = []`
  (features are per package; `kernel_ogeom` does today). The default
  build neither compiles nor runs them, and `test-budget.mjs` fails if a
  default program holds one. Keep a cheap test of the same behaviour in
  the default suite where the heavy one would be its only cover (a
  smaller model, fewer cases, through a shared helper). Run them with
  `node scripts/test-budget.mjs --heavy` (with `--changed` or `-p` too)
  or `node scripts/gates.mjs --heavy`; they have their own budget (300 s
  a program, 600 s in all). The lint gates clippy them with the feature
  on, so a change that breaks their build fails on its pull request. CI's
  `heavy` job runs them on every push to master, on
  `workflow_dispatch`, and on a pull request carrying the `heavy-tests`
  label.
- CI (`.github/workflows/ci.yml`): a push to master builds, lints and
  tests everything on Linux, Windows and macOS. A pull request lints
  everything on all three, but tests on Linux only what it reaches
  (`scripts/ci-scope.mjs`), and on Windows and macOS only when it touches
  code that differs by platform (the system-facing crates and files, any
  file with a `cfg(windows)`/`cfg(unix)`/`target_os` gate, the manifests)
  or carries the `platforms` label. Changes to CI, the test scripts or the
  manifests test everything.
- What a test costs is its build and run time, which the budget bounds;
  `scripts/test-lines.mjs` reports tests' share of the lines per crate
  (in the lint gates, for information: it never fails). A new case goes
  into an existing table or shared harness (the sketcher `Harness`,
  `tests/kernel/common.rs`) rather than a near-copy of a sibling test; a
  test whose property a stronger test already asserts is retired. A
  behaviour has one owner at its strongest boundary: a tool proved
  through the viewport's clicks is not proved again by unit clicks on its
  state, and a slow geometric check shares its builds (a `OnceLock`)
  and meshes no finer than its tolerance needs.

- Sketcher end-to-end tests drive `on_input` with real viewport-pixel clicks:
  `wb_sketch/tests/sketcher/interaction.rs` (reuse its `Harness`).
- Full-stack sketch→feature→solid pipelines:
  `kernel_ogeom/tests/kernel/design_stack.rs` (dev-deps on wb_design/wb_sketch).
- Solver/geometry math is unit-tested next to the code. Assert geometric
  properties (bounds, tangency, closure), not implementation details.
- A change people can see (a panel, the view, a solid's shape) shows it in
  its pull request: a picture before and after, taken the same way from
  the same document and view (`PRINTCAD_BENCH_PICTURE=<path>` with the
  bench hooks that set the scene up, e.g. `PRINTCAD_BENCH_SKETCH=pad`;
  `PRINTCAD_BENCH_WINDOW=<path>` for a panel or menu), or
  for a solid its measures before and after (`project-tool check`, the
  kernel's volume). Pictures go in the pull request, never the repository.

## Project corpus (local only)

- `project-tool corpus <folder>` (or `PRINTCAD_PROJECT_CORPUS`) imports
  every project archive in a folder of your own the way File › Import
  does, rebuilds each body and holds it to the archive's saved solid
  (volume, area, bounding box, centre of mass); it names the first
  feature whose step strays, lists unsupported features and lost
  parametric data apart from geometry, and writes a table, a summary line
  and a JSON report. One archive a process, with a time limit.
  `--baseline <earlier.json>` fails on regressions only. It never runs in
  CI and no one's archives are committed; its test entry point is an
  `#[ignore]`d heavy test in `project_tools/tests/projects/corpus.rs`.
  The tolerances, the verdicts and how to cut a failing case down to an
  ordinary test are in `crates/project_tools/README.md`.

## STEP fixtures

- STEP tests use the bundled fixture
  `crates/kernel_ogeom/tests/data/box_native.step`; set
  `PRINTCAD_TEST_STEP_FILE` to test against a richer model. (`box.step`, from
  another exporter, writes its edges as SURFACE_CURVE wrappers, which
  `imports_a_step_file_with_surface_curves` reads.)
