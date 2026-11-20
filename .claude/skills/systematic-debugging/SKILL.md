---
name: systematic-debugging
description: Use when encountering any bug, test failure, or unexpected behavior, before proposing fixes. Four-phase root-cause process. For a red CI run use ci-iterate; for a measured slowness use performance-optimizer.
---

# Systematic Debugging

## Overview

**Core principle:** ALWAYS find root cause before attempting fixes. Symptom fixes are failure.

**Violating the letter of this process is violating the spirit of debugging.**

## The Iron Law

```
NO FIXES WITHOUT ROOT CAUSE INVESTIGATION FIRST
```

If you haven't completed Phase 1, you cannot propose fixes.

## When to Use

Use for ANY technical issue:
- Test failures
- Bugs a user reports
- Unexpected behavior
- Performance problems
- Build failures
- Integration issues

**Use this ESPECIALLY when:**
- Under time pressure (emergencies make guessing tempting)
- "Just one quick fix" seems obvious
- You've already tried multiple fixes
- Previous fix didn't work
- You don't fully understand the issue

**Don't skip when:**
- Issue seems simple (simple bugs have root causes too)
- You're in a hurry (rushing guarantees rework)
- The owner wants it fixed NOW (systematic is faster than thrashing)

## The Four Phases

You MUST complete each phase before proceeding to the next.

### Phase 1: Root Cause Investigation

**BEFORE attempting ANY fix:**

1. **Read Error Messages Carefully**
   - Don't skip past errors or warnings
   - They often contain the exact solution
   - Read panics and backtraces completely (`RUST_BACKTRACE=1`)
   - Note line numbers, file paths, the op index a `ChainError` carries
   - GPU validation output arrives in `tracing` under `printcad.gpu`

2. **Reproduce Consistently**
   - Can you trigger it reliably?
   - What are the exact steps?
   - Does it happen every time?
   - If not reproducible: gather more data, don't guess
   - Prefer a test over a manual run: the sketcher `Harness` in
     `wb_sketch/tests/interaction.rs`, `kernel_ogeom/tests/design_stack.rs`,
     `core_document/tests/op_replay.rs`, or a script through
     `printcad --script` (`headless.rs`)
   - For a UI symptom, the `PRINTCAD_BENCH_*` hooks (`frame.rs`) replay a
     sketch, a click, a tool, a task or a capture without a hand on the mouse
   - A user's file is read-only evidence: copy it to `/tmp` before any
     test or probe reads it

3. **Check Recent Changes**
   - What changed that could cause this?
   - `git diff`, `git log`, a kernel bump in `Cargo.lock`
   - An active `[patch.crates-io]` in `Cargo.toml` means a local kernel,
     not the published one
   - Debug builds: our own crates are unoptimized, the kernel is not; a
     slow import in debug is expected, a hang is not

4. **Gather Evidence in Multi-Component Systems**

   **WHEN the symptom crosses a thread, a process or a seam:**

   **BEFORE proposing fixes, add diagnostic instrumentation:**
   ```
   For EACH component boundary:
     - Log what data enters component
     - Log what data exits component
     - Verify environment/config propagation
     - Check state at each layer

   Run once to gather evidence showing WHERE it breaks
   THEN analyze evidence to identify failing component
   THEN investigate that specific component
   ```

   **printCAD boundaries.** Name the suspected boundary before you
   instrument:
   - the UI thread and the kernel worker (the `kernel_worker` crate,
     `app/recompute.rs`): plans in, `SolidBuildResult` out, routed by body
     id or `import_owner`, a build dropped or replaced by a newer plan;
   - the document and its server (`core_document/src/server.rs`,
     `doc_server`): length-prefixed frames, the op log beside the file,
     `at_seq` on saves, `Rebase` on undo, new and open;
   - the host and a bench (the `Workbench` trait, `HostRequest` through
     `HookOutcome`, `UiFrameInputs` seeding, suspend and resume on a real
     tab switch only);
   - a package guest (`wb_wasm`: `Access` per call, the epoch deadline, a
     trap replacing the instance, jobs on their own instance);
   - the script thread (`Event::Call` run on the UI thread in
     `drive_scripts`, pinned to `in_script_tab`, the journal held per run);
   - the MCP relay and chats (`app/mcp.rs`, approvals, `agent_check`,
     `app/chats.rs` over the `assistant` crate);
   - the renderer (`scene_fingerprint`, the on-demand frame schedule and
     its pending flags, the mesh cache keyed by id and revision);
   - the kernel itself (a refusal surfaces as `ChainError` with its op
     index; `ChainCache` resumes from the first differing op; grep
     `kernel:` in `tests/` before assuming a feature is wired wrong);
   - the file formats (STEP, IGES, mesh readers, `.prtcad` tar, serde
     defaults on persisted types, `renamed` ids).

   Log what enters and leaves that boundary first (`tracing`, `RUST_LOG`;
   the 1 s `printcad.frame` log for anything per frame).

5. **Trace Data Flow**

   **WHEN error is deep in call stack:**

   See `root-cause-tracing.md` in this directory for the complete backward tracing technique.

   **Quick version:**
   - Where does bad value originate?
   - What called this with bad value?
   - Keep tracing up until you find the source
   - Fix at source, not at symptom

### Phase 2: Pattern Analysis

**Find the pattern before fixing:**

1. **Find Working Examples**
   - Locate similar working code in same codebase
   - What works that's similar to what's broken? (another op and its
     inverse, another bench's `derive_on_geometry`, another reader)

2. **Compare Against References**
   - If implementing pattern, read reference implementation COMPLETELY
   - Don't skim - read every line
   - Understand the pattern fully before applying

3. **Identify Differences**
   - What's different between working and broken?
   - List every difference, however small
   - Don't assume "that can't matter"

4. **Understand Dependencies**
   - What other components does this need?
   - What settings, config, environment?
   - What assumptions does it make? (`CLAUDE.md` lists the invariants
     that break subtly)

### Phase 3: Hypothesis and Testing

**Scientific method:**

1. **Form Single Hypothesis**
   - State clearly: "I think X is the root cause because Y"
   - Write it down
   - Be specific, not vague

2. **Test Minimally**
   - Make the SMALLEST possible change to test hypothesis
   - One variable at a time
   - Don't fix multiple things at once

3. **Verify Before Continuing**
   - Did it work? Yes: Phase 4
   - Didn't work? Form NEW hypothesis
   - DON'T add more fixes on top

4. **When You Don't Know**
   - Say "I don't understand X"
   - Don't pretend to know
   - Ask for help
   - Research more

### Phase 4: Implementation

**Fix the root cause, not the symptom:**

1. **Create Failing Test Case**
   - Simplest possible reproduction
   - Automated test if possible
   - One-off example or script if no harness fits
   - MUST have before fixing
   - Follow the authoring gate in the test-audit skill: it must fail on the pre-fix code for the intended reason

2. **Implement Single Fix**
   - Address the root cause identified
   - ONE change at a time
   - No "while I'm here" improvements
   - No bundled refactoring

3. **Verify Fix**
   - Test passes now?
   - No other tests broken?
   - Issue actually resolved?
   - Run the checks `CLAUDE.md` names before claiming success: `cargo fmt
     --all --check`, `cargo clippy --workspace --all-targets -- -D
     warnings`, `cargo test --workspace`, `node
     scripts/lint-comment-rot.mjs`, the seam grep; for rendering or UI
     work, a smoke run watching for `printcad.gpu` output

4. **If Fix Doesn't Work**
   - STOP
   - Count: How many fixes have you tried?
   - If < 3: Return to Phase 1, re-analyze with new information
   - **If ≥ 3: STOP and question the architecture (step 5 below)**
   - DON'T attempt Fix #4 without architectural discussion

5. **If 3+ Fixes Failed: Question Architecture**

   **Pattern indicating architectural problem:**
   - Each fix reveals new shared state/coupling/problem in different place
   - Fixes require "massive refactoring" to implement
   - Each fix creates new symptoms elsewhere

   **STOP and question fundamentals:**
   - Is this pattern fundamentally sound?
   - Are we "sticking with it through sheer inertia"?
   - Should we refactor architecture vs. continue fixing symptoms?

   **Discuss with the user before attempting more fixes**

   This is NOT a failed hypothesis - this is a wrong architecture.

6. **If the cause is in the kernel**

   Follow the kernel gap protocol in `CLAUDE.md`: wire the op anyway, let
   the refusal surface as a clean `ChainError`, add the intended-behaviour
   test under `#[ignore = "kernel: <reason> (ogeom-rs#N)"]`, file the issue
   on the kernel repository. Never a mesh-level hack or a silently degraded
   feature app-side.

## Red Flags - STOP and Follow Process

If you catch yourself thinking:
- "Quick fix for now, investigate later"
- "Just try changing X and see if it works"
- "Add multiple changes, run tests"
- "Skip the test, I'll manually verify"
- "It's probably X, let me fix that"
- "I don't fully understand but this might work"
- "Pattern says X but I'll adapt it differently"
- "Here are the main problems: [lists fixes without investigation]"
- Proposing solutions before tracing data flow
- **"One more fix attempt" (when already tried 2+)**
- **Each fix reveals new problem in different place**

**ALL of these mean: STOP. Return to Phase 1.**

**If 3+ fixes failed:** Question the architecture (see Phase 4.5)

## The User's Signals You're Doing It Wrong

**Watch for these redirections:**
- "Is that not happening?" - You assumed without verifying
- "Will it show us...?" - You should have added evidence gathering
- "Stop guessing" - You're proposing fixes without understanding
- "Ultra-think this" - Question fundamentals, not just symptoms
- "We're stuck?" (frustrated) - Your approach isn't working

**When you see these:** STOP. Return to Phase 1.

## Common Rationalizations

| Excuse | Reality |
|--------|---------|
| "Issue is simple, don't need process" | Simple issues have root causes too. Process is fast for simple bugs. |
| "Emergency, no time for process" | Systematic debugging is FASTER than guess-and-check thrashing. |
| "Just try this first, then investigate" | First fix sets the pattern. Do it right from the start. |
| "I'll write test after confirming fix works" | Untested fixes don't stick. Test first proves it. |
| "Multiple fixes at once saves time" | Can't isolate what worked. Causes new bugs. |
| "Reference too long, I'll adapt the pattern" | Partial understanding guarantees bugs. Read it completely. |
| "I see the problem, let me fix it" | Seeing symptoms ≠ understanding root cause. |
| "One more fix attempt" (after 2+ failures) | 3+ failures = architectural problem. Question pattern, don't fix again. |
| "The kernel is wrong, I'll work around it in the mesh" | A workaround hides the gap. Wire it, surface it, file it. |

## Quick Reference

| Phase | Key Activities | Success Criteria |
|-------|---------------|------------------|
| **1. Root Cause** | Read errors, reproduce, check changes, gather evidence | Understand WHAT and WHY |
| **2. Pattern** | Find working examples, compare | Identify differences |
| **3. Hypothesis** | Form theory, test minimally | Confirmed or new hypothesis |
| **4. Implementation** | Create test, fix, verify | Bug resolved, tests pass |

## When Process Reveals "No Root Cause"

If systematic investigation reveals issue is truly environmental, timing-dependent, or external:

1. You've completed the process
2. Document what you investigated
3. Implement appropriate handling (a clear error in the log, a bounded wait on a real condition) and never a `let _ =` on an error, an `#[allow]` to quiet a warning, a commented-out test, an `#[ignore]` without a `kernel:` reason, a sleep or a blind retry
4. Add logging for future investigation

**But:** 95% of "no root cause" cases are incomplete investigation.

## Supporting Techniques

These techniques are part of systematic debugging and available in this directory:

- **`root-cause-tracing.md`** - Trace bugs backward through call stack to find original trigger
- **`defense-in-depth.md`** - Add validation at multiple layers after finding root cause
- **`condition-based-waiting.md`** - Replace arbitrary timeouts with condition polling (the examples are in another language; in a Rust test, poll the condition with an `Instant` deadline)
- **`find-polluter.sh`** - Bisect which test file leaves a file or state behind; a cargo runner is one of its examples
