---
name: ci-iterate
description: "Loop on a red CI run (a push to master, a PR, or a release build) until the actionable checks pass: classify each red job, reproduce locally, fix the cause, hand the commit over, repeat. Use for a failing or flaky check, a release workflow that stopped part-way, or review feedback on an open PR. Not for authoring the fix's tests (use test-audit) and not for a bug with no CI signal (use systematic-debugging)."
---

# ci-iterate

Fix actionable CI failures on one run. Work lands on `master` by direct
push, so the run is usually `ci.yml` on `master`; a PR or a `release.yml`
run is handled the same way. Stop and report when only human steps
remain (the owner pushes, tags and approves).

## Rules

- Tie every diagnosis to the exact run, job and head SHA. Evidence from
  an older SHA is not evidence for this one.
- Classify before you fix: product defect, test defect, flake,
  infrastructure (runner, GitHub outage, a crate registry or Homebrew
  hiccup, a shader compiler build on the Windows runner), or a
  platform-only failure (the `platforms` job runs clippy and, on master
  or for platform code, the tests on Windows and macOS; nobody runs the
  app there by hand). Check
  <https://www.githubstatus.com/> when several unrelated jobs fail
  together.
- Fix the root cause. No `--no-verify`, no skipped or commented-out
  tests, no weakened assertions, no `#[allow]` to quiet clippy, no
  retries or sleeps to hide a flake, no `#[ignore]` without a `kernel:`
  reason.
- Never force push, rerun or cancel runs, edit workflow files, move a
  tag or post on a PR unless the user asks. The owner pushes; commit
  locally and say so.
- Before handing a commit over, run the local equivalent of every job
  (read `.github/workflows/ci.yml`):

  ```bash
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo clippy -p app_shell --target wasm32-unknown-unknown -- -D warnings
  node scripts/test-budget.mjs
  node scripts/lint-comment-rot.mjs --all
  node scripts/test-lines.mjs
  ! git grep -nE 'wb_design|wb_sketch|"wb\.(design|part|sketch)"|core\.datum' -- crates/app_shell/src ':!crates/app_shell/src/app/seam_lint.rs'
  ```

  Run them after the last edit: the format step is first and stops the
  job. CI builds against the published kernel and `sixdof`: an active
  `[patch.crates-io]` in `Cargo.toml` must be commented out (back up
  `Cargo.toml` and `Cargo.lock`, restore after) to see what CI sees.
- Same failure after two attempts: stop and ask.

## Loop

1. Identify the run and its state:

   ```bash
   gh run list --workflow=ci.yml --branch master --limit 5 --json databaseId,headSha,conclusion,status,createdAt
   gh run view <run-id> --json headSha,jobs
   ```

   For a PR: `gh pr view --json number,headRefOid,isDraft,reviewDecision`
   and `gh pr checks`. For a release: `gh run list --workflow=release.yml`.

2. Feedback, when there is a PR: read review threads and comments.

   ```bash
   gh api repos/{owner}/{repo}/pulls/{number}/comments
   gh api repos/{owner}/{repo}/pulls/{number}/reviews
   ```

   Fix high and medium items (correctness, an invariant from `CLAUDE.md`)
   after you verify each one against the code. List low items (naming,
   style, taste) for the user to choose. A false positive gets a short
   reason, not a code change.

3. Checks:

   | State | Action |
   |---|---|
   | a job failed, none pending | fix failures |
   | actionable jobs pending | wait and read feedback meanwhile |
   | only the owner's steps remain (push, tag, approve) | report `BLOCKED_BY_OWNER` |
   | no run for the head | report `NO_RUN` (a fork PR or a draft may not run them) |
   | every job passed | read feedback once more, then stop |

   Every `ci.yml` job gates: `scope` (what a pull request reaches,
   `scripts/ci-scope.mjs`), `lint` (Linux: format, clippy, the browser
   clippy, the standalone solver, the comment-rot lint, the test-lines
   ceiling, the seam grep), `test` (Linux: the tests within their budget,
   on a pull request only the crates it reaches) and `platforms` (Windows
   and macOS: clippy, and the tests on master or when the change can
   differ by platform). A pull request that skipped the platform tests
   can still go red on master; the `platforms` label runs them early. `release.yml` runs on a `v*` tag; a run that
   stopped part-way is finished by dispatching it with the tag, which the
   owner does.

4. Fix each failure:

   ```bash
   gh run view <run-id> --log-failed
   ```

   Trace from the assertion, panic, lint rule or compiler error to its
   source. State the cause in one line before you edit ("fails because
   X, reached by Y"). Search sibling call sites for the same defect and
   fix all of them. A platform-only failure is read from the log, then
   reasoned from the code (`local_ipc`, path and process handling, the
   graphics backend, line endings); there is no local reproduction.

5. Flake suspected: measure before you believe it.

   ```bash
   gh run list --workflow=ci.yml --branch master --status completed \
     --limit 60 --json conclusion,headSha,createdAt,databaseId
   ```

   Confirm it is the same test each time from the failing job's log.
   Interleaved pass and fail on unchanged code is a flake; a long
   unbroken red streak is a regression, so find the last green and first
   red commit. Reproduce locally, fix the cause, and prove it with an
   N-run loop sized to the measured rate:

   ```bash
   for i in $(seq 1 N); do cargo test -p <crate> <filter> -- --test-threads=4 || break; done
   ```

6. Verify locally, commit in the repo convention (one scope per commit,
   `fix(...)`/`feat(...)`/`docs:`, no co-author lines), tell the owner
   the commit is ready to push, and restart at step 3 once a run exists.

## Exit

| Result | When |
|---|---|
| `DONE` | every job passes and no high or medium feedback is open |
| `ASK` | the same failure after two attempts, unclear feedback, or an infrastructure issue |
| `STOP` | no run, only the owner's steps remain, or a conflict needs a decision |

Report the run, head SHA, each failure with its class, cause and fix,
the local checks run, and what remains.
