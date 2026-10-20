#!/usr/bin/env node
// Every gate CI holds a change to, in one run, stopping at the first that
// fails: format, layers, clippy, the comment lint, the tests within their
// budget (only what the change reaches, or all with --all) and the tests'
// share of the code.

import { spawnSync } from "node:child_process";

const all = process.argv.includes("--all");
const gates = [
  ["format", "cargo", ["fmt", "--all", "--check"]],
  ["layers", "node", ["scripts/check-layers.mjs"]],
  ["clippy", "cargo", ["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]],
  ["comments", "node", ["scripts/lint-comment-rot.mjs", ...(all ? ["--all"] : [])]],
  ["tests", "node", ["scripts/test-budget.mjs", ...(all ? [] : ["--changed"])]],
  ["test lines", "node", ["scripts/test-lines.mjs"]],
];
for (const [name, program, args] of gates) {
  console.log(`\n== ${name}: ${program} ${args.join(" ")}`);
  const run = spawnSync(program, args, { stdio: "inherit" });
  if (run.status !== 0) {
    console.error(`\n${name} failed`);
    process.exit(run.status ?? 1);
  }
}
console.log("\nevery gate passed");
