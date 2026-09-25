//! Recording: the commands a session ran through the UI, written as a Lua
//! script that does the same again.
//!
//! Every id a command answers is bound to a name (`pad1`, `rect2`, or a
//! path into its answer, `rect2.elements[3]`), and later calls refer to
//! that name rather than to this session's id, so the script makes its
//! own things when it runs. Ids the recording did not make stay as they
//! are: those things were there before it started.

use std::collections::HashMap;

use core_document::Recorded;
use serde_json::Value;

/// A recording in progress.
#[derive(Debug, Default)]
pub struct Recorder {
    lines: Vec<String>,
    /// What each id the recording made is called in the script.
    names: HashMap<String, String>,
    /// How many names each stem has had, for the next one's number.
    stems: HashMap<String, usize>,
}

impl Recorder {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Add one call.
    pub fn push(&mut self, call: &Recorded) {
        let args = self.lua_args(&call.args);
        let invocation = format!("pc.{}{args}", call.id);
        if ids_in(&call.result).is_empty() {
            self.lines.push(invocation);
            return;
        }
        let name = self.fresh_name(&stem(call));
        self.lines.push(format!("{name} = {invocation}"));
        self.bind(&call.result, name);
    }

    /// The script: a first line that says what it is, then every call.
    pub fn script(&self, about: &str) -> String {
        let mut out = format!("-- {about}\n\n");
        for line in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    fn fresh_name(&mut self, stem: &str) -> String {
        let n = self.stems.entry(stem.to_string()).or_insert(0);
        *n += 1;
        format!("{stem}{n}")
    }

    /// Name every id in `value`, a command's answer, by its path from
    /// `path`.
    fn bind(&mut self, value: &Value, path: String) {
        match value {
            Value::String(s) if is_id(s) => {
                self.names.entry(s.clone()).or_insert(path);
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    self.bind(item, format!("{path}[{}]", i + 1));
                }
            }
            Value::Object(fields) => {
                for (key, item) in fields {
                    if is_identifier(key) {
                        self.bind(item, format!("{path}.{key}"));
                    }
                }
            }
            _ => {}
        }
    }

    fn lua_args(&self, args: &serde_json::Map<String, Value>) -> String {
        let fields: Vec<String> = args
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| format!("{} = {}", key(k), self.lua(v)))
            .collect();
        if fields.is_empty() {
            "()".to_string()
        } else {
            format!("{{{}}}", fields.join(", "))
        }
    }

    fn lua(&self, value: &Value) -> String {
        match value {
            Value::Null => "nil".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => number(n.as_f64().unwrap_or(0.0)),
            Value::String(s) => match self.names.get(s) {
                Some(name) => name.clone(),
                None => quoted(s),
            },
            // A plain `{}` reads back as an empty table of names.
            Value::Array(items) if items.is_empty() => "array()".to_string(),
            Value::Array(items) => {
                let items: Vec<String> = items.iter().map(|v| self.lua(v)).collect();
                format!("{{{}}}", items.join(", "))
            }
            Value::Object(fields) => {
                let fields: Vec<String> = fields
                    .iter()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(k, v)| format!("{} = {}", key(k), self.lua(v)))
                    .collect();
                format!("{{{}}}", fields.join(", "))
            }
        }
    }
}

/// What a call's answer is named after: the thing it made.
fn stem(call: &Recorded) -> String {
    let from_arg = |name: &str| call.args.get(name).and_then(Value::as_str);
    let raw = match call.id.as_str() {
        "sketch.new" | "sketch.import_dxf" => "sketch",
        "doc.new_body" => "body",
        "sketch.draw" => from_arg("tool").unwrap_or("shape"),
        "part.datum" => from_arg("kind").unwrap_or("datum"),
        "sketch.constrain" => from_arg("kind").unwrap_or("constraint"),
        id => id.rsplit('.').next().unwrap_or("result"),
    };
    let clean: String = raw
        .trim_start_matches("sketch.")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    match clean.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => clean,
        _ => format!("r{clean}"),
    }
}

fn ids_in(value: &Value) -> Vec<&str> {
    match value {
        Value::String(s) if is_id(s) => vec![s.as_str()],
        Value::Array(items) => items.iter().flat_map(ids_in).collect(),
        Value::Object(fields) => fields.values().flat_map(ids_in).collect(),
        _ => Vec::new(),
    }
}

fn is_id(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !LUA_KEYWORDS.contains(&s)
}

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

fn key(k: &str) -> String {
    if is_identifier(k) {
        k.to_string()
    } else {
        format!("[{}]", quoted(k))
    }
}

fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A number exactly, in as few digits as read back to it. The document
/// keeps most values in single precision, which a replay has to meet to
/// the last bit (a rounded point can tip a solver's verdict), so a value
/// that is one is written as one: 1.99997, not 1.9999699592590332.
fn number(v: f64) -> String {
    let single = v as f32;
    let text = if f64::from(single) == v {
        format!("{single}")
    } else {
        format!("{v}")
    };
    if text == "-0" { "0".to_string() } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str, args: Value, result: Value) -> Recorded {
        Recorded {
            id: id.to_string(),
            args: args.as_object().unwrap().clone(),
            result,
        }
    }

    const S: &str = "5c0e3a44-0000-4000-8000-000000000001";
    const L1: &str = "5c0e3a44-0000-4000-8000-000000000002";
    const L2: &str = "5c0e3a44-0000-4000-8000-000000000003";
    const PAD: &str = "5c0e3a44-0000-4000-8000-000000000004";
    const OLD: &str = "5c0e3a44-0000-4000-8000-0000000000ff";

    #[test]
    fn ids_a_recording_made_become_names_and_the_rest_stay() {
        let mut rec = Recorder::default();
        rec.push(&call("sketch.new", json!({"plane": "XY"}), json!(S)));
        rec.push(&call(
            "sketch.draw",
            json!({"sketch": S, "tool": "sketch.line", "points": [[0, 0], [f64::from(1.99997f32), 0.1]]}),
            json!({"elements": [L1, L2], "constraints": []}),
        ));
        rec.push(&call(
            "sketch.constrain",
            json!({"sketch": S, "kind": "horizontal", "items": [L2, OLD]}),
            json!([]),
        ));
        rec.push(&call(
            "part.pad",
            json!({"sketch": S, "length": 12.5}),
            json!(PAD),
        ));
        rec.push(&call(
            "part.set",
            json!({"feature": PAD, "reversed": true}),
            json!(null),
        ));
        let script = rec.script("Recorded");
        let expected = format!(
            "-- Recorded\n\n\
             sketch1 = pc.sketch.new{{plane = \"XY\"}}\n\
             line1 = pc.sketch.draw{{points = {{{{0, 0}}, {{1.99997, 0.1}}}}, sketch = sketch1, tool = \"sketch.line\"}}\n\
             pc.sketch.constrain{{items = {{line1.elements[2], \"{OLD}\"}}, kind = \"horizontal\", sketch = sketch1}}\n\
             pad1 = pc.part.pad{{length = 12.5, sketch = sketch1}}\n\
             pc.part.set{{feature = pad1, reversed = true}}\n"
        );
        assert_eq!(script, expected);
    }

    #[test]
    fn the_script_it_writes_is_lua() {
        let mut rec = Recorder::default();
        rec.push(&call(
            "doc.new_body",
            json!({"name": "a \"quoted\" name"}),
            json!(S),
        ));
        rec.push(&call(
            "doc.rename",
            json!({"id": S, "name": "x", "end": 1}),
            json!(null),
        ));
        let lua = mlua::Lua::new();
        lua.load(rec.script("check"))
            .into_function()
            .expect("the recording parses as Lua");
        assert!(rec.script("check").contains("[\"end\"] = 1"));
    }
}
