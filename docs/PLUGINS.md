# Workbench packages

A workbench package adds a workbench to printCAD without changing or
rebuilding the app: tools, feature kinds with parametric solids, task
panels, viewport drawing, commands for scripts and agents, and a
Preferences page. A package is a WebAssembly component, so one file runs
on every system printCAD runs on, and it runs sandboxed: it reaches its own
folder and nothing else unless the user allows more.

A complete example sits under `sdk/examples/gear`: a spur gear
workbench. A larger
one is the CAM package,
[PrintCAD-cam-wb](https://github.com/gilbertorconde/PrintCAD-cam-wb), which
works out pocket toolpaths from a sketch's outline in a job, draws them
over the model and writes them as G-code. A repository to start
a package from, with CI and releases on tags already set up, is
[PrintCAD-example-wb](https://github.com/gilbertorconde/PrintCAD-example-wb).

## Installing one

Preferences › Workbench packages › Browse is the workbench store: the
packages every store you keep lists, to search, read about (what each asks
to reach beyond its own folder comes first) and install. A store is a
registry's index; a new install keeps one,
[printCAD's own](https://github.com/gilbertorconde/PrintCAD-wb-repo), and
the Stores tab beside it adds others by their index's address or
removes any, printCAD's included. A registry reviews each listing and
checks that the package is what it says it is; it does not audit the
code, so install what you trust, from stores you trust. A package
installed from a store comes from its repository's latest release, checked
against the list's checksum, and updates as any package from GitHub does;
one a store takes off its list is marked, and whoever has it is told why.
Anyone can run a store: the registry's README says how.

Installing does not need the store: the Installed page installs a `.pcbench` file (Install from
a file…) or a package published on GitHub: type the repository's address
(`https://github.com/owner/repo`, or `owner/repo`) to take its latest
release, or a release's address (`…/releases/tag/v1.2.0`) to take that
one. A package installed from GitHub remembers where it came from; Check
for updates looks for a newer release (and does so at every start unless
you turn that off), and Update to … installs it, keeping the package's
data and what you allowed it. An update that holds a different package,
or whose download does not match GitHub's checksum, is refused.

The page lists what is installed, whether each loaded, turns one
off, removes it, and says what each may reach beyond its own folder:

| Capability | What it allows |
| --- | --- |
| `save_dialog` | ask you where to save a file it made (allowed unless you turn it off) |
| `helper` | run native programs the package ships, outside the sandbox |
| `network` | open network connections |

An install, an update or a removal takes effect at once: the workbench
appears in the workbench list (or leaves it), and features it owns in open
documents rebuild with the version now running. Turning a package on or
off, and what it may reach, take effect when you apply Preferences.
Packages live in
`~/.local/share/printcad/workbenches/<id>/`, each with a `data/` folder that
is the only part of the disk it sees (as `/data`).

In a browser, printCAD runs packages too. The page keeps each installed
package itself (in the browser's storage for the site, found again at the
next visit) and installs from a `.pcbench` file or from a store whose list
carries a copy of each release the page may fetch (`mirror`, which
printCAD's registry serves; a GitHub release's own download refuses a
page). There a package has no `data/` folder, no network and no helpers,
and installs from a GitHub address and update checks are left to the
desktop app: a newer release installs from the store again.

A document made with a package keeps its features when opened without it:
a notice says which package it needs, the tree marks them "Needs <package>
<version>", their bodies keep the shape they were saved with, and they
cannot be deleted until the package is back. A feature records the package
and version that last wrote it and, for a package installed from GitHub,
its repository, so the feature's menu in the tree offers "Install
<package>". A package reads each feature with the version that wrote it
(`Node::made_by`), which is what lets it read data an older version saved.

## Writing one

A package is a Rust crate built as a `cdylib` for `wasm32-wasip2` against
the SDK, [`printcad-bench-sdk`](https://crates.io/crates/printcad-bench-sdk)
on crates.io (its source is `sdk/printcad-bench-sdk`). Other languages
that build WebAssembly components can implement
`crates/bench_api/wit/workbench.wit` directly; the values it carries are
the [`printcad-bench-api`](https://crates.io/crates/printcad-bench-api)
types as JSON.

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]

[dependencies]
printcad-bench-sdk = "0.1"
```

To build against a printCAD checkout instead, say while trying a change
to the SDK, patch the three crates to its folders:

```toml
[patch.crates-io]
printcad-bench-sdk = { path = "../printCAD/sdk/printcad-bench-sdk" }
printcad-bench-api = { path = "../printCAD/crates/bench_api" }
printcad-kernel-api = { path = "../printCAD/crates/kernel_api" }
```

```rust
use printcad_bench_sdk::{Bench, api::*, bench, host};

#[derive(Default)]
struct Hello;

impl Bench for Hello {
    fn describe(&self) -> Registration {
        Registration {
            label: "Hello".into(),
            tools: vec![Tool {
                id: "acme.hello.wave".into(),
                label: "Wave".into(),
                behavior: ToolBehavior::Action,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn input(&mut self, input: &Input) -> bool {
        if matches!(input.event, Event::ToolActivated) {
            host::info("Hello");
            return true;
        }
        false
    }
}

bench!(Hello);
```

Every method of `Bench` has a default; implement what the workbench offers.

### Versions

The SDK and the crates under it are versioned by the contract they speak:
0.1.x is `printcad:workbench@0.1`, which the manifest's `api` names. A
printCAD loads packages of its own contract and refuses others.

A patch release (0.1.1, 0.1.2) only adds, so a package written against an
earlier 0.1.x keeps building after `cargo update`, if it keeps to two
habits:

- **Match what the app sends with a wildcard arm.** `Event`,
  `PanelEvent`, `MenuScope`, `PanelSlot`, `Button`, and the kernel's
  `FaceSurface` and `ProfileSegment` are `#[non_exhaustive]`: a new kind
  of event or curve comes in a patch release, and a `_ =>` arm leaves it
  to the default. A package built with an older SDK passes over an event
  it does not know. Bind a received variant's fields with `..` too
  (`FaceSurface::Cone { axis, .. }`): a patch release may add one, as it
  added a cone's half angle and a torus's radii.
- **Fill the contract's structs with `..Default::default()`.** `Registration`,
  `Tool`, `Command`, `Param`, `Parameter`, `FeatureInfo`, `Rebuild`,
  `Task`, `Polyline`, `Label`, `Mesh`, `MenuItem` and the rest derive
  `Default`, so a field a patch release adds takes its default. (Clippy's
  `needless_update` lint flags a literal that names every field; the
  examples allow it.)

What a package builds as an enum variant with fields (`Widget::Number`,
`Request::SaveFile`, `SolidOp::Sweep`, `Plan::Ops`) keeps its fields
within a minor version: a new option comes as a new variant, or as a new
struct that derives `Default`. A field added anywhere reads as its default
when left out, so what an older package sends still reads.

A package that uses something a patch release added needs a printCAD of
at least that release. A new minor version (0.2) is a new contract: a
package moves to it by changing its dependency and its manifest's `api`,
and a printCAD of 0.2 does not load packages of 0.1.

The SDK is published from the printCAD repository: a tag `sdk-vX.Y.Z`
publishes `printcad-kernel-api`, `printcad-bench-api` and
`printcad-bench-sdk`, all at X.Y.Z.

### The manifest

`bench.toml` sits beside the component:

```toml
id = "acme.hello"            # lowercase, reverse-domain
name = "Hello"
version = "0.1.0"
api = "printcad:workbench@0.1"
description = "Says hello"
feature_kinds = ["acme.hello.note"]   # each starts with the id
memory_mb = 256              # default 1024, at most 4096

[capabilities]
save_dialog = true
```

Tool, action and command ids start with the package id too; others are
left out with a warning.

### Building and packing

```sh
cargo build --release --target wasm32-wasip2
mkdir -p pkg/icons
cp bench.toml pkg/
cp target/wasm32-wasip2/release/hello.wasm pkg/bench.wasm
cp icons/*.svg pkg/icons/
tar czf hello.pcbench -C pkg .
```

To publish, attach the `.pcbench` file to a GitHub release. The release's
tag is its version for updates (`v0.2.0` is newer than `v0.1.0`); the
first `.pcbench` asset of the latest release is what users get. To have it
in the store, add an entry to the
[workbench registry](https://github.com/gilbertorconde/PrintCAD-wb-repo)
by pull request; its README says how. Releases after that reach the store
by themselves.

Icons are 24×24 SVGs drawn in white (`#fff`) with a 1.5 px stroke; the app
tints them. A tool or feature names one by its file name (`gear` for
`icons/gear.svg`), or any icon of the app's own set.

## What a bench does

**Features.** A bench owns the feature kinds its manifest lists. It makes
and changes them only through `host` calls, each an ordinary edit: undone
with Ctrl+Z, recorded, sent to peers. `host::add_feature`,
`set_feature_data`, `remove_feature`, `create_body`, `remove_body` (a body
with only the package's own features on it, never an imported, linked or
copied one; a body removed with features still on it clears the undo
history, as deleting one in the app does, so a cancelled part removes its
feature first and then the empty body it made, which undoes), and
`host::call(api::calls::…)` for the rest (renaming a feature, showing or
hiding it, placing a body). Calls are allowed while the bench handles an
event, a panel change, a menu entry or a command, and refused while it
draws or plans. A feature's data is any JSON the bench likes.

Four calls only read, each answering where the body sits, and are
answered when the other calls are:

- `geom.holes {body}` lists the round holes in a body's solid (each
  `{entry, direction, diameter, depth, through, drill_point_deg}`).
- `geom.face_edges {body, point}` lists the edges round the face at a
  point (each `{point, direction}` halfway along it).
- `geom.section {body, plane: {origin, normal, x_axis?}, face?}`
  answers a `Section`: where the plane crosses the solid, or with `face`
  (a point) only the face there, as loops of curves end to end (closed
  round a solid's cut), each curve exact in the plane's 2D coordinates
  (along its `x_axis` and `y_axis`, which the answer's `plane` gives),
  with points along it and the name of the face it crosses.
- `geom.face_loops {body, point?, name?}` answers `FaceLoops`: the face's
  name and its boundary loops, the outer one first, each edge with its
  points in loop order, its length, a round one's `circle`, whether it
  is the seam a round face meets itself along, and the names of the two
  faces it runs between. A face is asked by its name (of several, the
  one nearest `point`) or at `point`.

The names are the kernel's: a face is named by the feature and the
sketch element that made it and keeps its name through rebuilds, so a
package keeps a face's or an edge's name rather than a point, and finds
it again with `geom.face_loops {name}`. A pick carries them too
(`FaceRef::name`, `EdgeRef::faces`). Imported solids name no faces.

**Bought parts.** `Registration::bought_kinds` names those of its kinds
whose bodies are bought rather than made, such as catalog screws and
bearings. The parts list marks such a body bought until the user clears
the mark, and an export of everything and the slicer leave it out.

**Parameters and formulas.** `parameters` lists a feature's numbers: a key,
a JSON pointer into its data, a dimension. Those numbers take formulas
anywhere the app shows them, and the data a bench reads (`host::feature`,
the nodes it is handed) already has the formulas' values in.

**Solids.** When a feature changes, the app asks `rebuild` for a plan per
body: kernel operations (`SolidOp`: extrusions, revolutions, lofts,
booleans, fillets and the rest), each naming the feature it is for. The
kernel runs them natively; a failing op marks its feature.

A plan may also declare cosmetic threads (`Rebuild::threads`, each a
`CosmeticThread`): a thread on a round face of the solid it builds,
never modelled. The face keeps its size (a bore its tap drill, a shank
its major diameter) and a helix of the thread's pitch is drawn just off
it, inside a bore and outside a shank, from the face's end nearest the
thread's `point` for its `length` (0 the whole face), in every bench and
never exported. The face is found by its `name` while the solid has a
face of that name, else at `point` (in the body's own frame, as the
ops are). The thread draws while its `feature` shows. A thread given a
`designation` ("M6"; optional) is named where the app names threads,
as a hole's cosmetic thread is: a callout at the face's end under
View › Annotations, the Assembly's parts list ("2 × M6 thread") and a
STEP export's callouts. Putting it in the feature's `kind_label` as
well ("Rod · M6 thread") is the package's choice.

**Panels.** A bench declares its task panel and Preferences page as
widgets (`Widget`): headings, a header with an icon, text, read-only
values, notes, numbers, sliders, choices, toggles, text fields, buttons (a small
one for an action on its row), pick rows, lists, tables, sheets (a table whose cells
may be links, text fields or check boxes, each change told by row and
column), groups, rows of
widgets side by side, a hint on any widget, progress, separators and
diagrams. A table's columns marked `editable` take typing, each cell
left coming back as `PanelEvent::Cell` (the CAM package's tool table is
one). A number bound
to a feature's parameter (`bind`) takes formulas like any field of the
app's own. Changes come back as `panel_event`; OK and Cancel as
`task_close`. A bench whose Preferences page declares no widgets gets no
page.

**Diagrams.** `Widget::Diagram` is a schematic of what the panel edits,
with its measures marked beside the parts they size. The bench draws it in
a space of its own, `[0, width] × [0, height]` with y up, in whatever unit
suits (a part's millimetres); the host fits that space to the panel's
width, at most 220 px tall, and keeps strokes, arrowheads and text at
their pixel size whatever the fit, so leave a margin for the labels.
`shapes` are paths, circles and text; a path or circle has a
`DiagramStroke` naming its role
(`outline`, `hidden` dashed, `axis` long-dashed, `thin`, `accent`) rather
than a colour, so they follow the theme; a filled path is shaded, which
reads right only for a convex outline, so shade a part as several convex
pieces. A `Dimension` measures `from` to `to`: its line stands `offset`
away, to the left of the way from `from` to `to` (negative for the right),
led out to the two points, with `text` on it. A `Callout` puts `text` at
`at` with a leader to a dot at `anchor`. Either drawn with `emphasis` takes
the accent colour: the bench sets it on the measure of the field last
changed, since the host does not know which field is being edited.
[PrintCAD-hardware-lib-wb](https://github.com/gilbertorconde/PrintCAD-hardware-lib-wb)
draws every part this way.

**Drawing.** `frame` answers everything the bench shows: the task, the
panel, the viewport's hint, badge and footer, the status bar's items,
lines, marks and labels in world space, and meshes. The app keeps it and
asks again after an event, a document or selection change, or
`host::redraw`, and moves the drawing with the view itself.

**Profiles.** `host::profile(feature)` answers a feature's closed loops
(a sketch's) where they stand in the world: the plane, then the loops,
holes inside their outlines, as lines, arcs, circles, ellipses and
splines. The CAM package reads a pocket's outline this way.

**Input.** `input` gets clicks, key presses, tool activations and actions,
with what is under the cursor and what is selected (`Pointer`: the ray,
the point and body hit, the picked face and every face picked (`faces`,
in the order picked), the picked edges (a circular one with its
`circle`: centre, axis and radius), the active feature; faces and edges
carry the kernel's names where the solid has them).
A double click on a feature's tree row arrives as `Event::EditFeature`,
the cue to open its task; selecting a feature alone never opens one.

**Menus.** `menu_items` adds entries to the viewport's body menu, the
tree's feature and body rows, the start page's New cards and the Edit
menu (`MenuScope`); a pick runs `menu_command` with the entry's id.

**Commands.** Commands in `describe` appear to Lua scripts
(`pc.acme.hello.…`), AI agents and recordings, and run in `run_command`.

**Jobs.** Long work runs as a job: `host::start_job(entry, input)` runs
`Bench::job` in an instance of its own away from the window. It reports
with `host::progress`, stops when `host::cancelled`, and a `Progress`
widget naming the job shows how far it is. Its result comes back as
`Event::JobFinished`. The CAM package works out every toolpath this
way and stops it when its task is cancelled. A job allowed `helper` may run a program from the
package's `helpers/<os>-<arch>/` folder with `host::helper`, for work
that needs every core.

## Limits

A call the window waits on has 25 ms: `frame`, `input`, and the feature
info, parameters and menu entries the tree and menus ask for. Any other
call has 1 s, and so do two events: a double click that opens a feature's
task (`Event::EditFeature`) and a finished job (`Event::JobFinished`). A job has no limit and stops when the user stops it. A call over its budget, a
panic or memory past `memory_mb` stops the bench's instance, which starts
afresh (its state lost); after three such failures in a session the bench
is turned off until the app starts again. In a browser no call has a
budget: a page cannot stop a call that runs on, so a package that never
returns holds the page.
