# Guides

How to use printCAD, script it and extend it. Each guide is the same file
the repository keeps beside the code, so it describes the version on
master; the [release notes](https://github.com/gilbertorconde/printCAD/blob/master/crates/app_shell/RELEASE_NOTES.md)
say what each release brought.

## Modelling

- [Editing workflow](EDITING.md): sketches, features, the feature tree and its history.
- [Sketcher](SKETCHER.md): drawing, constraints, regions, pictures, text and projected geometry.
- [Surfaces](SURFACES.md): sheets built from curves, joined into solids.
- [Assembly](ASSEMBLY.md): joints, motion, interference and the parts list.
- [Holes](HOLES.md): thread standards, seats and modeled threads.
- [Variables and formulas](VARIABLES.md): numbers driven by formulas, configuration tables.
- [Surface textures](TEXTURES.md): patterns pressed into faces for printing.
- [Measure](MEASURE.md): distances, angles, sizes and a body's mass and inertia.
- [Camera](CAMERA.md): moving around the model, views and 6-DoF mice.

## In a browser

- [printCAD in a browser](BROWSER.md): what the browser app keeps, runs and leaves to the desktop.

## Printing

- [Printing](PRINTING.md): the print layout, filament counts and nut traps.

## Automating

- [Scripting](SCRIPTING.md): Lua, the console, script files and every command.
- [AI agents](AI.md): an assistant that models beside you.

## Recipes {#recipes}

Whole parts as scripts, each one run by the test suite.

- [A plate with a pocket and a hole](recipes/plate-pocket-hole.md)
- [An angle bracket with holes and a fillet](recipes/bracket-fillet-holes.md)
- [A revolved bushing](recipes/revolved-bushing.md)
- [A flange with a bolt circle](recipes/flange-bolt-circle.md)
- [A plate sized by variables and a configuration table](recipes/variables-configurations.md)
- [A hinged arm, swung through a motion](recipes/hinged-arm.md)
- [A solid sewn from surfaces](recipes/surfaces-sewn-solid.md)
- [A shade from a revolved line, cut and thickened](recipes/surfaces-trimmed-shade.md)

## Extending

- [Workbench packages](PLUGINS.md): installing packages and writing your own.
- [Writing a workbench](WORKBENCH_GUIDE.md): how a workbench meets the application.

## Inside printCAD

- [Architecture](ARCHITECTURE.md)
- [Document model](DOCUMENT_MODEL.md)
- [Roadmap](ROADMAP.md)
