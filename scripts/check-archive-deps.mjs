#!/usr/bin/env node
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkStandaloneDependencies } from "./standalone-deps.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));
const reader = new Set([
  "project_archive", "glam", "roxmltree", "zip", "memchr",
  "displaydoc", "proc-macro2", "quote", "syn", "unicode-ident",
  "flate2", "crc32fast", "cfg-if", "miniz_oxide", "adler2", "simd-adler32",
  "arbitrary", "derive_arbitrary", "crossbeam-utils", "indexmap", "equivalent",
  "hashbrown", "thiserror", "thiserror-impl", "zopfli", "bumpalo", "log",
]);
checkStandaloneDependencies({
  root,
  source: process.argv[2] ? resolve(process.argv[2]) : join(root, "crates/project_archive"),
  name: "project_archive",
  modes: [
    ["default", []], ["no-default", ["--no-default-features"]],
    ["all-features", ["--all-features"]],
  ],
  allowed: () => reader,
  runTests: true,
});
