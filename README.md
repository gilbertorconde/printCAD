# printCAD

A parametric CAD application focused on designing parts for FDM/SLA 3D printing, built entirely in Rust with a Vulkan renderer.

![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)
![Rust](https://img.shields.io/badge/rust-1.98%2B-orange)
![Platform](https://img.shields.io/badge/platform-Linux-lightgrey)

> ⚠️ **Early development** — the modelling core works end to end (sketch →
> constraint solve → feature → solid → save), but this is not yet a tool to
> rely on for real work. Expect rough edges, and no STEP export yet.

## Overview

printCAD is a Linux-native, Wayland-first CAD application for parametric
models aimed at 3D printing. The geometry kernel, the renderer, the document
model and the UI are all Rust; there are no system CAD libraries to install.

### What works today

- **Sketcher** — lines, arcs, circles, rectangles and splines with geometric
  and dimensional constraints, solved by a Levenberg–Marquardt solver that
  reports degrees of freedom and diagnoses conflicts. The origin and its axes
  are constrainable, so a sketch can be driven fully constrained.
- **Part Design** — Pad, Pocket, Revolution, Groove, Loft, Pipe, Helix,
  primitives, Hole, Fillet, Chamfer, Draft, Thickness, patterns and booleans,
  each an editable feature in a dependency-tracked tree that rebuilds on
  change.
- **STEP import** — assemblies arrive as placed bodies with their hierarchy,
  per-face colours and the original file kept inside the document. Export is
  still to come.
- **Native documents** — `.prtcad` (a tar container, optionally gzip or zstd
  compressed) holding the feature tree, the B-rep snapshots and the source
  files an import came from. Saving and opening run off the UI thread.
- **Undo/redo** — per-edit inverse operations rather than snapshots, so a
  gesture is one step and the history survives a large document.
- **Vulkan rendering** — hardware-accelerated viewport, GPU picking with
  async readback, and a scene cached between changes so UI-only frames are
  cheap on any model.
- **6-DoF mouse** — SpaceMouse and friends drive the view, through the
  [sixdof](https://github.com/gilbertorconde/sixdof) client.

## Building

### Prerequisites

- Rust 1.98 or later (edition 2024)
- Vulkan drivers
- Linux with Wayland (X11 also works)

The geometry kernel ([ogeom](https://github.com/gilbertorconde/ogeom-rs)) is
pure Rust and builds with the workspace — no system CAD libraries needed.

### Build & run

```bash
git clone https://github.com/gilbertorconde/printCAD.git
cd printCAD

# Build everything, then run. The whole workspace matters: the app spawns a
# document server from its own directory, and `-p app_shell` alone would
# leave that binary unbuilt and quietly fall back to direct file I/O.
cargo build --release
./target/release/app_shell
```

For day-to-day work a debug build is fine — dependencies are compiled
optimized even in dev (the kernel is numeric code and runs about 26× slower
unoptimized), while the project's own crates stay unoptimized for fast
rebuilds:

```bash
cargo build && cargo run -p app_shell
```

### 6-DoF mouse

Install and start [spacenavd](https://spacenav.sourceforge.net/); printCAD
picks the device up on its own and names it in the status bar. The vendor's
own driver works too — the client falls back to the Magellan protocol over
the display server. Nothing is required to build or run without one.

## Controls

### Viewport

| Action | Control |
| ------ | ------- |
| **Orbit** | Middle drag |
| **Pivot on geometry** | Middle click — the orbit pivot snaps to the point under the cursor |
| **Pan** | Right drag |
| **Zoom** | Wheel (optionally toward the cursor) |
| **Select a face** | Left click |
| **Select a whole body** | Left double click on one of its faces (one part of an assembly, not the assembly); in Part Design the tree jumps to its row |
| **Body menu** | Right click on a body: show it in the tree, select it, hide it |
| **Box select** | Left drag, in a sketch |
| **Add to the selection** | Ctrl (a sketch selects cumulatively without it) |
| **Pivot on the focal plane** | **`H`** with the cursor over the viewport |
| **Fit the model** | **`F`** |
| Snap to a standard view | Orientation cube face, edge or corner |
| Nudge ±45° | Orientation cube arrows |

While a sketch is open the view is locked to its plane: pan, zoom and roll
about the plane's normal stay, and the rotations that would tilt out of it
are dropped.

### Everywhere

| Action | Control |
| ------ | ------- |
| Command palette | **Ctrl+K** |
| New / Open / Save / Save As | **Ctrl+N** / **Ctrl+O** / **Ctrl+S** / **Ctrl+Shift+S** |
| Import STEP | **Ctrl+I** |
| Undo / redo | **Ctrl+Z** / **Ctrl+Shift+Z** or **Ctrl+Y** |
| Preferences | **Ctrl+,** |
| Delete the selected row | **Del** in the tree |

## Configuration

Settings live in `~/.config/printcad/settings.json` and are edited in
Preferences (**Ctrl+,**):

- **General** — log panel, frame rate cap, and a diagnostics switch (off by
  default) that writes a report for every STEP import
- **Display** — camera (projection, field of view, clip planes, axis preset),
  lighting, rendering (MSAA, preferred GPU on hybrid systems)
- **Input** — mouse navigation (style, sensitivities, zoom to cursor, orbit
  around the point under the cursor) and the 6-DoF mouse: what each of its
  six movements does, how fast, which read backwards, and what its buttons do
- **Units**, **Import / Export**, and a page per workbench

## Reporting an import problem

The STEP reader says everything it had to say about a file — an edge that
misses its vertex by a micron, a face it could not trim — and on a real-world
file that is hundreds of lines. They stay out of your way unless you ask:
turn on **Preferences › General › Diagnostics › Write a report for every STEP
import**, import the file again, and the log names a file in the temp dir
(`/tmp/printcad/import-reports/<name>-<stamp>.txt`) holding the kernel's
version, the warnings counted by kind with the worst value and an entity to
look at first, the faces that will draw with gaps, and every line. Send that
file, with the STEP file if you can, to the
[kernel](https://github.com/gilbertorconde/ogeom-rs/issues) or printCAD
issue tracker. The temp dir clears itself, so nothing accumulates.

## Project structure

```
printCAD/
├── crates/
│   ├── app_shell/       # The binary: window, frame loop, UI, input
│   ├── core_document/   # Document, feature tree, undo, persistence
│   ├── doc_server/      # printcad-serverd: the document server + its client
│   ├── kernel_api/      # Geometry contract (meshes, profiles, solid ops)
│   ├── kernel_ogeom/    # ogeom (pure-Rust B-rep) implementation of it
│   ├── render_vk/       # Vulkan backend: data in, pixels out
│   ├── settings/        # Settings persistence
│   ├── ui_kit/          # Design system: tokens, widgets, icons, fonts
│   ├── axes/            # Axis presets, so nothing hardcodes X/Y/Z
│   └── workbenches/
│       ├── wb_part/     # Part Design
│       └── wb_sketch/   # Sketcher: tools, solver, constraints
└── docs/
    ├── plan.md            # Architecture and roadmap
    ├── DOCUMENT_MODEL.md  # How documents, features and assets are stored
    └── WORKBENCH_GUIDE.md # Writing a workbench
```

The app is a client of a document server: `printcad-serverd` owns the file,
one daemon per document, over a UNIX socket. When no daemon can start the app
falls back to direct file I/O and says so in the status bar.

## Roadmap

See [docs/plan.md](docs/plan.md).

- [x] Vulkan renderer, GPU picking, cached scene
- [x] Camera with orbit / pan / zoom / roll and an orientation cube
- [x] Settings persistence and GPU selection on hybrid systems
- [x] Sketcher with a constraint solver
- [x] Part Design feature set on a parametric feature tree
- [x] Undo/redo (per-edit inverse operations)
- [x] STEP import
- [x] Native `.prtcad` documents through a document server
- [x] 6-DoF mouse navigation
- [ ] STEP export
- [ ] Assembly constraints
- [ ] Slicing hand-off for printing

## Technology stack

- **Language**: Rust (edition 2024)
- **Windowing**: winit (Wayland-native)
- **Graphics**: Vulkan via ash
- **UI**: egui
- **Math**: glam
- **Geometry kernel**: [ogeom](https://github.com/gilbertorconde/ogeom-rs) — pure-Rust B-rep modelling and STEP exchange
- **6-DoF input**: [sixdof](https://github.com/gilbertorconde/sixdof) — dependency-free client for spacenavd

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.
