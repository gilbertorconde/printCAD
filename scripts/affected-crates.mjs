#!/usr/bin/env node
// The crates a change can break: those whose files changed, and every
// workspace crate that depends on them, directly or not. Prints them as
// `-p <crate>` arguments for cargo (or for scripts/test-budget.mjs), or
// `--workspace` when the change reaches everything (the workspace's
// Cargo.toml or Cargo.lock), or nothing when no crate is touched.
//
//   node scripts/affected-crates.mjs            changes against HEAD, untracked files too
//   node scripts/affected-crates.mjs <ref>      changes since the branch left <ref> (origin/master)
//   node scripts/affected-crates.mjs --json …   {everything, touched, touchedManifests, affected}
//
// A change is the working tree against the merge base with <ref>, so what
// <ref> gained since is not counted. The edit-test loop runs it through
// `scripts/test-budget.mjs --changed`; a pull request's CI through
// `scripts/ci-scope.mjs`.

import { execFileSync } from "node:child_process";
import { relative, dirname } from "node:path";

const args = process.argv.slice(2);
const json = args[0] === "--json";
if (json) args.shift();
const ref = args[0] ?? "HEAD";
const git = (...a) => execFileSync("git", a, { encoding: "utf8" });
const root = git("rev-parse", "--show-toplevel").trim();
const base = ref === "HEAD" ? ref : git("merge-base", ref, "HEAD").trim();
const changed = new Set(
  [
    ...git("diff", "--name-only", base).split("\n"),
    ...git("ls-files", "--others", "--exclude-standard").split("\n"),
  ].filter(Boolean),
);

const everything = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml"];
if ([...changed].some((f) => everything.includes(f))) {
  console.log(
    json
      ? JSON.stringify({ everything: true, touched: [], touchedManifests: [], affected: [] })
      : "--workspace",
  );
  process.exit(0);
}

const metadata = JSON.parse(
  execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
    encoding: "utf8",
    maxBuffer: 1 << 28,
  }),
);
const members = metadata.packages.map((p) => ({
  name: p.name,
  manifest: p.manifest_path,
  dir: relative(root, dirname(p.manifest_path)),
  deps: p.dependencies.filter((d) => d.path).map((d) => d.name),
}));

// Files whose test lives in another crate than their own directory says.
const elsewhere = [
  // app_shell's tests check the generated scripting guide, search the
  // guides and run every recipe.
  ["docs/", "app_shell"],
  // The guest SDK and its examples are built by wb_wasm's package tests.
  ["sdk/", "wb_wasm"],
  // The window's event layer, patched in place.
  ["third_party/egui-winit/", "app_shell"],
];

const touched = new Set();
for (const file of changed) {
  // The deepest crate directory holding the file owns it.
  const owner = members
    .filter((m) => file === m.dir || file.startsWith(`${m.dir}/`))
    .sort((a, b) => b.dir.length - a.dir.length)[0];
  if (owner) touched.add(owner.name);
  for (const [prefix, crate] of elsewhere) {
    if (file.startsWith(prefix)) touched.add(crate);
  }
}

// Everything that depends on a touched crate, transitively.
const affected = new Set(touched);
let grew = true;
while (grew) {
  grew = false;
  for (const m of members) {
    if (!affected.has(m.name) && m.deps.some((d) => affected.has(d))) {
      affected.add(m.name);
      grew = true;
    }
  }
}
const names = [...affected].sort();
if (json) {
  console.log(
    JSON.stringify({
      everything: false,
      touched: [...touched].sort(),
      touchedManifests: members.filter((m) => touched.has(m.name)).map((m) => m.manifest),
      affected: names,
    }),
  );
} else {
  console.log(names.map((name) => `-p ${name}`).join(" "));
}
