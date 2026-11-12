---
name: printcad-render-loop
description: "Rendering: the on-demand frame loop (what keeps frames coming, waking the loop from threads, fps_cap), render_wgpu (FrameSubmission, passes, shaders, picking, captures), the edge pass cost and the cached 3D scene whose scene_fingerprint must hash everything the scene pass reads. Use when working in crates/render_wgpu or app frame scheduling, when async work does not surface until the next input, when the scene shows stale, or when adding a GPU pass, shader or per-body draw property. Not for the PRINTCAD_BENCH_* capture hooks (use printcad-bench-hooks) or input bindings (use printcad-interaction-model)."
---

# printCAD: render loop

The invariants in `CLAUDE.md` still bind here; this skill never weakens one. The renderer never waits on the GPU, `app/gfx.rs` field order is the teardown contract, and GPU validation stays clean.

## Render loop

Frames are rendered **on demand**, not continuously: a frame is scheduled
while input is fresh (150 ms tail), async work is pending (kernel jobs,
document open/save, file dialog, deferred import), a 6-DoF puck is deflected,
the camera tween or bench orbit is running, or egui asked for a repaint (`repaint_delay` == 0; finite delays
become `WaitUntil`, e.g. caret blink). Otherwise the event loop sleeps in
`ControlFlow::Wait` until the next OS event. Consequences: anything that
completes on a background channel must be covered by one of the
"work pending" flags or it will not surface until the next input event; a
thread whose work produces no window event (the 6-DoF reader) must also wake
the loop itself, through `EventLoopProxy::send_event` and the `AppEvent`
user event; and
`fps_cap` now caps the *active* rate rather than implying continuous
rendering.

The bench hooks (`PRINTCAD_OPEN_FILE`, `PRINTCAD_BENCH_*` and the rest) are listed in the `printcad-bench-hooks` skill.

The 1 s `printcad.frame` log reports
fps + phase costs while frames are being produced.

Face-boundary edges draw on every frame, moving or still. They are cheap
next to the solids: a 123-body assembly (9 M triangle indices, 419 k edge
indices) spends 1–2 ms of its frame on the edge pass, and a 272-body one
(133 M triangle indices, ~130 ms/frame) spends none, because
`PRINTCAD_EDGE_MIN_PX` (24 px of body AABB on screen, mesh.rs) has already
culled every body's edges. Measure with `PRINTCAD_NO_EDGES=1` against the
same orbit before assuming the pass costs anything.

**The 3D scene is cached between changes.** The scene pass resolves into a
persistent scene image and runs only when `scene_fingerprint(frame)`
(`render_wgpu/src/core.rs`) changes; every frame copies that image under the
UI pass. UI-only frames (hover, panels, typing) therefore cost ~2 ms on any
model. **Completeness of the fingerprint is the contract**: anything the
scene pass reads (camera, viewport, lighting, per-body id/revision/mesh
pointer/color/highlight/wireframe) must be hashed there,
or a change shows stale. The status bar shows both numbers because they are
two things: `FPS` (UI frames presented) and `scene: N/s` (scene redraws;
"cached" when zero). `PRINTCAD_BENCH_SPIN` keeps the loop awake with no
scene change (expect zero redraws); `PRINTCAD_BENCH_ORBIT` expects one per
frame.

## render_wgpu

- `render_wgpu`: data-only renderer (`FrameSubmission` in, pixels out) on
  wgpu: Vulkan, Metal or DirectX 12, the first the platform offers
  (`WGPU_BACKEND` picks one), the GPU chosen by `preferred_gpu` or for
  performance. Shaders are WGSL (`shaders/`), validated by naga in its
  tests. GPU picking with async readback (`readback.rs`: a buffer per pick
  or picture, mapped once its frame is done); mesh cache keyed by the
  body's id, or the shared mesh's key when a `BodySubmission` carries a
  `MeshInstance` (its mesh drawn moved by `Draw.model`), and revision.
  Edges are instanced screen-space quads (`vs_edge`), any
  width on any backend; the clipping plane discards per fragment. The pick
  pass writes depth into a colour target as well, since a depth texture
  cannot be copied in part, and, where the device has
  `Features::PRIMITIVE_INDEX`, the triangle drawn
  (`pick_primitive.wgsl`, `PickResult::triangle` with the revision the
  body was drawn at); the host takes it as the face under the cursor when
  that body was drawn with its document mesh at that revision
  (`DocumentSession::drawn.meshes`, not a textured body), and otherwise
  searches for the triangle nearest the picked point. The window takes a
  plain format, as egui blends for; the scene draws into an sRGB texture
  that the blit encodes onto it.
  A `BodySubmission::scalar` (`ScalarField`: a value per vertex with
  its revision, range, bands, ramp and the colours below, above and for
  `NaN`) paints a body through a ramp: the values upload in the vertex
  colour slot once per revision (an unmeasured one as a flag in the
  slot's second lane, since a GPU compiler may take NaN as equal to
  itself), the rest is per draw (`DrawUniform`,
  the ramp a row of a 256×16 atlas, `ramp.rs`), so recolouring uploads
  nothing; the scene fingerprint hashes it all. The host attaches a
  bench's `ViewportOutput::scalar_maps` (`ScalarMap`, in the document mesh's
  vertex order) to that body's submission in `frame.rs`, and
  `ui/legend.rs` draws the first map's `MapLegend` at the view's bottom
  left.
  `FrameSubmission.side_pane` (`SidePane`: a rect the viewport's size,
  the bodies drawn there instead, and an optional `PaneCamera` of its
  own, hashed in the fingerprint, with its own frame uniform) draws a
  second pane; a pick inside its rect draws its bodies with its camera
  (`Pane::at`). View › Split view puts every body but the scans, and the
  selection and hover paints drawn for them (`overlay_bodies`), in it
  (`DocumentSession::view.split`, the UI halving the viewport); View ›
  Link split cameras off gives it `DocumentSession::view.side_camera`
  (`CameraController::detached`, kept as `KeptView::side`). Pointer
  input over it (`input.rs::side_pane_input`) runs with the side camera
  swapped in as the session's and the cursor in its pixels, so benches,
  navigation and selection read it as the viewport; overlay hooks get
  `ctx.side_pane` (`SidePaneView`) and place what lies on its bodies
  with `world_to_viewport_on`, marks and labels clipped to both panes.
  Pick pins are `core_document::numbered_pins` (constant size, scaled by
  `pixels_per_point`): the measure tool's picks, Inspection's Shift-click
  seeds and its Align by points pairs (`wb_inspect/src/picks.rs`).
  `FrameSubmission.grids` (`GridSubmission`, `grid.rs`): a patch of any
  plane whose lines `grid.wgsl` works out per pixel, three decades at once,
  each faded by its spacing on screen, depth-tested and never picked.
  `step` is any length (the finest decade); the fade's thresholds come
  from `core_document::grid::FADE_PX` in `fade_px`, which the step
  choice (`finest_step`) and the sketcher's grid snap (`clear_step`, the
  finest decade past half its fade in) read too.
  `app/scene_guides.rs` lays out the ground grid and origin planes; View ›
  Grid and Origin planes turn them on (both start off), and an edit
  session holding the view hides them. A bench's `ViewportOutput::view_grid`
  (`PlaneGrid`: a plane, a base step, colour, opacity and dash; the
  sketcher's on its plane while its grid is on) is drawn in the ground
  grid's place (`scene_guides::bench_grid`); packages have no such hook. While the active bench's
  `shows_origin_planes` is true (the sketcher's plane picker) the planes
  draw whatever the setting, and the one under the cursor (a ray test, a
  body in front of it winning) reaches the bench as
  `ctx.hovered_base_plane`.
