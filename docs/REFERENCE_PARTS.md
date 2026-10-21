# Reference parts

Parts whose volume and area are known from a formula, built the way a
user builds them, measured, and held to that formula. They say how close
the kernel's solids come to the shapes they stand for, and they catch a
change that moves a result.

The rules:

- **The expected value comes from the formula**, written out in the
  block with its source, never from what the program printed. A case
  whose result moves is a finding to explain, never a number to update.
- **Each case states its tolerance**: relative, 1e-9 of the value, unless
  it says otherwise and why.
- **A known failure** is a case the program gets wrong today. Its block's
  fence reads ```` ```lua known-failure: <why> ```` and names the issue.
  The suite runs it and expects it to fail; once it passes, the suite
  fails until the mark is taken off.

The test suite runs every block below from an empty document
(`every_command_example_and_recipe_runs`).

## Primitives

A box, length × width × height: V = lwh, A = 2(lw + lh + wh).

```lua
local function near(got, want, what)
  assert(math.abs(got - want) <= 1e-9 * math.abs(want), what .. ": " .. got .. ", not " .. want)
end
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = 20, width = 30, height = 40}}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
near(m.volume, 20 * 30 * 40, "volume")
near(m.area, 2 * (20 * 30 + 20 * 40 + 30 * 40), "area")
```

A cylinder of radius r and height h: V = πr²h, A = 2πr(r + h).

```lua
local function near(got, want, what)
  assert(math.abs(got - want) <= 1e-9 * math.abs(want), what .. ": " .. got .. ", not " .. want)
end
local r, h = 5, 12
local body = pc.doc.new_body{}
pc.design.primitive{body = body, variant = "cylinder",
  kind = {Cylinder = {radius = r, height = h, angle_deg = 360}}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
near(m.volume, math.pi * r * r * h, "volume")
near(m.area, 2 * math.pi * r * (r + h), "area")
```

A sphere of radius r: V = 4/3 πr³, A = 4πr².

```lua
local function near(got, want, what)
  assert(math.abs(got - want) <= 1e-9 * math.abs(want), what .. ": " .. got .. ", not " .. want)
end
local r = 7
local body = pc.doc.new_body{}
pc.design.primitive{body = body, variant = "sphere",
  kind = {Sphere = {radius = r, angle1_deg = -90, angle2_deg = 90, angle3_deg = 360}}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
near(m.volume, 4 / 3 * math.pi * r ^ 3, "volume")
near(m.area, 4 * math.pi * r * r, "area")
```

A cone frustum, radii R and r, height h: V = πh(R² + Rr + r²)/3,
A = π(R² + r²) + π(R + r)√((R − r)² + h²).

```lua
local function near(got, want, what)
  assert(math.abs(got - want) <= 1e-9 * math.abs(want), what .. ": " .. got .. ", not " .. want)
end
local R, r, h = 6, 2, 9
local body = pc.doc.new_body{}
pc.design.primitive{body = body, variant = "cone",
  kind = {Cone = {radius1 = R, radius2 = r, height = h, angle_deg = 360}}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
near(m.volume, math.pi * h * (R * R + R * r + r * r) / 3, "volume")
near(m.area, math.pi * (R * R + r * r) + math.pi * (R + r) * math.sqrt((R - r) ^ 2 + h * h), "area")
```

A torus, ring radius R and tube radius r (Pappus): V = 2π²Rr², A = 4π²Rr.

```lua
local function near(got, want, what)
  assert(math.abs(got - want) <= 1e-9 * math.abs(want), what .. ": " .. got .. ", not " .. want)
end
local R, r = 10, 3
local body = pc.doc.new_body{}
pc.design.primitive{body = body, variant = "torus", kind = {Torus = {radius1 = R, radius2 = r,
  angle1_deg = -180, angle2_deg = 180, angle3_deg = 360}}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
near(m.volume, 2 * math.pi ^ 2 * R * r * r, "volume")
near(m.area, 4 * math.pi ^ 2 * R * r, "area")
```

## Features

A plate s × s × t with a hole of diameter d through it:
V = s²t − π(d/2)²t, A = 2(s² − π(d/2)²) + 4st + πdt.

```lua
local function near(got, want, what)
  assert(math.abs(got - want) <= 1e-9 * math.abs(want), what .. ": " .. got .. ", not " .. want)
end
local s, t, d = 40, 5, 6
local base = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = base, x = 0, y = 0, width = s, height = s}
local body = pc.doc.feature{id = pc.design.pad{sketch = base, length = t}}.body
local drill = pc.sketch.new{body = body, plane = "XY", offset = t}
pc.sketch.circle{sketch = drill, x = s / 2, y = s / 2, radius = 1}
pc.design.hole{sketch = drill, diameter = d, through_all = true}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
local hole = math.pi * (d / 2) ^ 2
near(m.volume, s * s * t - hole * t, "volume")
near(m.area, 2 * (s * s - hole) + 4 * s * t + math.pi * d * t, "area")
```

A washer turned from its section, radii a < b, height h:
V = π(b² − a²)h, A = 2π(b² − a²) + 2π(a + b)h.

```lua
local function near(got, want, what)
  assert(math.abs(got - want) <= 1e-9 * math.abs(want), what .. ": " .. got .. ", not " .. want)
end
local a, b, h = 4, 10, 3
local section = pc.sketch.new{plane = "XZ"}
pc.sketch.rect{sketch = section, x = a, y = 0, width = b - a, height = h}
local body = pc.doc.feature{id = pc.design.revolve{sketch = section}}.body
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
near(m.volume, math.pi * (b * b - a * a) * h, "volume")
near(m.area, 2 * math.pi * (b * b - a * a) + 2 * math.pi * (a + b) * h, "area")
```

A cube of side s with one edge rounded to radius r: the round takes away
the corner square less its quarter circle along the edge,
V = s³ − (1 − π/4)r²s, and swaps the two strips of width r and the edge
for a quarter cylinder, A = 6s² − 2rs − 2(1 − π/4)r² + (π/2)rs.

```lua
local function near(got, want, what)
  assert(math.abs(got - want) <= 1e-9 * math.abs(want), what .. ": " .. got .. ", not " .. want)
end
local s, r = 20, 3
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = s, width = s, height = s}}}
pc.design.fillet{body = body, radius = r, edges = {Edges = {
  {point = {s, s, s / 2}, direction = {0, 0, 1}}}}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
near(m.volume, s ^ 3 - (1 - math.pi / 4) * r * r * s, "volume")
near(m.area, 6 * s * s - 2 * r * s - 2 * (1 - math.pi / 4) * r * r + math.pi / 2 * r * s, "area")
```

A cube of side s with one edge chamfered by c: V = s³ − c²s/2, and the
two strips of width c give way to a band c√2 wide,
A = 6s² − 2cs − c² + c√2·s.

```lua
local function near(got, want, what)
  assert(math.abs(got - want) <= 1e-9 * math.abs(want), what .. ": " .. got .. ", not " .. want)
end
local s, c = 20, 2
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = s, width = s, height = s}}}
pc.design.chamfer{body = body, size = c, edges = {Edges = {
  {point = {s, s, s / 2}, direction = {0, 0, 1}}}}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
near(m.volume, s ^ 3 - c * c * s / 2, "volume")
near(m.area, 6 * s * s - 2 * c * s - c * c + c * math.sqrt(2) * s, "area")
```

An open box: a block l × w × h shelled inward by t, its top taken away:
V = lwh − (l − 2t)(w − 2t)(h − t).

```lua
local function near(got, want, what)
  assert(math.abs(got - want) <= 1e-9 * math.abs(want), what .. ": " .. got .. ", not " .. want)
end
local l, w, h, t = 30, 20, 10, 1
local base = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = base, x = 0, y = 0, width = l, height = w}
local body = pc.doc.feature{id = pc.design.pad{sketch = base, length = h}}.body
pc.design.thickness{body = body, value = t,
  face_point = {l / 2, w / 2, h}, face_normal = {0, 0, 1}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
near(m.volume, l * w * h - (l - 2 * t) * (w - 2 * t) * (h - t), "volume")
```
