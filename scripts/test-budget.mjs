#!/usr/bin/env node
// Run the workspace's tests within a time budget.
//
// Builds every test program (`cargo test --no-run`), then runs them two at
// a time, the tests inside each in parallel as `cargo test` would, so the
// rest run while the longest one finishes its slowest tests. Fails when a
// test fails, when one program takes longer than its budget, or when the
// run as a whole does: a slow test is a cost every build pays, so it is
// caught when it comes in rather than found later. A shared CI runner can
// stall a program for a while, so one that passes but runs over its budget
// is timed again on its own, and only fails if it is over again.
//
//   node scripts/test-budget.mjs                  every crate
//   node scripts/test-budget.mjs -p wb_sketch …   only these packages
//   node scripts/test-budget.mjs --changed        what a change reaches
//   node scripts/test-budget.mjs --heavy [...]    the heavy tests instead
//
// `--changed` asks scripts/affected-crates.mjs what the working tree's
// changes since origin/master reach, and runs the programs of the crates
// changed first, so their failures show before their dependents' do.
//
// The heavy tests (slow or big-model ones, in `heavy` modules a package
// compiles only with its `heavy-tests` feature) stay out of the default
// build; a default run fails if a program holds one. `--heavy`, with any
// of the forms above, builds the packages declaring the feature with it
// on and runs only their heavy tests, against budgets of their own.
//
// TEST_BUDGET_PROGRAM_S (default 60, 300 with --heavy) and
// TEST_BUDGET_TOTAL_S (default 150, 600 with --heavy) set the budgets in
// seconds; a slower machine raises them, the defaults fit a CI runner with
// room to spare. TEST_JOBS (default 2) is how many programs run at once.

import { execFileSync, spawn } from "node:child_process";
import { basename } from "node:path";
import { HEAVY_FEATURE, heavyPackages } from "./lib/workspace.mjs";

let args = process.argv.slice(2);
const heavy = args.includes("--heavy");
args = args.filter((a) => a !== "--heavy");

const programBudget = Number(process.env.TEST_BUDGET_PROGRAM_S ?? (heavy ? 300 : 60));
const totalBudget = Number(process.env.TEST_BUDGET_TOTAL_S ?? (heavy ? 600 : 150));
const jobs = Math.max(1, Number(process.env.TEST_JOBS ?? 2));
// A heavy test's name runs through its module: `<module>::heavy::<test>`.
const HEAVY_TESTS = "::heavy::";

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
let scope = args.length ? args : ["--workspace"];
if (heavy) {
  // Only the packages that have heavy tests, those the scope names among them.
  const named = new Set(args.filter((a, i) => args[i - 1] === "-p"));
  const all = !args.length || args.includes("--workspace");
  const packages = heavyPackages().filter((p) => all || named.has(p));
  if (!packages.length) {
    console.log("no package in scope has heavy tests; nothing to test");
    process.exit(0);
  }
  scope = [
    ...packages.flatMap((p) => ["-p", p]),
    "--features",
    packages.map((p) => `${p}/${HEAVY_FEATURE}`).join(","),
  ];
}

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

// What the default build compiled must hold no heavy test: they cost every
// pull request their build time, run or not.
if (!heavy) {
  const compiled = programs.flatMap((program) =>
    execFileSync(program.path, ["--list", HEAVY_TESTS], { encoding: "utf8" })
      .split("\n")
      .filter((line) => line.endsWith(": test"))
      .map((line) => `${program.crate}: ${line.slice(0, -": test".length)}`),
  );
  if (compiled.length) {
    console.error(
      `heavy tests in the default build (a package turns its ${HEAVY_FEATURE} feature on ` +
        `for another, or a heavy module lost its cfg):\n  ${compiled.join("\n  ")}`,
    );
    process.exit(1);
  }
}

function run(program) {
  return new Promise((resolve) => {
    const started = process.hrtime.bigint();
    const filter = heavy ? [HEAVY_TESTS] : [];
    const child = spawn(program.path, ["-q", ...filter], { stdio: ["ignore", "pipe", "pipe"] });
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
// A heavy run reports the programs that hold heavy tests.
const shown = heavy ? results.filter((r) => r.passed || r.failed) : results;
const tests = results.reduce((sum, r) => sum + r.passed, 0);
let failed = results.some((r) => r.failed);
console.log(heavy ? "heavy test programs:" : "slowest test programs:");
for (const r of shown.slice(0, 8)) {
  const over = r.seconds > programBudget ? "  OVER BUDGET" : "";
  console.log(`  ${r.seconds.toFixed(1).padStart(6)} s  ${String(r.passed).padStart(4)} tests  ${r.crate}${over}`);
}
console.log(
  `${tests} ${heavy ? "heavy " : ""}tests in ${shown.length} programs, ${total.toFixed(1)} s, ${jobs} at a time ` +
    `(budget ${programBudget} s a program, ${totalBudget} s in all)`,
);

// Over its budget once may be a stalled runner; over it again, alone, is
// the program.
const overProgram = [];
for (const r of results.filter((r) => r.seconds > programBudget && !r.failed)) {
  const again = await run(r);
  console.log(`  ${r.crate} again on its own: ${again.seconds.toFixed(1)} s`);
  if (again.failed || again.seconds > programBudget) overProgram.push(again);
}
if (overProgram.length) {
  failed = true;
  console.error(
    `over budget: ${overProgram.map((r) => r.crate).join(", ")}; make the slow tests ` +
      (heavy
        ? "smaller"
        : `smaller, or move the ones that need a big model or a long build into the ` +
          `package's \`heavy\` module under its ${HEAVY_FEATURE} feature`),
  );
}
if (total > totalBudget) {
  failed = true;
  console.error(`the suite took ${total.toFixed(1)} s, over its ${totalBudget} s budget`);
}
// Set rather than exited with: an exit drops what is still on its way
// down a pipe, the summary above with it.
process.exitCode = failed ? 1 : 0;
