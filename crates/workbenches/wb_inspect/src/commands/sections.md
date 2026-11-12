# Inspection section commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## inspect.section

The plane is kept in the scan's frame as it is when made: aligning the scan
carries it, refitting the element does not move it. Chains shorter than 1 mm
are dropped; area is the material inside the closed chains.

## inspect.shadow

The outline is lines, arcs and circles, every point of it between play and
play + 2 × tolerance outside the shadow; holes through the part stay holes. It
lives on a body of its own whose solid is the block, and follows the scan when
the scan is moved or its settings change.
