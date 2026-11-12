# Design measuring commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## design.centre_line

The body must be built first (`pc.doc.rebuild()`). The two faces are its ends,
each given by a point on it and its outward normal.

See also: `doc.faces`, `doc.measure`

### The centre line of a quarter bend

```lua
local s = pc.sketch.new{plane = "XZ"}
pc.sketch.circle{sketch = s, x = 10, y = 0, radius = 2}
local bend = pc.design.revolve{sketch = s, angle_deg = 90}
assert(#pc.doc.rebuild() == 0, "the bend builds")
local body = pc.doc.feature{id = bend}.body
local line = pc.design.centre_line{body = body,
  from_point = {10, 0, 0}, from_normal = {0, -1, 0},
  to_point = {0, 10, 0}, to_normal = {-1, 0, 0}}
-- A quarter circle of radius 10.
assert(math.abs(line.length - math.pi * 10 / 2) < 0.05, line.length)
assert(not line.straight)
```
