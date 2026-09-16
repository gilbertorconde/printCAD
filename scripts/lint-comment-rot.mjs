#!/usr/bin/env node
// Comment-rot lint: fails when a comment narrates change over time instead of
// describing present behaviour, or cites a PR / issue / milestone / commit as
// explanation. Both go stale the moment the next change lands, and neither
// survives as documentation once the change it describes is forgotten.
//
// Default mode lints only comment lines ADDED against a base ref, so it gates
// new rot at authoring time without demanding the tree be clean first.
// --staged lints the lines the index adds (the pre-commit hook's mode, reading
// staged content rather than the working tree). --all sweeps the whole tree.
//
// Run: node scripts/lint-comment-rot.mjs [--base <ref>] [--staged] [--all] [--json]
// Exit 0 = clean. Exit 1 = violations.

import { readFileSync, existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join, extname, basename } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL(".", import.meta.url)), "..");

const argv = process.argv.slice(2);
const flag = (n) => argv.includes(n);
const opt = (n, d) => (argv.includes(n) ? argv[argv.indexOf(n) + 1] : d);
const MODE_ALL = flag("--all");
const MODE_STAGED = flag("--staged");
const AS_JSON = flag("--json");
const BASE = opt("--base", process.env.LINT_COMMENT_ROT_BASE ?? "origin/master");

// Narration of change. Each pattern must be specific enough that a comment
// describing present behaviour cannot trip it — a bare "now" is excluded for
// that reason, since "now that the key is derived" is legitimate.
// Tier 1 — gating rules. Every pattern here names a construction that cannot
// describe present behaviour, so a hit is a defect rather than a judgement
// call. Vocabulary that merely CAN signal rot is deliberately excluded: this
// codebase edits documents, so "the removed feature", "no longer visible" and
// "a previously selected body" are domain language, and a rule matching them
// fires on correct comments far more often than on rotten ones. A gate that
// cries wolf gets disabled, which is worse than no gate.
const GATING = [
  // "used to" is history only when narration takes the code as its
  // grammatical subject: a pronoun or relative ("we/it/this/which/that
  // used to …"). "The queue used to copy image data" and "Used to convert
  // from …" are the purpose reading — "employed to" — which no lookbehind
  // separates from history reliably, so noun-subject forms stay advisory.
  [/\b(?:we|it|this|which|that|they|there|you|code|logic)\s+used to\b/i, "used to"],
  [/\bformerly\b/i, "formerly"],
  [/\bnowadays\b/i, "nowadays"],
  [/\bhistorically\b/i, "historically"],
  [/\bwe now\b|\bit now\b|\bthis now\b|\bnow returns\b|\bnow uses\b|\bnow rejects\b|\bnow accepts\b/i, "now <verb>"],
  [/\bbefore (?:the |this )?(?:fix|change|patch|refactor)\b/i, "before the fix"],
  [/\bafter (?:the |this )?(?:fix|change|patch|refactor)\b/i, "after the fix"],
  [/\bsince .{0,40}\b(?:landed|shipped|was added|were added|was introduced)\b/i, "since X landed"],
  [/\bpre-fix\b|\bpost-fix\b|\bpre-#\d{2,6}\b/i, "pre/post-fix"],
  [/\binstead of the old\b|\bthe old \w+ (?:path|call|field|name|behaviour|behavior)\b/i, "the old X"],
  [/\bstays? .{0,30}\bit was\b/i, "stays what it was"],
  [/\bused to be\b/i, "used to be"],
  [/\b(?:redeployed|deployed|landed|shipped|introduced|migrated)\b[^\n]{0,24}\b\d{4}-\d{2}-\d{2}\b/i, "dated event"],
  // Provenance cited as explanation.
  [/(?:^|[\s(\[])#\d{2,6}\b|\b[\w.-]+(?:\/[\w.-]+)?#\d{1,6}\b/, "#NNN issue/PR reference"],
  [/\b(?:PR|pull request|issue|ticket)\s*#?\d{2,6}\b/i, "PR/issue reference"],
  // Require at least one a-f so decimal constants (timestamps, u32 maxima)
  // do not read as abbreviated hashes. Kernel rev pins belong in Cargo.toml
  // and commit messages, never in a comment as the reason for a behaviour.
  [/(?:^|[\s(\[])(?=[0-9a-f]{7,40}(?:[\s).,\]]|$))(?=[0-9]*[a-f])[0-9a-f]{7,40}(?=[\s).,\]]|$)/, "bare commit SHA"],
];

// Tier 2 — advisory only, never gates. These fire on correct comments often
// enough that they are a review aid, not a rule. Enable with --pedantic.
const ADVISORY = [
  [/\bused to\b/i, "used to (noun subject)"],
  [/\bpreviously\b/i, "previously"],
  [/\bno longer\b/i, "no longer"],
  [/\bthe removed\b|\bthat (?:was|were) removed\b/i, "the removed X"],
  [/\bwas (?:renamed|moved|replaced|removed|split|merged) (?:from|to|into|out of)\b/i, "was renamed/moved"],
  [/\brenamed from\b|\bmoved from\b|\breplaced by\b/i, "renamed/moved/replaced"],
  [/\bthe (?:failure|bug|defect|regression) that\b/i, "the failure that …"],
  // A semicolon joining two independent clauses reads better as two sentences.
  // Matching only a following determiner or pronoun keeps the rule off the two
  // shapes where a semicolon earns its place: a list whose items carry commas,
  // and a literal that contains one, such as `text/html; charset=utf-8`.
  // Parenthetical glosses — "(unordered; the CLI sorts by semver)" — still trip
  // it, which is why this advises rather than gates.
  [
    /[a-z0-9)\]]{2};\s+(?:an?|the|it|its|this|that|these|those|they|we|you|there|one|each|every|only|otherwise|so|then|no|not|any|all|both|either|neither|his|her|their|our)\b/i,
    "semicolon joining clauses",
  ],
];

// What this tree holds: Rust crates, GLSL shaders compiled by the renderer's
// build script, TOML manifests, the CI workflow, and this script. Markdown
// is prose rather than comments, SVG icons and STEP fixtures carry no
// commentary worth gating, and `.spv` and `.lock` are generated.
const LANG_BY_EXT = new Map([
  [".rs", "rust"],
  [".vert", "c"],
  [".frag", "c"],
  [".comp", "c"],
  [".glsl", "c"],
  [".mjs", "js"],
  [".js", "js"],
  [".toml", "hash"],
  [".yml", "hash"],
  [".yaml", "hash"],
  [".sh", "hash"],
]);

const SKIP_DIRS = new Set(["target", ".git"]);
// Vendored third-party code is not ours to reword; test data is not prose.
const SKIP_PATH = [/(^|\/)crates\/egui-ash-renderer-vendored\//, /(^|\/)tests\/data\//];

function lang(rel) {
  return LANG_BY_EXT.get(extname(rel)) ?? null;
}

// `#` is a comment only outside a quoted string. YAML doubles a single
// quote to escape it inside single-quoted scalars.
function hashComment(line) {
  if (line.trim().startsWith("#!")) return "";
  let i = 0;
  while (i < line.length) {
    const ch = line[i];
    if (ch === '"' || ch === "'") {
      i++;
      while (i < line.length) {
        if (line[i] === "\\") { i += 2; continue; }
        if (ch === "'" && line[i] === "'" && line[i + 1] === "'") { i += 2; continue; }
        if (line[i] === ch) { i++; break; }
        i++;
      }
      continue;
    }
    if (ch === "#" && (i === 0 || /\s/.test(line[i - 1]))) return line.slice(i + 1);
    i++;
  }
  return "";
}

// Per-language lexical facts for the `//` and `/* */` family. Rust is the
// one that needs care: `'` opens a char literal only when one closes it
// (`'x'`, `'\n'`, `'\u{1F600}'`) and is otherwise a lifetime or a loop
// label; `r"…"`, `r#"…"#` and `br"…"` take no escapes and close on the
// matching hash count; block comments nest. A Rust string literal may span
// lines, so an open one is carried in `state` exactly like an open block
// comment, or a `//` inside a multi-line `format!` template would read as
// a comment. Only the quotes listed in `multiline` carry over: a GLSL
// string or a JS single- or double-quoted string ends with its line, and
// forgetting that lets one stray quote in a regex literal swallow the rest
// of the file.
const SLASH_LANGS = {
  rust: { quotes: ['"'], multiline: ['"'], charLiterals: true, rawStrings: true, nestedBlocks: true },
  c: { quotes: ['"'], multiline: [], charLiterals: false, rawStrings: false, nestedBlocks: false },
  js: { quotes: ['"', "'", "`"], multiline: ["`"], charLiterals: false, rawStrings: false, nestedBlocks: false },
};

const CHAR_LITERAL = /^'(?:\\(?:u\{[0-9a-fA-F]{1,6}\}|x[0-9a-fA-F]{2}|.)|[^'\\])'/;
const RAW_STRING_OPEN = /^b?r(#*)"/;

// Advance past the rest of an open string; -1 when it runs off the line.
function closeString(line, i, str) {
  if (str.rawHashes !== null) {
    const term = '"' + "#".repeat(str.rawHashes);
    const k = line.indexOf(term, i);
    return k === -1 ? -1 : k + term.length;
  }
  while (i < line.length) {
    if (line[i] === "\\") { i += 2; continue; }
    if (line[i] === str.quote) return i + 1;
    i++;
  }
  return -1;
}

function slashComment(line, facts, state) {
  let out = "";
  let i = 0;
  while (i < line.length) {
    if (state.block > 0) {
      const end = line.indexOf("*/", i);
      const open = facts.nestedBlocks ? line.indexOf("/*", i) : -1;
      if (open !== -1 && (end === -1 || open < end)) {
        out += line.slice(i, open);
        state.block++;
        i = open + 2;
        continue;
      }
      if (end === -1) return out + line.slice(i);
      out += line.slice(i, end);
      state.block--;
      i = end + 2;
      continue;
    }
    if (state.str) {
      const close = closeString(line, i, state.str);
      if (close === -1) return out;
      state.str = null;
      i = close;
      continue;
    }
    const ch = line[i];
    if (ch === "/" && line[i + 1] === "/") return out + line.slice(i + 2);
    if (ch === "/" && line[i + 1] === "*") { state.block = 1; i += 2; continue; }
    if (facts.rawStrings && (ch === "r" || ch === "b") && !/[A-Za-z0-9_]/.test(line[i - 1] ?? "")) {
      const raw = RAW_STRING_OPEN.exec(line.slice(i));
      if (raw) {
        state.str = { quote: '"', rawHashes: raw[1].length };
        i += raw[0].length;
        continue;
      }
    }
    if (facts.quotes.includes(ch)) {
      state.str = { quote: ch, rawHashes: null };
      i++;
      continue;
    }
    if (facts.charLiterals && ch === "'") {
      const lit = CHAR_LITERAL.exec(line.slice(i));
      i += lit ? lit[0].length : 1;
      continue;
    }
    i++;
  }
  if (state.str && state.str.rawHashes === null && !facts.multiline.includes(state.str.quote)) {
    state.str = null;
  }
  return out;
}

// The comment text on this line, or "" when there is none. `state` carries
// open block comments and open strings from line to line.
function commentOf(line, l, state) {
  if (l === "hash") return hashComment(line);
  return slashComment(line, SLASH_LANGS[l], state);
}

const PEDANTIC = flag("--pedantic");

// Escape hatch for a comment that must quote the banned shapes — this lint's
// own documentation, or a note reproducing a historical error string verbatim.
const IGNORE = /lint-comment-rot:\s*ignore/i;

function check(text) {
  if (IGNORE.test(text)) return [];
  const hits = [];
  const rules = PEDANTIC ? [...GATING, ...ADVISORY] : GATING;
  for (const [re, label, except] of rules) {
    if (!re.test(text)) continue;
    if (except && except.test(text)) continue;
    hits.push(label);
  }
  return hits;
}

// Tracked files only, so build output and untracked scratch never reach the
// rules.
function tracked() {
  return execFileSync("git", ["ls-files"], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 })
    .split("\n")
    .filter(Boolean);
}

function eligible(rel) {
  if (SKIP_PATH.some((re) => re.test(rel))) return false;
  if (rel.split("/").some((part) => SKIP_DIRS.has(part))) return false;
  return lang(rel) !== null;
}

const violations = [];

if (MODE_ALL) {
  for (const norm of tracked()) {
    if (!eligible(norm) || !existsSync(join(root, norm))) continue;
    const state = { block: 0, str: null };
    readFileSync(join(root, norm), "utf8").split("\n").forEach((line, i) => {
      const c = commentOf(line, lang(norm), state);
      if (!c.trim()) return;
      const hits = check(c);
      if (hits.length) violations.push({ file: norm, line: i + 1, hits, text: line.trim() });
    });
  }
} else {
  // Diff mode: collect the new-side line numbers the diff adds, then parse
  // each complete post-change file. Parsing only zero-context hunk text loses
  // lexical state when an added line sits inside an existing block comment.
  // Staged mode diffs the index and reads post-change content from the index
  // too, so unstaged edits in the working tree neither hide nor cause a hit.
  const git = (args) => execFileSync("git", args, { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
  let diff;
  try {
    diff = MODE_STAGED ? git(["diff", "--cached", "-U0", "--no-renames"]) : git(["diff", "-U0", `${BASE}...HEAD`]);
  } catch {
    console.error(`comment-rot lint: cannot diff against '${BASE}'. Pass --base <ref> or use --all.`);
    process.exit(1);
  }
  const postChange = (rel) => (MODE_STAGED ? git(["show", `:${rel}`]) : readFileSync(join(root, rel), "utf8"));
  const added = new Map();
  let file = null;
  let lineNo = 0;
  for (const raw of diff.split("\n")) {
    if (raw.startsWith("+++ ")) { file = raw.startsWith("+++ b/") ? raw.slice(6) : null; continue; }
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)/.exec(raw);
    if (hunk) { lineNo = Number(hunk[1]); continue; }
    if (raw.startsWith("+") && !raw.startsWith("+++")) {
      if (file) {
        if (!added.has(file)) added.set(file, new Set());
        added.get(file).add(lineNo);
      }
      lineNo++;
    } else if (raw.startsWith(" ")) {
      lineNo++;
    }
  }

  for (const [rel, linesAdded] of added) {
    if (!eligible(rel) || (!MODE_STAGED && !existsSync(join(root, rel)))) continue;
    const state = { block: 0, str: null };
    postChange(rel).split("\n").forEach((line, i) => {
      const c = commentOf(line, lang(rel), state);
      const currentLine = i + 1;
      if (!linesAdded.has(currentLine) || !c.trim()) return;
      const hits = check(c);
      if (hits.length) violations.push({ file: rel, line: currentLine, hits, text: line.trim() });
    });
  }
}

if (AS_JSON) {
  console.log(JSON.stringify(violations, null, 2));
  process.exit(violations.length ? 1 : 0);
}

if (violations.length === 0) {
  console.log(`comment-rot lint: clean — no change-narration or provenance references in ${MODE_ALL ? "tree" : MODE_STAGED ? "staged" : "added"} comments.`);
  process.exit(0);
}

console.error(`comment-rot lint: ${violations.length} violation(s):\n`);
for (const v of violations) {
  console.error(`  ${v.file}:${v.line}  [${[...new Set(v.hits)].join(", ")}]`);
  console.error(`      ${v.text.slice(0, 110)}`);
}
console.error(
  `\nA comment describes what the code does now. Narrating the change ("used to", ` +
    `"no longer", "since X landed") or citing a PR, issue, milestone or commit as ` +
    `explanation goes stale on the next change and reads as history to everyone after. ` +
    `Rewrite it in the present tense, or delete it.`,
);
process.exit(1);
