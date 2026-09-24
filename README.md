# printCAD

Parametric CAD for designing 3D-printed parts. Linux, Rust, Vulkan.

![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)
![Rust](https://img.shields.io/badge/rust-1.98%2B-orange)
![Platform](https://img.shields.io/badge/platform-Linux-lightgrey)

> **Early development.** The full path from sketch to printable file works,
> but expect rough edges.

## Features

- **Sketcher:** lines, polylines, arcs, circles, ellipses, splines, slots and
  more, with geometric and dimensional constraints solved live. Edges of a
  solid can be projected in as fixed reference geometry.
- **Part Design:** pad, pocket, revolve, loft, pipe, helix, holes, fillets,
  chamfers, patterns and booleans, all editable in a feature tree.
- **Assembly:** place bodies against each other with joints: mate two flat
  faces, line up two round ones on one axis, or hold two faces at an angle.
- **Import:** STEP and IGES as solids; STL, OBJ and 3MF as meshes that can
  be converted to solids.
- **Export:** STEP, STL and 3MF, or straight to your slicer.
- **Documents:** `.prtcad` files, one tab each, with undo and redo.
- **View:** GPU picking of faces and edges, a clipping plane, and 6-DoF mouse
  support.

The geometry kernel, [ogeom](https://github.com/gilbertorconde/ogeom-rs), is
pure Rust. No system CAD libraries are needed.

## Build and run

You need Rust 1.98 or later, Vulkan drivers, and Wayland or X11.

```bash
git clone https://github.com/gilbertorconde/printCAD.git
cd printCAD
cargo build --release
./target/release/app_shell
```

Build the whole workspace, not only `app_shell`. The app starts a document
server binary from its own folder, and without it the app falls back to
plain file access.

A debug build (`cargo run -p app_shell`) is fine for development.

To use a 6-DoF mouse, install and start
[spacenavd](https://spacenav.sourceforge.net/). printCAD finds the device on
its own.

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
| Box select in a sketch | Left drag |
| Fit the model | **F** |
| Standard views | Click the orientation cube, or **0** to **6** |

### Keyboard

Every shortcut can be changed in Preferences › Keyboard. The defaults:

| Action | Keys |
| --- | --- |
| Command palette | Ctrl+K |
| New, Open, Save, Save As | Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S |
| Import, Export | Ctrl+I, Ctrl+E |
| Send to slicer | Ctrl+P |
| Undo, Redo | Ctrl+Z, Ctrl+Shift+Z or Ctrl+Y |
| New tab, Close tab | Ctrl+T, Ctrl+W |
| Next, Previous tab | Ctrl+Tab, Ctrl+Shift+Tab |
| Cut, Copy, Paste in a sketch | Ctrl+X, Ctrl+C, Ctrl+V |
| Preferences | Ctrl+, |
| Recompute all | Ctrl+R |
| Delete the selected tree row | Delete |
| Show or hide the selected tree row | Space |

View keys:

| Action | Keys |
| --- | --- |
| Fit all, fit the selection | F, Shift+F |
| Isometric, front, top, right | 0, 1, 2, 3 |
| Rear, bottom, left | 4, 5, 6 |
| Orthographic, perspective | O, P |

A workbench's keys apply while it is active and win over the view keys.
A plain letter picks a tool; Shift and a letter picks its partner.

| Sketcher | Keys |
| --- | --- |
| Point, line, polyline, arc, circle | O, L, P, A, C |
| Ellipse, B-spline, rectangle, polygon, slot | E, B, R, G, S |
| Trim, external geometry, construction | T, X, N |
| Switch a polyline between lines and arcs | M |
| Coincident, point on object | Shift+C, Shift+O |
| Horizontal, vertical | Shift+H, Shift+V |
| Parallel, perpendicular, tangent | Shift+P, Shift+N, Shift+T |
| Equal, symmetric, block, lock | Shift+E, Shift+S, Shift+B, Shift+K |
| Dimension, radius, angle | Shift+D, Shift+R, Shift+A |
| Horizontal, vertical distance | Shift+L, Shift+I |

While a length is being typed, number keys go to the length.

| Part Design | Keys |
| --- | --- |
| Body, sketch | B, S |
| Pad, pocket | E, Shift+E |
| Revolution, groove | R, Shift+R |
| Loft, subtractive loft | L, Shift+L |
| Pipe, subtractive pipe | W, Shift+W |
| Hole | Shift+H |
| Fillet, chamfer, mirrored | U, C, M |

| Assembly | Keys |
| --- | --- |
| Mate, align, angle | M, A, N |
| Move a body, solve | G, S |

## Settings

Settings are stored in `~/.config/printcad/settings.json`. Change them in
Preferences (Ctrl+,).

## Reporting an import problem

1. Turn on **Preferences › General › Diagnostics › Write a report for every
   STEP import**.
2. Import the file again. The log shows where the report was written, under
   `/tmp/printcad/import-reports/`.
3. Open an issue on the
   [kernel tracker](https://github.com/gilbertorconde/ogeom-rs/issues) with
   the report, and the STEP file if you can share it.

## Project layout

| Crate | Purpose |
| --- | --- |
| `app_shell` | The application: window, frame loop, UI and input |
| `core_document` | Documents, feature tree, undo, file format |
| `doc_server` | The document server and its client |
| `kernel_api` | The geometry interface: meshes, profiles, solid operations |
| `kernel_ogeom` | That interface implemented with ogeom |
| `render_vk` | Vulkan renderer |
| `settings` | User settings |
| `ui_kit` | Colours, widgets, icons and fonts |
| `axes` | Axis presets, so no code assumes which way is up |
| `workbenches/wb_sketch` | Sketcher |
| `workbenches/wb_part` | Part Design |
| `workbenches/wb_assembly` | Assembly: joints between bodies |

More detail in [docs](docs/):

- [Architecture and roadmap](docs/plan.md)
- [Editing workflow](docs/WB_IMP.md)
- [Document model](docs/DOCUMENT_MODEL.md)
- [Writing a workbench](docs/WORKBENCH_GUIDE.md)
- [Camera](camera_system.md)
- [Project status](PROJECT_STEPS.md)

## Roadmap

- More joint kinds: gears, limits on a slide or a turn

## License

MIT or Apache 2.0, at your option. See [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).
