# printCAD

A parametric CAD application focused on designing parts for FDM/SLA 3D printing, built entirely in Rust with a Vulkan renderer.

![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)
![Rust](https://img.shields.io/badge/rust-1.79%2B-orange)
![Platform](https://img.shields.io/badge/platform-Linux-lightgrey)

> ⚠️ **Early Development** - This project is in early development and is not yet usable for actual CAD work. Core features like sketch constraints, part operations, and file I/O are still being implemented.

## Overview

printCAD is a Linux-native, Wayland-first CAD application designed for creating parametric 3D models optimized for 3D printing workflows. It features a modular architecture with clean abstractions for future extensibility.

### Key Features

- **Vulkan Rendering** - Hardware-accelerated 3D viewport with perspective/orthographic projection
- **FreeCAD-style Navigation** - Gesture camera (orbit, pan, zoom, roll) plus optional orbit around GPU-picked points without reframing the view
- **Interactive Orientation Cube** - Click faces, edges, or corners for standard views; arc and triangle arrows for incremental rotation
- **Modular Workbenches** - Extensible architecture for Sketch and Part Design workflows
- **Parametric Core** - Feature tree with dependency graph, transactions, and undo/redo (planned)
- **GPU Selection** - Choose between available graphics cards in hybrid GPU systems

## Screenshots

_Coming soon_

## Building

### Prerequisites

- Rust 1.79 or later
- Vulkan SDK and drivers
- Linux with Wayland (X11/XWayland fallback supported)

### Build & Run

```bash
# Clone the repository
git clone https://github.com/yourusername/printCAD.git
cd printCAD

# Build and run
cargo run -p app_shell

# For release build
cargo run -p app_shell --release
```

### GPU Selection (Hybrid Systems)

For systems with multiple GPUs, you can select the preferred GPU in Settings > Rendering.

## Project Structure

```
printCAD/
├── crates/
│   ├── app_shell/       # Main application, windowing, UI
│   ├── core_document/   # Document model and feature tree
│   ├── kernel_api/      # Geometry kernel abstraction trait
│   ├── kernel_occt/     # OpenCASCADE kernel implementation
│   ├── render_vk/       # Vulkan rendering backend
│   ├── settings/        # Application settings persistence
│   └── workbenches/
│       ├── wb_part/     # Part Design workbench
│       └── wb_sketch/   # Sketch workbench
└── docs/
    ├── plan.md          # Detailed architecture and roadmap
    └── WORKBENCH_GUIDE.md # Guide for creating custom workbenches
```

## Controls

### Camera Navigation

Gesture-style navigation applies in the central 3D viewport (click vs drag distinguishes select from orbit):

| Action | Control |
| ------ | ------- |
| **Orbit** | Left drag (after a small pixel threshold — short click selects instead) |
| **Pan** | Right drag |
| **Roll / tilt camera** | Left + right buttons drag |
| **Zoom** | Scroll wheel (optional **zoom toward cursor**) |
| **Pivot on geometry** | Middle click snaps the orbit pivot to the point under the cursor (reframes to that pivot on the lens axis) |
| **Pivot on focal plane** | **`H`** with the cursor over the viewport — moves pivot to intersection of the ray under the cursor with the current focal plane |
| Snap to standard view | Orientation cube face / edge / corner |
| Nudge ±45° | Orientation cube triangular or arc arrows |

When **Settings → Camera → “Orbit around point under cursor”** is enabled, an LMB orbit that starts over mesh uses that pick as an **off-axis orbit anchor**: the scene does **not** jump to center it; the small red orbit marker is drawn at the **projected anchor** while you drag.

### Orientation Cube

- **Faces** - Snap to front, back, left, right, top, bottom views
- **Edges** - Snap to 45° between two faces
- **Corners** - Snap to isometric views (45° in two axes)

## Configuration

Settings are stored in `~/.config/printCAD/settings.json` and include:

- Preferred GPU selection
- FPS cap (0 = uncapped)
- Camera: projection (perspective / orthographic), FOV / ortho height, orbit & pan sensitivity, zoom-to-cursor, **orbit around point under cursor** (GPU pick orbit anchor), yaw axis for orbit, focal distance clamps, clip auto near/far, click↔drag threshold
- Rendering quality (MSAA sample count)
- Debug options such as the in-app log panel

## Documentation

- **[Development Plan](docs/plan.md)** - Detailed architecture and roadmap
- **[Workbench Development Guide](docs/WORKBENCH_GUIDE.md)** - Guide for creating custom workbenches

## Roadmap

See [docs/plan.md](docs/plan.md) for the detailed development roadmap.

### Current Status

- [x] Vulkan renderer with basic mesh display
- [x] Camera controller with gesture navigation (orbit / pan / roll / zoom-to-cursor / optional orbit around pick)
- [x] Interactive orientation cube (FreeCAD-style NaviCube)
- [x] Settings persistence
- [x] GPU selection for hybrid or multi gpu systems
- [ ] Sketch workbench with constraint solver
- [ ] Part Design workbench (pad, pocket, revolve)
- [x] STEP/STP import (OpenCASCADE) — experimental; export and full solid history still planned
- [ ] Full parametric feature tree
- [ ] Undo/redo system

## Technology Stack

- **Language**: Rust
- **Windowing**: winit (Wayland-native)
- **Graphics**: Vulkan via ash
- **UI**: egui
- **Math**: glam
- **Geometry Kernel**: OpenCASCADE (STEP import; parametric modelling still planned)

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

## Acknowledgments

- Inspired by FreeCAD's navigation and UI patterns
- Built with the excellent Rust ecosystem
