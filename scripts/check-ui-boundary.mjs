#!/usr/bin/env node
// The app's two halves meet only through the contract. The UI is the
// app_ui crate's private `ui` module, which app_shell cannot see: it draws
// from `app_ui::contract` and answers with its commands, and the crate's
// root hands out nothing of it but `UiLayer`. What else app_ui holds
// beside the UI (the contract, the keymap, the shared helpers) names
// nothing of it, so nothing of the UI leaks out through them, and
// app_shell names no `app_ui::ui` path.
import { readFileSync, readdirSync } from "node:fs";
import { resolve, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const uiCrate = resolve(root, "crates/app_ui/src");
const appCrate = resolve(root, "crates/app_shell/src");

// What app_ui's root may hand out of its UI.
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

// The names in a `{a, b::c, d as e}` list or a single path segment.
function listed(group) {
  const items = group.startsWith("{") ? group.slice(1, -1).split(",") : [group];
  return items.map(item => item.trim().split(/ ?:: ?| as /)[0]).filter(Boolean);
}

// The names `text` reaches under `module` of `base` (`crate::ui`,
// `app_ui::ui`): each path `base::module::name`, each name of a
// `base::module::{...}` list, and `*` for the module itself
// (`use base::module;`, `base::{module, ..}`, `super::super::module`).
export function reaches(text, module, base = "crate") {
  const body = code(text).replace(/\s+/g, " ");
  const names = [];
  const path = new RegExp(`\\b${base} ?:: ?${module} ?:: ?(\\{[^}]*\\}|\\w+)`, "g");
  for (const m of body.matchAll(path)) names.push(...listed(m[1]));
  const whole = [
    new RegExp(`\\b${base} ?:: ?${module} ?(?:;|as\\b)`),
    new RegExp(`\\b${base} ?:: ?\\{[^}]*\\b${module}\\b(?! ?::)`),
    new RegExp(`\\bsuper ?:: ?(?:super ?:: ?)+${module}\\b`),
  ];
  if (whole.some(re => re.test(body))) names.push("*");
  return names;
}

// What a crate root hands out of its `ui` module: `*` for the module made
// public beyond the crate, else each name of a `pub use ui::...`.
export function uiExports(lib) {
  const body = code(lib).replace(/\s+/g, " ");
  const names = [];
  if (/\bpub mod ui\b/.test(body)) names.push("*");
  for (const m of body.matchAll(/\bpub use (?:self ?:: ?|crate ?:: ?)?ui ?:: ?(\{[^}]*\}|[\w*]+)/g)) {
    names.push(...listed(m[1]));
  }
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
  assert.deepEqual(reaches("use crate::ui::{\n    UiLayer,\n    keymap::Keymap,\n};", "ui"),
    ["UiLayer", "keymap"]);
  assert.deepEqual(reaches("let x = crate::ui::TreeItemId::Body(b);", "ui"), ["TreeItemId"]);
  assert.deepEqual(reaches("use crate::ui;", "ui"), ["*"]);
  assert.deepEqual(reaches("use crate::{Document, PrintCadApp, ui};", "ui"), ["*"]);
  assert.deepEqual(reaches("use super::super::ui::hud;", "ui"), ["*"]);
  assert.deepEqual(reaches("use app_ui::ui::frame::FrameIn;", "ui", "app_ui"), ["frame"]);
  assert.deepEqual(reaches("use app_ui::{UiLayer, contract};", "ui", "app_ui"), []);
  // Comments and strings name nothing; neighbours of the name do not count.
  assert.deepEqual(reaches("/// Drawn by `crate::ui::hud`.\nlet s = \"crate::ui::x\";", "ui"), []);
  assert.deepEqual(reaches("use crate::ui_kit::Y; use crate::uikit::Z;", "ui"), []);
  assert.deepEqual(reaches("use crate::contract::{TreeItemId, UiCommand};", "ui"), []);
  // A quote in a character literal or a raw string opens no string.
  assert.deepEqual(reaches("let q = '\"'; let n = crate::ui::hud::x;", "ui"), ["hud"]);
  assert.deepEqual(reaches("let r = r#\"a \" b\"#; crate::ui::hud::x;", "ui"), ["hud"]);
  assert.deepEqual(uiExports("mod ui;\npub use ui::UiLayer;"), ["UiLayer"]);
  assert.deepEqual(uiExports("pub mod ui;"), ["*"]);
  assert.deepEqual(uiExports("mod ui;\npub use ui::{UiLayer, keymap::Keymap};"), ["UiLayer", "keymap"]);
  assert.deepEqual(uiExports("mod ui;\npub use ui::*;"), ["*"]);
  assert.deepEqual(uiExports("pub(crate) mod ui;\n// pub use ui::Hud;"), []);
}

function check() {
  fixtures();
  const problems = [];
  const report = (path, names, what) => {
    if (names.length) problems.push(`${relative(root, path)}: ${what} ${[...new Set(names)].join(", ")}`);
  };
  const lib = resolve(uiCrate, "lib.rs");
  report(lib, uiExports(readFileSync(lib, "utf8")).filter(n => !UI_ENTRY.has(n)),
    `app_ui hands out of its UI beyond ${[...UI_ENTRY].join(", ")}:`);
  const ui = resolve(uiCrate, "ui") + sep;
  for (const path of sources(uiCrate)) {
    if (path === lib || path.startsWith(ui)) continue;
    report(path, reaches(readFileSync(path, "utf8"), "ui"), "names the UI from beside it:");
  }
  for (const path of sources(appCrate)) {
    report(path, reaches(readFileSync(path, "utf8"), "ui", "app_ui"), "app_shell names app_ui's UI:");
  }
  if (problems.length) throw new Error(`ui/app boundary violations:\n${problems.join("\n")}`);
  console.log("ui boundary: app_ui hands out only UiLayer of its UI, nothing beside the UI names it; checker fixtures pass");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) check();
