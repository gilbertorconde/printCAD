---
name: printcad-interaction-model
description: "How the user drives the app: the current bindings (tabs, mouse orbit/pan/zoom, context and radial menus, face-first selection, Ctrl-additive faces and edges, hover and selection overlays, the planar lock while sketching, the tree as body history with its tip, visibility eyes, the start page), the keymap and shortcuts, viewport_camera, transform_gizmo and 6-DoF input through sixdof. Use when changing a mouse or keyboard binding, selection or picking behaviour, tree navigation, the camera, the gizmo or SpaceMouse handling. Not for frame scheduling (use printcad-render-loop) or widget styling (use printcad-ui-conventions)."
---

# printCAD: interaction model

The invariants in `CLAUDE.md` still bind here; this skill never weakens one. NDC is Y-down and camera orientation is preset-relative: both bite here.

## Interaction model (current bindings)

Tabs: one document per tab (`ui/tab_bar.rs`, under the menu bar), Ctrl+T
new, Ctrl+W close, Ctrl+Tab / Ctrl+Shift+Tab cycle, middle click closes;
a fresh tab shows the start page and New/Open take it over, a tab with
content keeps its edits and the new document opens beside it; closing
asks about unsaved edits per tab, quitting asks for each dirty tab.
MMB drag = orbit (MMB click = pivot pick) · RMB drag = pan · RMB click on a
body = context menu (`ui/context_menu.rs`: show in tree, select body, hide,
then whatever the benches offer through `menu_items` for
`MenuScope::ViewportBody`; the host owns the open `ViewportMenu`, the UI
draws it and answers with commands; tree rows and the start page's New
cards take bench entries the same way, and a pick runs the bench's
`on_command`) · RMB held still (300 ms) = the radial menu (`app/radial.rs`,
`ui/radial.rs`: the active bench's slices round the press, the user's
`UserSettings::radial` or the bench's `WorkbenchDescriptor::radial` (a
package's `Registration::radial`) or its most used tools by
`UserSettings::tool_uses`; letting go over one starts the tool as its
button would or runs the command; Preferences › Input › Radial menu
edits them) · wheel = zoom · LMB = select (click sketch → tree-select; click
solid → face-first, double click → the whole body the face belongs to, one
part of an assembly, and in Design the tree opens to its row; LMB drag
in sketch = box select; ctrl = additive: Ctrl+click adds or removes a
face of the selected body (`session.selection.earlier_faces` beside the last,
`last_face_hit`; `face_pick::toggled`), and an edge, faces and edges mixing;
benches see every picked face as `ctx.selected_faces` /
`selected_faces_in(body)`, `selected_face` the last; the status bar says
"3 faces, 2 edges"). The
face under the cursor draws translucent in the hover paint (an edge within
reach takes the hover instead, as a line), resolved from the pick's point
and kept while that point stays on the same face; the
selected face, or the whole selected body, draws as a translucent overlay
over itself, never as a tint (`rendering.selection_color` /
`selection_opacity`, Preferences › Display › Rendering) through the
renderer's blended pass: any `BodySubmission` with `opacity < 1` draws
after the opaque bodies and their edges, depth-tested, never writing depth,
and the pick pass skips it: an overlay is never what the cursor is over.
While editing a sketch the view is locked planar (orbit + cube rotation
disabled; pan/zoom/roll allowed). The tree reads as the body's history in
order: selecting a feature moves the body's tip to it (off it, when it is
the last; `move_in_time_to`), later features draw muted and out of the
solid, and a feature made then goes in right after the tip, which follows
it (`Document::insert_at_tip`, a seq swap per step, like Move up). A tip
move on its own is navigation: `OpJournal` makes no undo step of a gesture
that only moves tips. In the tree, bodies and features start
open and imported assemblies start closed; the filter looks through closed
branches, a double click or "Show in tree" opens a body's way to itself,
and a body row's double click or "Select body" selects the whole body.
Every row that stands for something drawn has an eye: features, imported
parts and bodies (`Body.hidden`, `SetBodyVisible`, undoable), and the
viewport's Hide hides whatever body was clicked; a hidden body is out of
drawing, picking and the clip bounds (`imported_body_effective_visible`). An
instance row that wraps a single part is one row with the instance's id and
the part's body: resolve it through `Document::body_of_imported_object`,
never through the node's own `body_id`, which an instance lacks. The window
opens
on the start page (`Screen::Start`); the recent list lives in
`settings::recent`.

## Keyboard shortcuts

**Keyboard shortcuts.** One keymap (`ui/keymap.rs`) holds the
application's commands (`HOST`, ids like `file.save`) and every bench's
tools and registered actions, each with default keys
(`ToolDescriptor::shortcut`, `WorkbenchContext::register_action`,
`core_document::Chord`). The user's changes live in
`UserSettings.keyboard` by id and are edited in Preferences › Keyboard.
`take_pressed` runs at the start of each UI frame and removes the keys it
uses from egui's input: the active bench's keys win over the
application's, keys without Ctrl or Alt stay with a focused text field,
nothing fires while Preferences or the palette is open, and bare
number keys stay with a bench whose `takes_numeric_input` is true (the
sketcher while a length is typed). Shortcuts are single chords, never
sequences. Defaults: a plain letter picks a bench tool, Shift and a
letter its partner (a sketch constraint, a subtractive feature); 0 to 6
are the standard views, O/P the projection, Space and Delete act on the
tree selection (`HostState`). A tool key
activates the tool as a click would; an action key reaches the bench as
`WorkbenchInputEvent::Action`. Benches learn their keys in effect through
`Workbench::shortcuts_changed`. Contextual keys (Escape, Enter, Delete in
the sketcher, Escape for the measure tool) stay in the bench or host that
owns the moment.

## Camera, gizmo and 6-DoF input

- `viewport_camera`: camera pose, projection, rays, press-relative navigation,
  fit and timed transitions, shared with other applications. It depends only
  on `glam` and `serde`; its README describes the copy contract. The app's
  `camera/core.rs` converts preset-relative orientations, model coordinates
  and physical pixels; `camera/` owns input routing, clip planes and device input.
- `transform_gizmo`: move, turn and scale handles without a window
  (`gizmo::Gizmo`: layout, hit-test, press/drag/release and the delta from
  the press; `paint::Shape`s in colour roles, `paint::Ink`, for the host to
  draw in its palette; `hand` turns a delta into increments of the freedoms
  a handle stands for). Plain `glam` in `f64` and `emath` at its boundary,
  no printCAD type: it is shared with other applications, and a change goes
  to the shared copy first (its README).
- 6-DoF input (SpaceMouse and the like) comes from the `sixdof` crate
  (crates.io, this project's own), consumed by version exactly as the kernel
  is, with the same commented `[patch.crates-io]` for local work. It needs no
  system library: it speaks the spacenavd socket
  protocol itself, with the display-server (Magellan) protocol behind its
  `magellan` feature, and on Windows and macOS (without spacenavd) reads the
  device's USB HID reports, giving the numbers the daemon would. Nothing is required to build or run without a device.
