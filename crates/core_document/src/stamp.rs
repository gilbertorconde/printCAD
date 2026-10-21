//! What wrote a `.prtcad` container: the application's version and the
//! container's format number, kept as an entry of their own (`meta.json`)
//! beside the document so a reader learns them without parsing the
//! document, and a reader that does not know the entry passes it by.
//!
//! A container without the entry was written before stamps existed, by a
//! version older than any that writes one.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// The archive entry naming what wrote the container.
pub const STAMP_ENTRY: &str = "meta.json";

/// The container's layout and meaning as this build writes it: the
/// entries it holds, how they are encoded, and what the document's fields
/// mean. A field added with a default that keeps old files meaning what
/// they meant needs no new number; a change that makes old data mean
/// something else does, with its step in [`UPGRADES`]. A build refuses a
/// file of a larger number than its own: it would read it wrong.
pub const CONTAINER_FORMAT: u32 = 1;

/// One step bringing a document's JSON from one format to the next.
pub type Upgrade = fn(&mut serde_json::Value) -> Result<(), String>;

/// The steps from each older format to this one: step `n` takes a
/// format-`n + 1` document to format `n + 2`, so there is one for every
/// format below [`CONTAINER_FORMAT`].
pub const UPGRADES: &[Upgrade] = &[];

/// Bring the JSON of a document written in format `from` up to
/// [`CONTAINER_FORMAT`]. A container without a stamp is format 1.
pub fn upgrade(json: &mut serde_json::Value, from: u32) -> Result<(), String> {
    let first = from.max(1) as usize - 1;
    for step in UPGRADES.get(first..).unwrap_or_default() {
        step(json)?;
    }
    Ok(())
}

/// Why a file of format `format` does not open here, or `None` when it
/// does.
pub fn refusal(stamp: Option<&FileStamp>) -> Option<String> {
    let stamp = stamp.filter(|s| s.format > CONTAINER_FORMAT)?;
    let by = match stamp.app_version.as_str() {
        "" => "a newer printCAD".to_string(),
        version => format!("printCAD {version}"),
    };
    Some(format!(
        "this file was written by {by} in a newer file format ({}) than this printCAD \
         reads ({CONTAINER_FORMAT}); update printCAD to open it",
        stamp.format
    ))
}

/// The application version saves are stamped with, set once by the
/// application at start.
static WRITER_VERSION: OnceLock<String> = OnceLock::new();

/// Name the application version every save from now on is stamped with.
/// The first call wins.
pub fn set_writer_version(version: &str) {
    let _ = WRITER_VERSION.set(version.to_string());
}

/// The application version saves are stamped with: empty until the
/// application names it.
pub fn writer_version() -> &'static str {
    WRITER_VERSION.get().map_or("", String::as_str)
}

/// What wrote a container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileStamp {
    /// The container's format number ([`CONTAINER_FORMAT`] when written).
    pub format: u32,
    /// The writing application's version (`0.8.1`); empty when it did
    /// not say.
    pub app_version: String,
}

impl Default for FileStamp {
    fn default() -> Self {
        Self {
            format: CONTAINER_FORMAT,
            app_version: String::new(),
        }
    }
}

impl FileStamp {
    /// The stamp a save made now carries.
    pub fn current() -> Self {
        Self {
            format: CONTAINER_FORMAT,
            app_version: writer_version().to_string(),
        }
    }
}

/// How a file's writer compares with the running application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAge {
    /// Written by an older version, or before files were stamped.
    Older,
    /// Written by this version, or one that cannot be told apart from it.
    Same,
    /// Written by a newer version: it may hold what this one drops.
    Newer,
}

/// How the file stamped `stamp` (`None`: a container without a stamp)
/// compares with the application at `running` version. The format number
/// decides first; the versions, read as `major.minor.patch`, decide
/// between files of one format, and a version that does not read as one
/// tells nothing.
pub fn file_age(stamp: Option<&FileStamp>, running: &str) -> FileAge {
    let Some(stamp) = stamp else {
        return FileAge::Older;
    };
    match stamp.format.cmp(&CONTAINER_FORMAT) {
        std::cmp::Ordering::Greater => return FileAge::Newer,
        std::cmp::Ordering::Less => return FileAge::Older,
        std::cmp::Ordering::Equal => {}
    }
    match (version_triple(&stamp.app_version), version_triple(running)) {
        (Some(file), Some(app)) => match file.cmp(&app) {
            std::cmp::Ordering::Less => FileAge::Older,
            std::cmp::Ordering::Equal => FileAge::Same,
            std::cmp::Ordering::Greater => FileAge::Newer,
        },
        _ => FileAge::Same,
    }
}

/// `major.minor.patch` of a version (`v` in front, a pre-release or build
/// suffix after are passed over); `None` when it does not read as one.
fn version_triple(version: &str) -> Option<(u64, u64, u64)> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(format: u32, version: &str) -> FileStamp {
        FileStamp {
            format,
            app_version: version.to_string(),
        }
    }

    #[test]
    fn versions_compare_by_number_not_text() {
        let s = stamp(CONTAINER_FORMAT, "0.10.0");
        assert_eq!(file_age(Some(&s), "0.9.3"), FileAge::Newer);
        assert_eq!(file_age(Some(&s), "0.10.0"), FileAge::Same);
        assert_eq!(file_age(Some(&s), "1.0.0"), FileAge::Older);
        assert_eq!(file_age(Some(&s), "v0.10.1-rc.1"), FileAge::Older);
    }

    #[test]
    fn the_format_number_decides_before_the_version() {
        let ahead = stamp(CONTAINER_FORMAT + 1, "0.1.0");
        assert_eq!(file_age(Some(&ahead), "9.0.0"), FileAge::Newer);
        let behind = stamp(CONTAINER_FORMAT - 1, "9.0.0");
        assert_eq!(file_age(Some(&behind), "0.1.0"), FileAge::Older);
    }

    #[test]
    fn a_version_that_does_not_read_tells_nothing() {
        let s = stamp(CONTAINER_FORMAT, "");
        assert_eq!(file_age(Some(&s), "0.8.1"), FileAge::Same);
        let s = stamp(CONTAINER_FORMAT, "0.8.1");
        assert_eq!(file_age(Some(&s), "dev"), FileAge::Same);
        assert_eq!(version_triple("1.2.3.4"), None);
        assert_eq!(version_triple("2"), Some((2, 0, 0)));
    }
}
