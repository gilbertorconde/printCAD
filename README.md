# printCAD

Parametric CAD for designing 3D-printed parts. Linux, Windows and macOS; Rust and wgpu.

![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)
![Rust](https://img.shields.io/badge/rust-1.98%2B-orange)
![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-lightgrey)

> **Early development.** The full path from sketch to printable file works,
> but expect rough edges.

## Features

- **Sketcher:** lines, arcs, circles, ellipses, conics, weighted splines,
  slots, text and more, with geometric and dimensional constraints solved
  live. Solid edges, other sketches and datums come in as references, and
  generators draw gears, sprockets and shafts from their numbers.
- **Design:** pad, pocket, revolve, loft, pipe, helix, primitives, holes to
  thread standards, fillets, chamfers, draft, thickness, patterns and
  booleans, all editable in a feature tree, with datums and geometry
  borrowed from other bodies. Imported solids take features too, and their
  faces can be deleted, offset or moved, their holes recognized.
- **Surface:** extruded, revolved, planar, filled, ruled, lofted and swept
  surfaces from open or closed curves; offset, extended, blended, split,
  filleted, trimmed and mirrored; sewn into shells and solids or thickened,
  with a continuity check.
- **Assembly:** joints of every common kind (mate, align, hinge, slider,
  ball, cam, gears and more), components that move as one, linked copies
  and arrays of them, parts linked from other files, motion over time, interference and
  clearance checks, exploded views and a parts list.
- **Import:** STEP and IGES as solids, with the dimensions, tolerances,
  datums, notes and layers they carry; STL, OBJ, 3MF, PLY, glTF and VRML
  as meshes that convert to solids, their round faces found on request.
- **Export:** STEP, STL and 3MF, or straight to your slicer, the bodies
  where they sit or laid out for printing: each part flat on the bed, as
  many as the parts list prints. Nut traps for captive nuts, and the
  filament each part takes; see [docs/PRINTING.md](docs/PRINTING.md).
- **Documents:** `.prtcad` files, one tab each, with undo and redo.
- **View:** GPU picking of faces and edges, a clipping plane, a measure
  tool, body and face colours, and 6-DoF mouse support.
- **Scripting:** a Lua console that reaches every command of the
  application and the workbenches.
- **Variables and formulas:** any number can follow named variables and
  other dimensions, with units checked, and configurations switch a model
  between sizes.
- **AI agents:** chat with ACP agents that work the document through the
  same commands, and an MCP server for any other client.
- **Workbench packages:** workbenches built for WebAssembly, installed from
  a file, a GitHub release or a store, and run sandboxed.

The geometry kernel, [ogeom](https://github.com/gilbertorconde/ogeom-rs), is
pure Rust. No system CAD libraries are needed.

## Build and run

printCAD runs on Linux (Wayland or X11), Windows 10 or later, and macOS 11
or later. Ready-made downloads are on the
[releases page](https://github.com/gilbertorconde/printCAD/releases). On
Linux, unpack the archive and run `./install.sh`: it puts printCAD in your
applications menu and `printcad` on your path (`./install.sh --remove`
takes it away). The macOS and Windows builds are not signed yet, so the
first launch needs right-click › Open on macOS, and "Run anyway" on
Windows.

To build it you need Rust 1.98 or later. It draws through wgpu: Vulkan on
Linux, Metal on macOS, and DirectX 12 or Vulkan on Windows, so any GPU with
current drivers for one of those runs it. Nothing else needs installing:
the shaders are WGSL, compiled by wgpu as the app starts. `WGPU_BACKEND`
(`vulkan`, `metal`, `dx12` or `gl`) picks one for a test.

```bash
git clone https://github.com/gilbertorconde/printCAD.git
cd printCAD
cargo build --release
./target/release/printcad
```

Build the whole workspace, not only `app_shell`. The app starts a document
server binary from its own folder, and without it the app falls back to
plain file access.

A debug build (`cargo run -p app_shell`) is fine for development.

To use a 6-DoF mouse on Linux, install and start
[spacenavd](https://spacenav.sourceforge.net/). On Windows and macOS printCAD
reads the device directly. Either way it finds the device on its own.

## Controls

### Mouse

| Action | Control |
| --- | --- |
| Orbit | Middle drag |
| Set the orbit pivot | Middle click on the model, or **H** |
| Pan | Right drag |
| Zoom | Wheel |
| Select a face or edge | Left click (Ctrl adds) |
| Select a whole body | Left double click |
| Body menu | Right click on a body |
| Box select | Left drag from empty space: rightwards takes what lies wholly inside, leftwards (dashed) what the box touches; bodies, faces or edges by the pick filter, Ctrl adds |
| Fit the model | **F** |
| Standard views | Click the orientation cube (double click also fits), or **0** to **6** |
| Orbit from the cube | Left drag on the orientation cube; its house returns home |

### Keyboard

Every shortcut can be changed in Preferences › Keyboard. The defaults:

| Action | Keys |
| --- | --- |
| Command palette | Ctrl+K |
| New, Open, Save, Save As | Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S |
| Import, Export | Ctrl+I, Ctrl+E |
| Send to slicer | Ctrl+P |
| Quit | Ctrl+Q |
| Undo, Redo | Ctrl+Z, Ctrl+Shift+Z or Ctrl+Y |
| New tab, Close tab | Ctrl+T, Ctrl+W |
| Reopen the last tab closed | Ctrl+Shift+T |
| Next, Previous tab | Ctrl+Tab, Ctrl+Shift+Tab |
| Cut, Copy, Paste sketch geometry or features | Ctrl+X, Ctrl+C, Ctrl+V |
| Preferences | Ctrl+, |
| Recompute all | Ctrl+R |
| Repeat the last tool | Shift+Space |
| Delete the selected tree row | Delete |
| Show or hide the selected tree row | Space |
| Rename the selected tree row | F2 |
| Properties of the selected tree row | Alt+Enter |

View keys:

| Action | Keys |
| --- | --- |
| Fit all, fit the selection | F, Shift+F |
| Isometric, front, top, right | 0, 1, 2, 3 |
| Rear, bottom, left | 4, 5, 6 |
| Orthographic, perspective | O, P |
| Full screen | F11 |

Right click the status bar to choose which of its readouts show.

### Row and body menus

Right click a row of the tree, or a body in the view, for what can be
done to it. The menus are flat lists; anything that takes numbers or
choices opens as a task in the right panel, where edits show in the view
as they are made, OK keeps them as one undo step and Cancel puts back
what was there.

- A feature: Edit, Rename, its body's Appearance and Placement, Suppress,
  Hide, Set as tip, Move up, Move down, Move after (the task lists the
  body's features to click; a feature row also drags to its place: above
  a row's middle it goes before it, below it after), Freeze body, Cut, Copy, Paste, Delete, Copy
  formulas and Paste formulas (onto a feature of the same kind),
  Recompute, Send to console, Properties. Ctrl+click adds feature rows to
  the selection; Suppress on one of them suppresses (or unsuppresses)
  them all as one undo step.
- A body: Select, Rename, Hide, Show only this, Show all, Appearance,
  Placement, Freeze (its features are not rebuilt until it thaws; the row
  shows FROZEN), Make unselectable (clicks in the view pass through it),
  Linked copy, Array of copies… (a row, a ring, a path or points of
  linked copies; Edit array and Skip this copy on a copy), Select the
  original of a copy,
  Replace shape…, Cut, Copy,
  Paste, Delete, Recompute, Send to console, Properties.
- An imported body also offers Repair shape when the checker finds it
  broken, a mesh body Convert to solid, and a converted one Refine shape.
- In the view, Repeat for the last tool started, Look at (turns the view
  square to the face under the pointer), the body's entries, and Face
  colour for that face.

The pick button at the view toolbar's right end says what a click in the
view takes: faces and edges (the default), faces only, edges only, or
whole bodies. A double click on an edge takes every edge that runs on
from it smoothly, the whole round outline for a fillet or chamfer.
Pointing at a row of the tree lights up its body in the view.

A file dropped on the window opens (a `.prtcad`) or imports (a model
file), where the windowing system delivers drops. File › Save view as
picture writes the view as drawn to a PNG. The command palette lists what
you ran lately first.

A body's **Surface texture…** (tree and view menus) presses a pattern into
chosen faces for printing: knurling, ribs, hexagons, noise or your own
greyscale picture, laid flat, wrapped round an axis or blended on any
shape. It shows in the view and goes into STL and 3MF exports and the
slicer; see [docs/TEXTURES.md](docs/TEXTURES.md).

Every few minutes (Preferences › General › Autosave) an edited document
is copied aside; after a crash the start page offers the copy back, as
an untitled document to save where it belongs.

The Appearance task holds the body's colour (a palette, your own colours,
which you keep or forget from the custom colour picker and which every
body offers, and the colour it came with), how much shows through, the
colours of single faces (click a face in the view; a face's colour
follows it through a rebuild) and its material, from a list or typed in
with its density, which gives its mass in the Physical group. The
Placement task moves and turns the body by numbers, negative ones too;
the bodies of its rigid component move with it.

`doc.set_body`, `doc.set_face_color`, `doc.linked_copy`, `doc.move_after`
and `doc.recompute` do the same from a script.

A workbench's keys apply while it is active and win over the view keys.
A plain letter picks a tool; Shift and a letter picks its partner.

| Sketcher | Keys |
| --- | --- |
| Point, line, polyline, arc, circle | O, L, P, A, C |
| Ellipse, B-spline, rectangle, polygon, slot | E, B, R, G, S |
| Trim, external geometry, construction | T, X, N |
| Show or hide internal geometry | I |
| Switch a polyline between lines and arcs | M |
| Coincident, point on object | Shift+C, Shift+O |
| Horizontal, vertical | Shift+H, Shift+V |
| Parallel, perpendicular, tangent | Shift+P, Shift+N, Shift+T |
| Equal, symmetric, block, lock | Shift+E, Shift+S, Shift+B, Shift+K |
| Dimension, radius, angle | Shift+D, Shift+R, Shift+A |
| Horizontal, vertical distance | Shift+L, Shift+I |

While a length is being typed, number keys go to the length.

Drawing lines, press on the end the line tool draws from and drag: the
segment is an arc tangent to the line before it, and the tool goes on
with lines from its end. While a sketch is edited, what its profile
closes is shaded (what a pad takes, holes open; Preferences › Sketcher
turns it off), and a curve's loose end, which the profile leaves out,
has a red ring once no shape is being drawn.

File › Import takes a PNG or JPEG as a reference picture: it lies on the
plane of the sketch being edited (else in a new sketch), shown while the
sketch is edited, to draw over. The panel's Reference images section
moves, sizes, turns and fades it.

| Design | Keys |
| --- | --- |
| Body, sketch | B, S |
| Pad, pocket | E, Shift+E |
| Revolution, groove | R, Shift+R |
| Loft, subtractive loft | L, Shift+L |
| Pipe, subtractive pipe | W, Shift+W |
| Hole | Shift+H |
| Fillet, chamfer, mirrored | U, C, M |

Pad and Pocket take a sketch selected in the tree, or with none, a flat
face of the solid picked in the view: that face pushed out or pressed in,
no sketch needed. The task shows the face and takes another one picked.

| Assembly | Keys |
| --- | --- |
| Mate, align, angle, hinge, slider | M, A, N, H, L |
| Fixed, parallel, perpendicular, distance, tangent | X, R, Shift+R, D, T |
| Ball, universal, pin in a slot, path, cam, width | Shift+B, Shift+U, Shift+S, Shift+P, Shift+C, Shift+W |
| Couple joints, rigid group, ground | K, U, F |
| Move a body, solve | G, S |
| Interference, collisions on drag, mass | I, C, W |
| Exploded view, parts list | E, B |
| Linked copies, replace a body | Y, Shift+Y |

## Scripting

Scripts are Lua. Every command of the application and the workbenches is
a function under `pc`, called with named arguments:

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 20}
local pad = pc.design.pad{sketch = s, length = 12}
pc.design.set{feature = pad, length = 20}
```

Run them in the console (Windows › Console), from the Scripts menu and
toolbar button (every `.lua` file in the configuration folder's `scripts`
folder, `~/.config/printcad/scripts` on Linux), or
without a window: `printcad --script build.lua`. See
[Scripting](docs/SCRIPTING.md) for the guide and every command.

## Settings

Settings are stored in `settings.json` in the configuration folder
(`~/.config/printcad/` on Linux). Change them in
Preferences (Ctrl+,).

## Reporting an import problem

1. Turn on **Preferences › General › Diagnostics › Write a report for every
   STEP or IGES import**.
2. Import the file again. The log shows where the report was written, under
   `printcad/import-reports/` in the system's temp folder (`/tmp` on Linux).
3. Open an issue on the
   [kernel tracker](https://github.com/gilbertorconde/ogeom-rs/issues) with
   the report, and the file if you can share it.

## Project layout

| Crate | Purpose |
| --- | --- |
| `app_shell` | The application: window, frame loop, UI and input |
| `core_document` | Documents, feature tree, undo, file format |
| `doc_server` | The document server and its client |
| `kernel_api` | The geometry interface: meshes, profiles, solid operations (on crates.io as `printcad-kernel-api`) |
| `kernel_ogeom` | That interface implemented with ogeom |
| `render_wgpu` | The renderer (wgpu: Vulkan, Metal, DirectX 12) |
| `settings` | User settings |
| `ui_kit` | Colours, widgets, icons and fonts |
| `surface_texture` | Patterns pressed into faces for printing |
| `axes` | Axis presets, so no code assumes which way is up |
| `workbenches/wb_sketch` | Sketcher |
| `workbenches/wb_design` | Design |
| `workbenches/wb_surface` | Surface: sheets from curves, sewn into shells and solids |
| `workbenches/wb_assembly` | Assembly: joints between bodies |
| `workbenches/wb_wasm` | Workbench packages, run sandboxed |
| `workbenches` | Registers the built-in workbenches and installed packages |
| `workbenches/fixtures` | Ready-made scenes for tests and demos |
| `bench_api` | What a workbench package and the app exchange (on crates.io as `printcad-bench-api`) |
| `local_ipc` | Local sockets and helpers the app's processes talk through |
| `scripting` | The Lua engine scripts and the console run in |
| `agents` | Agent Client Protocol client and MCP server core |

More detail in [docs](docs/):

- [Architecture](docs/ARCHITECTURE.md)
- [Editing workflow](docs/EDITING.md)
- [Document model](docs/DOCUMENT_MODEL.md)
- [Writing a workbench](docs/WORKBENCH_GUIDE.md)
- [Scripting](docs/SCRIPTING.md)
- [Variables and formulas](docs/VARIABLES.md)
- [Assembly](docs/ASSEMBLY.md)
- [Surfaces](docs/SURFACES.md)
- [Holes](docs/HOLES.md)
- [Surface textures](docs/TEXTURES.md)
- [Printing](docs/PRINTING.md)
- [Workbench packages](docs/PLUGINS.md)
- [AI agents](docs/AI.md)
- [Roadmap](docs/ROADMAP.md)
- [Camera](docs/CAMERA.md)

## Roadmap

Everything still open (what the workbenches lack, what waits on the
kernel, faster kernel operations) is in
[docs/ROADMAP.md](docs/ROADMAP.md).

## License

MIT or Apache 2.0, at your option. See [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).

## Inspiration

printCAD stands on ideas from these projects:

- [FreeCAD](https://www.freecad.org): the parametric workflow, workbenches,
  the model tree and the task panel.
- [Open CASCADE Technology (OCCT)](https://dev.opencascade.org): the B-rep
  kernel printCAD first ran on, and the model for much of how it treats
  shapes.
- [stlTexturizer](https://github.com/CNCKitchen/stlTexturizer) by CNC
  Kitchen: the idea behind surface textures. Its code and textures are not
  used here.
