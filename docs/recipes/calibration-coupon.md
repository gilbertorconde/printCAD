# A calibration coupon

A test part to print and scan once per printer and material, to learn
how the printer comes out: `pc.doc.coupon` (or the start page's
Calibration coupon) makes a body of its own with a 3 mm plate, a row of
holes Ø3 to Ø8 through it, a row of pins of the same sizes standing 6 mm
on it, a ladder of slots 0.3 to 0.8 mm wide, and along its back edge a
fan of teeth leaning out 30° to 70°. Print it, scan it, align the scan
onto the coupon's body (Inspection's Best fit), and the check below fits
each hole and pin and dimensions it against its drawn size, then
`pc.inspect.compensation` reads the whole print: each axis's shrinkage,
the holes' and pins' radius error, the first layer's flare and the
bottom's warp. Which slots printed open and which teeth sagged are read
off the print itself.

The test suite runs this block from an empty document, and its asserts
check what it made.

```lua
local c = pc.doc.coupon{}
assert(#pc.doc.rebuild() == 0, "every feature builds")
assert(#c.holes == 5 and #c.pins == 5 and #c.gaps == 5 and #c.overhangs == 4)
local m = pc.doc.measure{body = c.body}
assert(math.abs(m.max[3] - 9) < 1e-6, "the pins stand 6 mm on the 3 mm plate")

-- After printing: the scan aligned onto the coupon, each hole and pin
-- fitted where it should be and dimensioned against its drawn size.
local function check(scan)
  for _, list in ipairs({c.holes, c.pins}) do
    for _, round in ipairs(list) do
      local r = round.diameter / 2
      local at = {round.at.x + r, round.at.y, round.at.z}
      local fit = pc.inspect.grow{kind = "cylinder", scan = scan, at = at}
      pc.inspect.dimension{kind = "diameter", elements = {fit.element},
        nominal = round.diameter, plus = 0.1, minus = 0.1}
    end
  end
  return pc.inspect.compensation{scan = scan, reference = c.body}
end
local scan
for _, b in ipairs(pc.doc.bodies()) do
  if b.scan then scan = b.id end
end
if scan then check(scan) end
```
