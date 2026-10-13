#!/usr/bin/env node
// Run the workspace's tests within a time budget.
//
// Builds every test program (`cargo test --no-run`), then runs them two at
// a time, the tests inside each in parallel as `cargo test` would, so the
// rest run while the longest one finishes its slowest tests. Fails when a
// test fails, when one program takes longer than its budget, or when the
// run as a whole does: a slow test is a cost every build pays, so it is
// caught when it comes in rather than found later.
//
//   node scripts/test-budget.mjs                  every crate
//   node scripts/test-budget.mjs -p wb_sketch …   only these packages
//   node scripts/test-budget.mjs --changed        what a change reaches
//
// `--changed` asks scripts/affected-crates.mjs what the working tree's
// changes since origin/master reach, and runs the programs of the crates
// changed first, so their failures show before their dependents' do.
//
// TEST_BUDGET_PROGRAM_S (default 60) and TEST_BUDGET_TOTAL_S (default 150)
// set the budgets in seconds; a slower machine raises them, the defaults
// fit a CI runner with room to spare. TEST_JOBS (default 2) is how many
// programs run at once.

import { execFileSync, spawn } from "node:child_process";
import { basename } from "node:path";

const programBudget = Number(process.env.TEST_BUDGET_PROGRAM_S ?? 60);
const totalBudget = Number(process.env.TEST_BUDGET_TOTAL_S ?? 150);
const jobs = Math.max(1, Number(process.env.TEST_JOBS ?? 2));

let args = process.argv.slice(2);
let first = new Set();
if (args[0] === "--changed") {
  const reach = JSON.parse(
    execFileSync("node", ["scripts/affected-crates.mjs", "--json", args[1] ?? "origin/master"], {
      encoding: "utf8",
    }),
  );
  if (!reach.everything && reach.affected.length === 0) {
    console.log("the change reaches no crate; nothing to test");
    process.exit(0);
  }
  args = reach.everything ? [] : reach.affected.flatMap((name) => ["-p", name]);
  first = new Set(reach.touchedManifests);
}
const scope = args.length ? args : ["--workspace"];

const built = execFileSync(
  "cargo",
  ["test", ...scope, "--no-run", "--message-format=json"],
  { encoding: "utf8", maxBuffer: 1 << 30, stdio: ["ignore", "pipe", "inherit"] },
);
const programs = [];
for (const line of built.split("\n")) {
  if (!line.startsWith("{")) continue;
  const message = JSON.parse(line);
  if (message.reason === "compiler-artifact" && message.profile?.test && message.executable) {
    programs.push({ path: message.executable, crate: message.target.name, manifest: message.manifest_path });
  }
}
// The changed crates' programs go first; the order is otherwise cargo's.
programs.sort((a, b) => Number(first.has(b.manifest)) - Number(first.has(a.manifest)));

function run(program) {
  return new Promise((resolve) => {
    const started = process.hrtime.bigint();
    const child = spawn(program.path, ["-q"], { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("close", (status) => {
      const seconds = Number(process.hrtime.bigint() - started) / 1e9;
      const passed = Number(/(\d+) passed/.exec(stdout)?.[1] ?? 0);
      if (status !== 0) {
        process.stdout.write(stdout);
        process.stderr.write(stderr);
        console.error(`FAILED: ${program.crate} (${basename(program.path)})`);
      }
      resolve({ ...program, seconds, passed, failed: status !== 0 });
    });
  });
}

const started = process.hrtime.bigint();
const results = [];
let next = 0;
await Promise.all(
  Array.from({ length: Math.min(jobs, programs.length) }, async () => {
    while (next < programs.length) results.push(await run(programs[next++]));
  }),
);
const total = Number(process.hrtime.bigint() - started) / 1e9;

results.sort((a, b) => b.seconds - a.seconds);
const tests = results.reduce((sum, r) => sum + r.passed, 0);
let failed = results.some((r) => r.failed);
console.log("slowest test programs:");
for (const r of results.slice(0, 8)) {
  const over = r.seconds > programBudget ? "  OVER BUDGET" : "";
  console.log(`  ${r.seconds.toFixed(1).padStart(6)} s  ${String(r.passed).padStart(4)} tests  ${r.crate}${over}`);
}
console.log(
  `${tests} tests in ${results.length} programs, ${total.toFixed(1)} s, ${jobs} at a time ` +
    `(budget ${programBudget} s a program, ${totalBudget} s in all)`,
);

const overProgram = results.filter((r) => r.seconds > programBudget);
if (overProgram.length) {
  failed = true;
  console.error(
    `over budget: ${overProgram.map((r) => r.crate).join(", ")}; make the slow tests ` +
      "smaller, or mark the ones that need a big model #[ignore] with the reason",
  );
}
if (total > totalBudget) {
  failed = true;
  console.error(`the suite took ${total.toFixed(1)} s, over its ${totalBudget} s budget`);
}
process.exit(failed ? 1 : 0);
