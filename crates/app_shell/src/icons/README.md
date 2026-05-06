# printCAD icon conventions

Icons in this directory back the *global* (app-wide) UI: menu items, the
workbench combo, and any other surface that isn't owned by a specific
workbench. Workbench-specific icons live alongside the workbench's source at
`crates/workbenches/<crate>/src/icons/<tool_id>.svg`.

To keep the look coherent and the rasterizer happy, every SVG in here should
follow the same rules. New contributions that don't follow the rules will
read visually inconsistent next to the existing set.

## Format

- **Canvas:** `width="24" height="24" viewBox="0 0 24 24"`. The renderer
  rasterizes at this exact size; anything bigger will be scaled and look
  fuzzy at the toolbar's natural button height.
- **Margin:** keep all artwork inside `2 ≤ x,y ≤ 22`. The 2 px breathing
  room prevents the strokes from touching the button border on hover.
- **Strokes only:** `fill="none"`, `stroke="#FFFFFF"`, `stroke-width="1.5"`
  with `stroke-linecap="round"` / `stroke-linejoin="round"`. Solid white
  matches the default egui dark theme; the rasterizer multiplies in the
  active text color so a single white stroke renders correctly in any
  theme.
- **No filters / gradients / drop shadows:** the rasterizer used by
  `crate::orientation_cube::rasterize_svg` is a small subset of SVG. Stick
  to `path`, `line`, `polyline`, `polygon`, `circle`, `rect`.
- **No external references:** no `<image>`, no `<use>` outside the file.

## Adding a new icon

1. Drop the SVG in this folder (global icons) or in the workbench's
   `src/icons/` (workbench tool icons).
2. Name the file after the command id you want it served for. Workbench
   tools are looked up as `<tool_id>.svg`; e.g. `part.new_body` resolves to
   `crates/workbenches/wb_part/src/icons/part.new_body.svg`.
3. Reload the app. The icon cache is process-lifetime, so the new icon
   becomes visible on the next launch.

## Existing icons

| File | Used by | Visual |
| --- | --- | --- |
| [open.svg](open.svg) | (reserved) | folder with up arrow |
| [save.svg](save.svg) | (reserved) | floppy disk |
| [save_as.svg](save_as.svg) | (reserved) | floppy disk with pencil |
| [import_step.svg](import_step.svg) | (reserved) | document with downward arrow into stage |
| [fit_view.svg](fit_view.svg) | (reserved) | bracket frame |

> The menu bar itself is icon-free by design. These assets are kept around
> because future surfaces (a quick-action palette, a touch-friendly mode,
> etc.) will reuse them. Workbench-specific commands (e.g. `part.new_body`)
> live next to their workbench under
> `crates/workbenches/<crate>/src/icons/<tool_id>.svg` instead.
