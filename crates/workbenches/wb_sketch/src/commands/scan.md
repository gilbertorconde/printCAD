# Sketcher scan commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.candidates

Each unsized line's length and arc's or circle's radius comes with its
probable design value: halves under 10 mm, whole millimetres above, radii to
0.5 mm. Relations are horizontal, vertical, parallel, perpendicular, equal and
concentric, each within 2° or 1 % (at least 0.05 mm) and not yet constrained;
`offset` is how far from keeping it, in degrees or mm. Nothing is applied:
`sketch.design_values` holds sizes, `sketch.constrain` adds a relation.

See also: `sketch.design_values`, `sketch.fit_scan`

## sketch.design_values

See also: `sketch.candidates`

## sketch.fit_scan

Each chain of the section becomes lines and arcs end to end, closed where the
slice closes, ends that meet sharing one point, so a closed slice is a
profile. Chains shorter than 1 mm are left out. The scan's section shows as a
dashed guide while the sketch is open.

See also: `sketch.intersection`
