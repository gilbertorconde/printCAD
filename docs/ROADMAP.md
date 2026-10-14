# Roadmap

Everything still open, in one place: what the workbenches lack, what waits
on the kernel, and the larger work ahead. What is built is in the release
notes (`crates/app_shell/RELEASE_NOTES.md`). Sizes are rough effort (S: a
day or less, M: a few days, L: a week or more).

A kernel capability that is missing is filed on the
[kernel's repository](https://github.com/gilbertorconde/ogeom-rs/issues),
noted on its item here, and the rest of the item is built around it, with
a test marked `#[ignore = "kernel: … (ogeom-rs#N)"]` that passes once the
kernel has it.

## Waiting on the kernel

Gaps in the kernel printCAD runs into. When a fix is
released: raise the `ogeom` requirement in the workspace `Cargo.toml`,
take the `#[ignore]` off its tests, run the suite. Every filed gap is
closed in the ogeom the app runs (0.11.0), and no test is ignored on one.

Not yet filed:

- [ ] **A pad on a converted solid that was not refined** (M). Fusing a
  pad into a mesh converted as it is (its curved areas still facets)
  fails where the pad's edges run within micrometres of the facets'. The
  app says to refine the body first (a FACETED badge, the conversion's
  log line, and the failed feature's task); the kernel's owner is working
  on the boolean itself.

## Faster rebuilds

The rebuild skips the history an edit did not change, and the kernel's
operations work near what an edit touches: a boolean passes the faces far
from its tool through as they are (ogeom-rs#134), and a fillet reads only
what is near each edge (ogeom-rs#135). What is left in printCAD:

- [ ] With the history's exact copies (`History::copy_of`, which the
  refine after a feature already reads): face naming carries an
  unchanged face's names straight across, about 60 ms an op on a
  400-face part, and mesh, outline and bounds reuse key on the face
  itself instead of hashing its geometry.

A kernel operation found slow goes to the kernel's repository as an issue
with its API, a repro, `rebuild_bench`'s numbers and an acceptance test
(the time follows the touched region, the result the same as now).
