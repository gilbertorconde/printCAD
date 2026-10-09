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
released: bump `ogeom` in the workspace `Cargo.toml`, take the `#[ignore]`
off its tests, run the suite.

Not yet filed:

- [ ] **A pad on a converted solid that was not refined** (M). Fusing a
  pad into a mesh converted as it is (its curved areas still facets)
  fails where the pad's edges run within micrometres of the facets'. The
  app says to refine the body first (a FACETED badge, the conversion's
  log line, and the failed feature's task); the kernel's owner is working
  on the boolean itself.

## Faster kernel operations

The rebuild skips the history an edit did not change; the operations
themselves still cost what the whole solid holds, not what an edit
touches. The kernel work, in order of payoff (`rebuild_bench`'s holed plates: one
hole added to a 582-face plate costs 0.3 s on ogeom 0.9.9, the boolean
0.01 s of it, the snapshot 0.01 s, and resuming the chain from its kept
state, cloning a model that holds every intermediate result, most of
the rest):

- [ ] **A. Local boolean** (L, ogeom-rs#134, in 0.9.9: the corner hole's
  boolean 0.14 s to 0.01 s). Faces whose box misses the tool's and that
  no section crosses pass through as they are, not split, classified or
  rebuilt; a connected region of them is classified once, by one probe;
  only the region near the tool is sewn again; a face tree replaces the
  all-pairs box test.
- [ ] **D. Fillets in one pass** (M, ogeom-rs#135). All of a fillet's blends applied in
  one boolean, or by replacing faces locally, not one whole-solid boolean
  per edge piece.

And in printCAD, as each lands:

- [ ] With the history's exact copies (`History::copy_of`, in the
  kernel): face naming carries an unchanged face's names straight
  across, about 60 ms an op on a 400-face part, and mesh, outline and
  bounds reuse key on the face itself instead of hashing its geometry.

Each goes to the kernel's repository as an issue with its API, a repro, the
bench numbers and an acceptance test (the time follows the touched region,
the result the same as now).

## Proposed sketch solver extraction

[RFC 0003](rfcs/0003-standalone-sketch-solver.md) proposes a standalone 2D
constraint solver with a numerical problem/result API, replayable fixtures
and independent tests. Its five milestones cover behaviour capture, pure
mathematics, the library and sketch adapter, replay and generated tests,
and dependency checks. The extraction preserves the current algorithm and
sketch behaviour; the RFC is open for discussion before implementation.
