# Surface step editing commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## surface.set

It takes the fields the step's command takes, by the same names; `sketches`
replaces the curves it is built from. A name the step does not have is refused
("is not a field of this surface"); the kind of step stays.

The step keeps its place in the body's history: everything after it builds
again on the change at `pc.doc.rebuild()`.

### A wall made taller

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
local wall = pc.surface.extrude{sketches = {s}, length = 5}
local body = pc.doc.feature{id = wall}.body
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.area - 50) < 1e-6)
pc.surface.set{feature = wall, length = 8}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.area - 80) < 1e-6)
```
