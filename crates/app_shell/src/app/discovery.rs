//! What an agent searches and describes: every command it can run and the
//! guides, as the entries `agents::discovery` ranks.
//!
//! Commands come from `scripts::command_specs` (the application's own and
//! every bench's), each through `scripting::command_entry`, the same entry
//! the console's `help` searches. Documents are the hand-written sections
//! of the scripting guide, the AI guide's sections, and the recipes under
//! `docs/recipes/`.

use agents::discovery::{Catalog, DEFAULT_LIMIT, Entry, render_described, render_search};
use serde_json::Value;

use crate::app::scripts::command_specs;

const SCRIPTING_GUIDE: &str = include_str!("../../../../docs/SCRIPTING.md");
const AI_GUIDE: &str = include_str!("../../../../docs/AI.md");
/// `(stem, text)` of every `docs/recipes/*.md`, gathered by the build.
const RECIPES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/recipes.rs"));

/// Where the scripting guide's generated reference starts: the entries
/// stand for it, one per command.
const REFERENCE_MARK: &str = "<!-- commands:";

/// What the instructions say of `search`, `describe` and the index.
pub(crate) const ABOUT_INDEX: &str = "\
`search` finds commands and guides by the words of a task (\"round the edges\", \
several queries at once), and `describe` gives one's whole entry (arguments, \
notes, examples) by id, bare name or a near spelling; the index below lists every \
one in a line.";

/// Every command and document, ready to search.
pub(crate) fn catalog(registry: &core_document::DocumentService) -> Catalog {
    let mut entries: Vec<Entry> = command_specs(registry)
        .iter()
        .map(scripting::command_entry)
        .collect();
    entries.extend(documents());
    Catalog::new(entries)
}

/// The guides' sections and the recipes.
fn documents() -> Vec<Entry> {
    let scripting = SCRIPTING_GUIDE
        .split(REFERENCE_MARK)
        .next()
        .unwrap_or_default();
    let mut out = sections("guide/scripting", scripting);
    out.extend(sections("guide/ai", AI_GUIDE));
    for (stem, text) in RECIPES {
        let title = text
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .unwrap_or(stem)
            .trim();
        out.push(Entry::document(
            format!("recipe/{stem}"),
            "recipes",
            title,
            *text,
        ));
    }
    out
}

/// A guide cut at its `## ` headings: the text before the first under the
/// guide's own title (`id`), each section as `id#slug`. A section that
/// is only a heading (the reference's) is left out.
fn sections(id: &str, text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut title = text
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or(id)
        .trim()
        .to_string();
    let mut key = id.to_string();
    let mut body = String::new();
    let mut push = |key: &str, title: &str, body: &str| {
        let body = body.trim();
        if !body.is_empty() {
            out.push(Entry::document(key, "guides", title, body));
        }
    };
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            push(&key, &title, &body);
            title = heading.trim().to_string();
            key = format!("{id}#{}", slug(&title));
            body.clear();
        } else if !line.starts_with("# ") {
            body.push_str(line);
            body.push('\n');
        }
    }
    push(&key, &title, &body);
    out
}

/// A heading as an anchor: lower case, words joined by `-`.
fn slug(heading: &str) -> String {
    heading
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// The words of `args[key]`: a list of strings, or one string.
fn strings(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(one)) => vec![one.clone()],
        _ => Vec::new(),
    }
}

/// The `search` tool's answer: `{queries: [string], limit?}`.
pub(crate) fn search(
    registry: &core_document::DocumentService,
    args: &Value,
) -> Result<String, String> {
    let queries = strings(args, "queries");
    if queries.is_empty() {
        return Err("`search` takes `queries`, a list of what to look for".into());
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_LIMIT, |n| n.clamp(1, 50) as usize);
    Ok(render_search(&catalog(registry).search(&queries, limit)))
}

/// The `describe` tool's answer: `{keys: [string]}`.
pub(crate) fn describe(
    registry: &core_document::DocumentService,
    args: &Value,
) -> Result<String, String> {
    let keys = strings(args, "keys");
    if keys.is_empty() {
        return Err("`describe` takes `keys`, a list of command ids, names or guides".into());
    }
    let catalog = catalog(registry);
    Ok(render_described(&catalog.describe(&keys)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agents::discovery::{Described, Kind};

    fn registry() -> core_document::DocumentService {
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        registry
    }

    /// What users say, and the command that must come first for it. A
    /// query that ranks wrong is a synonym to add, or one that misleads.
    const FIRST: &[(&str, &[&str])] = &[
        ("hole", &["design.hole"]),
        ("drill a bore", &["design.hole"]),
        ("round the edges", &["design.fillet"]),
        ("fillet", &["design.fillet"]),
        ("bevel the corners", &["design.chamfer"]),
        ("shell a box", &["design.thickness"]),
        ("hollow out the part", &["design.thickness"]),
        ("emboss a logo", &["design.project"]),
        ("engrave text on a curved face", &["design.project"]),
        // A solid's and a surface's command both say so.
        ("extrude a sketch", &["design.pad", "surface.extrude"]),
        ("cut a sketch into the body", &["design.pocket"]),
        ("revolve a profile", &["design.revolve", "surface.revolve"]),
        ("lathe", &["design.revolve", "surface.revolve"]),
        ("sweep along a path", &["design.pipe", "surface.sweep"]),
        ("repeat in a circle", &["design.polar_pattern"]),
        ("repeat a feature along a line", &["design.linear_pattern"]),
        ("array elements in rows", &["sketch.array"]),
        ("mirror", &["design.mirror"]),
        ("union two bodies", &["design.boolean"]),
        ("sew", &["surface.sew"]),
        ("sew surfaces into a solid", &["surface.sew"]),
        (
            "make a surface from curves",
            &[
                "surface.extrude",
                "surface.revolve",
                "surface.planar",
                "surface.fill",
                "surface.ruled",
                "surface.loft",
                "surface.sweep",
            ],
        ),
        ("dimension a line", &["sketch.constrain"]),
        ("add a constraint", &["sketch.constrain"]),
        ("draw a rectangle", &["sketch.rect"]),
        ("draw a circle", &["sketch.circle"]),
        ("new sketch", &["sketch.new"]),
        ("measure volume", &["doc.measure"]),
        ("the faces of a body", &["doc.faces"]),
        ("undo", &["edit.undo"]),
        ("variable", &["var.new", "var.set", "var.list"]),
        ("export stl", &["file.export"]),
        ("gear", &["design.gear"]),
    ];

    #[test]
    fn each_query_ranks_its_command_first() {
        let catalog = catalog(&registry());
        let queries: Vec<&str> = FIRST.iter().map(|(q, _)| *q).collect();
        let found = catalog.search(&queries, 5);
        let mut wrong = Vec::new();
        for ((query, wanted), found) in FIRST.iter().zip(&found) {
            let first = found.hits.first().map_or("", |h| h.entry.id.as_str());
            if !wanted.contains(&first) {
                let top: Vec<&str> = found.hits.iter().map(|h| h.entry.id.as_str()).collect();
                wrong.push(format!("{query:?}: wanted {wanted:?}, got {top:?}"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn every_synonym_names_words_the_entries_use() {
        let catalog = catalog(&registry());
        let found: std::collections::HashSet<String> = catalog
            .entries()
            .iter()
            .flat_map(|e| {
                let mut text = format!("{} {} {}", e.id, e.summary, e.text);
                e.params
                    .iter()
                    .for_each(|p| text.push_str(&format!(" {}", p.name)));
                agents::discovery::words(&text)
            })
            .collect();
        for (_, meant) in agents::discovery::SYNONYMS {
            for word in *meant {
                assert!(found.contains(*word), "no entry says {word:?}");
            }
        }
    }

    #[test]
    fn describe_resolves_ids_names_typos_and_reports_what_it_cannot() {
        let catalog = catalog(&registry());
        let d = catalog.describe(&[
            "design.pad",
            "fillet",
            "desing.pockt",
            "doc.mesure",
            "frobnicate",
        ]);
        assert!(matches!(d[0], Described::Entry(e) if e.id == "design.pad"));
        let Described::Choices { entries, .. } = &d[1] else {
            panic!("{:?}", d[1]);
        };
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert!(
            ids.contains(&"design.fillet") && ids.contains(&"surface.fillet"),
            "{ids:?}"
        );
        let Described::NotFound { nearest, .. } = &d[2] else {
            panic!("{:?}", d[2]);
        };
        assert_eq!(nearest[0].id, "design.pocket");
        let Described::NotFound { nearest, .. } = &d[3] else {
            panic!("{:?}", d[3]);
        };
        assert_eq!(nearest[0].id, "doc.measure");
        assert!(matches!(&d[4], Described::NotFound { .. }));
        let text = describe(
            &registry(),
            &serde_json::json!({"keys": ["design.pad", "frobnicate"]}),
        )
        .unwrap();
        assert!(text.contains("## design.pad (command)"), "{text}");
        assert!(text.contains("- `sketch` (id"), "{text}");
        assert!(text.contains("\"frobnicate\": not found."), "{text}");
    }

    #[test]
    fn the_guides_are_entries_and_the_reference_is_not() {
        let catalog = catalog(&registry());
        let docs: Vec<&Entry> = catalog
            .entries()
            .iter()
            .filter(|e| e.kind == Kind::Document)
            .collect();
        assert!(docs.iter().any(|e| e.id == "guide/scripting#recording"));
        assert!(docs.iter().any(|e| e.id == "guide/ai#rules"));
        assert!(docs.iter().all(|e| !e.text.contains("`pc.design.pad`:")));
        assert_eq!(
            docs.iter().filter(|e| e.id.starts_with("recipe/")).count(),
            RECIPES.len()
        );
        let found = catalog.search(&["record what I do as a script"], 3);
        assert_eq!(found[0].hits[0].entry.id, "guide/scripting#recording");
    }

    /// The index goes into every connection's instructions: it stays
    /// small enough to read in full (about 5,000 tokens).
    #[test]
    fn the_index_lists_every_command_in_little_room() {
        let registry = registry();
        let index = catalog(&registry).index();
        for spec in command_specs(&registry) {
            assert!(
                index.contains(&format!("\n{}: ", spec.id)),
                "{} is not in the index",
                spec.id
            );
        }
        assert!(index.len() < 20_000, "the index is {} bytes", index.len());
        assert!(!index.contains('\u{2014}'));
    }

    #[test]
    fn a_search_or_describe_call_says_what_it_looks_for() {
        let label = |title: &str, input: Value| {
            crate::app::agent_context::tool_label(title, Some(&input), &|_| None)
        };
        assert_eq!(
            label(
                "mcp__printcad__search",
                serde_json::json!({"queries": ["hole", "round"]})
            )
            .as_deref(),
            Some("Search for hole, round")
        );
        assert_eq!(
            label("describe", serde_json::json!({"keys": ["design.pad"]})).as_deref(),
            Some("Read about design.pad")
        );
    }

    #[test]
    fn the_tools_refuse_calls_without_their_lists() {
        let registry = registry();
        assert!(search(&registry, &serde_json::json!({})).is_err());
        assert!(describe(&registry, &serde_json::json!({"keys": []})).is_err());
        let text = search(
            &registry,
            &serde_json::json!({"queries": "fillet", "limit": 1}),
        )
        .unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
    }
}
