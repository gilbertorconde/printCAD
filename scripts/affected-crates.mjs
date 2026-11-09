#!/usr/bin/env node
// The crates a change can break: its owners and their transitive dependents.
// With no ref, includes working-tree and untracked changes against HEAD.
// With a ref, compares against its merge base. --json includes the owners
// and their manifests so the test runner can run those programs first.

import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { members, ownerOf, changedSince } from "./lib/workspace.mjs";

export function affectedCrates(changed) {
  const everything = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml"];
  if ([...changed].some(f => everything.includes(f))) {
    return { everything: true, touched: [], touchedManifests: [], affected: [] };
  }
  const packages = members();
  // These files are exercised by a host crate rather than their own package.
  const elsewhere = [["docs/", "app_shell"], ["sdk/", "wb_wasm"]];
  const touched = new Set();
  for (const file of changed) {
    const owner = ownerOf(file);
    if (owner?.name) touched.add(owner.name);
    for (const [prefix, crate] of elsewhere) {
      if (file.startsWith(prefix)) touched.add(crate);
    }
  }
  const affected = new Set(touched);
  let grew = true;
  while (grew) {
    grew = false;
    for (const member of packages) {
      if (!affected.has(member.name) && member.deps.some(d => affected.has(d))) {
        affected.add(member.name);
        grew = true;
      }
    }
  }
  return {
    everything: false,
    touched: [...touched].sort(),
    touchedManifests: packages.filter(m => touched.has(m.name)).map(m => m.manifest),
    affected: [...affected].sort(),
  };
}

export function cargoScope(reach) {
  return reach.everything ? "--workspace" : reach.affected.map(name => `-p ${name}`).join(" ");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const json = args[0] === "--json";
  if (json) args.shift();
  const reach = affectedCrates(changedSince(args[0] ?? "HEAD").files);
  console.log(json ? JSON.stringify(reach) : cargoScope(reach));
}
