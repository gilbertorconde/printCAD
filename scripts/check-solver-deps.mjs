#!/usr/bin/env node
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkStandaloneDependencies } from "./standalone-deps.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));
const serialization = new Set([
  "sketch_solver", "serde", "serde_core", "serde_derive", "serde_json",
  "proc-macro2", "quote", "syn", "unicode-ident", "itoa", "memchr", "zmij",
]);
checkStandaloneDependencies({
  root,
  source: process.argv[2] ? resolve(process.argv[2]) : join(root, "crates/sketch_solver"),
  name: "sketch_solver",
  modes: [
    ["default", []], ["no-default", ["--no-default-features"]],
    ["serde", ["--no-default-features", "--features", "serde"]],
    ["all-features", ["--all-features"]],
  ],
  allowed: mode => mode === "default" || mode === "no-default" ? new Set(["sketch_solver"]) : serialization,
});
