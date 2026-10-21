#!/usr/bin/env node
// The workspace's crates in layers, lowest first: a crate depends only on
// crates of lower layers (its normal and build dependencies; tests may
// reach across). A crate missing from the table fails, so a new crate
// takes its place here when it is added.

import { execFileSync } from "node:child_process";

const LAYERS = [
  // Foundations: nothing of the workspace beneath them.
  ["axes", "feature_walk", "local_ipc", "printcad-kernel-api", "project_archive", "sketch_solver",
   "transform_gizmo", "ui_kit", "viewport_camera"],
  // Contracts and services over the foundations.
  ["printcad-bench-api", "surface_texture", "kernel_ogeom", "render_wgpu", "settings", "agents"],
  // The document.
  ["core_document"],
  // What works on the document: the server, scripting and the benches.
  ["doc_server", "scripting", "wb_sketch", "wb_assembly", "wb_wasm"],
  // Benches built on the sketcher's.
  ["wb_design", "wb_surface"],
  // What composes benches.
  ["bench_fixtures", "project_tools"],
  // The bench facade.
  ["workbenches"],
  // The application.
  ["app_shell"],
];

const layerOf = new Map(LAYERS.flatMap((crates, layer) => crates.map(c => [c, layer])));
const metadata = JSON.parse(
  execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], {
    encoding: "utf8",
    maxBuffer: 64 << 20,
  }),
);
const members = new Set(metadata.packages.map(p => p.name));
const problems = [];
for (const pkg of metadata.packages) {
  const layer = layerOf.get(pkg.name);
  if (layer === undefined) {
    problems.push(`${pkg.name} has no layer: add it to scripts/check-layers.mjs`);
    continue;
  }
  for (const dep of pkg.dependencies) {
    if (!members.has(dep.name) || dep.kind === "dev") continue;
    const below = layerOf.get(dep.name);
    if (below !== undefined && below >= layer) {
      problems.push(
        `${pkg.name} (layer ${layer}) depends on ${dep.name} (layer ${below}), which is not beneath it`,
      );
    }
  }
}
for (const name of layerOf.keys()) {
  if (!members.has(name)) problems.push(`${name} is in a layer but not in the workspace`);
}
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log(`layers: ${members.size} crates, each on crates beneath it`);
