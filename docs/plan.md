# Architecture and roadmap

printCAD is parametric CAD for 3D-printed parts. It runs on Linux and is
written in Rust.

## Layers

| Layer | Crate | What it does |
| --- | --- | --- |
| Application | `app_shell` | Window, frame loop, input, UI, tabs |
| Design system | `ui_kit` | Colours, sizes, widgets, icons, fonts |
| Workbenches | `wb_sketch`, `wb_part` | Tools and features, behind the `Workbench` trait |
| Document | `core_document` | Feature tree, bodies, undo, `.prtcad` files |
| Document server | `doc_server` | Owns the file on disk, one process per document |
| Geometry interface | `kernel_api` | Meshes, profiles and solid operations as plain data |
| Geometry kernel | `kernel_ogeom` | The interface implemented with the ogeom kernel |
| Renderer | `render_vk` | Vulkan: scene data in, pixels out |
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
4. The kernel thread runs them and returns a mesh per body.
5. The renderer draws the meshes and answers pick requests.

## Roadmap

Built: the sketcher, Part Design, STEP, IGES and mesh import, STEP, STL and
3MF export, tabs, the document server, undo, and 6-DoF navigation.

Next:

- Sketch external geometry, which waits on the kernel's projection of an
  edge onto a plane
- Assembly constraints between bodies
- Sending a part straight to a slicer
- Stable face and edge identity across rebuilds, so references do not have
  to be matched by geometry
