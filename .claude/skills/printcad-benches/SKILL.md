---
name: printcad-benches
description: "The built-in modelling benches other than the sketcher: Design (wb_design: Pad, Pocket, Hole, Fillet, patterns, Boolean...), Surface (wb_surface), Assembly (wb_assembly: joints, parts list), Mesh (wb_mesh, plus surface_texture), Inspection (wb_inspect, plus metrology), and archived-project import (project_archive, project_tools, project-tool). Use when working in any of crates/workbenches/wb_design, wb_surface, wb_assembly, wb_mesh, wb_inspect or crates/metrology, crates/surface_texture, crates/project_archive, crates/project_tools. Not for the sketcher (use printcad-sketcher) or the Workbench trait and packages (use printcad-workbench-api)."
---

# printCAD: modelling benches

The invariants in `CLAUDE.md` still bind here; this skill never weakens one. Each bench talks to the host only through the `Workbench` trait (`printcad-workbench-api`). How solids build and references find faces is in `printcad-kernel`.

Read only the bench you are working on:

- `references/design.md`: `wb_design` (solid features). Feature approximations (Through all, Helix, Pipe, hole threads) are in `printcad-kernel/references/known-approximations.md`.
- `references/surface.md`: `wb_surface`.
- `references/assembly.md`: `wb_assembly`.
- `references/mesh.md`: `wb_mesh` and `surface_texture`.
- `references/inspect.md`: `wb_inspect` and `metrology`.
- `references/project-import.md`: `project_archive` and `project_tools` (`ProjectImportBench`, `project-tool`).
