# Assembly component commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

In an example, a line `{{cube}}` stands for the setup this file defines
below under that name.

## {{cube}}

A 5 mm cube with its corner at `x`, its body returned.

```lua
local function box(x)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
end
```

## asm.group

The bodies hold to the first as they sit now, which leaves the others no
motion. Fewer than two bodies are refused.

`asm.move` and `asm.place` move one member only; the rest follow at the next
solve. The bodies of a rigid `asm.component` move together at once.

See also: `asm.component`, `asm.fix`

### Two blocks locked together

```lua
{{cube}}
local a, b = box(0), box(10)
assert(#pc.doc.rebuild() == 0)
pc.asm.group{bodies = {a, b}}
assert(pc.asm.freedom{body = b}[1].free == 0, "b holds to a")
pc.asm.move{body = a, by = {0, 10, 0}}
assert(pc.asm.placement{body = b}.translation[2] == 0, "a move places one body")
pc.asm.solve{}
local at = pc.asm.placement{body = b}.translation
assert(math.abs(at[2] - 10) < 1e-4, "the solve brings b along")
```

## asm.component

Rigid, the default: `asm.move` or `asm.place` of any body in it moves every
body in it and in the components nested in it. Flexible: each body moves
alone.

A body is taken out of any component it was in. Components are not features:
`pc.doc.features` does not list them.

See also: `asm.component_set`, `asm.component_add`, `asm.group`

### Two blocks that move as one

```lua
{{cube}}
local a, b, c = box(0), box(10), box(20)
assert(#pc.doc.rebuild() == 0)
pc.asm.component{bodies = {a, b}, name = "Pair"}
pc.asm.move{body = a, by = {0, 7, 0}}
assert(pc.asm.placement{body = b}.translation[2] == 7, "b moves with a")
assert(pc.asm.placement{body = c}.translation[2] == 0, "c is not in it")
```

## asm.component_set

Only what is given changes. Made rigid again, the bodies move together from
where they sit then.

Lua has no null in a table, so `parent = "top"` takes a component out of the
one it is in, to the top level.

A component cannot be put inside itself; an unknown component is refused.

See also: `asm.component`, `asm.component_add`

### A component renamed, made flexible and rigid again

```lua
{{cube}}
local a, b = box(0), box(10)
assert(#pc.doc.rebuild() == 0)
local k = pc.asm.component{bodies = {a, b}}
pc.asm.component_set{component = k, name = "Loose pair", flexible = true}
pc.asm.move{body = a, by = {0, 7, 0}}
assert(pc.asm.placement{body = b}.translation[2] == 0, "flexible: a moves alone")
pc.asm.component_set{component = k, flexible = false}
pc.asm.move{body = a, by = {0, 1, 0}}
assert(pc.asm.placement{body = b}.translation[2] == 1, "rigid: they move as one")
```

### A component put in another and taken out to the top level

```lua
{{cube}}
local a, b = box(0), box(10)
assert(#pc.doc.rebuild() == 0)
local outer = pc.asm.component{bodies = {a}, name = "Frame"}
local inner = pc.asm.component{bodies = {b}, name = "Arm"}
pc.asm.component_set{component = inner, parent = outer}
pc.asm.move{body = a, by = {0, 4, 0}}
assert(pc.asm.placement{body = b}.translation[2] == 4, "inside the frame, it follows")
pc.asm.component_set{component = inner, parent = "top"}
pc.asm.move{body = a, by = {0, 4, 0}}
assert(pc.asm.placement{body = b}.translation[2] == 4, "at the top, it stays")
```

## asm.component_add

A body is in one component at a time: adding it takes it out of the one it was
in. Nothing moves.

See also: `asm.component`, `asm.component_remove`

### A body put in a component and taken out again

```lua
{{cube}}
local a, b, c = box(0), box(10), box(20)
assert(#pc.doc.rebuild() == 0)
local k = pc.asm.component{bodies = {a, b}}
pc.asm.component_add{bodies = {c}, component = k}
pc.asm.move{body = a, by = {0, 0, 3}}
assert(pc.asm.placement{body = c}.translation[3] == 3, "c moves with them")
pc.asm.component_add{bodies = {c}}
pc.asm.move{body = a, by = {0, 0, 3}}
assert(pc.asm.placement{body = c}.translation[3] == 3, "taken out, it stays")
```

## asm.component_remove

Only the component goes: its bodies and nested components move one level up,
and nothing moves in the model.

See also: `asm.component`

### A component taken apart

```lua
{{cube}}
local a, b = box(0), box(10)
assert(#pc.doc.rebuild() == 0)
local k = pc.asm.component{bodies = {a, b}}
pc.asm.component_remove{component = k}
pc.asm.move{body = a, by = {0, 0, 3}}
assert(pc.asm.placement{body = b}.translation[3] == 0, "apart, a moves alone")
local renamed = pcall(pc.asm.component_set, {component = k, name = "Gone"})
assert(not renamed, "the component is gone")
```
