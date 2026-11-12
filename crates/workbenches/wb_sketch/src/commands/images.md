# Sketcher picture commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.image

Without `sketch` it goes in the sketch being edited, else in a new sketch
named after the file and placed as `sketch.new` places one (`body`, `plane`,
`on`).

It lies 100 mm wide, centred on (0, 0), at opacity 0.5, unless `width`, `x`,
`y` or `opacity` say otherwise; `angle` is degrees counter-clockwise.

The file is kept in the document. The picture only draws: no profile and no
constraint come of it.

A file that cannot be read, or is not a PNG or JPEG, is refused. The `image`
returned is what `sketch.set_image` takes.

See also: `sketch.set_image`

## sketch.set_image

`image` is the id `sketch.image` returned, and `sketch` must be the sketch
holding it, or it is refused.

Only what is given changes. `width` must be more than 0, `opacity` is held
between 0 and 1, and `angle` is degrees counter-clockwise.

See also: `sketch.image`

## sketch.align_image

See also: `sketch.turn_image`

## sketch.turn_image

See also: `sketch.align_image`

## sketch.bolt_circle

Adds the least-squares circle through the circles' centres as construction,
with its centre point. `strays` is each hole's distance from it (positive
outside), `angles` where each sits round it, degrees from the sketch's x axis.

See also: `sketch.fit_edge`

## sketch.calibrate_axis

A flatbed scanner's sensor and its carriage may not agree: calibrate along x
with a ruler laid across the glass, then along y with one laid down it. The
picture's other axis keeps its size.

See also: `sketch.calibrate_image`, `sketch.save_scanner_profile`

## sketch.save_scanner_profile

Kept in the configuration folder's scanner_profiles.json. A later picture
whose file states the same resolution takes the profile's scale when it is
placed without a width (one profile for that resolution), or
`sketch.image{profile = name}` names it.

## sketch.image_profile

The part nearest the point (as `sketch.image_objects` finds parts) is read
round its centroid: its outline's farthest radius every half degree, and the
peaks of that profile standing out by at least `prominence`: a gear's teeth, a
knurl's ridges. `pitch` is the mean angle between them, `outer` and `inner`
the profile's envelope.

See also: `sketch.image_objects`

## sketch.calibrate_image

The circle is fitted to the picture's edge round the point, as
`sketch.fit_edge` fits it, and the picture scaled about its centre: a gauge
pin or a washer of known size laid on the glass beside the part calibrates the
scan.

See also: `sketch.fit_edge`

## sketch.image_objects

The picture is split by its grey level into part and ground (the ground is
what its border mostly is), and each connected part measured in the sketch's
millimetres as the picture lies: its material's area (holes left out), its
holes, centroid, the diameter of a disc of its area, its widest and narrowest
caliper width, the direction of its long axis from the sketch's x axis in
degrees, and with `outlines` its outer boundary.

See also: `sketch.fit_edge`

## sketch.fit_edge

The picture's edges are found once, to a tenth of a pixel, at up to 4096
pixels on its longer side, and drawn over it while the sketch is open. The fit
starts from a dozen points on the edge nearest the point and runs on both ways
while the next stays within the edge's own noise of the shape, so it stops at
a corner; it reports how far its points stray (`sigma`, mm). Without `kind` it
takes the shape that runs further. A line on a curved edge is refused, and so
is a circle on less than 8° of round.

See also: `sketch.image`
