# Inspection map commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## inspect.pin

The pin reads the map shown on the scan at its nearest point, again whenever
the map changes; its label shows on the part.

## inspect.element_map

Every scan point within the element's drawn extent is measured to its ideal
shape, positive where the material faces out as the scan's normals say: is
this face flat, is this bore round. No reference or alignment is needed.
