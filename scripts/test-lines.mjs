#!/usr/bin/env node
// How much of the Rust code is tests, per crate, and a ceiling on the whole.
//
// A line is test code when it sits in a `tests/` directory, in a module
// declared under `#[cfg(test)]` (inline, or the file such a `mod x;`
// names, with every module that file declares), or in an item marked
// `#[cfg(test)]`. The SDK's test packages
// (`sdk/tests/`) are tests too; examples and benches are code.
//
//   node scripts/test-lines.mjs              the table, and the check
//   TEST_LINES_CEILING=0.3 node …            another ceiling
//
// Fails when tests are more than the ceiling's share of all lines, so
// the suite grows by consolidating and replacing rather than piling up.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, basename } from "node:path";

const ceiling = Number(process.env.TEST_LINES_CEILING ?? 0.268);
const root = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
const files = execFileSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", "*.rs"], { cwd: root, encoding: "utf8" })
  .split("\0")
  .filter((f) => f && !f.startsWith("third_party/") && existsSync(join(root, f)));

// The crate a file belongs to: the deepest directory holding a Cargo.toml.
const crateOf = new Map();
function crateDir(file) {
  let dir = dirname(file);
  while (dir !== "." && dir !== "") {
    if (crateOf.has(dir)) return crateOf.get(dir);
    if (existsSync(join(root, dir, "Cargo.toml"))) {
      crateOf.set(dir, dir);
      return dir;
    }
    dir = dirname(dir);
  }
  return ".";
}

// The file a `mod name;` in `file` refers to.
function moduleFile(file, name, pathAttr) {
  const dir = dirname(file);
  if (pathAttr) return join(dir, pathAttr);
  const stem = basename(file, ".rs");
  const base = ["lib", "main", "mod"].includes(stem) ? dir : join(dir, stem);
  const flat = join(base, `${name}.rs`);
  return existsSync(join(root, flat)) ? flat : join(base, name, "mod.rs");
}

const testFiles = new Set(
  files.filter((f) => /(^|\/)tests\//.test(f) && !f.startsWith("sdk/printcad-bench-sdk/")),
);
const counts = new Map(); // file -> { total, test }
const pending = [...files];
const seen = new Set();

function count(file) {
  const lines = readFileSync(join(root, file), "utf8").split("\n");
  const whole = testFiles.has(file);
  let total = 0;
  let test = 0;
  let depth = 0; // brace depth inside a test item, 0 when outside
  let armed = false; // a `#[cfg(test)]` waits for its item
  let pathAttr = null;
  for (const raw of lines) {
    const line = raw.trim();
    total++;
    const pathMatch = /^#\[path\s*=\s*"([^"]+)"\]/.exec(line);
    if (pathMatch) pathAttr = pathMatch[1];
    const modDecl = /^(?:pub(?:\([^)]*\))?\s+)?mod\s+([a-z_0-9]+)\s*;/.exec(line);
    if (modDecl) {
      const target = moduleFile(file, modDecl[1], pathAttr);
      if ((whole || armed || depth > 0) && files.includes(target)) testFiles.add(target);
      pathAttr = null;
    } else if (!line.startsWith("#[")) {
      pathAttr = null;
    }
    if (whole) {
      test++;
      continue;
    }
    if (depth > 0) {
      test++;
      depth += braces(line);
      continue;
    }
    if (/^#\[cfg\(test\)\]/.test(line)) {
      armed = true;
      test++;
      continue;
    }
    if (armed) {
      test++;
      if (line.startsWith("#[")) continue;
      const delta = braces(line);
      if (delta > 0) depth = delta;
      armed = false;
    }
  }
  return { total, test };
}

function braces(line) {
  const code = line.replace(/"(?:\\.|[^"\\])*"/g, "").replace(/'(?:\\.|[^'\\])'/g, "").replace(/\/\/.*$/, "");
  let d = 0;
  for (const c of code) {
    if (c === "{") d++;
    else if (c === "}") d--;
  }
  return d;
}

// Files a test module declares become test files as they are found, so
// count until nothing new turns up.
let changed = true;
while (changed) {
  changed = false;
  for (const file of pending) {
    const key = `${file}:${testFiles.has(file)}`;
    if (seen.has(key)) continue;
    seen.add(key);
    counts.set(file, count(file));
    changed = true;
  }
}

const byCrate = new Map();
for (const [file, { total, test }] of counts) {
  const crate = crateDir(file);
  const c = byCrate.get(crate) ?? { total: 0, test: 0 };
  c.total += total;
  c.test += test;
  byCrate.set(crate, c);
}
const rows = [...byCrate].sort((a, b) => b[1].test - a[1].test);
let total = 0;
let test = 0;
console.log("   tests    lines  share  crate");
for (const [crate, c] of rows) {
  total += c.total;
  test += c.test;
  const share = c.total ? c.test / c.total : 0;
  console.log(
    `${String(c.test).padStart(8)} ${String(c.total).padStart(8)}  ${(share * 100).toFixed(0).padStart(4)}%  ${crate}`,
  );
}
const share = test / total;
console.log(
  `${test} of ${total} lines are tests: ${(share * 100).toFixed(1)}% (ceiling ${(ceiling * 100).toFixed(1)}%)`,
);
if (share > ceiling) {
  console.error(
    "tests are over their share of the code: fold a new case into a table or a " +
      "shared harness, retire what a stronger test already proves, or raise " +
      "TEST_LINES_CEILING in this script with the reason",
  );
  process.exit(1);
}
