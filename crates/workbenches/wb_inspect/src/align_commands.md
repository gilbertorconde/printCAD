# Inspection alignment by elements commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## inspect.align_datums

The primary fixes its direction and zero, the secondary turns square to it and
a plane fixes its remaining coordinate. The zero element fixes only
coordinates still free. Parallel directions and mixed scans are refused.

## inspect.align_points

The scan is moved and turned so its points land on their partners as nearly as
can be (least squares); `rms` says how nearly. Pick features both have, well
apart: corners, hole centres. A Best fit after it refines from there.

See also: `inspect.best_fit`

## inspect.auto_align

Largest flat face or planar open rim down, long side along X, bounding-box
centre on the standing face at zero. Round walls take their axis as Z;
otherwise principal axes provide a fallback.

## inspect.use_symmetry

Shortest turn of the mirror plane's normal to its nearest signed world axis,
then a shift normal to the plane through the origin. Mirror RMS above 0.5 mm
is refused.
