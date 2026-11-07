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
take the `#[ignore]` off its tests, run the suite. The open ones, each
with an ignored test naming it (`grep -rn "kernel:" crates/*/tests`):

- Fillets and chamfers: chamfer on any edge (curved faces, a sphere's
  rim, spline outlines; ogeom-rs#711), a convex and a concave round
  meeting at one vertex (#712), a radius per edge in one call (#713),
  refusals naming the failing edge (#714), largest-fit queries (#715),
  overlapping rounds on a padded thin plate (#721) and a slow tangent
  loop of arcs (#723). Variable radius on any edge (#716), chamfer
  corners (#717), setback corners (#718), a rim's round taking a narrow
  strip off (#719) and a round over a staircase of narrow faces (#720)
  are filed and wait for an app side.
- Offset solid on free-form faces (#710).

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
