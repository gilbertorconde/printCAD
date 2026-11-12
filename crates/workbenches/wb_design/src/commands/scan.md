# Design scan commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## design.read_hole

A cylinder is grown over the scan from `at`: `diameter`, `depth` (how far its
points run along the axis), `hole` (true when the wall faces its axis, false
for a boss or pin), `sigma` its fit's RMS, `start` and `axis` where it begins
and which way it runs. `design` is the diameter rounded to the value it was
likely drawn at, and `nearest` the three nearest standard sizes, each
{standard, size, as, diameter, off}: for a hole the tap drills and clearance
holes of every thread standard, for a boss the threads' major diameters.

`sweep` is how far round its axis the round's points reach, in degrees. Under
300° it is a fillet: `fillet` is then {radius, design, concave}, the radius it
was likely drawn at in `design`, `concave` for an inside round (the wall faces
its axis), and `nearest` is empty.

See also: `design.hole`, `design.fillet`

### A clearance hole scanned and read as an M6's

```lua
local plate = pc.doc.new_body{}
pc.design.primitive{body = plate, kind = {Box = {length = 30, width = 20, height = 5}}}
pc.design.subtractive_primitive{body = plate, variant = "cylinder",
  kind = {Cylinder = {radius = 3.3, height = 7, angle_deg = 360}},
  placement = {origin = {12, 10, -1}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {plate}, tolerance = 0.002}
pc.doc.set_visible{id = plate, visible = false}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local r = pc.design.read_hole{scan = scan, at = {15.3, 10, 2.5}}
assert(r.hole and math.abs(r.diameter - 6.6) < 0.01 and math.abs(r.depth - 5) < 0.01, r.diameter)
assert(r.nearest[1].size == "M6" and r.nearest[1].as == "normal clearance", r.nearest[1].size)
assert(r.fillet == nil and r.sweep > 300, r.sweep)
```

### A rounded edge scanned and read as an R2 fillet

```lua
local block = pc.doc.new_body{}
pc.design.primitive{body = block}
pc.design.fillet{body = block, radius = 2,
  edges = {Edges = {{point = {5, 10, 10}, direction = {1, 0, 0}}}}}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {block}, tolerance = 0.002}
pc.doc.set_visible{id = block, visible = false}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local h = 2 * math.sqrt(0.5)
local r = pc.design.read_hole{scan = scan, at = {5, 8 + h, 8 + h}}
assert(r.fillet and math.abs(r.fillet.radius - 2) < 0.01 and r.fillet.design == 2, r.sweep)
assert(not r.fillet.concave and math.abs(r.sweep - 90) < 2 and #r.nearest == 0, r.sweep)
```

## design.snap_to_scan

A plane or cylinder, as the face is, is grown over the scan from `at` and
fitted. A flat face is moved along its normal onto the fitted plane by a Move
Faces feature (`moved`, mm, and `angle`, how far the scan's plane is turned
from the face); a round face is offset to the fitted radius by an Offset Faces
feature (`design` is the radius it was likely drawn at, `axis_offset` how far
the two axes lie apart). A scan surface turned more than 20° from the face is
refused, and the face is never turned: a draft or a tilt is a feature of its
own.

See also: `design.move_faces`, `design.offset_faces`

### A block's top snapped 0.4 mm up and its bore widened to the scan's

```lua
local function block(height, radius)
  local b = pc.doc.new_body{}
  pc.design.primitive{body = b, kind = {Box = {length = 20, width = 10, height = height}}}
  pc.design.subtractive_primitive{body = b, variant = "cylinder",
    kind = {Cylinder = {radius = radius, height = 20, angle_deg = 360}},
    placement = {origin = {10, 5, -5}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
  return b
end
local printed = block(5.4, 3.2)
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {printed}, tolerance = 0.002}
pc.doc.set_visible{id = printed, visible = false}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local model = block(5, 3)
assert(#pc.doc.rebuild() == 0)
local top = pc.design.snap_to_scan{body = model, face_point = {2, 2, 5},
  face_normal = {0, 0, 1}, scan = scan, at = {2, 2, 5.4}}
assert(math.abs(top.moved - 0.4) < 1e-3 and top.angle < 0.01, top.moved)
local bore = pc.design.snap_to_scan{body = model, face_point = {13, 5, 2},
  face_normal = {-1, 0, 0}, scan = scan, at = {13.2, 5, 2.5}}
assert(math.abs(bore.fitted - 3.2) < 0.01 and bore.design == 3, bore.fitted)
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = model}
local want = 20 * 10 * 5.4 - math.pi * 3.2 ^ 2 * 5.4
assert(math.abs(m.volume - want) < 0.5, m.volume)
```

## design.scan_symmetry

The kernel searches for the plane that mirrors the scan onto itself best; it
is refused when its RMS residual is over `accept`. The datum plane is named
Scan symmetry, and Mirror then mirrors about it by default in that body (a
face selected still wins), so half a part modelled from a symmetric scan is
mirrored whole.

See also: `design.mirror`, `inspect.use_symmetry`

### A scan of a block with a blind hole on its middle line, symmetric about x = 10 alone

```lua
local block = pc.doc.new_body{}
pc.design.primitive{body = block, kind = {Box = {length = 20, width = 12, height = 5}}}
pc.design.subtractive_primitive{body = block, variant = "cylinder",
  kind = {Cylinder = {radius = 2, height = 7, angle_deg = 360}},
  placement = {origin = {10, 3, 2}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {block}, tolerance = 0.002}
pc.doc.set_visible{id = block, visible = false}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local model = pc.doc.new_body{}
local s = pc.design.scan_symmetry{scan = scan, body = model}
assert(s.rms < 0.01 and math.abs(math.abs(s.normal.x) - 1) < 1e-3, s.rms)
assert(math.abs(s.point.x - 10) < 0.01, s.point.x)
-- Half the block modelled, then mirrored about the scan's plane.
pc.design.primitive{body = model, kind = {Box = {length = 10, width = 12, height = 5}}}
pc.design.mirror{body = model}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = model}
assert(math.abs(m.min[1]) < 1e-6 and math.abs(m.max[1] - 20) < 0.01, m.max[1])
```

## design.read_thread

The thread's axis comes from a cylinder grown over its crests and flanks from
`at`; every scan point within a third of its radius of it, along the grown
stretch, is then read for the helix it follows: the pitch (tried every 0.02 mm
from 0.2 to 6.5, then to the thousandth about the best) and hand whose turns
the points' radii follow most strongly. `clarity` is that helix's share of the
radii's spread, near 1 on a clean scan of a thread and low where there is
none. `major` and `minor` are twice the radius the outermost and innermost 3 %
of points reach, `internal` true in a hole, and `nearest` the three standard
threads nearest by diameter and pitch, each {standard, size, major, pitch}.

See also: `design.read_hole`, `design.hole`

### A scanned M8 thread read back: a synthetic scan of a bolt's thread, 1.25 pitch, right-handed, written as an STL and imported

```lua
local path = os.tmpname() .. ".stl"
local f = io.open(path, "w")
f:write("solid bolt\n")
local pitch, turns, around, along = 1.25, 5, 48, 120
local function at(i, j)
  local a = 2 * math.pi * i / around
  local t = turns * pitch * j / along
  local phase = (t / pitch - a / (2 * math.pi)) % 1
  local r = 4 - 0.68 * math.abs(2 * phase - 1)
  return {r * math.cos(a), r * math.sin(a), t}
end
local function facet(p, q, s)
  f:write("facet normal 0 0 0\nouter loop\n")
  for _, v in ipairs({p, q, s}) do
    f:write(string.format("vertex %.6f %.6f %.6f\n", v[1], v[2], v[3]))
  end
  f:write("endloop\nendfacet\n")
end
for j = 0, along - 1 do
  for i = 0, around - 1 do
    local a, b = at(i, j), at(i + 1, j)
    local c, d = at(i + 1, j + 1), at(i, j + 1)
    facet(a, b, c)
    facet(a, c, d)
  end
end
f:write("endsolid bolt\n")
f:close()
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local p = at(10, 60)
local t = pc.design.read_thread{scan = scan, at = {p[1], p[2], p[3]}}
assert(math.abs(t.pitch - 1.25) < 0.01 and t.right_handed and not t.internal, t.pitch)
assert(t.nearest[1].size == "M8", t.nearest[1].size)
```

## design.scan_deviation

Each scan point's signed distance to the body's solid, where both sit:
`within` is the share within ±range, `inside` the share deeper inside the body
(material the scan does not have: a hole or pocket still to cut), `outside`
the share farther out (the scan's other parts, or material still to add), and
`rms` the RMS distance of those within. The Deviation from scan tool paints
the scans so while modelling, against the body being edited or the one
selected.

See also: `inspect.compare`

### A scan of a 10 mm cube against a body 2 mm short of it

```lua
local cube = pc.doc.new_body{}
pc.design.primitive{body = cube}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {cube}}
pc.doc.set_visible{id = cube, visible = false}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local short = pc.doc.new_body{}
pc.design.primitive{body = short, kind = {Box = {length = 10, width = 10, height = 8}}}
assert(#pc.doc.rebuild() == 0)
local d = pc.design.scan_deviation{body = short, scan = scan}
assert(d.within == 0.5 and d.outside == 0.5 and d.inside == 0, d.within)
```
