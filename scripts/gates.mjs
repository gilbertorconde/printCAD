#!/usr/bin/env node
// The gate list shared by local runs and CI. --only selects a group or a
// named gate; arguments after -- go to the budgeted workspace tests.

import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { root } from "./lib/workspace.mjs";

const HEADLESS = [
  "core_document", "wb_sketch", "wb_design", "wb_surface", "wb_assembly", "wb_mesh", "wb_inspect",
  "wb_wasm",
];

export function gateList({ all = false, testArgs } = {}) {
  return [
    ["format", "lint", "cargo", ["fmt", "--all", "--check"]],
    ["tooling", "lint", "node", ["--test", "scripts/workspace.test.mjs"]],
    ["layers", "lint", "node", ["scripts/check-layers.mjs"]],
    ["workbench seam", "lint", "node", ["scripts/check-workbench-seam.mjs"]],
    ["solver dependencies", "lint", "node", ["scripts/check-solver-deps.mjs"]],
    ["archive dependencies", "lint", "node", ["scripts/check-archive-deps.mjs"]],
    ...[
      ["default", []], ["no-default", ["--no-default-features"]], ["serde", ["--features", "serde"]],
    ].map(([name, features]) => [
      `solver ${name}`, "lint", "cargo", ["test", "-p", "sketch_solver", "--locked", ...features],
    ]),
    ...[["--no-default-features"], ["--features", "serde"]].map(features => [
      `solver clippy ${features.join(" ")}`, "lint", "cargo",
      ["clippy", "-p", "sketch_solver", "--all-targets", "--locked", ...features, "--", "-D", "warnings"],
    ]),
    ...["wasm32-unknown-unknown", "wasm32-wasip2"].map(target => [
      `solver ${target}`, "lint", "cargo",
      ["check", "-p", "sketch_solver", "--all-features", "--target", target, "--locked"],
    ]),
    ["clippy", "lint", "cargo", ["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]],
    // Each bench builds without its panels (egui), as its dependents' tests take it.
    ...HEADLESS.map(name => [
      `${name} without egui`, "lint", "cargo",
      ["clippy", "-p", name, "--lib", "--no-default-features", "--", "-D", "warnings"],
    ]),
    ["comments", "lint", "node", ["scripts/lint-comment-rot.mjs", ...(all ? ["--all"] : [])]],
    ["test lines", "lint", "node", ["scripts/test-lines.mjs"]],
    ["tests", "tests", "node", ["scripts/test-budget.mjs", ...(testArgs ?? (all ? [] : ["--changed"]))]],
  ];
}

export function selectedGates(args) {
  const separator = args.indexOf("--");
  const testArgs = separator < 0 ? undefined : args.slice(separator + 1);
  const options = separator < 0 ? [...args] : args.slice(0, separator);
  let only;
  let all = false;
  while (options.length) {
    const option = options.shift();
    if (option === "--all") all = true;
    else if (option === "--only" && options.length) only = options.shift();
    else throw new Error(`unknown gate option: ${option}`);
  }
  const gates = gateList({ all, testArgs }).filter(([name, group]) => !only || only === group || only === name);
  if (!gates.length) throw new Error(`unknown gate selection: ${only}`);
  return gates;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  for (const [name, , program, args] of selectedGates(process.argv.slice(2))) {
    console.log(`\n== ${name}: ${program} ${args.join(" ")}`);
    const run = spawnSync(program, args, { cwd: root, stdio: "inherit" });
    if (run.status !== 0) {
      console.error(`\n${name} failed${run.error ? `: ${run.error.message}` : ""}`);
      process.exit(run.status ?? 1);
    }
  }
  console.log("\nevery selected gate passed");
}
