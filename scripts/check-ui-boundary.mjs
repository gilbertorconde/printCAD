#!/usr/bin/env node
// The app's two halves meet only through crates/app_shell/src/contract/:
// ui/ draws from it and never names app/, app/ fills it and names nothing
// of ui/ but its entry point, and the contract names neither.
import { readFileSync, readdirSync } from "node:fs";
import { resolve, relative } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const src = resolve(root, "crates/app_shell/src");

// What app/ may name of ui/.
const UI_ENTRY = new Set(["UiLayer"]);

// The code of `text` with comments, strings and character literals
// blanked, so a doc comment or a message naming a module is no reference
// to it.
function code(text) {
  let out = "";
  let i = 0;
  while (i < text.length) {
    const rest = text.slice(i, i + 3);
    if (rest.startsWith("//")) {
      const end = text.indexOf("\n", i);
      i = end < 0 ? text.length : end;
    } else if (rest.startsWith("/*")) {
      const end = text.indexOf("*/", i + 2);
      i = end < 0 ? text.length : end + 2;
      out += " ";
    } else if (/^r#*"/.test(text.slice(i, i + 8)) && !/\w/.test(text[i - 1] ?? "")) {
      const hashes = text.slice(i + 1).match(/^#*/)[0];
      const end = text.indexOf(`"${hashes}`, i + 2 + hashes.length);
      i = end < 0 ? text.length : end + 1 + hashes.length;
      out += '""';
    } else if (text[i] === '"') {
      i += 1;
      while (i < text.length && text[i] !== '"') i += text[i] === "\\" ? 2 : 1;
      i += 1;
      out += '""';
    } else if (/^'(?:\\.[^']*|[^'\\])'/.test(text.slice(i, i + 12))) {
      i = text.indexOf("'", i + 2) + 1;
      out += "' '";
    } else {
      out += text[i];
      i += 1;
    }
  }
  return out;
}

// The names `text` reaches under `module` (`ui`, `app`): each path
// `crate::module::name`, each name of a `crate::module::{...}` list, and
// `*` for the module itself (`use crate::module;`, `crate::{module, ..}`,
// `super::super::module`).
export function reaches(text, module) {
  const body = code(text).replace(/\s+/g, " ");
  const names = [];
  const path = new RegExp(`\\bcrate ?:: ?${module} ?:: ?(\\{[^}]*\\}|\\w+)`, "g");
  for (const m of body.matchAll(path)) {
    if (m[1].startsWith("{")) {
      for (const item of m[1].slice(1, -1).split(",")) {
        const name = item.trim().split(/ ?:: ?| as /)[0];
        if (name) names.push(name);
      }
    } else {
      names.push(m[1]);
    }
  }
  const whole = [
    new RegExp(`\\bcrate ?:: ?${module} ?(?:;|as\\b)`),
    new RegExp(`\\bcrate ?:: ?\\{[^}]*\\b${module}\\b(?! ?::)`),
    new RegExp(`\\bsuper ?:: ?(?:super ?:: ?)+${module}\\b`),
  ];
  if (whole.some(re => re.test(body))) names.push("*");
  return names;
}

function* sources(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) yield* sources(path);
    else if (entry.name.endsWith(".rs")) yield path;
  }
}

function fixtures() {
  assert.deepEqual(reaches("use crate::app::export::ExportDraft;", "app"), ["export"]);
  assert.deepEqual(reaches("use crate::ui::{\n    UiLayer,\n    keymap::Keymap,\n};", "ui"),
    ["UiLayer", "keymap"]);
  assert.deepEqual(reaches("let x = crate::ui::TreeItemId::Body(b);", "ui"), ["TreeItemId"]);
  assert.deepEqual(reaches("use crate::ui;", "ui"), ["*"]);
  assert.deepEqual(reaches("use crate::{Document, PrintCadApp, ui};", "ui"), ["*"]);
  assert.deepEqual(reaches("use super::super::app::marking;", "app"), ["*"]);
  // Comments and strings name nothing; neighbours of the name do not count.
  assert.deepEqual(reaches("/// Drawn by `crate::ui::hud`.\nlet s = \"crate::app::x\";", "ui"), []);
  assert.deepEqual(reaches("/// crate::app::x\nuse crate::application::X; use crate::ui_kit::Y;", "app"), []);
  assert.deepEqual(reaches("use crate::contract::{TreeItemId, UiCommand};", "ui"), []);
  // A quote in a character literal or a raw string opens no string.
  assert.deepEqual(reaches("let q = '\"'; let n = crate::app::scans::x;", "app"), ["scans"]);
  assert.deepEqual(reaches("let r = r#\"a \" b\"#; crate::app::scans::x;", "app"), ["scans"]);
}

function check() {
  fixtures();
  const problems = [];
  const report = (path, names, what) => {
    if (names.length) problems.push(`${relative(root, path)}: ${what} ${[...new Set(names)].join(", ")}`);
  };
  for (const path of sources(resolve(src, "ui"))) {
    report(path, reaches(readFileSync(path, "utf8"), "app"), "ui/ names app/:");
  }
  for (const path of sources(resolve(src, "app"))) {
    const names = reaches(readFileSync(path, "utf8"), "ui").filter(n => !UI_ENTRY.has(n));
    report(path, names, `app/ names ui/ beyond ${[...UI_ENTRY].join(", ")}:`);
  }
  for (const path of sources(resolve(src, "contract"))) {
    const text = readFileSync(path, "utf8");
    report(path, [...reaches(text, "app"), ...reaches(text, "ui")], "the contract names a side:");
  }
  if (problems.length) throw new Error(`ui/app boundary violations:\n${problems.join("\n")}`);
  console.log("ui boundary: ui/, app/ and contract/ keep to their sides; checker fixtures pass");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) check();
