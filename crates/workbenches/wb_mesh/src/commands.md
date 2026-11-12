# Mesh clean-up commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## mesh.delete

Without a box or a ball it deletes the marked part of the scan. A face goes
when all three of its corners are in the region.

## mesh.flip

Without a region it turns the marked faces over, or every face when nothing is
marked.

## mesh.deviation

Each point of the mesh the scan draws now is measured to its mesh as imported:
`max` and `rms` in mm, `p99` the 99th percentile, and `added` the share of
points on surface an edit added (caps, bridges: a triangle whose middle lies
off the start). The tool shows it as a map, added surface grey.

## mesh.inspect

Of the mesh the body draws now, its history applied. A closed mesh has no open
edges; its volume is positive when its faces point out.
