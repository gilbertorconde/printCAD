import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import test from "node:test";
import { workspaceAt, root } from "./lib/workspace.mjs";
import { selectedGates } from "./gates.mjs";
import { affectedCrates } from "./affected-crates.mjs";

test("metadata and merge base are read once; ownership follows the nearest manifest", () => {
  const directory = resolve("fixture-workspace");
  const pkg = (name, dir) => ({
    id: name, name, manifest_path: join(directory, dir, "Cargo.toml"), dependencies: [],
  });
  const calls = [];
  const api = workspaceAt(directory, (program, args) => {
    calls.push([program, ...args]);
    if (program === "cargo") return JSON.stringify({
      workspace_members: ["host", "nested"],
      packages: [pkg("host", "crates/host"), pkg("nested", "crates/host/nested"), pkg("external", "vendor/external")],
    });
    if (args[0] === "merge-base") return "abc123\n";
    if (args[0] === "diff") return "crates/host/src/deleted.rs\0a file with\na newline.rs\0";
    if (args[0] === "ls-files") return "crates/host/src/new.rs\0";
    throw new Error(`unexpected ${program} ${args}`);
  }, path => path === join(directory, "sdk/guest/Cargo.toml"));
  assert.deepEqual(api.members().map(p => p.name), ["host", "nested"]);
  assert.equal(api.ownerOf("crates/host/src/deleted.rs").name, "host");
  assert.equal(api.ownerOf("crates/host/nested/src/lib.rs").name, "nested");
  assert.equal(api.ownerOf("sdk/guest/src/lib.rs").dir, "sdk/guest");
  assert.equal(api.ownerOf("docs/guide.md"), null);
  assert.equal(api.ownerOf("crates/hostile/src/lib.rs"), null);
  const change = api.changedSince("origin/master");
  assert.equal(change.base, "abc123");
  assert(change.files.has("a file with\na newline.rs"));
  assert(change.files.has("crates/host/src/new.rs"));
  assert.equal(api.changedSince("origin/master"), change);
  api.members();
  assert.equal(calls.filter(c => c[0] === "cargo").length, 1);
  assert.equal(calls.filter(c => c[1] === "merge-base").length, 1);
  assert(calls.some(c => c[1] === "diff" && c.at(-1) === "abc123"));
});

test("affected crates retain host-owned guides and guest package tests", () => {
  assert(affectedCrates(["Cargo.lock"]).everything);
  assert(affectedCrates(["docs/recipes/example.md"]).touched.includes("app_shell"));
  assert.deepEqual(affectedCrates(["sdk/examples/gear/src/lib.rs"]).touched, ["wb_wasm"]);
  const reach = affectedCrates(["crates/local_ipc/src/lib.rs"]);
  assert(reach.affected.includes("doc_server"));
  assert(reach.affected.includes("app_shell"));
});

test("local and CI select the same gate list, including standalone checks", () => {
  const local = selectedGates(["--all"]);
  const lint = selectedGates(["--only", "lint", "--all"]);
  const tests = selectedGates(["--only", "tests", "--all"]);
  assert.deepEqual([...lint, ...tests], local);
  for (const script of ["check-solver-deps.mjs", "check-archive-deps.mjs"]) {
    assert(lint.some(([, , program, args]) => program === "node" && args.includes(`scripts/${script}`)));
  }
  assert.deepEqual(selectedGates(["--only", "tests", "--", "-p", "settings"])[0][3],
    ["scripts/test-budget.mjs", "-p", "settings"]);
  assert.throws(() => selectedGates(["--only", "typo"]));
  const workflow = readFileSync(join(root, ".github/workflows/ci.yml"), "utf8");
  for (const selection of ["lint --all", "clippy", "tests --"]) {
    assert(workflow.includes(`node scripts/gates.mjs --only ${selection}`));
  }
  assert(!workflow.includes("run: cargo clippy"));
  assert(!workflow.includes("run: node scripts/test-budget.mjs"));
});
