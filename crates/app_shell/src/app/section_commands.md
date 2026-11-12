# Section view commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## doc.save_section, view.section

The plane is `axis` ("x", "y" or "z", `offset` the coordinate along it), or
`origin` and `normal` (`offset` moving it along the normal from there). It
keeps the side the normal points to; `flipped` keeps the other. `uncut` lists
bodies it leaves whole. `wedge_normal` (with `wedge_origin` and
`wedge_flipped`) adds a second plane: only what both would cut away is cut,
opening a quarter of the part.

## doc.sections

`origin` is the point nearest the document's origin of the plane as it was
aimed, `normal` the way it faces, `offset` how far it stands along it from
there; `uncut` the ids of the bodies it leaves whole.

See also: `doc.save_section`, `view.section`

## doc.save_section

Given no plane, it keeps the one the view cuts at (a script run without a
window has none and refuses). Saving changes no feature and no geometry, but
it is an edit: it is saved with the file and undone as one step.

See also: `doc.sections`, `doc.remove_section`

### Sections kept, one replaced, one a wedge

```lua
pc.doc.save_section{name = "Mid", axis = "z", offset = 5}
pc.doc.save_section{name = "Bore", origin = {0, 18, 13}, normal = {0, 1, 0}}
pc.doc.save_section{name = "Mid", axis = "z", offset = 7, flipped = true}
pc.doc.save_section{name = "Quarter", axis = "z", offset = 7,
  wedge_normal = {1, 0, 0}, wedge_origin = {20, 0, 0}}
local kept = pc.doc.sections()
assert(#kept == 3 and kept[1].name == "Mid" and kept[1].offset == 7)
assert(kept[1].flipped and kept[2].origin[2] == 18 and kept[2].wedge == nil)
assert(kept[3].wedge.origin[1] == 20)
```

## doc.remove_section

A name no section has is refused.

See also: `doc.sections`

## view.section

With no argument it answers the plane and changes nothing. The plane belongs
to the view: setting it is no edit, and nothing of it is saved unless
`doc.save_section` keeps it.

It needs the app's window: a `printcad --script` run refuses it.

See also: `doc.save_section`
