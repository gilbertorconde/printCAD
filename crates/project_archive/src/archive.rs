// Copyright (c) 2026 Stan Yolo
// SPDX-License-Identifier: MIT OR Apache-2.0
// See ../NOTICE for the retained permission notice.
//! The `.FCStd` container: a ZIP with `Document.xml` and saved object shapes.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Seek};
use std::path::Path;

/// External-file failures carry a reason in `Result`.
#[derive(Debug, Clone, PartialEq)]
pub enum FcstdError {
    Io(String),
    Zip(String),
    /// A required archive entry is missing.
    MissingEntry(&'static str),
    Xml(String),
    /// An inclusive reader budget was exceeded (bytes or entries, named by resource).
    Limit {
        resource: String,
        limit: u64,
        observed: u64,
    },
    /// Parsed XML violates the format: a missing property, wrong type or invalid number.
    Format {
        object: String,
        reason: String,
    },
}

impl fmt::Display for FcstdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "read: {error}"),
            Self::Zip(error) => write!(f, "zip: {error}"),
            Self::MissingEntry(name) => write!(f, "archive has no {name}"),
            Self::Xml(error) => write!(f, "Document.xml: {error}"),
            Self::Limit {
                resource,
                limit,
                observed,
            } => {
                write!(f, "reader limit for {resource}: {observed} exceeds {limit}")
            }
            Self::Format { object, reason } => write!(f, "object {object}: {reason}"),
        }
    }
}

impl std::error::Error for FcstdError {}

/// Byte budgets apply to uncompressed data, except for `ArchiveLimits::container`.
#[derive(Debug, Clone, Copy)]
pub struct Bytes(pub u64);

/// Inclusive budgets. The total includes every declared entry, including ignored files;
/// actual decompression is charged only for the XML and shapes that the reader consumes.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    pub container: Bytes,
    pub xml: Bytes,
    pub entry: Bytes,
    pub total: Bytes,
    pub entries: usize,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            container: Bytes(256 * 1024 * 1024),
            xml: Bytes(16 * 1024 * 1024),
            entry: Bytes(128 * 1024 * 1024),
            total: Bytes(512 * 1024 * 1024),
            entries: 10_000,
        }
    }
}

pub(crate) fn check_limit(
    resource: impl Into<String>,
    observed: u64,
    limit: u64,
) -> Result<(), FcstdError> {
    if observed > limit {
        return Err(FcstdError::Limit {
            resource: resource.into(),
            limit,
            observed,
        });
    }
    Ok(())
}

fn charge_total(
    resource: &str,
    consumed: &mut u64,
    amount: u64,
    limit: u64,
) -> Result<(), FcstdError> {
    if amount > limit - *consumed {
        return Err(FcstdError::Limit {
            resource: resource.into(),
            limit,
            observed: consumed.saturating_add(amount),
        });
    }
    *consumed += amount;
    Ok(())
}

/// Decoded archive: `Document.xml`, all entry names and saved shape texts.
#[derive(Debug, Clone)]
pub struct FcstdArchive {
    pub document_xml: String,
    pub entries: Vec<String>,
    /// `*.brp` entry to its text (`BRepTools::Write`). An empty string represents an
    /// empty saved shape, such as `SuppressedShape` or a sketch without geometry.
    pub shapes: BTreeMap<String, String>,
}

impl FcstdArchive {
    pub fn open(path: &Path) -> Result<Self, FcstdError> {
        Self::open_with_limits(path, ArchiveLimits::default())
    }

    pub fn open_with_limits(path: &Path, limits: ArchiveLimits) -> Result<Self, FcstdError> {
        let file = std::fs::File::open(path)
            .map_err(|error| FcstdError::Io(format!("{}: {error}", path.display())))?;
        let size = file
            .metadata()
            .map_err(|error| FcstdError::Io(format!("{}: {error}", path.display())))?
            .len();
        check_limit("container bytes", size, limits.container.0)?;
        Self::from_reader(file, limits)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FcstdError> {
        Self::from_bytes_with_limits(bytes, ArchiveLimits::default())
    }

    pub fn from_bytes_with_limits(bytes: &[u8], limits: ArchiveLimits) -> Result<Self, FcstdError> {
        check_limit("container bytes", bytes.len() as u64, limits.container.0)?;
        Self::from_reader(std::io::Cursor::new(bytes), limits)
    }

    fn from_reader(reader: impl Read + Seek, limits: ArchiveLimits) -> Result<Self, FcstdError> {
        let mut zip =
            zip::ZipArchive::new(reader).map_err(|error| FcstdError::Zip(error.to_string()))?;
        check_limit("archive entries", zip.len() as u64, limits.entries as u64)?;
        let mut declared_total = 0u64;
        for index in 0..zip.len() {
            let entry = zip
                .by_index_raw(index)
                .map_err(|error| FcstdError::Zip(error.to_string()))?;
            check_limit(
                format!("entry {} bytes", entry.name()),
                entry.size(),
                limits.entry.0,
            )?;
            if entry.name() == "Document.xml" {
                check_limit("Document.xml bytes", entry.size(), limits.xml.0)?;
            }
            // The subtraction avoids overflow even for a forged ZIP64 size.
            charge_total(
                "total declared bytes",
                &mut declared_total,
                entry.size(),
                limits.total.0,
            )?;
        }
        let entries: Vec<String> = zip.file_names().map(str::to_owned).collect();
        let mut actual_total = 0u64;
        let document_xml = match zip.by_name("Document.xml") {
            Ok(mut entry) => read_text(&mut entry, "Document.xml", limits, &mut actual_total)?,
            Err(zip::result::ZipError::FileNotFound) => {
                return Err(FcstdError::MissingEntry("Document.xml"));
            }
            Err(other) => return Err(FcstdError::Zip(other.to_string())),
        };
        let mut shapes = BTreeMap::new();
        for name in entries.iter().filter(|name| name.ends_with(".brp")) {
            let mut entry = zip
                .by_name(name)
                .map_err(|error| FcstdError::Zip(format!("{name}: {error}")))?;
            shapes.insert(
                name.clone(),
                read_text(&mut entry, name, limits, &mut actual_total)?,
            );
        }
        Ok(Self {
            document_xml,
            entries,
            shapes,
        })
    }
}

fn read_text(
    entry: &mut impl Read,
    name: &str,
    limits: ArchiveLimits,
    actual_total: &mut u64,
) -> Result<String, FcstdError> {
    let entry_limit = if name == "Document.xml" {
        limits.entry.0.min(limits.xml.0)
    } else {
        limits.entry.0
    };
    let allowance = entry_limit.min(limits.total.0 - *actual_total);
    let mut bytes = Vec::new();
    // One extra byte proves an overrun without exhausting the decompressor.
    entry
        .take(allowance.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| FcstdError::Zip(format!("{name}: {error}")))?;
    let actual = bytes.len() as u64;
    check_limit(format!("entry {name} bytes"), actual, limits.entry.0)?;
    if name == "Document.xml" {
        check_limit("Document.xml bytes", actual, limits.xml.0)?;
    }
    charge_total(
        "total decompressed bytes",
        actual_total,
        actual,
        limits.total.0,
    )?;
    String::from_utf8(bytes).map_err(|error| FcstdError::Zip(format!("{name}: {error}")))
}

impl FcstdArchive {
    /// Shape text named by `Part::PropertyPartShape`; a missing target is malformed data.
    pub fn shape(&self, file: &str) -> Result<&str, FcstdError> {
        self.shapes
            .get(file)
            .map(String::as_str)
            .ok_or_else(|| FcstdError::Format {
                object: file.to_owned(),
                reason: "shape entry is not in the archive".into(),
            })
    }
}
