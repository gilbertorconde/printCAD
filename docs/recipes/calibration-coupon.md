# A calibration coupon

A test part to print and scan once per printer and material, to learn
how the printer's holes and pins come out. It's a 70 × 34 × 3 mm plate
with a row of holes Ø3 to Ø8 through it, and a row of pins of the same
sizes standing 6 mm on it. Print it, scan it, align the scan onto the
coupon's body (Inspection's Best fit), and the check below fits each
hole and pin and dimensions it against its drawn size. The table in
the report then reads how much smaller each hole and how much larger
each pin came out, size by size. `pc.inspect.compensation` reads the
whole print's shrinkage.

The test suite runs this block from an empty document, and its asserts
check what it made.

```lua
local sizes = {3, 4, 5, 6, 8}
local w, d, t, tall = 70, 34, 3, 6
local x = function(k) return 8 + (k - 1) * 13 end

local base = pc.sketch.new{plane = "XY", name = "Coupon"}
pc.sketch.rect{sketch = base, x = 0, y = 0, width = w, height = d}
local pad = pc.design.pad{sketch = base, length = t}
local coupon = pc.doc.feature{id = pad}.body

-- The holes along the front, drilled at their drawn sizes.
for k, size in ipairs(sizes) do
  local s = pc.sketch.new{body = coupon, plane = "XY", offset = t, name = "Hole " .. size}
  pc.sketch.circle{sketch = s, x = x(k), y = 9, radius = 1}
  pc.design.hole{sketch = s, diameter = size, through_all = true}
end

-- The pins along the back.
local pins = pc.sketch.new{body = coupon, plane = "XY", offset = t, name = "Pins"}
for k, size in ipairs(sizes) do
  pc.sketch.circle{sketch = pins, x = x(k), y = 25, radius = size / 2}
end
pc.design.pad{sketch = pins, length = tall}

assert(#pc.doc.rebuild() == 0, "every feature builds")
local volume = 0
for _, size in ipairs(sizes) do
  volume = volume + math.pi * (size / 2) ^ 2 * (tall - t)
end
local expected = w * d * t + volume
assert(math.abs(pc.doc.measure{body = coupon}.volume - expected) < 0.05)

-- After printing: the scan aligned onto the coupon, each hole and pin
-- fitted where it should be and dimensioned against its drawn size.
local function check(scan)
  for k, size in ipairs(sizes) do
    for _, part in ipairs({{y = 9, z = t / 2, r = -size / 2}, {y = 25, z = t + tall / 2, r = size / 2}}) do
      local fit = pc.inspect.grow{kind = "cylinder", scan = scan, at = {x(k) + math.abs(part.r), part.y, part.z}}
      pc.inspect.dimension{kind = "diameter", elements = {fit.element}, nominal = size, plus = 0.1, minus = 0.1}
    end
  end
end
local scan
for _, b in ipairs(pc.doc.bodies()) do
  if b.scan then scan = b.id end
end
if scan then check(scan) end
```
