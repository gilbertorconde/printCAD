import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const here = fileURLToPath(new URL(".", import.meta.url));

// A copy of the script in a scratch tree with the real vendored icons, so a
// run touches nothing in the checkout.
function scratchTree() {
  const tree = mkdtempSync(join(tmpdir(), "vendor-icons-"));
  mkdirSync(join(tree, "scripts"));
  cpSync(join(here, "vendor-icons.mjs"), join(tree, "scripts", "vendor-icons.mjs"));
  cpSync(join(here, "..", "crates", "ui_kit", "icons"), join(tree, "crates", "ui_kit", "icons"), { recursive: true });
  mkdirSync(join(tree, "crates", "ui_kit", "src"));
  writeFileSync(join(tree, "crates", "ui_kit", "src", "icon_table.rs"), "table\n");
  return tree;
}

const run = (tree, src) =>
  spawnSync(process.execPath, [join(tree, "scripts", "vendor-icons.mjs"), src], { encoding: "utf8" });

const snapshot = (tree) => {
  const icons = join(tree, "crates", "ui_kit", "icons");
  return Object.fromEntries(readdirSync(icons).map((f) => [f, readFileSync(join(icons, f), "utf8")]));
};

test("a missing or empty source folder refuses and leaves every vendored icon", (t) => {
  const tree = scratchTree();
  t.after(() => rmSync(tree, { recursive: true, force: true }));
  const before = snapshot(tree);
  const empty = join(tree, "empty");
  mkdirSync(empty);
  writeFileSync(join(empty, "notes.txt"), "no icons here\n");
  for (const src of [join(tree, "missing"), empty]) {
    const result = run(tree, src);
    assert.notEqual(result.status, 0, src);
    assert.match(result.stderr, /vendor-icons:/);
    assert.deepEqual(snapshot(tree), before);
    assert.equal(readFileSync(join(tree, "crates", "ui_kit", "src", "icon_table.rs"), "utf8"), "table\n");
  }
});

test("a source set replaces the vendored copies and keeps the local icons", (t) => {
  const tree = scratchTree();
  t.after(() => rmSync(tree, { recursive: true, force: true }));
  const icons = join(tree, "crates", "ui_kit", "icons");
  writeFileSync(join(icons, "stale.svg"), "<svg/>\n");
  const src = join(tree, "source");
  mkdirSync(src);
  writeFileSync(join(src, "fresh.svg"), '<svg stroke="currentColor"><metadata>x</metadata></svg>');
  const result = run(tree, src);
  assert.equal(result.status, 0, result.stderr);
  const after = readdirSync(icons);
  assert(!after.includes("stale.svg"));
  assert.equal(readFileSync(join(icons, "fresh.svg"), "utf8"), '<svg stroke="#FFFFFF"></svg>\n');
  assert(after.includes("pen-path.svg"));
  const table = readFileSync(join(tree, "crates", "ui_kit", "src", "icon_table.rs"), "utf8");
  assert.match(table, /\("fresh", include_str!/);
  assert.match(table, /\("pen-path", include_str!/);
});
