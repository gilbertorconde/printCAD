#!/usr/bin/env node
// Resolve a standalone copy so workspace feature unification cannot hide
// a dependency. Traverse normal, build, dev and target edges.

import { execFileSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export function checkStandaloneDependencies({ root, source, name, modes, allowed, runTests = false }) {
  const temporary = mkdtempSync(join(tmpdir(), `printcad-${name}-deps-`));
  const copy = join(temporary, name);
  try {
    const manifest = readFileSync(join(source, "Cargo.toml"), "utf8");
    if (/\.workspace\s*=|workspace\s*=\s*true/.test(manifest)) {
      throw new Error(`${name} manifest inherits workspace configuration`);
    }
    cpSync(source, copy, { recursive: true });
    cpSync(join(root, "Cargo.lock"), join(copy, "Cargo.lock"));
    for (const [mode, features] of modes) {
      const common = ["--manifest-path", join(copy, "Cargo.toml"), "--offline", ...features];
      const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--format-version", "1", ...common], {
        encoding: "utf8", maxBuffer: 1 << 26,
      }));
      const packages = new Map(metadata.packages.map(p => [p.id, p]));
      const nodes = new Map(metadata.resolve.nodes.map(n => [n.id, n]));
      const pending = [metadata.resolve.root];
      const reachable = new Set();
      while (pending.length) {
        const id = pending.pop();
        if (reachable.has(id)) continue;
        reachable.add(id);
        for (const dependency of nodes.get(id).deps) pending.push(dependency.pkg);
      }
      const names = [...reachable].map(id => packages.get(id).name).sort();
      const forbidden = names.filter(dependency => !allowed(mode).has(dependency));
      if (forbidden.length) throw new Error(`${mode}: forbidden ${name} dependencies: ${forbidden.join(", ")}`);
      const tree = execFileSync("cargo", ["tree", "-e", "all", "--target", "all", ...common], { encoding: "utf8" });
      if (!tree.includes(`${name} v`)) throw new Error(`${mode}: cargo tree omitted ${name}`);
      console.log(`${name} dependency gate ${mode}: ${names.length} reachable packages (${names.join(", ")})`);
      if (runTests) execFileSync("cargo", ["test", "--release", ...common], { stdio: "inherit" });
    }
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}
