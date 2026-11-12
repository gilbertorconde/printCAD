---
name: printcad-testing
description: "printCAD's testing conventions: one integration-test program per crate (tests/<name>/main.rs plus modules), the suite's time budget and --changed runs, CI scope on Linux, Windows and macOS, the test-lines ceiling and shared harnesses, where sketcher end-to-end and full-stack design tests live, STEP fixtures, and before/after pictures or measures in pull requests. Use when adding, moving or speeding up a test, choosing where a test goes, reasoning about what CI runs, or preparing a pull request's evidence. Not for judging whether a test earns its place (use test-audit) or a red CI run (use ci-iterate)."
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
  150 s in all, on CI, two programs running at a time); a test that needs
  a big model is `#[ignore]` with the reason, run on request. While
  working, test what a change reaches: `node scripts/test-budget.mjs
  --changed` (`scripts/affected-crates.mjs` against the merge base with
  origin/master: changed crates and everything depending on them,
  dev-dependencies included; the changed crates' programs run first).
- CI (`.github/workflows/ci.yml`): a push to master builds, lints and
  tests everything on Linux, Windows and macOS. A pull request lints
  everything on all three, but tests on Linux only what it reaches
  (`scripts/ci-scope.mjs`), and on Windows and macOS only when it touches
  code that differs by platform (the system-facing crates and files, any
  file with a `cfg(windows)`/`cfg(unix)`/`target_os` gate, the manifests)
  or carries the `platforms` label. Changes to CI, the test scripts or the
  manifests test everything.
- Tests stay a bounded share of the code: `scripts/test-lines.mjs` counts
  test lines per crate and fails CI above its ceiling. A new case goes
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

## STEP fixtures

- STEP tests use the bundled fixture
  `crates/kernel_ogeom/tests/data/box_native.step`; set
  `PRINTCAD_TEST_STEP_FILE` to test against a richer model. (`box.step`, from
  another exporter, writes its edges as SURFACE_CURVE wrappers, which
  `imports_a_step_file_with_surface_curves` reads.)
