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

The converter handles points, lines, circles, circular arcs and B-splines, common
sketch constraints, external geometry (held fixed where it was last projected,
in the profile when it defines it), Pad/Pocket with every end but up to a shape
(one side, two sides or symmetric), Hole, Fillet, Chamfer, Loft, Mirrored,
LinearPattern, PolarPattern and MultiTransform (of the whole body or of
features chosen in it), body booleans, box/cylinder/cone/sphere/torus
primitives, fuse/cut/common operations, texts and their extrusions, and links.
The edges and faces a feature names (`Edge3`, `Face1`) are found on the saved
shape of the object it names them on (`project_archive::saved_shape`) and
become picks.

A source body is one native body: its features in history order, its tip the
source's, its name the source's. Objects outside bodies chain the same way: an
operation continues the body of its base when nothing else takes that base, and
otherwise starts a body with a Clone of it; a body started from another solid
begins with a Clone of that solid's body. A feature another object takes from
the middle of its body's history is taken as its saved shape. Active
coincidences become shared sketch vertices, except with points of projected
guides and points drawn on their own, which keep their role (a hole's centre).
A text becomes a sketch text block, laid out at the file's size, spacing and
justification, in the file's font where this machine has its file (kept in the
document), else an installed font of its family, else a bundled one, a `font`
finding saying which stands in; its extrusion is a Pad in a body of its own. A
link becomes a linked copy, placed where the link puts it.

An object the converter cannot rebuild (a kind or a setting with no
counterpart, an input that did not come in) comes in as the shape the file
saved for it: a body with that shape and no history
(`Document::shape_body_from_file`), named in a `saved shape` finding ("kept as
its saved shape: …"); what is built on it starts from it. Every body built from
features keeps its tip's saved shape as its stand-in
(`Document::set_body_stand_in`), shown while its history does not build. The
saved `.brp` text reaches the kernel through `project_archive::restate`, which
writes it in the plain form the kernel's reader takes; the kernel then fits the
flat faces' pcurves and holds the tolerances to what the text measures
(`OgeomKernel::read_brep`). Expressions and attachments keep their evaluated
values and placements, with findings explaining the lost dependency.
Unsupported geometry and constraint variants are listed too. Missing linked
objects and dependency cycles are `dependency` findings for the objects on
them; what depends on them goes on from their saved shapes. Each rejected
object is reported once, with the original chain or skipped dependency.
The app runs the same conversion for File › Import (`ProjectImportBench`, the
`project.import` command), adding to the open document rather than writing a
file; the bodies then build as any others do, and the saved shapes and
stand-ins are read on the kernel thread as a replaced shape is.

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

`check` writes JSON containing per-body measurements, loss categories and
explicit file/sketch/body/check counts. Read, rebuild and parity failures
produce a nonzero exit status; conversion losses are reported separately.
The JSON summary separates `error_categories` from `loss_categories`: archive
reading, XML parsing, conversion, kernel initialization, rebuild and stationarity.
`build_failures` values carry a category and the full planning/kernel reason.
Kernel initialization preserves its error source in the library's `ImportError`.
`convert` writes a partial document when losses only omit recipes; a body that
neither builds nor has a stand-in prevents writing it. `check` returns zero when the imported recipes
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
recipe rebuilds and sketch stationarity, and holds each body to the shape the
file saved for its tip (a linked copy's moved as the copy is): the box it
reaches (within 0.05 mm, `bounds_match`, `deviation_mm`), its face count and
its volume beside the saved one's, reported per body under `bodies` with its
kind (`features`, `saved_shape`, `linked_copy`), its features and whether it
shows its stand-in (`stood_in`), and summed as `bodies`,
`built_from_features`, `saved_shapes`, `linked_copies`, `features`,
`stood_in`, `bounds_matched` and `bounds_differing`. A face count differs where
the two kernels' merging of coplanar faces differs, so it is reported, not
failed.
The generated test archives are bundled under `crates/project_archive/tests/data`;
private project corpora remain outside the repository and run explicitly locally.
