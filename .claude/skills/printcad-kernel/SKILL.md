---
name: printcad-kernel
description: "The kernel side of printCAD: kernel_api (TriMesh, ProfileWire, SolidOp, ChainError), kernel_ogeom (STEP, IGES and mesh import, ops, export, face naming, shape health), mesh starts and base solids, the recompute loop (rebuild_jobs, BuildPlan, the kernel worker, ChainCache, partial builds on failure), feature preview and size limits, import performance, progress and cancel, and the known approximations (how references find faces and edges, Through all, Helix, Pipe, hole threads). Use when working in crates/kernel_api, crates/kernel_ogeom, app/recompute.rs or app/kernel_worker.rs, when a rebuild or import is wrong or slow, or when a reference loses its face. Not for sketch solving (use printcad-sketcher); the kernel gap protocol itself is in CLAUDE.md."
---

# printCAD: kernel, rebuilds and imports

When ogeom lacks something, follow the kernel gap protocol in `CLAUDE.md` (never paper over it app-side). The invariants in `CLAUDE.md` still bind here; this skill never weakens one.

Read the reference that matches the task:

- `references/crates.md`: `kernel_api` and `kernel_ogeom` (import paths, ops, export, the import bench, the ogeom `rev` pin and its local patch).
- `references/solids.md`: mesh starts, base solids, Delete Faces, recognised holes, Project.
- `references/rebuild-and-import.md`: the recompute loop, feature preview, size limits, import performance, progress/cancel, shape health and announcement discipline.
- `references/known-approximations.md`: face and edge naming, how references are found again, Through all / Up to face, Helix, Pipe, hole threads (the `Known approximations / roadmap` notes).
