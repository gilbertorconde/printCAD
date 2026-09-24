# Architecture and roadmap

printCAD is parametric CAD for 3D-printed parts. It runs on Linux and is
written in Rust.

## Layers

| Layer | Crate | What it does |
| --- | --- | --- |
| Application | `app_shell` | Window, frame loop, input, UI, tabs |
| Design system | `ui_kit` | Colours, sizes, widgets, icons, fonts |
| Workbenches | `wb_sketch`, `wb_part`, `wb_assembly` | Tools and features, behind the `Workbench` trait |
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

Built: the sketcher, Part Design, assembly joints between bodies, STEP,
IGES and mesh import, STEP, STL and 3MF export, tabs, the document server,
undo, configurable keyboard shortcuts, sending a part to the slicer,
6-DoF navigation, and the command API: typed commands registered by the
application and the workbenches, run from Lua scripts.

Next:

- More joint kinds: gears, limits on a slide or a turn
- Stable face and edge identity across rebuilds, so references do not have
  to be matched by geometry

### Command API

Scripts and AI agents both need to drive the app. They share one layer,
so neither reaches into a workbench:

- Every command the app and the workbenches offer is registered with an
  id, a description and typed parameters and results. The tools,
  actions and menu entries that exist today become entries in it.
- A workbench registers its commands through the `Workbench` trait, as
  it does its tools. The host never names a workbench.
- Queries read the document: bodies, features, sketches, selection,
  measurements. Commands change it by recording ordinary operations, so
  undo, the document server and replay work unchanged.
- A command started by a script or an agent runs on the UI thread
  between frames, like a toolbar click.
- The same list drives the command palette, the keyboard map, the
  script API, the MCP tools and the reference docs.

### Scripts

Built: Lua 5.4 (the `scripting` crate) over every command, the console
(completion, several lines, history kept), script files and the scripts
folder in the Scripts menu, toolbar, palette and keymap, Save as script,
one undo step per run, scripts on a thread of their own with Stop,
recording, and runs without a window. See [Scripting](SCRIPTING.md). Workbenches stay
Rust.

Built as well: tools that end in a command (a sketch tool's clicks are
`sketch.draw` over the points they landed on; Part Design and Assembly
tasks close into the command that makes their feature or joint) and
recording, which writes a session as the list of those commands.

### AI

Built (the `agents` crate and the Assistant panel, see [AI agents](AI.md)):

- Agent registry: agents that speak the Agent Client Protocol (ACP),
  configured in Preferences with the command that starts each, its
  arguments and environment.
- An MCP server inside the app that offers the command API as tools
  (`commands`, `call`, `lua`, `view`, `log`), to the ACP agents and, through
  `printcad --mcp`, to any MCP client.
- A chat panel: several chats, each in its own tab with its own agent and
  history, the thinking, tool calls, plan and permission requests shown
  inline.
- Every change an agent makes goes through the command API, one undo step
  per call. Changes wait for the user's approval unless the chat says
  otherwise; commands declared read-only never wait.

Next: attaching the selection or a picture of the view to a prompt by
hand, and the document as MCP resources.

### Variables and formulas

Built (see [Variables and formulas](VARIABLES.md)): every number in a model
can be a formula over named variables and other objects' dimensions, so a
design is driven by a few values:

- Variable sets: named objects in the document (`Printer`, `Bracket`),
  each a list of variables defined by formulas (`nozzle = 0.4 mm`,
  `wall = 3 * Printer.nozzle`).
- Formulas (`core_document::expr`, built): arithmetic, `if`, the usual
  functions, and units that are checked: a length field takes a length,
  a bare number takes the field's unit. Every reference is
  `object.property`; renaming an object or a property rewrites the
  formulas that use it.
- Every numeric field of Part Design, the Sketcher's dimensions, the
  Assembly's joints and the datums' offsets can hold a formula. The
  document keeps what was typed; values are worked out on each replica,
  like solids, and a feature rebuilds when a value it reads changes.
  Sketches solve again when their dimensions do.
- Workbenches list their numeric properties through the `Workbench`
  trait and read evaluated values; the host evaluates and never names a
  bench.
- Configurations: a table whose rows (S, M, L) set chosen variables; one
  row is active, and export can write every row.
- Commands: `var.*`, `config.*`, and formulas wherever a command takes a
  number, so scripts and agents drive them.
