# Assembly check commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

In an example, a line `{{box}}` stands for the setup the code hands this
file under that name.

## asm.interference

Faces that only touch, a body resting on another, are no clash. `volume` is
the shared material in mm³ and `centre` its middle, in world space.

`clearance` answers the nearby pairs instead of the clashes, with the nearest
point on each body; it takes longer.

See also: `asm.motion_clashes`, `asm.mass`

### A lid sunk 1 mm into its base

```lua
{{box}}
local base = box(0, 20, 20, 5)
local lid = box(5, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.move{body = lid, by = {0, 0, 5}}
assert(#pc.asm.interference{}.clashes == 0, "resting on the top is no clash")
pc.asm.move{body = lid, by = {0, 0, -1}}
local found = pc.asm.interference{}
assert(found.checked == 2 and #found.clashes == 1)
local clash = found.clashes[1]
assert(math.abs(clash.volume - 10 * 10 * 1) < 1e-3, "1 mm of the lid sunk in")
assert(math.abs(clash.centre[3] - 4.5) < 1e-3)
```

## asm.mass

`centre` is the centre of mass in world space, mm. An id in `bodies` that is
not a body is passed over, so a list of feature ids answers a mass of 0.

`inertia` is about the centre of mass, g·mm² on the world's axes, each body's
moved there by the parallel axis theorem where it sits: `tensor`, three rows,
and `principal`, the principal moments smallest first, each {moment, axis}.

Bought parts weigh in, as the parts list counts them among the assembly's
parts; each body says whether it is `bought`, and `bought = false` leaves them
out.

See also: `asm.parts`, `asm.interference`

### A block's mass at 1.24 g/cm³

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
local block = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
assert(#pc.doc.rebuild() == 0)
local m = pc.asm.mass{density = 1.24}
assert(math.abs(m.volume - 2000) < 1e-3)
assert(math.abs(m.mass - 2.48) < 1e-6, "2 cm³ at 1.24 g/cm³")
assert(m.centre[1] == 10 and m.centre[3] == 2.5)
-- About its centre, g·mm²: m (20² + 20²) / 12 about Z.
assert(math.abs(m.inertia.tensor[3][3] - 2.48 * 800 / 12) < 1e-6)
```
