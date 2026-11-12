//! Commands' notes, related commands and examples written as markdown, one
//! file per family of commands, kept apart from the specs so the prose
//! reads and edits as text.
//!
//! The file's sections are headed `## id` (or `## id, id`, which several
//! commands share, applied before a section of one of them further down).
//! In a section each paragraph is a note, its lines joined by spaces; a
//! paragraph `See also: `a.b`, `c.d`` names related commands; `### title`
//! followed by a ```` ```lua ```` block is an example. A line of an example
//! that is only `{{name}}` stands for a shared piece of setup: one handed
//! to [`CommandProse::parse`], or one the file defines in a section headed
//! `## {{name}}`: its ```` ```lua ```` block, after any words on what it is. What comes before the
//! first section is the file's own introduction and is left out.

use super::{CommandSpec, Example, dedent};

/// One section's notes, related commands and examples.
#[derive(Debug, Default)]
struct Section {
    ids: Vec<String>,
    notes: Vec<String>,
    see_also: Vec<String>,
    examples: Vec<Example>,
}

/// A family's prose, parsed: [`CommandProse::explain`] adds a command's to
/// its spec.
#[derive(Debug, Default)]
pub struct CommandProse {
    sections: Vec<Section>,
}

impl CommandProse {
    /// Read `text`; `snippets` are the pieces of setup its examples may
    /// name beside the ones it defines. An error names the line it stopped
    /// at.
    pub fn parse(text: &str, snippets: &[(&str, &str)]) -> Result<Self, String> {
        let mut defined: Vec<(String, String)> = snippets
            .iter()
            .map(|(name, text)| ((*name).to_string(), dedent(text)))
            .collect();
        let lines: Vec<&str> = text.lines().collect();
        let mut sections = Vec::new();
        let mut i = lines
            .iter()
            .position(|l| l.starts_with("## "))
            .unwrap_or(lines.len());
        while i < lines.len() {
            let heading = lines[i].trim_start_matches("## ").trim();
            i += 1;
            if let Some(name) = heading
                .strip_prefix("{{")
                .and_then(|h| h.strip_suffix("}}"))
            {
                let start = (i..lines.len())
                    .find(|&j| lines[j].trim() == "```lua" || lines[j].starts_with("## "))
                    .unwrap_or(lines.len());
                let (script, next) = fenced(&lines, start)?;
                defined.push((name.to_string(), dedent(&script)));
                i = skip_blank(&lines, next);
                continue;
            }
            let ids: Vec<String> = heading.split(',').map(|s| s.trim().to_string()).collect();
            if ids.iter().any(|id| id.is_empty() || id.contains(' ')) {
                return Err(format!("line {i}: `## {heading}` names no command"));
            }
            let mut section = Section {
                ids,
                ..Section::default()
            };
            i = skip_blank(&lines, i);
            while i < lines.len() && !lines[i].starts_with("## ") {
                if let Some(title) = lines[i].strip_prefix("### ") {
                    let (script, next) = fenced(&lines, skip_blank(&lines, i + 1))?;
                    section.examples.push(Example {
                        title: title.trim().to_string(),
                        script: dedent(&expand(&script, &defined, i + 1)?),
                    });
                    i = next;
                } else {
                    let end = (i..lines.len())
                        .find(|&j| lines[j].trim().is_empty())
                        .unwrap_or(lines.len());
                    let paragraph = lines[i..end]
                        .iter()
                        .map(|l| l.trim())
                        .collect::<Vec<_>>()
                        .join(" ");
                    match paragraph.strip_prefix("See also: ") {
                        Some(ids) => section.see_also.extend(
                            ids.split(',')
                                .map(|id| id.trim().trim_matches('`').to_string()),
                        ),
                        None => section.notes.push(paragraph),
                    }
                    i = end;
                }
                i = skip_blank(&lines, i);
            }
            sections.push(section);
        }
        Ok(Self { sections })
    }

    /// `text` parsed once into `cell`, for a family's file built in: a
    /// mistake in it is the program's, so it panics, naming `file`.
    pub fn once(
        cell: &'static std::sync::OnceLock<CommandProse>,
        file: &str,
        text: &str,
        snippets: &[(&str, &str)],
    ) -> &'static CommandProse {
        cell.get_or_init(|| Self::parse(text, snippets).unwrap_or_else(|e| panic!("{file}: {e}")))
    }

    /// `spec` with the notes, related commands and examples of every
    /// section naming it added after its own, in the file's order.
    pub fn explain(&self, mut spec: CommandSpec) -> CommandSpec {
        self.add_to(&mut spec);
        spec
    }

    /// [`CommandProse::explain`] in place.
    pub(super) fn add_to(&self, spec: &mut CommandSpec) {
        for section in self.sections.iter().filter(|s| s.ids.contains(&spec.id)) {
            spec.notes.extend(section.notes.iter().cloned());
            spec.see_also.extend(section.see_also.iter().cloned());
            spec.examples.extend(section.examples.iter().cloned());
        }
    }

    /// Every command the file names, in its order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.sections
            .iter()
            .flat_map(|s| s.ids.iter().map(String::as_str))
    }
}

fn skip_blank(lines: &[&str], mut i: usize) -> usize {
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    i
}

/// The ```` ```lua ```` block starting at line `i`, and the line after it.
fn fenced(lines: &[&str], i: usize) -> Result<(String, usize), String> {
    if !lines.get(i).is_some_and(|l| l.trim() == "```lua") {
        return Err(format!("line {}: a ```lua block was expected", i + 1));
    }
    let end = (i + 1..lines.len())
        .find(|&j| lines[j].trim() == "```")
        .ok_or_else(|| format!("line {}: the ```lua block is not closed", i + 1))?;
    Ok((lines[i + 1..end].join("\n"), end + 1))
}

/// `script` with each `{{name}}` line replaced by that snippet.
fn expand(script: &str, snippets: &[(String, String)], at: usize) -> Result<String, String> {
    let mut out = Vec::new();
    for line in script.lines() {
        match line
            .trim()
            .strip_prefix("{{")
            .and_then(|l| l.strip_suffix("}}"))
        {
            Some(name) => {
                let (_, text) = snippets
                    .iter()
                    .find(|(n, _)| n == name)
                    .ok_or_else(|| format!("line {at}: no snippet `{name}`"))?;
                out.push(text.trim_end().to_string());
            }
            None => out.push(line.to_string()),
        }
    }
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared sections go first, paragraphs are notes with their lines
    /// joined, and an example's snippets are spliced in.
    #[test]
    fn a_file_reads_into_notes_related_commands_and_examples() {
        let text = "Intro, left out.\n\n\
            ## {{setup}}\n\nWhat it sets up.\n\n```lua\n  local a = 1\n```\n\n\
            ## x.one, x.two\n\nShared\nnote.\n\nSee also: `x.three`, `x.four`\n\n\
            ## x.one\n\nOwn note.\n\n### Its use\n\n```lua\n{{given}}\n{{setup}}\nassert(a == b)\n```\n";
        let prose = CommandProse::parse(text, &[("given", "\n    local b = 1")]).unwrap();
        let one = prose.explain(CommandSpec::new("x.one", "").note("First."));
        assert_eq!(one.notes, ["First.", "Shared note.", "Own note."]);
        assert_eq!(one.see_also, ["x.three", "x.four"]);
        assert_eq!(one.examples[0].title, "Its use");
        assert_eq!(
            one.examples[0].script,
            "local b = 1\nlocal a = 1\nassert(a == b)\n"
        );
        let two = prose.explain(CommandSpec::new("x.two", ""));
        assert_eq!((two.notes.len(), two.examples.len()), (1, 0));
        assert_eq!(prose.ids().collect::<Vec<_>>(), ["x.one", "x.two", "x.one"]);
        assert!(CommandProse::parse("## x.y\n\n### t\n\nnot lua\n", &[]).is_err());
        assert!(CommandProse::parse("## x.y\n\n### t\n\n```lua\n{{none}}\n```\n", &[]).is_err());
    }
}
