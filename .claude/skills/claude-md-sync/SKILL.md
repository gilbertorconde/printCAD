---
name: claude-md-sync
description: "Keep CLAUDE.md true and keep the skills under .claude/skills in step with the repo, and review a new or changed skill for trigger boundaries, the naming ban and harness neutrality. Use when a change touches commands, CI, the crate map, an environment hook, an invariant or a skill, or when asked to update CLAUDE.md or review a skill. Not for user docs under docs/ or the README."
---

# CLAUDE.md sync

`CLAUDE.md` is the one instruction file for this repo. Skills live in
`.claude/skills/<name>/SKILL.md`. The memory directory outside the repo
holds what the repo cannot (preferences, decisions, timings); nothing
there duplicates `CLAUDE.md`.

## When CLAUDE.md must change in the same commit

CLAUDE.md keeps the commands, the condensed crate map, the kernel gap protocol and the invariants. The detail behind them (full crate entries, render loop, bench hooks `PRINTCAD_*`, UI, interaction, testing, release, kernel notes) lives in the `printcad-*` skills; a change to that detail updates the owning skill, and a new or renamed skill updates CLAUDE.md's "Load these skills when" index.

- a command, script, workflow job or environment hook (`PRINTCAD_*`)
  named in it changes, moves or disappears;
- a crate, module or file it maps moves, or a new crate appears;
- an invariant it lists changes shape, or a new one is learned the hard
  way;
- a seam (the `Workbench` trait, `bench_api`, the command contract, the
  op pattern) gains or loses a method it describes;
- a skill is added, renamed or removed and another skill names it.

## Checklist

1. Check that every path and command in `CLAUDE.md` exists:

   ```bash
   cat CLAUDE.md .claude/skills/printcad-*/SKILL.md .claude/skills/printcad-*/references/*.md 2>/dev/null | grep -oE '`[a-z_]+/[A-Za-z0-9_./-]+`' | tr -d '`' | sort -u | while read -r p; do [ -e "$p" ] || [ -n "$(find crates sdk docs scripts -path "*/$p" -print -quit)" ] || echo "missing: $p"; done
   cat CLAUDE.md .claude/skills/printcad-*/SKILL.md .claude/skills/printcad-*/references/*.md 2>/dev/null | grep -oE 'PRINTCAD_[A-Z_]+' | sort -u | while read v; do git grep -q "$v" -- crates scripts || echo "unread: $v"; done
   ```

   Open each hit by hand: the file lists many paths relative to a crate,
   so a miss is a lead, not a verdict.
2. Keep it dense and true, in its own voice: one paragraph per subject,
   present tense, the invariant and the reason it bites. An addition is
   as dense as its neighbours; nothing restates the code's own docs, the
   git history or the roadmap. Link to `docs/` instead of copying.
3. Keep the invariants in `CLAUDE.md`, not in skills. A skill may cite
   one by its paragraph; it never weakens one.
4. The naming ban holds everywhere but the README's Inspiration section.
   Build the list from that section (each full name, and the acronym in
   parentheses where there is one) and grep the tree:

   ```bash
   sed -n '/^## Inspiration/,$p' README.md | grep -oE '\[[^]]+\]\(' | tr -d '[(' | sed 's/\]$//' | sort -u
   git grep -n -i -e '<name>' -- ':!README.md'
   ```

   Name an actual platform requirement (the display systems the app runs
   on) only where it is one.
5. No em-dash character in `CLAUDE.md`, `docs/`, the README, the release
   notes or a skill:

   ```bash
   grep -rn $'\u2014' CLAUDE.md README.md docs crates/app_shell/RELEASE_NOTES.md .claude/skills
   ```

## Skill review

For a new or changed skill:

1. Frontmatter has `name` (equal to the folder name) and `description`,
   and at most `license` or `metadata` besides. Harness-only keys are
   removed:

   ```bash
   grep -lE '^(allowed-tools|model|color|disable-model-invocation|argument-hint|user-invocable):' .claude/skills/*/SKILL.md
   ```

   No `{baseDir}` paths and no instruction that needs one vendor's tool.
   Name a capability ("run a subagent", "search the web") and give a
   fallback.
2. The description states what the skill does, when it fires, and the
   nearest case where it must not fire (name the sibling skill).
3. The body obeys `CLAUDE.md`: the kernel gap protocol, no quieted
   warnings or swallowed errors, no `--no-verify`, no force push, no
   tagging or pushing (the owner does), the commit convention, no
   co-author lines.
4. A skill holding text copied verbatim from another project keeps one
   plain `LICENSE` beside it with that license text only; other skills
   have none.
5. Read-only skills stay read-only; a writer skill names what it may
   change.
6. Every path, command, example and environment hook the skill names
   exists in this repo.
7. Behavioral check, when the skill is new or its trigger changed: write
   three to five prompts that should fire it and two or three close
   prompts that should fire a sibling instead. Run them in a fresh
   context with only the repo loaded. Record the prompt, whether it
   fired, and what it did. An unrun scenario is `NOT RUN`, not a pass.
   Static review does not prove activation.

Verdict: `PASS`, `PASS WITH CONCERNS`, `FAIL` (wrong trigger, unsafe
instruction, broken path, a banned name, missing license) or `BLOCKED`,
with `file:line` for each finding.
