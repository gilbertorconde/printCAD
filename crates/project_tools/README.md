# project_tools

Converts a project archive (`.FCStd`) into printCAD sketches, Design features
and bodies, checks the converted sketches, and registers the conversion as File ›
Import's "FreeCAD file" through a workbench with no tools of its own
(`ProjectImportBench`, the `project.import` command), which the `workbenches`
crate registers with the built-in benches.

## `project-tool`

A developer program, built with the workspace and never published (the crate is
`publish = false`):

```bash
cargo run -p project_tools --bin project-tool -- inspect model.FCStd
cargo run -p project_tools --bin project-tool -- convert model.FCStd model.prtcad
cargo run -p project_tools --bin project-tool -- check ./projects
```

`inspect` prints the archive's entries and objects as JSON; `convert` writes the
converted document, rebuilt, to a `.prtcad`; `check` converts every `.FCStd` in
the files and folders given and reports, as JSON, what came in, what was lost and
whether each sketch holds still when solved.

## Conversion

The archive itself is read by [`project_archive`](../project_archive/README.md).

Library consumers import `project_archive::{archive, document_xml, sketch_decode}`
directly. Its public coordinates use `f64` values and arrays, independently of
the consumer's math-library version. Conversion, repair, solver and kernel
choices stay in the application adapter. `node scripts/check-archive-deps.mjs`
resolves and tests a copy outside the workspace with an explicit dependency
allowlist; it rejects application and native-kernel dependencies.

The converter handles points, lines, circles and circular arcs, common sketch
constraints, dimensioned Pad/Pocket and through-all Pocket, box/cylinder/cone/
sphere/torus primitives, and fuse/cut/common operations. Each solid object is a
native body, with a Clone feature linking its predecessor; active coincidences
become shared sketch vertices. Intermediate bodies stay available for checks.
Expressions and attachments keep their evaluated values and placements, with
findings explaining the lost dependency. Unsupported geometry, constraint
variants and object settings are listed too, and dependent objects are skipped.
Missing linked objects and dependency cycles are `dependency` findings for each
affected object and its consumers; independent recipes still import. Each
rejected object is reported once, with the original chain or skipped dependency.
The app runs the same conversion for File › Import (`ProjectImportBench`, the
`project.import` command), adding to the open document rather than writing a
file; the bodies then build as any others do.

Reader budgets are inclusive: 256 MiB for the ZIP container, 16 MiB for
`Document.xml`, 128 MiB per entry, 512 MiB for all declared uncompressed entries,
and 10,000 entries. Ignored entries still count towards the declared budgets.
XML and shape reads also enforce the actual decompressed byte budgets, even if
ZIP metadata understates them. Library callers can supply `ArchiveLimits` to
`open_with_limits` or `from_bytes_with_limits`; direct XML parsing enforces the
default XML byte budget, one million parser nodes, two million raw `<`/`=`
allocation hints (including comments and CDATA), and 128 element levels.
Dependency and parent-placement walks use heap storage with at most 128 pending
objects (including the starting object); cached dependencies consume no pending
depth. Cycles, missing targets and depth refusals name the object chain.
Unreadable archives/XML, invalid numeric data, ambiguous parent ownership,
invalid parent frames and depth limits reject the file without writing output.

`check` writes JSON containing per-object measurements, loss categories and
explicit file/sketch/solid/check counts. Read, rebuild and parity failures
produce a nonzero exit status; conversion losses are reported separately.
The JSON summary separates `error_categories` from `loss_categories`: archive
reading, XML parsing, conversion, kernel initialization, rebuild and stationarity.
`build_failures` values carry a category and the full planning/kernel reason.
Kernel initialization preserves its error source in the library's `ImportError`.
`convert` writes a partial document when losses only omit recipes; rebuild
failures prevent writing it. `check` returns zero when the imported recipes
rebuild and checked sketches stay still, even when the report lists losses.
The sketch check solves a copy and requires convergence and at most `1e-6` mm
of movement from the original coordinates. Source coordinates, radii, constraints
and sketch planes enter the native `f64` model without narrowing. The report
keeps import displacement separate from solver motion; Design lengths that
remain `f32` are checked before conversion. A passing check with omitted
constraints only validates the constraints actually imported.

The adapter uses `wb_sketch::operations::{join_points, solve}`: explicit point
aliases are checked before mutation, and solving validates active driving
relations through `sketch_solver` before application storage write-back.
Invalid references and relation domains return contextual errors; editing
repair and the workbench's solver internals remain private.

The tools build entirely in Rust, without system CAD libraries. `check` validates
recipe rebuilds and sketch stationarity; its summary explicitly reports
`shape_comparison: "not_performed"`. Comparing rebuilt solids to saved boundary
representations requires an external tool; `--shapes` returns an error.
The generated test archives are bundled under `crates/project_archive/tests/data`;
private project corpora remain outside the repository and run explicitly locally.
