# Sketcher generator commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.generator

Called with only `sketch` it changes nothing and answers the sizes: a gear's
base, pitch, root and tip diameters, a sprocket's pitch, root and tip, a
shaft's length.

A field the generator lacks is refused, naming the ones it has. The sketch's
curves are made again from the numbers, so anything drawn in it by hand goes.

`detach = true` keeps the curves and drops the numbers for good. Each number
is also a parameter by its name (`module`, `teeth`), which `doc.set_formula`
binds to a formula.

See also: `design.gear`, `design.sprocket`, `design.shaft`, `doc.set_formula`

### A gear's teeth changed, and the tip diameter with them

```lua
local gear = pc.design.gear{module = 1.5, teeth = 20}
assert(pc.sketch.generator{sketch = gear}.tip_diameter == 1.5 * 22)
local size = pc.sketch.generator{sketch = gear, teeth = 30}
assert(size.pitch_diameter == 1.5 * 30 and size.tip_diameter == 1.5 * 32)
pc.sketch.generator{sketch = gear, detach = true}
assert(pc.doc.feature{id = gear}.fields.generator == nil, "a plain sketch")
```
