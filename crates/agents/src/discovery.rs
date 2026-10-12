//! Finding a command or a guide by the words of a task: a ranked
//! [`Catalog::search`], a loose [`Catalog::describe`] by key, and a
//! compact [`Catalog::index`] of everything.
//!
//! The module knows no command of its own: the application hands it plain
//! [`Entry`]s (its commands and its documents), so the MCP server, the Lua
//! console's `help` and any other caller rank and resolve the same way.
//!
//! Ranking is BM25 over each entry's words: a command's id (split at dots
//! and underscores), summary, notes and parameter names; a document's
//! title and text. Words are lower-cased and plurals folded, and a query
//! is widened by [`SYNONYMS`], the words users say for what the commands
//! call otherwise.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// What an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Command,
    Document,
}

/// One argument of a command.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    /// Its kind and what it is, as one line: `(number, optional): the length`.
    pub about: String,
}

/// A working use of a command.
#[derive(Debug, Clone, PartialEq)]
pub struct Example {
    /// What it shows, one line.
    pub title: String,
    /// A Lua script.
    pub script: String,
}

/// A command or a document, as the application describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// A command's id (`design.pad`), or a document's (`guide/ai#rules`).
    pub id: String,
    pub kind: Kind,
    /// What the index lists it under: a command's prefix, a document's set.
    pub group: String,
    /// One line: a command's summary, a document's title.
    pub summary: String,
    pub params: Vec<Param>,
    /// The arguments a command takes beyond `params`, in words; empty
    /// when none.
    pub other_args: String,
    /// What a command answers, in words; empty when nothing.
    pub returns: String,
    pub notes: Vec<String>,
    pub examples: Vec<Example>,
    /// Ids of entries that do the related thing.
    pub see_also: Vec<String>,
    /// Lines `describe` shows as they are, such as what an agent may do
    /// with the command.
    pub details: Vec<String>,
    /// A document's text.
    pub text: String,
}

impl Entry {
    /// A command, listed under the part of its id before the first dot.
    pub fn command(id: impl Into<String>, summary: impl Into<String>) -> Self {
        let id = id.into();
        let group = id.split('.').next().unwrap_or_default().to_string();
        Self {
            id,
            kind: Kind::Command,
            group,
            summary: summary.into(),
            params: Vec::new(),
            other_args: String::new(),
            returns: String::new(),
            notes: Vec::new(),
            examples: Vec::new(),
            see_also: Vec::new(),
            details: Vec::new(),
            text: String::new(),
        }
    }

    /// A document (a guide's section, a recipe) under `group`.
    pub fn document(
        id: impl Into<String>,
        group: impl Into<String>,
        title: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            kind: Kind::Document,
            group: group.into(),
            text: text.into(),
            ..Self::command(id, title)
        }
    }

    /// The last part of the id: `fillet` of `design.fillet`.
    fn name(&self) -> &str {
        self.id.rsplit(['.', '/', '#']).next().unwrap_or(&self.id)
    }

    /// The whole entry, as `describe` gives it.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let kind = match self.kind {
            Kind::Command => "command",
            Kind::Document => "document",
        };
        let _ = writeln!(out, "## {} ({kind})\n{}", self.id, self.summary);
        if self.kind == Kind::Document {
            let _ = writeln!(out, "\n{}", self.text.trim());
            return out;
        }
        if !self.params.is_empty() {
            out.push_str("\nArguments:\n");
            for p in &self.params {
                let _ = writeln!(out, "- `{}` {}", p.name, p.about);
            }
        }
        if !self.other_args.is_empty() {
            let _ = writeln!(out, "\nOther arguments: {}", self.other_args);
        }
        if !self.details.is_empty() {
            out.push('\n');
            for line in &self.details {
                let _ = writeln!(out, "{line}");
            }
        }
        if !self.returns.is_empty() {
            let _ = writeln!(out, "\nReturns {}", self.returns);
        }
        if !self.notes.is_empty() {
            out.push_str("\nNotes:\n");
            for note in &self.notes {
                let _ = writeln!(out, "- {note}");
            }
        }
        for example in &self.examples {
            let _ = writeln!(
                out,
                "\nExample: {}\n```lua\n{}\n```",
                example.title,
                example.script.trim()
            );
        }
        if !self.see_also.is_empty() {
            let _ = writeln!(out, "\nSee also: {}", self.see_also.join(", "));
        }
        out
    }

    /// One line: id and summary.
    fn line(&self) -> String {
        format!("{}: {}", self.id, self.summary)
    }
}

/// The words users say, and the words the commands use for the same thing.
/// A query word on the left also searches every word on the right, the
/// first most. Words
/// are as [`words`] makes them (lower case, plurals folded). Each line here
/// is pinned by a ranking test: a synonym that ranks the wrong command
/// first is worse than none.
pub const SYNONYMS: &[(&[&str], &[&str])] = &[
    (&["hole", "bore", "drill"], &["hole", "pocket"]),
    (&["round", "blend", "rounding"], &["fillet"]),
    (&["bevel"], &["chamfer"]),
    (&["extrude", "extrusion"], &["pad", "extrude"]),
    (&["cut"], &["pocket", "groove", "boolean"]),
    (&["shell", "hollow"], &["thickness"]),
    (&["mirror", "reflect"], &["mirror"]),
    (&["array", "repeat"], &["pattern"]),
    (&["revolve", "lathe"], &["revolution", "revolve"]),
    (&["sweep", "pipe"], &["pipe", "sweep"]),
    (
        &["union", "fuse", "unite", "combine"],
        &["boolean", "combine"],
    ),
    (&["join", "merge"], &["boolean", "combine", "sew"]),
    (&["circular", "radial", "circle"], &["polar"]),
    (&["variable", "parameter"], &["var"]),
    (&["constraint", "dimension"], &["constrain", "constraint"]),
    (
        &["engrave", "imprint", "raise"],
        &["deboss", "emboss", "project"],
    ),
];

/// Words too common in a question to say anything about the answer.
const STOP_WORDS: &[&str] = &[
    "an", "and", "are", "as", "at", "be", "by", "do", "for", "from", "how", "in", "into", "is",
    "it", "its", "me", "my", "of", "on", "or", "so", "some", "that", "the", "then", "this", "to",
    "want", "what", "with",
];

/// `word` without its plural ending.
fn fold(word: &str) -> String {
    match word {
        "axes" => return "axis".into(),
        "vertices" => return "vertex".into(),
        _ => {}
    }
    let n = word.len();
    if n <= 3 {
        return word.into();
    }
    if let Some(stem) = word.strip_suffix("ies")
        && n > 4
    {
        return format!("{stem}y");
    }
    for ending in ["sses", "xes", "ches", "shes", "zes"] {
        if word.ends_with(ending) {
            return word[..n - 2].into();
        }
    }
    if ["ss", "us", "is"].iter().any(|e| word.ends_with(e)) {
        return word.into();
    }
    word.strip_suffix('s').unwrap_or(word).into()
}

/// The words of `text` a search compares: lower case, split at anything
/// not a letter or a digit (dots and underscores of ids too), plurals
/// folded, single letters and stop words left out.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(|w| fold(&w.to_lowercase()))
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .collect()
}

/// A word a search looks for, and how much finding it counts.
struct Term {
    word: String,
    weight: f32,
}

/// What a synonym counts for beside a word the query says: a query word
/// that is also a command's name keeps first place over a command a
/// synonym names (`hole` for `design.hole` before `design.pocket`).
const SYNONYM_WEIGHT: f32 = 0.6;

/// What each further word of a synonym line counts for beside the one
/// before it: the first is what the user most likely means.
const LATER_SYNONYM: f32 = 0.85;

/// Verbs a task opens with that say only that something is to be done:
/// they count for less than the thing named, so "draw a circle" finds
/// the circle rather than the drawing tool.
const GENERIC_VERBS: &[&str] = &["add", "create", "draw", "make", "put"];
const GENERIC_WEIGHT: f32 = 0.3;

/// A query's words with their synonyms, each once, at its best weight.
fn query_terms(query: &str) -> Vec<Term> {
    let mut terms: Vec<Term> = Vec::new();
    let mut add = |word: &str, weight: f32| match terms.iter_mut().find(|t| t.word == word) {
        Some(t) => t.weight = t.weight.max(weight),
        None => terms.push(Term {
            word: word.to_string(),
            weight,
        }),
    };
    for word in words(query) {
        let own = if GENERIC_VERBS.contains(&word.as_str()) {
            GENERIC_WEIGHT
        } else {
            1.0
        };
        add(&word, own);
        for (said, meant) in SYNONYMS {
            if said.contains(&word.as_str()) {
                let mut weight = SYNONYM_WEIGHT;
                for m in *meant {
                    add(m, weight);
                    weight *= LATER_SYNONYM;
                }
            }
        }
    }
    terms
}

/// How many times each field's words count: an id names what the entry
/// is, a summary says it, the rest mentions.
const ID_WEIGHT: f32 = 3.0;
const SUMMARY_WEIGHT: f32 = 2.0;
const OTHER_WEIGHT: f32 = 1.0;
/// What a note's words weigh: a match in one counts less than in the
/// summary, and the note does not make its entry longer, so a well-noted
/// command ranks by what it is, not by how much is said of it.
const NOTE_WEIGHT: f32 = 1.0;
/// A guide or recipe ranks below a command that matches as well: the
/// command is what a task runs, the guide what it reads after.
const DOCUMENT_FACTOR: f32 = 0.75;
/// What a query word naming an entry outright (`fillet` for
/// `design.fillet`) adds, in that word's weight.
const NAME_BONUS: f32 = 1.5;
/// BM25's term saturation and length normalisation.
const K1: f32 = 1.2;
const B: f32 = 0.75;

/// One entry's words, weighed.
struct Bag {
    counts: HashMap<String, f32>,
    /// The words outside the notes, which are what rarity counts: a word
    /// many notes mention stays as telling as the entries' own make it.
    own: HashSet<String>,
    length: f32,
}

impl Bag {
    fn of(entry: &Entry) -> Self {
        let mut bag = Bag {
            counts: HashMap::new(),
            own: HashSet::new(),
            length: 0.0,
        };
        bag.add(&entry.id, ID_WEIGHT);
        bag.add(&entry.summary, SUMMARY_WEIGHT);
        for p in &entry.params {
            bag.add(&p.name, OTHER_WEIGHT);
        }
        bag.add(&entry.other_args, OTHER_WEIGHT);
        bag.add(&entry.returns, OTHER_WEIGHT);
        for note in &entry.notes {
            bag.add_unmeasured(note, NOTE_WEIGHT);
        }
        bag.add(&entry.text, OTHER_WEIGHT);
        bag
    }

    fn add(&mut self, text: &str, weight: f32) {
        for w in words(text) {
            *self.counts.entry(w.clone()).or_default() += weight;
            self.own.insert(w);
            self.length += weight;
        }
    }

    /// Words that match without making the entry longer: a command's
    /// notes, which would otherwise lower its rank by being written.
    fn add_unmeasured(&mut self, text: &str, weight: f32) {
        for w in words(text) {
            *self.counts.entry(w).or_default() += weight;
        }
    }
}

/// One entry a query found.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit<'a> {
    pub entry: &'a Entry,
    pub score: f32,
}

/// What one query found, best first.
#[derive(Debug, Clone, PartialEq)]
pub struct Found<'a> {
    pub query: String,
    pub hits: Vec<Hit<'a>>,
}

/// What a key of `describe` resolved to.
#[derive(Debug, Clone, PartialEq)]
pub enum Described<'a> {
    /// The one entry it names.
    Entry(&'a Entry),
    /// A bare name or a group: the entries it could mean.
    Choices {
        key: String,
        entries: Vec<&'a Entry>,
    },
    /// Nothing by that key; the nearest ids, or what a search of its
    /// words finds.
    NotFound {
        key: String,
        nearest: Vec<&'a Entry>,
    },
}

/// How many hits a query answers unless asked otherwise.
pub const DEFAULT_LIMIT: usize = 8;

/// Past this many bytes the index lists only the groups and their sizes,
/// leaving the rest to `search` (about 10,000 tokens).
pub const INDEX_BUDGET: usize = 40_000;

/// The entries, and their words ready to rank.
pub struct Catalog {
    entries: Vec<Entry>,
    bags: Vec<Bag>,
    /// In how many entries each word appears.
    spread: HashMap<String, usize>,
    mean_length: f32,
}

impl Catalog {
    pub fn new(entries: Vec<Entry>) -> Self {
        let bags: Vec<Bag> = entries.iter().map(Bag::of).collect();
        let mut spread: HashMap<String, usize> = HashMap::new();
        for bag in &bags {
            for word in &bag.own {
                *spread.entry(word.clone()).or_default() += 1;
            }
        }
        let mean_length =
            (bags.iter().map(|b| b.length).sum::<f32>() / bags.len().max(1) as f32).max(1.0);
        Self {
            entries,
            bags,
            spread,
            mean_length,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// How much finding `term` says: rare words say more.
    fn rarity(&self, term: &str) -> f32 {
        let n = self.entries.len() as f32;
        let df = self.spread.get(term).copied().unwrap_or(0) as f32;
        (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
    }

    fn score(&self, at: usize, terms: &[Term]) -> f32 {
        let bag = &self.bags[at];
        let norm = K1 * (1.0 - B + B * bag.length / self.mean_length);
        let name = self.entries[at].name();
        let whole = fold(name);
        let name_words: Vec<String> = name.split('_').map(fold).collect();
        let mut score = 0.0;
        for Term { word, weight } in terms {
            let rarity = self.rarity(word) * weight;
            if let Some(&tf) = bag.counts.get(word) {
                score += rarity * tf * (K1 + 1.0) / (tf + norm);
            }
            // A name said whole, or begun (`rect` of `rectangle`).
            let names = *word == whole
                || name == word
                || (whole.len() >= 4 && word.len() > whole.len() && word.starts_with(&whole));
            if names {
                score += rarity * NAME_BONUS;
            } else if name_words.len() > 1 && name_words.contains(word) {
                score += rarity * NAME_BONUS / 2.0;
            }
        }
        if self.entries[at].kind == Kind::Document {
            score *= DOCUMENT_FACTOR;
        }
        score
    }

    /// For each query, the `limit` entries it matches best, best first;
    /// one group per query, so a broad word cannot crowd out the rest.
    pub fn search<S: AsRef<str>>(&self, queries: &[S], limit: usize) -> Vec<Found<'_>> {
        queries
            .iter()
            .map(|query| {
                let terms = query_terms(query.as_ref());
                let mut hits: Vec<Hit> = (0..self.entries.len())
                    .map(|at| Hit {
                        entry: &self.entries[at],
                        score: self.score(at, &terms),
                    })
                    .filter(|h| h.score > 0.0)
                    .collect();
                hits.sort_by(|a, b| b.score.total_cmp(&a.score));
                hits.truncate(limit);
                Found {
                    query: query.as_ref().to_string(),
                    hits,
                }
            })
            .collect()
    }

    /// Each key resolved loosely: an exact id, a bare name or a group
    /// (the entries it could mean), else the nearest ids.
    pub fn describe<S: AsRef<str>>(&self, keys: &[S]) -> Vec<Described<'_>> {
        keys.iter().map(|key| self.resolve(key.as_ref())).collect()
    }

    fn resolve(&self, key: &str) -> Described<'_> {
        let raw = key.trim();
        let wanted = raw.strip_prefix("pc.").unwrap_or(raw).to_lowercase();
        if let Some(entry) = self.entries.iter().find(|e| e.id.to_lowercase() == wanted) {
            return Described::Entry(entry);
        }
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| e.kind == Kind::Document && e.summary.to_lowercase() == wanted)
        {
            return Described::Entry(entry);
        }
        let group = wanted.trim_end_matches('.');
        let in_group: Vec<&Entry> = self.entries.iter().filter(|e| e.group == group).collect();
        if !in_group.is_empty() {
            return Described::Choices {
                key: raw.to_string(),
                entries: in_group,
            };
        }
        let folded = fold(&wanted);
        let mut named: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.name() == wanted || fold(e.name()) == folded)
            .collect();
        if named.is_empty() {
            named = self
                .entries
                .iter()
                .filter(|e| e.kind == Kind::Command)
                .filter(|e| e.name().split('_').any(|w| fold(w) == folded))
                .collect();
        }
        match named.len() {
            1 => return Described::Entry(named[0]),
            0 => {}
            _ => {
                return Described::Choices {
                    key: raw.to_string(),
                    entries: named,
                };
            }
        }
        let reach = if wanted.chars().count() >= 5 { 2 } else { 1 };
        let mut near: Vec<(usize, &Entry)> = self
            .entries
            .iter()
            .filter_map(|e| {
                let d = distance(&wanted, &e.id.to_lowercase())
                    .min(distance(&wanted, &e.name().to_lowercase()));
                (d <= reach).then_some((d, e))
            })
            .collect();
        near.sort_by_key(|(d, e)| (*d, e.id.len()));
        let mut nearest: Vec<&Entry> = near.into_iter().take(5).map(|(_, e)| e).collect();
        if nearest.is_empty() {
            nearest = self
                .search(&[raw], 3)
                .remove(0)
                .hits
                .into_iter()
                .map(|h| h.entry)
                .collect();
        }
        Described::NotFound {
            key: raw.to_string(),
            nearest,
        }
    }

    /// One line per entry, grouped: every command an agent can run and
    /// every document, small enough to read in full. Past
    /// [`INDEX_BUDGET`] only the groups and their sizes.
    pub fn index(&self) -> String {
        let mut groups: Vec<&str> = Vec::new();
        for e in &self.entries {
            if !groups.contains(&e.group.as_str()) {
                groups.push(&e.group);
            }
        }
        let mut out = String::new();
        for g in &groups {
            let _ = writeln!(out, "[{g}]");
            for e in self.entries.iter().filter(|e| e.group == *g) {
                let _ = writeln!(out, "{}", e.line());
            }
        }
        if out.len() <= INDEX_BUDGET {
            return out;
        }
        let mut out = String::new();
        for g in &groups {
            let _ = writeln!(
                out,
                "{g}: {} entries",
                self.entries.iter().filter(|e| e.group == *g).count()
            );
        }
        out
    }
}

/// The search's answer as text: a group per query, a line per hit.
pub fn render_search(found: &[Found]) -> String {
    let mut out = String::new();
    for f in found {
        let _ = writeln!(out, "\"{}\":", f.query);
        if f.hits.is_empty() {
            out.push_str("- nothing found\n");
        }
        for h in &f.hits {
            let _ = writeln!(out, "- {}", h.entry.line());
        }
    }
    out
}

/// What `describe` resolved, as text: whole entries, the choices of a
/// bare name, and the keys not found beside them.
pub fn render_described(described: &[Described]) -> String {
    let mut parts = Vec::new();
    for d in described {
        parts.push(match d {
            Described::Entry(e) => e.render(),
            Described::Choices { key, entries } => {
                let mut out = format!("\"{key}\" could mean (describe one by its id):\n");
                for e in entries {
                    let _ = writeln!(out, "- {}", e.line());
                }
                out
            }
            Described::NotFound { key, nearest } => {
                let mut out = format!("\"{key}\": not found.");
                if nearest.is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(" Nearest:\n");
                    for e in nearest {
                        let _ = writeln!(out, "- {}", e.line());
                    }
                }
                out
            }
        });
    }
    parts.join("\n")
}

/// Edits (insert, delete, change, swap two neighbours) from `a` to `b`.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        let mut fillet = Entry::command("design.fillet", "Round edges");
        fillet.params.push(Param {
            name: "radius".into(),
            about: "(number): how round".into(),
        });
        fillet
            .notes
            .push("Edges are picked by a point on them.".into());
        fillet.see_also.push("design.chamfer".into());
        Catalog::new(vec![
            Entry::command("design.pad", "Pad a sketch"),
            fillet,
            Entry::command("design.chamfer", "Bevel edges"),
            Entry::command(
                "design.linear_pattern",
                "Repeat the last feature along a line",
            ),
            Entry::command("surface.fillet", "Round edges where two faces meet"),
            Entry::command("sketch.line", "Add a line"),
            Entry::document(
                "guide/scripting#recording",
                "guides",
                "Recording",
                "What you do in the window is kept as a script.",
            ),
        ])
    }

    fn first(c: &Catalog, query: &str) -> String {
        c.search(&[query], 3)[0]
            .hits
            .first()
            .map(|h| h.entry.id.clone())
            .unwrap_or_default()
    }

    #[test]
    fn notes_naming_a_command_do_not_make_its_name_common() {
        let with = |note: &str| {
            let mut entries = vec![
                Entry::command("design.pocket", "Cut a sketch into the body"),
                Entry::command("design.groove", "Cut a sketch turned about an axis"),
            ];
            for n in 0..10 {
                let mut other = Entry::command(format!("design.other{n}"), "Something else");
                other.notes.push(note.into());
                entries.push(other);
            }
            Catalog::new(entries)
        };
        let plain = with("Its sketch goes in its body.");
        let noted = with("Its sketch goes in the pocket's body.");
        assert_eq!(plain.rarity("pocket"), noted.rarity("pocket"));
        assert_eq!(first(&noted, "pocket"), "design.pocket");
    }

    #[test]
    fn words_fold_plurals_and_split_ids() {
        assert_eq!(
            words("design.linear_pattern"),
            ["design", "linear", "pattern"]
        );
        assert_eq!(
            words("The Edges, holes and bodies; boxes"),
            ["edge", "hole", "body", "box"]
        );
        assert_eq!(words("axis axes class"), ["axis", "axis", "class"]);
    }

    #[test]
    fn a_query_finds_by_id_summary_synonym_and_document_text() {
        let c = catalog();
        assert_eq!(first(&c, "pad"), "design.pad");
        assert_eq!(first(&c, "round the edges"), "design.fillet");
        assert_eq!(first(&c, "bevel"), "design.chamfer");
        assert_eq!(first(&c, "array"), "design.linear_pattern");
        assert_eq!(first(&c, "recording"), "guide/scripting#recording");
        assert!(c.search(&["xyzzy"], 3)[0].hits.is_empty());
        // One group per query, in order.
        let found = c.search(&["pad", "bevel"], 3);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].query, "bevel");
        let text = render_search(&found);
        assert!(
            text.contains("\"bevel\":\n- design.chamfer: Bevel edges"),
            "{text}"
        );
    }

    #[test]
    fn describe_resolves_ids_bare_names_groups_and_typos() {
        let c = catalog();
        let d = c.describe(&[
            "pc.design.fillet",
            "fillet",
            "filet",
            "chamfr",
            "surface",
            "zzz",
        ]);
        assert!(matches!(d[0], Described::Entry(e) if e.id == "design.fillet"));
        let Described::Choices { entries, .. } = &d[1] else {
            panic!("{:?}", d[1])
        };
        assert_eq!(entries.len(), 2);
        let Described::NotFound { nearest, .. } = &d[2] else {
            panic!("{:?}", d[2])
        };
        assert!(nearest.iter().any(|e| e.id == "design.fillet"));
        let Described::NotFound { nearest, .. } = &d[3] else {
            panic!("{:?}", d[3])
        };
        assert_eq!(nearest[0].id, "design.chamfer");
        assert!(matches!(&d[4], Described::Choices { entries, .. } if entries.len() == 1));
        assert!(matches!(&d[5], Described::NotFound { nearest, .. } if nearest.is_empty()));
        let text = render_described(&d);
        assert!(text.contains("## design.fillet (command)"));
        assert!(text.contains("- `radius` (number): how round"));
        assert!(text.contains("See also: design.chamfer"));
        assert!(text.contains("\"zzz\": not found."));
        // A bare name naming one entry is that entry; part of a name too.
        assert!(matches!(c.describe(&["line"])[0], Described::Entry(e) if e.id == "sketch.line"));
        assert!(matches!(
            c.describe(&["pattern"])[0],
            Described::Entry(e) if e.id == "design.linear_pattern"
        ));
        // A document by its title.
        assert!(
            matches!(c.describe(&["Recording"])[0], Described::Entry(e) if e.kind == Kind::Document)
        );
    }

    #[test]
    fn the_index_lists_every_entry_by_group() {
        let index = catalog().index();
        assert!(index.starts_with("[design]\ndesign.pad: Pad a sketch\n"));
        assert!(index.contains("[guides]\nguide/scripting#recording: Recording\n"));
        assert_eq!(index.lines().count(), 7 + 4);
    }

    #[test]
    fn edits_count_a_swap_as_one() {
        assert_eq!(distance("desing", "design"), 1);
        assert_eq!(distance("filet", "fillet"), 1);
        assert_eq!(distance("", "abc"), 3);
    }
}
