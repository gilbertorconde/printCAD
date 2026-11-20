#!/usr/bin/env node
// Vendor the design system's icons into crates/ui_kit/icons and regenerate
// the compile-time table crates/ui_kit/src/icon_table.rs.
//
// Each source SVG is 24×24, 1.5px stroke, `currentColor`. The copy drops
// the embedded provenance metadata, paints the strokes white so a texture
// tints at draw time, and renames or skips a few files: the app's document
// icon carries a neutral name, and icons for features outside the Part
// Design and Sketcher scope are not vendored.
//
// Run: node scripts/vendor-icons.mjs [<source icon dir>]  (default ui-mockup/icons)

import { readFileSync, writeFileSync, readdirSync, mkdirSync, existsSync, statSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL(".", import.meta.url)), "..");
const src = process.argv[2] ?? join(root, "ui-mockup", "icons");
const dest = join(root, "crates", "ui_kit", "icons");
const table = join(root, "crates", "ui_kit", "src", "icon_table.rs");

// The source names its document icon after its own file format; the copy
// carries a neutral name. Exactly one `file-*` icon is expected.
const DOCUMENT_ICON = "file-document";
const SKIP = new Set([
  "migrate", "shape-binder", "sub-shape-binder",
  "switch-virtual-space", "tree-spreadsheet",
  "workbench-spreadsheet",
  // The source draws these with text glyphs, which need a font the
  // rasterizer does not carry; the local copies are pure geometry.
  "expression",
  "dimensional-constraint",
  "constraint-distance-x",
  "constraint-distance-y",
  "constraint-diameter",
  "constraint-radius",
  "delete-all-constraints",
  "show-hide-constraints",
  "auto-constraints",
  "constraint-auto",
  "toggle-driving",
  // The source sets the corner tools' change at the corner, too small
  // to tell a round from a bevel; the local copies draw it mid-leg.
  "sketch-fillet",
  "sketch-chamfer",
]);
// Icons authored in this repo (not part of the source set) that the table
// must keep.
const LOCAL = new Set([
  "pen-path",
  "panel-left-close",
  "panel-left-open",
  "select",
  "expression",
  "dimensional-constraint",
  "constraint-distance-x",
  "constraint-distance-y",
  "constraint-diameter",
  "constraint-radius",
  "delete-all-constraints",
  "show-hide-constraints",
  "auto-constraints",
  "constraint-auto",
  "toggle-driving",
  "join-curves",
  "bspline-degree-up",
  "bspline-degree-down",
  "bspline-convert",
  "external-defining",
  "fit-scan",
  "select-constraints",
  "select-elements",
  "workbench-assembly",
  "joint-mate",
  "joint-align",
  "script",
  "console",
  "constraint-arc-length",
  "constraint-collinear",
  "sketch-text",
  "sketch-fillet",
  "sketch-chamfer",
  "assistant",
  "log",
  "chevrons-down",
  "home",
  "history",
  "workbench-surface",
  "surface-extrude",
  "surface-revolve",
  "surface-planar",
  "surface-fill",
  "surface-ruled",
  "surface-loft",
  "surface-sweep",
  "surface-offset",
  "surface-extend",
  "surface-blend",
  "surface-split",
  "surface-sew",
  "surface-thicken",
  "surface-trim",
  "surface-mirror",
  "surface-continuity",
  "surface-fillet",
  "circular-pattern",
  "point-pattern",
  "path-pattern",
  "offset-solid",
  "separate-pieces",
  "section-sketch",
  "additive-tube",
  "subtractive-tube",
  "bspline-control-polygon",
  "bspline-degree-label",
  "bspline-multiplicity",
  "bspline-weight",
  "text-align-end",
  "text-align-middle",
  "text-align-start",
  "text-path-above",
  "text-path-below",
  "text-path-centred",
  "text-path-fill",
  "text-path-reverse",
  "text-shrink-fit",
  "text-valign-baseline",
  "text-valign-bottom",
  "text-valign-middle",
  "text-valign-top",
  "text-wrap",
  "text-wrap-along",
  "text-wrap-around",
  "text-wrap-turn",
  "thread",
  "rib",
]);

// The source set also carries a handful of 200x200 illustrations of the
// movements a 6-DoF mouse makes. They are drawings rather than icons: they
// keep their own colours, so the copy strips metadata and nothing else, and
// they are named apart from the icon set.
const MOTION_DIR = "motion";
const MOTION_PREFIX = "motion-";

function stripMetadata(svg) {
  return (
    svg
      .replace(/<metadata>.*?<\/metadata>/s, "")
      .replace(/\s+xmlns:c2pa="[^"]*"/, "")
      .replace(/>\s+</g, "><")
      .trim() + "\n"
  );
}

// The whole source set is read before anything in the destination is
// touched: a missing or empty source refuses, leaving the vendored copies
// as they are.
if (!existsSync(src) || !statSync(src).isDirectory()) {
  console.error(`vendor-icons: source folder ${src} does not exist`);
  process.exit(1);
}
const copies = new Map();
for (const file of readdirSync(src).sort()) {
  if (!file.endsWith(".svg")) continue;
  const base = file.slice(0, -4);
  if (SKIP.has(base)) continue;
  const name = base.startsWith("file-") ? DOCUMENT_ICON : base;
  if (name === DOCUMENT_ICON && copies.has(DOCUMENT_ICON)) {
    console.error(`vendor-icons: more than one file-* icon in the source set`);
    process.exit(1);
  }
  const svg = stripMetadata(readFileSync(join(src, file), "utf8"));
  copies.set(name, svg.replace(/currentColor/g, "#FFFFFF"));
}
if (copies.size === 0) {
  console.error(`vendor-icons: no SVG icons in ${src}`);
  process.exit(1);
}
const motionSrc = join(src, MOTION_DIR);
if (existsSync(motionSrc)) {
  for (const file of readdirSync(motionSrc).sort()) {
    if (!file.endsWith(".svg")) continue;
    copies.set(
      MOTION_PREFIX + file.slice(0, -4),
      stripMetadata(readFileSync(join(motionSrc, file), "utf8")),
    );
  }
}
for (const local of LOCAL) {
  if (!existsSync(join(dest, `${local}.svg`))) {
    console.error(`vendor-icons: missing local icon ${local}.svg`);
    process.exit(1);
  }
}

mkdirSync(dest, { recursive: true });
for (const f of readdirSync(dest)) {
  if (f.endsWith(".svg") && !LOCAL.has(f.slice(0, -4))) unlinkSync(join(dest, f));
}
const names = [];
for (const [name, svg] of copies) {
  writeFileSync(join(dest, `${name}.svg`), svg);
  names.push(name);
}
names.push(...LOCAL);
names.sort();

const rows = names.map((n) => `    ("${n}", include_str!("../icons/${n}.svg")),`).join("\n");
writeFileSync(
  table,
  `// Generated by scripts/vendor-icons.mjs — do not edit by hand.\n` +
    `// One entry per file in crates/ui_kit/icons, sorted by name.\n\n` +
    `pub const ICONS: &[(&str, &str)] = &[\n${rows}\n];\n`,
);
console.log(`vendored ${names.length} icons`);
