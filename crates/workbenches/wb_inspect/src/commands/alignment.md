# Inspection alignment commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## inspect.best_fit

Rigid and point to plane, pairs past three times the median distance left out.
`free` counts the motions the fit could not pin down (a cylinder turning about
its axis): above 0 the scan may sit anywhere along them. The scan's placement
changes in one undoable step and an alignment row records how.

## inspect.move

The motion is applied to where the scan sits now.
