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
- **FreeCAD-style Navigation** - Familiar camera controls with turntable orbit, pan, and zoom
- **Interactive Orientation Cube** - Click faces, edges, or corners to snap to standard views
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
│   ├── wb_part/         # Part Design workbench
│   └── wb_sketch/       # Sketch workbench
└── docs/
    └── plan.md          # Detailed architecture and roadmap
```

## Controls

### Camera Navigation

| Action       | Control                                 |
| ------------ | --------------------------------------- |
| Orbit        | Middle mouse button drag                |
| Pan          | Shift + Middle mouse button drag        |
| Zoom         | Scroll wheel                            |
| Snap to view | Click orientation cube face/edge/corner |
| Rotate 45°   | Click orientation cube arrows           |

### Orientation Cube

- **Faces** - Snap to front, back, left, right, top, bottom views
- **Edges** - Snap to 45° between two faces
- **Corners** - Snap to isometric views (45° in two axes)

## Configuration

Settings are stored in `~/.config/printCAD/settings.json` and include:

- Preferred GPU selection
- FPS cap (0 = uncapped)
- Camera projection (Perspective/Orthographic)
- Field of view

## Roadmap

See [docs/plan.md](docs/plan.md) for the detailed development roadmap.

### Current Status

- [x] Vulkan renderer with basic mesh display
- [x] Camera controller with turntable navigation
- [x] Interactive orientation cube (FreeCAD-style NaviCube)
- [x] Settings persistence
- [x] GPU selection for hybrid systems
- [ ] Sketch workbench with constraint solver
- [ ] Part Design workbench (pad, pocket, revolve)
- [ ] STEP import/export via OpenCASCADE
- [ ] Full parametric feature tree
- [ ] Undo/redo system

## Technology Stack

- **Language**: Rust
- **Windowing**: winit (Wayland-native)
- **Graphics**: Vulkan via ash
- **UI**: egui
- **Math**: glam
- **Geometry Kernel**: OpenCASCADE (planned)

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
