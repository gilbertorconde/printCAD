# Inspection tolerance commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## inspect.tolerance

A form tolerance reads the element's own band: a plane's flatness, a line's
straightness, a circle's circularity, a cylinder's cylindricity, a sphere's
sphericity. An orientation tolerance reads a plane at its measured points (a
constructed plane at its drawn corners, said in `note`), an axis over its
fitted length and a centre as a point, against the datum's attitude. Form uses
the fitted minimum zone when available. Position reads axis ends or a centre
in a cylindrical zone (diameter), a plane in two parallel planes. The primary
datum sets Z, the next transverse datum X. Basic offsets and nominal faces use
that frame. Profiles with datums hold the scan in this frame against a
reference in matching coordinates; without datums they retain the current
best-fit placement. The profile limit is the full zone width. Circular runout
reads sampled station bands; total reads the whole region, removing a cone's
nominal slope. Position bonus is the departure of the fitted size from
material_size; use an inscribed fit for holes or circumscribed for bosses.
Datum shifts are reported, never applied.

### Datum position, profiles, runout and a median plane

```lua
local model = pc.doc.new_body{}
pc.design.primitive{body = model, variant = "cylinder",
  kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {model}}
pc.doc.set_visible{id = model, visible = false}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local axis = pc.inspect.grow{kind = "cylinder", scan = scan, at = {5,0,5}, method = "circumscribed"}.element
local bottom = pc.inspect.grow{kind = "plane", scan = scan, at = {0,0,0}}.element
local top = pc.inspect.grow{kind = "plane", scan = scan, at = {0,0,10}}.element
local middle = pc.inspect.construct{made = "plane", from = {bottom,top}}.element
local comparison = pc.inspect.compare{scan = scan, reference = model}.comparison
local section = pc.inspect.section{scan = scan}.section
local position = pc.inspect.tolerance{characteristic = "position", element = axis,
  datums = {{element = bottom}}, basic = {0,0,0}, limit = 0.05,
  material = "mmc", material_size = 10.2}
assert(position.pass and position.value < 0.01 and position.bonus > 0.19)
for _, kind in ipairs({"surface_profile", "line_profile"}) do
  local reading = pc.inspect.tolerance{characteristic = kind, element = axis,
    comparison = comparison, section = section, limit = 0.1, unequal = 0.01}
  assert(reading.pass and reading.value < 0.1)
end
for _, kind in ipairs({"circular_runout", "total_runout"}) do
  local reading = pc.inspect.tolerance{characteristic = kind, element = axis,
    datum = axis, limit = 0.05, section_width = 0.1}
  assert(reading.pass and reading.value < 0.05)
end
local symmetry = pc.inspect.tolerance{characteristic = "symmetry", element = bottom,
  opposed = top, datum = middle, limit = 0.05}
assert(symmetry.pass and symmetry.value < 0.01)
assert(#pc.inspect.report{}.tolerances == 6)
```
