# Archived-project import

- `project_archive`: a standalone reader of `.FCStd` archives (zip and
  `Document.xml` through `roxmltree`, the only XML parser: byte cap,
  allocation-hint count, a linear nesting scan before its recursive
  parse, `nodes_limit`), their objects, placements and sketch geometry,
  and the topology of a saved `.brp` shape (`saved_shape`: edges and
  faces numbered as features name them, with where each lies, read
  without the kernel); its `tests/data` holds the generated fixtures.
  `project_tools` turns one into sketches, Design features and bodies
  (`convert_into`, through the document's own mutators; each solid step
  a body starting with a Clone of the step before, the edges and faces a
  feature names found on the saved shapes and made picks, what cannot be
  rebuilt left out with what stands on it and listed as findings), and
  is registered by the
  `workbenches` facade as `ProjectImportBench`: no tools and no feature
  kinds, not in `REGISTERED_WORKBENCHES` (so no switcher shows it), only
  File › Import's FreeCAD file and `project.import`, which converts into
  a scratch document first so a refusal leaves the open one untouched.
  Its `project-tool` program (`inspect`, `convert`, `check`, which also
  holds each rebuilt solid to the file's saved shape by bounds and face
  count) is a developer tool, never published.
