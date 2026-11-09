#!/usr/bin/env node
// The host composes workbenches through the facade and registry, never by
// naming a concrete bench, its kinds or its feature types.
import { readFileSync, readdirSync } from "node:fs";
import { resolve, relative } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const concreteFeatures = /\b(?:Design|Sketch|Surface|Mesh|Joint|Coupling)Feature\b/;
const crates = readdirSync(resolve(root, "crates/workbenches"))
  .filter(name => name.startsWith("wb_"));
const cratePattern = new RegExp(`\\b(?:${crates.join("|")})\\b`);
const kindPattern = new RegExp(`["']wb\\.(?:${["part", ...crates.map(n => n.slice(3))].join("|")})(?:\\.[a-z0-9_]+)*["']`);
const types = crates.flatMap(name => [...sources(resolve(root, "crates/workbenches", name, "src"))]
  .flatMap(path => [...readFileSync(path, "utf8").matchAll(/pub struct (\w+Workbench)\b/g)].map(m => m[1])));
const typePattern = new RegExp(`\\b(?:${types.join("|")})\\b`);

export function bypass(line) {
  return cratePattern.test(line) || kindPattern.test(line) || typePattern.test(line)
    || concreteFeatures.test(line) || line.includes("core.datum");
}

function* sources(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) yield* sources(path);
    else if (entry.name.endsWith(".rs")) yield path;
  }
}

function fixtures() {
  // Directory discovery holds every built-in crate to the same rule.
  for (const name of crates) {
    assert(bypass(`use ${name}::Something;`), name);
    assert(bypass(`"${name.replace("wb_", "wb.")}"`), name);
  }
  for (const type of ["DesignFeature", "SketchFeature", "SurfaceFeature", "MeshFeature",
    "JointFeature", "CouplingFeature", ...types]) {
    assert(bypass(`use workbenches::${type};`), type);
  }
  assert(bypass('"wb.inspect.dimension"'));
  assert(bypass('"wb.part"'));
  assert(bypass('"core.datum"'));
  for (const line of ["use core_document::Workbench;", "use workbenches::PackageStatus;",
    "registry.workbench_mut(&id)", "WorkbenchRuntimeContext", "WorkbenchId",
    "ActiveWorkbench", "HostRequest::SwitchWorkbench", "wb_id", '"wb.test"']) {
    assert(!bypass(line), line);
  }
}

function check() {
  fixtures();
  const problems = [];
  for (const path of sources(resolve(root, "crates/app_shell/src"))) {
    readFileSync(path, "utf8").split("\n").forEach((line, i) => {
      if (bypass(line)) problems.push(`${relative(root, path)}:${i + 1}: ${line.trim()}`);
    });
  }
  // Also catch renamed dependencies: the package/path still identifies
  // a concrete workbench even when its import has a neutral alias.
  const manifest = readFileSync(resolve(root, "crates/app_shell/Cargo.toml"), "utf8");
  manifest.split("\n").forEach((line, i) => {
    if (cratePattern.test(line)) {
      problems.push(`crates/app_shell/Cargo.toml:${i + 1}: ${line.trim()}`);
    }
  });
  if (problems.length) throw new Error(`host/workbench boundary violations:\n${problems.join("\n")}`);
  console.log("workbench seam: source, dependencies and checker fixtures pass");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) check();
