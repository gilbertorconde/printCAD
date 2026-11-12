# Project import commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## project.import

Each body of the file becomes one body: its features in their order, its tip
the file's. Operations outside bodies continue the body of the solid they
start from when nothing else uses it, else start one with a Clone of it.
Sketches keep their geometry and the constraints printCAD has, placed where
the file places them; a text becomes a sketch's text block. The edges a fillet
or chamfer rounds and the faces an extrusion ends on are found on the shapes
the file saved. A link becomes a linked copy.

What printCAD cannot rebuild as features (a solid kept without its history, a
kind or setting it has no counterpart for) comes in as the shape the file
saved for it, `kind` "saved_shape"; each body built from features keeps its
tip's saved shape to show while its history does not build. Attachments and
formulas are frozen to the values the file saved. Each of these is named in
the log and in `findings`. An import that keeps shapes from the file cannot be
undone, as a STEP import cannot.

A file that cannot be read, or holds nothing that can be brought in, is
refused and the document is left as it was.
