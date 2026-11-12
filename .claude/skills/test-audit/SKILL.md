---
name: test-audit
description: "Invoke whenever writing, changing, reviewing, or sweeping tests. Authoring gate for new tests plus audit workflow for low-value, implementation-coupled, or duplicative tests and the test-only production seams they demand. Not for comment or slop cleanup (use comment-cleanup) and not for a bug hunt in the diff (use review)."
---

# Test Audit

Three modes, one value bar. Authoring mode gates every new or changed test
at write time. Audit mode runs focused sweeps of tests that re-assert
source, duplicate stronger proof, couple behavior to implementation, or
keep test-only production seams alive. Continue broad audits as separate
coherent follow-up commits; optimize for confidence, not deletion count.
Campaign mode prunes one whole crate's test surface; before starting one,
read [CAMPAIGN.md](CAMPAIGN.md).

Two references carry the method. [references/test-strategy.md](references/test-strategy.md)
maps risks to one portfolio action each and states the proof a deletion
needs. [references/test-map.md](references/test-map.md) names the owning
layer and harness for each kind of contract in this repo.

## Authoring gate

Before adding any test, answer four questions; a missing answer means do
not add it yet:

1. What observable behavior, invariant, or independent contract does it
   protect?
2. What credible regression makes it fail?
3. Why does existing coverage not already catch that failure? Each
   contract has one primary test owner at the strongest boundary; another
   layer needs its own distinct risk, such as a thread or lifecycle
   failure the owner cannot reach. Prefer extending a table-driven case or
   a shared harness (the sketcher `Harness`, the design stack) over a
   near-duplicate test; consolidate duplicated setup in the same change.
4. Does it need a production seam (a `pub` widened for the test, a flag, a
   wrapper, an injection hook) that no production caller needs? If yes,
   move the test to the real boundary instead.

Then check the test against every [junk pattern](#junk-patterns); a match
fails the gate unless the [retention bar](#retention-bar) names the
contract it independently guards. A test that would break under
behavior-preserving refactoring is asserting implementation, not behavior;
rewrite it at the owning boundary before landing it.

Geometry tests assert geometric properties (bounds, tangency, closure,
volume, face count) rather than implementation details, as the
`printcad-testing` skill asks. Bug regression tests must fail on the pre-fix code for the intended
reason and pass after the owner-boundary repair. A regression test that
never demonstrably failed proves the fixture, not the fix. One regression
at the owner boundary covers the bug; do not replay the same scenario at
every layer it crosses.

A kernel gap gets its intended-behavior test under
`#[ignore = "kernel: <reason> (ogeom-rs#N)"]`; that is the one sanctioned
ignore, and it flips green when the kernel bump lands.

## Junk patterns

The shared checklist for both modes: the authoring gate rejects a new test
that matches one, and audits hunt for existing tests that do.

- assertion-free coverage probes (a build that "does not panic" with no
  property checked);
- self-comparisons and identity copiers;
- copied fixtures, inventories, manifests, or export lists;
- exact source, import, or string greps;
- private predicate or call-shape tests duplicated at real boundaries;
- duplicate invocations of the same contract;
- tests whose only purpose is preserving test-only exports or wrappers;
- dead production code whose only callers are tests;
- expected values produced by the helper or kernel under test (a volume
  computed by the same op it checks);
- fakes that implement the asserted behavior, or one identical fake
  standing in for different traits (`Fetch`, `DocumentServer`,
  `KernelQueries`);
- fixtures that supply the ordering or the answer the owner should produce
  (probe answers written by hand where the chain should answer them);
- capability tests that restate declared flags instead of exercising what
  the flag promises;
- negative controls that pass for an unrelated reason, such as a refusal
  from a different check or an error the production path never reaches;
- names or fixtures that promise more than the input exercises.

## Value bar

Tests justify their maintenance cost by protecting behavior, a credible
regression, or an independently meaningful contract. In an audit, an
existing test that must change for behavior-preserving source
reorganization is suspect, not automatically deletable; the authoring gate
still rejects new ones.

Before judging a candidate, read the complete test and production owner,
its entry point, callers, callees, sibling implementations, overlapping
tests, CI routing, and relevant history. Read `CLAUDE.md` first. When the
test claims kernel-backed behavior, inspect the kernel's source or docs
directly (`cargo doc -p ogeom` or the checkout the commented
`[patch.crates-io]` points at).

## Discovery

Keep discovery read-only and report evidence before editing. For broad
scope, run parallel discovery lanes when available:

- the document (`crates/core_document`: ops and replay, undo, expressions
  and evaluation, datums, components, placement, persistence);
- the kernel adapter (`crates/kernel_ogeom`: chain ops, import, health,
  naming, holes, export, the `kernel:` ignores);
- the benches (`crates/workbenches/*`: the sketcher harness and solver,
  Design's build and features, the Assembly's solver and joints, the
  package host and the SDK packages it builds);
- the app and the rest (`crates/app_shell`, `doc_server`, `render_wgpu`,
  `ui_kit`, `scripting`, `agents`, `surface_texture`, `settings`,
  `local_ipc`, `axes`);
- a cross-cutting pattern sweep.

Outside campaign mode, prefer a few high-confidence candidates over a
large speculative inventory. Hunt for the [junk patterns](#junk-patterns).

## Retention bar

Keep a test when it independently enforces a public API, a file format,
a wire frame, a settings layout, a command spec, a security check, a
platform behavior, a default, a generated artifact, a package contract, a
release rule, or an architecture contract. Also keep:

- call ordering when order is observable behavior (ops, `seq`);
- regressions with a credible failure mode;
- source inspection when it is the cheapest independent guard: it fails
  when the contract changes and survives an identifier-only refactor
  (the seam lint, the icon table test, the docs drift test, the release
  notes test);
- a retained test that fails on the baseline: treat it as a possible
  product bug, reproduce it, and repair the owner rather than deleting it;
- a `kernel:`-ignored test, which documents a gap and waits for its fix.

Static or slow is not a deletion reason (`cargo test -p wb_wasm` builds
the SDK packages and is slow by design). A test that resembles
implementation may still be the independent contract; prove otherwise
before removing it.

## Candidate evidence

Record every field below before editing. A missing field means the
candidate is not ready for deletion:

- exact test name and location;
- what failure it can actually detect;
- non-test callers of the covered production or support seam;
- stronger remaining owner-boundary proof, or why no proof is needed;
- relevant history and the reason the test or seam exists;
- production or test-support deletion unlocked;
- risk and the focused validation command.

## Edit shape

Choose one coherent owner-boundary batch. Delete obsolete test-only
exports, wrappers, and dead production paths instead of preserving
aliases. Move retained regressions to their canonical owners. Consolidate
repeated assertions into one generic contract.

Prefer net-negative production LOC. Do not add replacement tests that
restate the same implementation, and do not convert uncertain candidates
into cleanup to increase deletion counts.

## Validation

Never edit source or tests while a test runner is running in the
checkout.

1. Run the smallest owner and sibling tests: `cargo test -p <crate>
   <filter>`.
2. For a removed source inspection, run the script or check that owns the
   real contract (the vendoring scripts, `PRINTCAD_WRITE_DOCS=1` for the
   command reference, the CI seam grep).
3. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
   -- -D warnings`, `node scripts/lint-comment-rot.mjs`, then `git diff
   --check`.
4. Run `cargo test --workspace` once before claiming the batch is done.
5. Inspect `git diff --numstat`; report production and tooling separately
   from tests and test support.
6. After final audit edits, run an adversarial review of the diff (the
   review skill).

## Landing and continuation

Commit only when authorized, one coherent scope per commit in the repo's
convention (`fix(...)`, `feat(...)`, `docs:`), no co-author lines. After
landing, rerun read-only discovery for the next high-confidence batch.

## Handoff

Report:

- root cause and removed low-value categories;
- production owner simplifications;
- retained false positives and why they remain valuable;
- focused and full proof actually run;
- production versus test LOC;
- commit state;
- named follow-ups.
