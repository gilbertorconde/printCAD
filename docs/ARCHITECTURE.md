# Architecture

printCAD is parametric CAD for 3D-printed parts. It runs on Linux, Windows
and macOS. It is written in Rust.

## Layers

| Layer | Crate | What it does |
| --- | --- | --- |
| Application | `app_shell` | Window, frame loop, input, tabs: the host |
| UI | `app_ui` | The window's panels, menus and tasks, drawn each frame from the host's inputs and answering with commands |
| Application services | `assistant`, `doc_export`, `file_session`, `package_store`, `kernel_worker` | What the application keeps beside its window: chats with agents, export and the slicer, recovery copies and file safety, workbench packages and the release check, the kernel thread |
| View camera | `view_camera`, `orientation_cube` | Window input steering the camera, the clipping plane and kept views; the orientation cube |
| Application support | `app_log`, `app_platform` | The log panel's lines; threads beside the window, atomic writes, private folders |
| Design system | `ui_kit` | Colours, sizes, widgets, icons, fonts |
| Workbenches | `wb_sketch`, `wb_design`, `wb_surface`, `wb_assembly` | Tools and features, behind the `Workbench` trait |
| Workbench packages | `wb_wasm`, `bench_api`, `panel_api` | Workbenches built for WebAssembly, run sandboxed, and what they exchange with the app |
| Workbench registry | `workbenches`, `bench_fixtures` | Registers the built-in workbenches and installed packages, builds the calibration coupon; ready-made scenes for the bench hooks and tests |
| Sketch model | `sketch_model` | The sketch as data the sketcher, Design, Surface and import share: geometry, constraints and solving, profiles and regions, text, generated profiles |
| Sketch solver | `sketch_solver` | The 2D constraint solver: a numerical problem in, a solution out, no sketch or document |
| Scripting | `scripting` | The Lua engine scripts and the console run in |
| AI agents | `agents` | Agent Client Protocol client and MCP server core |
| Document | `core_document` | Feature tree, bodies, undo, formulas, `.prtcad` files |
| Document server | `doc_server` | Owns the file on disk, one process per document (desktop only) |
| Geometry interface | `kernel_api` | Meshes, profiles and solid operations as plain data |
| Geometry kernel | `kernel_ogeom` | The interface implemented with the ogeom kernel |
| Build driver | `build_driver` | Builds a document's solids from the workbenches' plans, the same way for the app, scripts and project import |
| Renderer | `render_wgpu` | wgpu: scene data in, pixels out |
| Camera | `viewport_camera` | Camera pose, projection, rays, navigation and fit, with no window |
| Transform gizmo | `transform_gizmo` | Move, turn and scale handles: layout, hit test and drag, drawn by the host |
| Surface textures | `surface_texture` | Patterns pressed into faces of a mesh for printing |
| Local IPC | `local_ipc` | Local sockets and system helpers on every platform |
| Settings | `settings` | User preferences on disk |
| Axes | `axes` | Axis presets, so no code assumes which way is up |

## Key decisions

- **The application knows no workbench by name.** Every workbench talks to
  it through the `Workbench` trait, which holds a workbench's description
  and lifecycle, and its supertraits, one per concern (features, tools and
  input, panels, builds, parameters, output, commands). A test fails if a
  workbench name appears in `app_shell`. See [Writing a workbench](WORKBENCH_GUIDE.md).
- **The UI and the application meet in one place.** The UI (`app_ui`)
  draws each frame from the inputs the application hands it and answers
  with commands, which `app_shell` applies. Both name only
  `app_ui::contract`; the UI's modules are private, `UiLayer` all the
  application sees of them, and `scripts/check-ui-boundary.mjs` keeps it
  so.
- **Solids are derived.** A document stores features. The solid of a body is
  rebuilt from them by the kernel on a background thread.
- **Every edit is an operation.** The document records one operation per
  edit. Undo applies the inverse operation. See
  [Document model](DOCUMENT_MODEL.md).
- **The document server owns the file.** The application sends it the saved
  bytes and the edit log. This keeps the door open for several people
  editing one document.
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
   the last build of the body kept, and returns a mesh per body. What a
   build leaves in the document (the solid, warnings, a failing feature's
   error) is landed by `build_driver`, which a script run without a window
   and a project import use to build on their own thread, so all three
   build a document alike.
5. The renderer draws the meshes and answers pick requests.

## What comes next

The [roadmap](ROADMAP.md) lists everything still open. How the command API, scripts, AI agents and formulas work is in
[Scripting](SCRIPTING.md), [AI agents](AI.md) and
[Variables and formulas](VARIABLES.md).
