---
name: review
description: "Read-only review of the branch diff, the working tree or a commit range for correctness, the project's invariants and release risk, with an optional independent panel (at most three lenses, at most two rounds). Use before committing, before a release, or when asked for an adversarial or multi-agent review. Not for whole-repo audits, comment or slop cleanup (use comment-cleanup) or test-portfolio audits (use test-audit)."
---

# Review

One lead owns scope, evidence and verdict. The review is read-only: no
edits, commits, pushes or issue writes unless the user asks.

## 1. Scope and tier

1. Resolve the diff. Work lands on `master` directly, so the review target
   is usually the working tree plus unpushed commits:
   `git diff origin/master` (everything since the remote) or
   `git diff origin/master...HEAD` plus `git diff HEAD` when the two must be
   told apart. A range or a commit given by the user replaces that. If the
   output is truncated, read each changed file until every changed line is
   seen. Name exactly what was reviewed (committed, staged, unstaged).
2. State the protected outcome, the acceptance criteria and the non-goals
   from the request, the commit messages or the roadmap entry
   (`docs/ROADMAP.md`). Mark a guess `UNKNOWN`.
3. Drop diff noise (`Cargo.lock` bumps, vendored icons and fonts under
   `crates/ui_kit`, generated `icon_table.rs`, the generated command
   reference in `docs/SCRIPTING.md`) but keep persisted-type and wire
   changes.
4. Pick the tier:

| Tier | When | Depth |
|---|---|---|
| trivial | 10 changed lines or fewer, no sensitive path | direct read, no panel |
| lite | 100 lines or fewer, no sensitive path | direct read, one lens at most |
| full | larger, or any sensitive path | direct read plus the panel as needed |

Sensitive paths always get `full`:

- the document and its ops: `crates/core_document/src/{lib.rs,op.rs,history.rs,undo.rs,server.rs}`, persistence (`.prtcad` reading and writing);
- the document server: `crates/doc_server/`;
- teardown and the renderer core: `crates/app_shell/src/app/gfx.rs`, `crates/render_wgpu/src/core.rs`;
- the package sandbox and installs: `crates/workbenches/wb_wasm/src/{host,guest,jobs,package,remote,store}.rs`, `crates/package_store/src/packages.rs`, `crates/app_shell/src/app/packages.rs`;
- agents and scripts: `crates/agents/`, `crates/scripting/`, `crates/app_shell/src/app/{mcp,agent_context,scripts,chats}.rs`;
- file readers on untrusted input: `crates/kernel_ogeom/src/{import,mesh,annotations}.rs`, the DXF, image and tar readers;
- CI and release: `.github/workflows/`, `scripts/package-release.sh`, `scripts/linux-install.sh`.

## 2. Rules

Each finding cites the rule it breaks. The slugs name the invariants in
`CLAUDE.md`; read the matching paragraph before citing one.

| Slug | Rule |
|---|---|
| `one-op-per-edit` | Every user-edit mutator on `Document` records exactly one op; derived state never does; no `&mut` escape hatch |
| `undo-is-inverse-ops` | Undo and redo are ordinary forward ops; never `Rebase`, never a document replacement; non-invertible ops are history barriers |
| `seq-orders-history` | `FeatureNode.seq` orders build history, never `created_at` |
| `serde-default` | A new field on a persisted type takes `#[serde(default)]` |
| `saves-flushed-on-exit` | Every exit path calls `wait_for_all_document_saves()` |
| `host-names-no-bench` | `crates/app_shell/src` never names a bench crate, id or feature kind (`seam_lint.rs`, the CI grep) |
| `ui-owns-no-host-state` | `UiLayer` is seeded from `UiFrameInputs`; a bench talks back only through `ctx.request` |
| `ui-action-two-phase` | A UI action is one variant in `contract/commands.rs` and one arm in `app/commands.rs::apply_ui_commands` |
| `gfx-drop-order` | Field order in `gfx.rs` and the `take()`s in `RendererCore::drop` are the teardown contract |
| `no-wait-idle` | The renderer hot path has no queue or device wait; buffers retire through the `MeshCache` queue |
| `scene-fingerprint-complete` | Anything the scene pass reads is hashed in `scene_fingerprint` |
| `work-pending-wakes-loop` | Work that completes off the window is covered by a pending flag or wakes the loop with an `AppEvent` |
| `no-nested-map-ordered` | Never nest two `map_ordered` passes; output identical at any thread count |
| `progress-per-phase` | `progress::context` once per phase; loops use `detail` or `stage_at` |
| `kernel-gap-protocol` | A kernel gap is wired, surfaced as `ChainError`, tested under `#[ignore = "kernel: …"]` and filed; never papered over app-side |
| `camera-preset-relative` | Orientation is preset-relative; transform helpers come from `core_document::runtime`, never re-derived |
| `sketch-shared-vertices` | Endpoint snapping reuses point ids; no coincident duplicate points |
| `tokens-only` | No colour or size literals in UI code; icons through `ui_kit::icon` and the vendoring script |
| `placeholders-planned` | Planned controls only in Design and the Sketcher, with a `// PLANNED:` comment and `ToolDescriptor::planned` |
| `wasm-sandbox` | A package reaches only what the call's `Access` allows; network and helpers only under their grants |
| `agent-access` | Commands declare `read_only` and `AgentAccess`; `agent_check` gates every agent call |
| `naming-ban` | No inspiration named outside the README's Inspiration section |
| `comments-present-tense` | Comments describe present behaviour; `node scripts/lint-comment-rot.mjs` |
| `user-files-read-only` | Tests and probes never touch a user's files; copy them to `/tmp` first |

## 3. Direct review

For each changed file, map what it touches: document mutations, ops and
their inverses, kernel calls, renderer state, threads and channels,
sockets and files, package or agent boundaries. Then check every item for
every file:

- a mutation outside the mutator pattern (validate, resolve, build op,
  `apply_op`, record), a derived update that records, a missing inverse;
- replay and peers: does the op carry everything replay needs, and does an
  old `.prtcad` or op log still read (serde defaults, `renamed` ids)?
- threads: a result that lands on a background channel with nothing to wake
  the loop, a build routed to the wrong tab, a `with_tab` turn touching
  bench state, a kernel response for a body the user already changed;
- renderer: a scene input left out of the fingerprint, a wait on the hot
  path, a device object dropped after the device, a validation message;
- parsers and readers on untrusted bytes: tar member paths, sizes,
  recursion, a panic on malformed input that takes the app down;
- sandbox and agents: a new host call a package or an agent can reach, a
  command without an access declaration, a grant checked at install but
  not at call time;
- unbounded work, missing limits, leaks, blocking the UI thread;
- business logic at edges: empty profile, zero length, a hidden or linked
  body, a mesh body, a body with no history, undo mid-task, a tab switch
  mid-build, cancellation, partial failure;
- producers and consumers of a changed wire, file, settings or `bench_api`
  type agree, including the SDK under `sdk/` and the example workbench
  repository;
- platforms: a path, socket name, process spawn or line-ending assumption
  that only holds on Linux;
- superseded code, aliases, re-exports and dual paths are removed.

Trace each critical scenario from the gesture through the op to the
observable outcome (tree, view, file). Read unchanged code only to prove an
affected path.

## 4. Independent panel

Use no panel when direct evidence settles the verdict. Otherwise read
[references/independent-review.md](references/independent-review.md)
before you launch anyone. Limits: at most three lenses, at most two rounds.
Every reviewer gets the same frozen packet and never sees a sibling's
findings. After round two the lead checks the remaining evidence directly.

## 5. Materiality gate

Accept a finding only when all hold:

- the diff introduced, exposed or worsened it;
- it has `file:line`, a defect class, a concrete failure scenario and the
  smallest fix;
- it is not already handled elsewhere (search the code and the tests;
  grep `kernel:` in `tests/` before calling a feature wrongly wired).

Reject taste, theory, generic practice, hypothetical scale, and a design
that is only different. Deduplicate by root cause. Every correctness claim
needs a counterexample.

## 6. Checks

Run what CI runs, read-only for the tree:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node scripts/lint-comment-rot.mjs
! git grep -nE 'wb_design|wb_sketch|"wb\.(design|part|sketch)"|core\.datum' -- crates/app_shell/src ':!crates/app_shell/src/app/seam_lint.rs'
grep -rn $'\u2014' README.md docs/*.md crates/app_shell/RELEASE_NOTES.md
```

CI builds against the published kernel: an active `[patch.crates-io]` in
`Cargo.toml` means the local run proves less than CI will.

## 7. Verdict

Severity: `P0` loses or corrupts a document, `P1` blocks release, `P2`
important, `P3` minor.

- `FAIL`: an open P0 or P1, unmet acceptance, a check the change broke, or
  a rule slug broken on a sensitive path.
- `CONCERNS`: only non-blocking risk remains.
- `PASS`: every acceptance criterion has evidence and no finding survives.
- `BLOCKED`: required evidence (a check, a device, a fixture file) is
  missing and has no credible substitute.

Report in this order: verdict, scope (base, head, tier, exclusions),
findings (severity, `file:line`, slug, failure scenario, fix), checks run
with results, and what stays unproven. "No findings" is a valid result.
