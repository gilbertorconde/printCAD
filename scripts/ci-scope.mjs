#!/usr/bin/env node
// What a pull request's CI tests, from what it changes against its base.
//
//   node scripts/ci-scope.mjs <base ref>
//
// Writes two outputs (to $GITHUB_OUTPUT when set, else to stdout):
//   tests      the `-p` arguments of the crates the change reaches
//              (scripts/affected-crates.mjs), `--workspace` when it reaches
//              everything (the workspace's manifests or toolchain, the CI
//              itself, the test scripts, vendored crates), empty when it
//              reaches no crate;
//   platforms  `true` when Windows and macOS should test it too: the
//              change reaches everything, touches code that differs by
//              platform (files under `cfg(windows)`, `cfg(unix)`,
//              `target_os` and the like, or the crates that talk to the
//              system), or the pull request carries the `platforms` label
//              (CI_PLATFORMS_LABEL=true).
//
// A push to master tests everything on every platform; this only narrows
// what a pull request waits for.

import { changedSince, root } from "./lib/workspace.mjs";
import { affectedCrates, cargoScope } from "./affected-crates.mjs";
import { join } from "node:path";
import { appendFileSync, existsSync, readFileSync } from "node:fs";

const base = process.argv[2];
if (!base) {
  console.error("usage: node scripts/ci-scope.mjs <base ref>");
  process.exit(2);
}
const changed = [...changedSince(base).files];

// Changes whose reach no crate graph shows: run everything.
const everywhere = [
  ".github/workflows/ci.yml",
  ".cargo/",
  "scripts/affected-crates.mjs",
  "scripts/ci-scope.mjs",
  "scripts/test-budget.mjs",
  "scripts/lib/",
  "scripts/gates.mjs",
  "scripts/workspace.test.mjs",
  "scripts/check-solver-deps.mjs",
  "scripts/check-archive-deps.mjs",
  "scripts/standalone-deps.mjs",
];
const tests = everywhere.some((p) => changed.some((f) => f === p || f.startsWith(p)))
  ? "--workspace"
  : cargoScope(affectedCrates(changed));

// Crates and files that talk to the operating system: sockets, processes,
// paths, the GPU's backends, the 6-DoF device, file dialogs.
const systemPaths = [
  "crates/local_ipc/",
  "crates/doc_server/",
  "crates/settings/",
  "crates/agents/",
  "crates/render_wgpu/",
  "crates/workbenches/wb_wasm/",
  "crates/app_shell/build.rs",
  "crates/app_shell/src/platform",
  "crates/app_shell/src/app/sixdof",
  "crates/app_shell/src/app/mcp.rs",
  "crates/app_shell/src/app/gfx.rs",
  "crates/app_shell/src/app/server.rs",
  "rust-toolchain.toml",
  "Cargo.toml",
  "Cargo.lock",
];
const platformCfg = /cfg!?\((?:[^)]*\b)?(windows|unix|target_os|target_family|target_vendor)\b/;
const differsByPlatform = (file) =>
  systemPaths.some((p) => file === p || file.startsWith(p)) ||
  (file.endsWith(".rs") && existsSync(join(root, file)) && platformCfg.test(readFileSync(join(root, file), "utf8")));
const platforms =
  tests === "--workspace" ||
  process.env.CI_PLATFORMS_LABEL === "true" ||
  changed.some(differsByPlatform);

const out = `tests=${tests}\nplatforms=${platforms}\n`;
if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, out);
process.stdout.write(out);
