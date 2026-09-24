//! The user's scripts: every `.lua` file in the scripts folder is a
//! command of the application, `script.<name>`, run from the Scripts
//! menu, the toolbar, the palette or a key bound in Preferences.
//!
//! A script's first comment line, when it has one, says what it does.

use std::path::{Path, PathBuf};

/// One script of the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptEntry {
    /// `script.` and the file's name, lowercased, letters and digits only.
    pub id: String,
    /// The file's name as words.
    pub name: String,
    pub path: PathBuf,
    /// Its first comment line.
    pub about: Option<String>,
}

/// What a new script starts as.
pub const TEMPLATE: &str = "\
-- What this script does, in one line (the menus show it).
-- Every command is a function under pc; help() in the console lists them.

local s = pc.sketch.new{plane = \"XY\"}
pc.sketch.rect{sketch = s, x = -10, y = -10, width = 20, height = 20}
pc.part.pad{sketch = s, length = 10}
pc.doc.rebuild()
";

/// A script of what the console ran, in order.
pub fn from_runs(runs: &[String]) -> String {
    let mut out = String::from("-- Saved from the console\n\n");
    for run in runs {
        out.push_str(run.trim_end());
        out.push('\n');
    }
    out
}

/// The scripts in `dir`, by name. A folder that is not there has none.
pub fn scan(dir: &Path) -> Vec<ScriptEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<ScriptEntry> = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lua")))
        .filter_map(|path| entry(&path))
        .collect();
    out.sort_by_key(|e| e.name.to_lowercase());
    out.dedup_by(|a, b| a.id == b.id);
    out
}

fn entry(path: &Path) -> Option<ScriptEntry> {
    let stem = path.file_stem()?.to_str()?;
    let slug: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let about = std::fs::read_to_string(path).ok().and_then(|text| {
        let first = text.lines().find(|l| !l.trim().is_empty())?;
        let comment = first.trim().strip_prefix("--")?.trim();
        (!comment.is_empty()).then(|| comment.to_string())
    });
    Some(ScriptEntry {
        id: format!("script.{slug}"),
        name: stem.replace(['_', '-'], " "),
        path: path.to_path_buf(),
        about,
    })
}

/// A file name for a new script in `dir` that is not taken yet.
pub fn fresh_path(dir: &Path) -> PathBuf {
    (1..)
        .map(|n| {
            dir.join(if n == 1 {
                "new_script.lua".to_string()
            } else {
                format!("new_script_{n}.lua")
            })
        })
        .find(|p| !p.exists())
        .expect("some name is free")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_s_lua_files_are_its_scripts() {
        let dir = std::env::temp_dir().join(format!("printcad-scripts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Make_Block.lua"),
            "\n-- A block to start from\npc.x()",
        )
        .unwrap();
        std::fs::write(dir.join("plain.lua"), "print(1)").unwrap();
        std::fs::write(dir.join("notes.txt"), "-- not a script").unwrap();
        let found = scan(&dir);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].id, "script.make_block");
        assert_eq!(found[0].name, "Make Block");
        assert_eq!(found[0].about.as_deref(), Some("A block to start from"));
        assert_eq!(found[1].about, None);
        assert_eq!(fresh_path(&dir), dir.join("new_script.lua"));
        let saved = from_runs(&["x = 1".into(), "print(x)\n".into()]);
        assert_eq!(saved, "-- Saved from the console\n\nx = 1\nprint(x)\n");
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(scan(&dir).is_empty(), "a missing folder has none");
    }
}
