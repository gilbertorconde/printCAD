#!/usr/bin/env node
// Resolve a standalone copy so workspace feature unification cannot hide
// a dependency of the solver. Traverse normal, build, dev and target edges.

import { execFileSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const source = process.argv[2] ? resolve(process.argv[2]) : join(root, "crates/sketch_solver");
const temporary = mkdtempSync(join(tmpdir(), "printcad-solver-deps-"));
const copy = join(temporary, "sketch_solver");
const serialization = new Set([
  "sketch_solver", "serde", "serde_core", "serde_derive", "serde_json",
  "proc-macro2", "quote", "syn", "unicode-ident", "itoa", "memchr", "zmij",
]);

try {
  const manifest = readFileSync(join(source, "Cargo.toml"), "utf8");
  if (/\.workspace\s*=|workspace\s*=\s*true/.test(manifest)) {
    throw new Error("solver manifest inherits workspace configuration");
  }
  cpSync(source, copy, { recursive: true });
  cpSync(join(root, "Cargo.lock"), join(copy, "Cargo.lock"));
  for (const [mode, features] of [
    ["default", []], ["no-default", ["--no-default-features"]],
    ["serde", ["--no-default-features", "--features", "serde"]],
    ["all-features", ["--all-features"]],
  ]) {
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
    const allowed = mode === "default" || mode === "no-default"
      ? new Set(["sketch_solver"]) : serialization;
    const forbidden = names.filter(name => !allowed.has(name));
    if (forbidden.length) throw new Error(`${mode}: forbidden solver dependencies: ${forbidden.join(", ")}`);
    const tree = execFileSync("cargo", ["tree", "-e", "all", "--target", "all", ...common], { encoding: "utf8" });
    if (!tree.includes("sketch_solver v")) throw new Error(`${mode}: cargo tree omitted the solver`);
    console.log(`solver dependency gate ${mode}: ${names.length} reachable packages (${names.join(", ")})`);
  }
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
