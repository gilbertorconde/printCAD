---
name: printcad-ui-conventions
description: "UI code conventions and the design system: ui_kit (tokens, theme, fonts, widgets, icons), bench_panel (declared panels and formula fields drawn with egui), the UI rules (no color literals, icons only through vendor-icons.mjs, UI-local state on UiLayer, ui-mockup/ as reference) and drag limits (steps::nearest, hold_dragged, drag_range). Use when writing or reviewing egui panel code, adding a color, size, icon, widget or declared panel field, or changing how a dragged number stops. Not for input bindings (use printcad-interaction-model) or app module layout (use printcad-app-shell)."
---

# printCAD: UI conventions

The invariants in `CLAUDE.md` still bind here; this skill never weakens one. UiLayer never owns state the host mutates; adding a UI action is one `ui/commands.rs` variant plus one `fold_command` arm.

## UI conventions

- `ui_kit::tokens` and the font helpers are the only source of colors and
  sizes in UI code: no color literals in panels (converting a palette or
  settings color to `Color32` is fine).
- Every icon is named in `ToolDescriptor::icon` or drawn through
  `ui_kit::icon`; each workbench's tests assert every registered tool's
  icon (and variant icon) exists. New icons go through `scripts/vendor-icons.mjs`, never by hand
  (the script strips metadata and normalises the stroke color).
- UI-local state (filters, drafts, palette query) lives on `UiLayer`;
  anything the host mutates is seeded from `UiFrameInputs` every frame.
- `ui-mockup/` is the design reference and stays untracked; only tokens,
  icons, motion drawings and fonts are vendored from it.

## Crates

- `ui_kit`: the design system, below the workbenches so their panel code
  can use it (behind their `egui` feature): `tokens` (the palette and
  size constants, named after the design's variables), `theme`
  (`apply_theme`, bundled IBM Plex Sans/Mono under `fonts/`, fetched by
  `scripts/vendor-fonts.sh`; `sans/sans_medium/sans_semibold/mono` font
  helpers), `widgets` (Card, overline, badge, key chip, toggle, check row,
  note card, the button set, `tool_button`, `section_header` and
  `section_fold` (every panel's folds: a hairline, the title in small
  capitals, a count and a summary while closed), `status_pill`,
  `filter_field`, `list_row` (`RowLook`: selected, problem, dim, and
  `related`/`lit` for rows tied to the item in hand) and `row_value` (a
  panel's list rows: icon, name, dim detail, value at the end;
  `list_row_with` draws its own detail), `chip`/`chips` (small rounded
  names inside a row, hoverable and clickable, "+N" past the room) and
  `badges` (glyph badges, capped with "+N"); the sketcher's lists use them
  over `wb_sketch/src/relations.rs`, `QtyField`,
  `select_field`, `PrefRow` + `pref_group`, `planned`,
  `completion::completing_text_edit` (a text edit whose dropdown offers
  the five names nearest the word at the cursor, arrows, Enter/Tab and
  clicks, fed by `bench_panel::formula_candidates` wherever a formula
  is typed), and `Tab` +
  `tab_plus`, the one tab every strip draws: documents, chats, the property
  panel's pages, Preferences), `icon` (the SVG
  set under `icons/`, vendored by `scripts/vendor-icons.mjs` into a
  generated `icon_table.rs`; `icon::texture/draw` rasterize with a
  font-free usvg, since the system-font scan is far too slow for 200 icons;
  `select.svg`, `expression.svg`, `chevrons-down.svg` and `home.svg` are
  hand-authored locals). The same
  table carries the `motion-*` drawings (200×200, their own colours, one
  per movement of a 6-DoF mouse), drawn through `icon::drawing`, which
  rasterizes for the size it is shown at rather than the icon size. A test
  fails when the table and the directory disagree. `markdown` draws the
  chat's messages: `pulldown-cmark` into a small block tree, each run of
  text one selectable label whose links are found from where a click lands
  in the laid-out text, code blocks with a copy button.
- `bench_panel`: the declared panels drawn with egui (`show`, `show_card`,
  `show_filtered`, `focus_entry`, `PanelOutput`, `FormulaSet`), and the
  formula fields over a document (`DocumentFormulas`,
  `formula_candidates`), on `core_document` and `ui_kit`. The benches take
  it behind their `egui` feature; app_shell always. It holds a bench's
  drawing side, `BenchUi` (`drawing.rs`: `ui_task_panel`, `ui_settings`,
  `ui_left_panel`, `ui_viewport_strip`), which each bench implements on
  its own type behind its `egui` feature; `attach::<T>` keeps the cast to
  it in the registry (`DocumentService::set_drawing_side`, opaque to the
  core; `Workbench: Any` lets it downcast), done by the `workbenches`
  facade for every listed bench and for packages in `register_prepared`,
  and the app reaches it with `bench_ui(registry, id)`. `core_document`
  has no egui at all.

## Drags stop at limits

**Drags stop at limits.** A drag never carries a number past its limit;
only typing may. In the view, `core_document::steps::nearest` steps a
dragged number and lands on a limit that is no whole step; in fields,
`ui_kit::widgets::{hold_dragged, step_within}` hold a drag and the wheel
to `QtyField::drag_range`/`FormulaField::drag_range`, which a declared
`Quantity`'s `min`/`max` and `drag_min`/`drag_max` set. The property
panel's parameter rows take a bench's `Workbench::parameter_drag_range`;
the app's scalar arrows (`app/scalar_arrow.rs`) carry `ScalarDrag::stops`
(the clipping plane's scene span).
