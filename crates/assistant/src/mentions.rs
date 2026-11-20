//! `@` mentions in a chat's messages: an open file, the selection or a
//! note, named in the text by a token (`@bracket.prtcad`, `@selection`,
//! `@#3`) that the
//! chat draws as a chip, and sent to the agent with what it names as a
//! block of its own after the message:
//! `<printcad:mention>{"token": …, "kind": …, …}</printcad:mention>`.
//! The block is taken when the mention is picked, so a selection is what
//! was selected then. A replayed conversation brings the blocks back with
//! the message, so a chat restored from its file shows its chips again.
//!
//! A new kind of mention is a variant of [`MentionSource`]: what the list
//! offers for it and what its block says are the app's to answer.

use serde_json::{Map, Value, json};
use uuid::Uuid;

/// What a mention names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MentionSource {
    /// The document of an open tab.
    File(Uuid),
    /// What is selected in a tab.
    Selection(Uuid),
    /// A note of a tab's document.
    Note(Uuid, core_document::NoteId),
}

impl MentionSource {
    /// The block's `kind`.
    pub fn kind(self) -> &'static str {
        match self {
            MentionSource::File(_) => "file",
            MentionSource::Selection(_) => "selection",
            MentionSource::Note(..) => "note",
        }
    }

    /// Whether each pick takes it anew, so a second one in a message is a
    /// mention of its own (a selection then and a selection now).
    pub fn taken_anew(self) -> bool {
        matches!(self, MentionSource::Selection(_))
    }
}

/// An entry of the `@` list: its token and what the list says beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct MentionOption {
    pub source: MentionSource,
    pub token: String,
    pub detail: String,
}

/// A mention as a message carries it: the token its text holds and the
/// block that goes to the agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Mention {
    pub token: String,
    pub kind: String,
    /// The whole block, markers and all.
    pub block: String,
}

const OPEN: &str = "<printcad:mention>";
const CLOSE: &str = "</printcad:mention>";

/// The mention `token` of `kind` with `body` as its block. Numbers are
/// rounded to a ten-thousandth of a millimetre, and `<` is escaped so no
/// name in it can end the block early.
pub fn mention(token: &str, kind: &str, body: Map<String, Value>) -> Mention {
    let mut fields = Map::new();
    fields.insert("token".into(), json!(token));
    fields.insert("kind".into(), json!(kind));
    fields.extend(body);
    let text = serde_json::to_string(&rounded(Value::Object(fields)))
        .unwrap_or_default()
        .replace('<', "\\u003c");
    Mention {
        token: token.to_string(),
        kind: kind.to_string(),
        block: format!("{OPEN}{text}{CLOSE}"),
    }
}

fn rounded(value: Value) -> Value {
    match value {
        Value::Number(n) if !n.is_i64() && !n.is_u64() => n
            .as_f64()
            .map(|f| json!((f * 1e4).round() / 1e4))
            .unwrap_or(Value::Number(n)),
        Value::Array(items) => Value::Array(items.into_iter().map(rounded).collect()),
        Value::Object(fields) => {
            Value::Object(fields.into_iter().map(|(k, v)| (k, rounded(v))).collect())
        }
        other => other,
    }
}

/// Take the whole blocks out of `text`, as the mentions they are; a block
/// not yet whole (a replay still streaming) stays.
pub fn take_blocks(text: &mut String) -> Vec<Mention> {
    let mut found = Vec::new();
    while let Some(start) = text.find(OPEN) {
        let Some(len) = text[start..].find(CLOSE) else {
            break;
        };
        let end = start + len + CLOSE.len();
        let block = text[start..end].to_string();
        let body: Value =
            serde_json::from_str(&block[OPEN.len()..block.len() - CLOSE.len()]).unwrap_or_default();
        let field = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_string);
        if let (Some(token), Some(kind)) = (field("token"), field("kind")) {
            found.push(Mention { token, kind, block });
        }
        text.replace_range(start..end, "");
    }
    found
}

/// Where `tokens` stand in `text`, in order: each byte range and the
/// token's index. A token stands alone (not the start of a longer word),
/// and the longest one that fits wins, so `@selection-2` is not
/// `@selection`.
pub fn find_tokens(text: &str, tokens: &[&str]) -> Vec<(std::ops::Range<usize>, usize)> {
    let mut order: Vec<usize> = (0..tokens.len())
        .filter(|i| !tokens[*i].is_empty())
        .collect();
    order.sort_by_key(|i| std::cmp::Reverse(tokens[*i].len()));
    let word = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(offset) = text[at..].find('@') {
        let start = at + offset;
        let opens = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || c == '(');
        let hit = order.iter().copied().find(|i| {
            let end = start + tokens[*i].len();
            opens
                && text[start..].starts_with(tokens[*i])
                && text[end..].chars().next().is_none_or(|c| !word(c))
        });
        match hit {
            Some(i) => {
                let end = start + tokens[i].len();
                out.push((start..end, i));
                at = end;
            }
            None => at = start + 1,
        }
    }
    out
}

/// The mentions whose tokens `text` holds, each once.
pub fn in_text(mentions: &[Mention], text: &str) -> Vec<Mention> {
    let tokens: Vec<&str> = mentions.iter().map(|m| m.token.as_str()).collect();
    let mut kept: Vec<Mention> = Vec::new();
    for (_, i) in find_tokens(text, &tokens) {
        if !kept.iter().any(|m| m.token == mentions[i].token) {
            kept.push(mentions[i].clone());
        }
    }
    kept
}

/// What the `@` list offers of a tab's notes: `@#3` with the start of
/// what each shows, the open ones first, then the resolved; a lost note
/// says so.
pub fn note_options(document: &core_document::Document, tab: Uuid) -> Vec<MentionOption> {
    let mut notes: Vec<(core_document::BodyId, &core_document::Note)> = document.notes().collect();
    notes.sort_by_key(|(_, n)| (n.resolved.is_some(), n.number));
    notes
        .into_iter()
        .map(|(body, note)| {
            let shows = document.note_text(note);
            let first = shows.lines().next().unwrap_or_default();
            let mut detail: String = first.chars().take(48).collect();
            if detail.len() < first.len() {
                detail.push('…');
            }
            if note.asks && note.resolved.is_none() {
                detail.insert_str(0, "? ");
            }
            let state = [
                document.note_placed(body, note).lost.then_some("lost"),
                note.resolved.is_some().then_some("resolved"),
            ];
            let state: Vec<&str> = state.into_iter().flatten().collect();
            if !state.is_empty() {
                detail.push_str(&format!(" ({})", state.join(", ")));
            }
            MentionOption {
                source: MentionSource::Note(tab, note.id),
                token: format!("@#{}", note.number),
                detail,
            }
        })
        .collect()
}

/// Whether `name` calls the tab with this file and title: the file's
/// path or name, or the title.
pub fn calls(name: &str, file: Option<&std::path::Path>, title: &str) -> bool {
    let by_file = file.is_some_and(|f| {
        f.display().to_string() == name || f.file_name().is_some_and(|n| n == name)
    });
    by_file || title == name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_comes_back_out_of_a_replayed_message() {
        let mut fields = Map::new();
        fields.insert("name".into(), json!("a</printcad:mention> trick"));
        fields.insert("point".into(), json!([1.000_000_1_f32, 2.5, -0.3]));
        let sent = mention("@selection", "selection", fields);
        assert!(
            sent.block.contains("[1.0,2.5,-0.3]"),
            "rounded: {}",
            sent.block
        );
        // A replay streams the message and its blocks as pieces, joined.
        let mut text = format!("thicken @selection{}", &sent.block[..20]);
        assert!(take_blocks(&mut text).is_empty(), "not whole yet");
        text.push_str(&sent.block[20..]);
        assert_eq!(take_blocks(&mut text), [sent]);
        assert_eq!(text, "thicken @selection");
    }

    #[test]
    fn a_token_stands_alone_and_the_longest_wins() {
        let tokens = ["@selection", "@selection-2", "@a.prtcad"];
        let found = |text: &str| -> Vec<String> {
            find_tokens(text, &tokens)
                .into_iter()
                .map(|(r, _)| text[r].to_string())
                .collect()
        };
        assert_eq!(
            found("join @selection-2 to @selection, see (@a.prtcad)."),
            ["@selection-2", "@selection", "@a.prtcad"]
        );
        assert!(found("me@selection").is_empty(), "inside a word");
        let file = Some(std::path::Path::new("/parts/bracket.prtcad"));
        for (name, called) in [
            ("/parts/bracket.prtcad", true),
            ("bracket.prtcad", true),
            ("Bracket", true),
            ("bracket", false),
            ("/other/bracket.prtcad", false),
        ] {
            assert_eq!(calls(name, file, "Bracket"), called, "{name}");
        }
        assert!(
            calls("Untitled 2", None, "Untitled 2"),
            "an unsaved tab by its title"
        );
        assert!(
            found("@selections").is_empty(),
            "the start of a longer word"
        );
    }
}
