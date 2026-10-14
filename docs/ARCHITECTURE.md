# Architecture

printCAD is parametric CAD for 3D-printed parts. It runs on Linux, Windows
and macOS, and in a browser page ([printCAD in a browser](BROWSER.md)). It
is written in Rust.

## Layers

| Layer | Crate | What it does |
| --- | --- | --- |
| Application | `app_shell` | Window, frame loop, input, UI, tabs |
| Design system | `ui_kit` | Colours, sizes, widgets, icons, fonts |
| Workbenches | `wb_sketch`, `wb_design`, `wb_surface`, `wb_assembly` | Tools and features, behind the `Workbench` trait |
| Workbench packages | `wb_wasm`, `bench_api` | Workbenches built for WebAssembly, run sandboxed, and what they exchange with the app |
| Workbench registry | `workbenches`, `bench_fixtures` | Registers the built-in workbenches and installed packages; ready-made scenes for tests |
| Sketch solver | `sketch_solver` | The 2D constraint solver: a numerical problem in, a solution out, no sketch or document |
| Scripting | `scripting` | The Lua engine scripts and the console run in |
| AI agents | `agents` | Agent Client Protocol client and MCP server core |
| Document | `core_document` | Feature tree, bodies, undo, formulas, `.prtcad` files |
| Document server | `doc_server` | Owns the file on disk, one process per document (desktop only) |
| Geometry interface | `kernel_api` | Meshes, profiles and solid operations as plain data |
| Geometry kernel | `kernel_ogeom` | The interface implemented with the ogeom kernel |
| Renderer | `render_wgpu` | wgpu: scene data in, pixels out |
| Camera | `viewport_camera` | Camera pose, projection, rays, navigation and fit, with no window |
| Transform gizmo | `transform_gizmo` | Move, turn and scale handles: layout, hit test and drag, drawn by the host |
| Surface textures | `surface_texture` | Patterns pressed into faces of a mesh for printing |
| Local IPC | `local_ipc` | Local sockets and system helpers on every platform |
| Settings | `settings` | User preferences on disk |
| Axes | `axes` | Axis presets, so no code assumes which way is up |

## Key decisions

- **The application knows no workbench by name.** Every workbench talks to
  it through the `Workbench` trait. A test fails if a workbench name appears
  in `app_shell`. See [Writing a workbench](WORKBENCH_GUIDE.md).
- **Solids are derived.** A document stores features. The solid of a body is
  rebuilt from them by the kernel on a background thread.
- **Every edit is an operation.** The document records one operation per
  edit. Undo applies the inverse operation. See
  [Document model](DOCUMENT_MODEL.md).
- **The document server owns the file.** The application sends it the saved
  bytes and the edit log. This keeps the door open for several people
  editing one document. The server is desktop-only: a browser page keeps
  its files in the page's storage (`BrowserFiles`).
- **Frames render on demand.** The loop sleeps until something changes. The
  3D scene is cached, so a frame that only changes the UI is cheap.
- **Kernel gaps are fixed in the kernel.** When ogeom lacks something, the
  feature is wired anyway, a test marked `#[ignore = "kernel: ..."]` records
  the gap, and an issue goes to the
  [kernel repository](https://github.com/gilbertorconde/ogeom-rs/issues).

## Data flow

1. A workbench edits the document.
2. Changed features and everything that depends on them are marked dirty.
3. Each frame, the workbenches turn dirty bodies into lists of solid
   operations.
4. The kernel worker runs them on background threads, resuming from what
   the last build of the body kept, and returns a mesh per body.
5. The renderer draws the meshes and answers pick requests.

## What comes next

The [roadmap](ROADMAP.md) lists everything still open. How the command API, scripts, AI agents and formulas work is in
[Scripting](SCRIPTING.md), [AI agents](AI.md) and
[Variables and formulas](VARIABLES.md).
