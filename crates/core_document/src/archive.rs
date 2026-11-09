//! Document container reading, writing, and binary sidecar encoding.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::Path;

use tar::{Archive, Builder, Header};

use super::{
    ArchiveProgress, BodyId, CONTAINER_FORMAT, Compression, Document, DocumentError,
    DocumentResult, FileStamp, MeshesApart, TriMesh, stamp,
};

/// The archive entry holding a document's PNG preview.
const THUMBNAIL_ENTRY: &str = "thumbnail.png";

impl Document {
    /// Save document to a .prtcad file (tar archive, optionally compressed).
    pub fn save_to_file(&mut self, path: &Path, compression: Compression) -> DocumentResult<()> {
        let file = File::create(path)?;
        self.save_to_writer(file, compression, None)
    }

    /// Serialize the whole `.prtcad` container into memory: what a client
    /// hands a document server that owns the file but never parses it.
    pub fn save_to_bytes(&mut self, compression: Compression) -> DocumentResult<Vec<u8>> {
        let mut bytes = Vec::new();
        self.save_to_writer(&mut bytes, compression, None)?;
        Ok(bytes)
    }

    /// The same, reporting how much of the archive has been packed.
    ///
    /// The archive carries the document, the file every import came from and
    /// every snapshot blob, so a document with an import takes long enough
    /// that the caller wants to say so.
    pub fn save_to_bytes_watched(
        &mut self,
        compression: Compression,
        progress: ArchiveProgress<'_>,
    ) -> DocumentResult<Vec<u8>> {
        // Room for the archive up front: whole when it is stored as it is,
        // a share when compressed, which packs the blobs well below their
        // size (reserving all of it would ask for memory never used).
        let payload = self.archive_payload_bytes() as usize;
        let mut bytes = Vec::with_capacity(match compression {
            Compression::None => payload,
            Compression::Gzip | Compression::Zstd => payload / 4,
        });
        self.save_to_writer(&mut bytes, compression, Some(progress))?;
        Ok(bytes)
    }

    /// What the blobs in this document add up to: the part of a save whose
    /// cost grows with the model.
    pub fn archive_payload_bytes(&self) -> u64 {
        let assets: u64 = self
            .assets
            .keys()
            .filter_map(|id| self.asset_blobs.get(id))
            .map(|bytes| bytes.len() as u64)
            .sum();
        let breps: u64 = self
            .imported_brep_blobs
            .values()
            .map(|bytes| bytes.len() as u64)
            .sum();
        let colors: u64 = self
            .imported_brep_face_colors
            .values()
            .map(|colors| colors.len() as u64 * 4)
            .sum();
        let bases: u64 = self
            .base_brep_blobs
            .values()
            .map(|bytes| bytes.len() as u64)
            .sum::<u64>()
            + self
                .base_face_colors
                .values()
                .map(|colors| colors.len() as u64 * 4)
                .sum::<u64>();
        assets + breps + colors + bases
    }

    fn save_to_writer<W: Write>(
        &mut self,
        out: W,
        compression: Compression,
        progress: Option<ArchiveProgress<'_>>,
    ) -> DocumentResult<()> {
        Self::sync_brep_paths_for_archive(self);
        let file = out;

        match compression {
            Compression::None => {
                let mut builder = Builder::new(file);
                Self::write_archive(&mut builder, self, progress)?;
                builder.finish()?;
            }
            Compression::Gzip => {
                let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
                let mut builder = Builder::new(encoder);
                Self::write_archive(&mut builder, self, progress)?;
                let encoder = builder.into_inner().map_err(|e| {
                    DocumentError::Compression(format!("gzip encoder finalize failed: {e}"))
                })?;
                encoder.finish()?;
            }
            Compression::Zstd => {
                let mut encoder = zstd::Encoder::new(file, 0)
                    .map_err(|e| DocumentError::Compression(e.to_string()))?;
                {
                    let mut builder = Builder::new(&mut encoder);
                    Self::write_archive(&mut builder, self, progress)?;
                    builder.finish()?;
                }
                encoder
                    .finish()
                    .map_err(|e| DocumentError::Compression(e.to_string()))?;
            }
        }

        Ok(())
    }

    /// Load document from a .prtcad file (auto-detects compression).
    pub fn load_from_file(path: &Path) -> DocumentResult<Self> {
        let (file, compression) = Self::open_container(path)?;
        Self::load_from_reader(file, compression)
    }

    /// The PNG preview a `.prtcad` file carries, read from the front of the
    /// container without unpacking the document: `None` for a file saved
    /// without one, or one that is not a container.
    pub fn read_thumbnail(path: &Path) -> Option<Vec<u8>> {
        let (file, compression) = Self::open_container(path).ok()?;
        Self::thumbnail_in(file, compression)
    }

    /// What wrote the `.prtcad` file at `path`, read from the front of the
    /// container without unpacking the document: `Ok(None)` for a
    /// container from before files were stamped, an error for a file that
    /// is not a container or cannot be read.
    pub fn read_stamp(path: &Path) -> DocumentResult<Option<FileStamp>> {
        let (file, compression) = Self::open_container(path)?;
        Self::stamp_in(file, compression)
    }

    /// [`Document::read_stamp`] of a container held in memory.
    pub fn stamp_of_bytes(bytes: &[u8]) -> DocumentResult<Option<FileStamp>> {
        Self::stamp_in(bytes, compression_of(bytes))
    }

    fn stamp_in<'a, R: Read + 'a>(
        file: R,
        compression: Compression,
    ) -> DocumentResult<Option<FileStamp>> {
        let reader: Box<dyn Read + 'a> = match compression {
            Compression::None => Box::new(file),
            Compression::Gzip => Box::new(flate2::read::GzDecoder::new(file)),
            Compression::Zstd => Box::new(
                zstd::Decoder::new(std::io::BufReader::new(file))
                    .map_err(|e| DocumentError::Compression(e.to_string()))?,
            ),
        };
        let mut archive = Archive::new(reader);
        // The stamp comes before the document, after the preview if any.
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_path_buf();
            if path == Path::new(THUMBNAIL_ENTRY) {
                continue;
            }
            if path == Path::new(stamp::STAMP_ENTRY) {
                let mut json = Vec::new();
                entry.read_to_end(&mut json)?;
                return Ok(Some(serde_json::from_slice(&json)?));
            }
            return Ok(None);
        }
        Err(DocumentError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "an empty archive",
        )))
    }

    fn thumbnail_in<'a, R: Read + 'a>(file: R, compression: Compression) -> Option<Vec<u8>> {
        let reader: Box<dyn Read + 'a> = match compression {
            Compression::None => Box::new(file),
            Compression::Gzip => Box::new(flate2::read::GzDecoder::new(file)),
            Compression::Zstd => Box::new(zstd::Decoder::new(std::io::BufReader::new(file)).ok()?),
        };
        let mut archive = Archive::new(reader);
        // The preview is written first; the document after it means there
        // is none, and the rest of the file is not worth reading to be sure.
        let mut entry = archive.entries().ok()?.next()?.ok()?;
        if entry.path().ok()?.as_ref() != Path::new(THUMBNAIL_ENTRY) {
            return None;
        }
        let mut png = Vec::new();
        entry.read_to_end(&mut png).ok()?;
        Some(png)
    }

    /// A container file, opened, with the compression its name or its
    /// first bytes say.
    fn open_container(path: &Path) -> DocumentResult<(File, Compression)> {
        let mut file = File::open(path)?;

        let mut magic = [0u8; 4];
        let _n = file.read(&mut magic)?;
        file.rewind()?;

        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let compression = if file_name.ends_with(".gz")
            || file_name.ends_with(".prtcad.gz")
            || magic.starts_with(&[0x1f, 0x8b])
        {
            Compression::Gzip
        } else if file_name.ends_with(".zst")
            || file_name.ends_with(".prtcad.zst")
            || magic.starts_with(&[0x28, 0xb5, 0x2f, 0xfd])
        {
            Compression::Zstd
        } else {
            Compression::None
        };
        Ok((file, compression))
    }

    /// Parse a `.prtcad` container from memory: the client side of a
    /// byte-serving document server. Compression is detected from the magic
    /// bytes alone (no filename to consult).
    pub fn load_from_bytes(bytes: Vec<u8>) -> DocumentResult<Self> {
        let compression = compression_of(&bytes);
        Self::load_from_reader(std::io::Cursor::new(bytes), compression)
    }

    fn load_from_reader<R: Read + 'static>(
        file: R,
        compression: Compression,
    ) -> DocumentResult<Self> {
        let mut archive: Archive<Box<dyn Read>> = match compression {
            Compression::None => Archive::new(Box::new(file)),
            Compression::Gzip => {
                let decoder = flate2::read::GzDecoder::new(file);
                Archive::new(Box::new(decoder))
            }
            Compression::Zstd => {
                let decoder = zstd::Decoder::new(std::io::BufReader::new(file))
                    .map_err(|e| DocumentError::Compression(e.to_string()))?;
                Archive::new(Box::new(decoder))
            }
        };

        // A streaming archive cannot seek, so one traversal collects
        // document.json, the preview and every asset and snapshot entry by
        // archive path.
        let mut doc_json: Option<Vec<u8>> = None;
        let mut thumbnail: Option<Vec<u8>> = None;
        let mut written_by: Option<FileStamp> = None;
        let mut blobs_by_path: HashMap<String, Vec<u8>> = HashMap::new();
        for entry in archive.entries()? {
            let mut entry = entry?;
            let entry_path = entry.path()?.to_path_buf();
            let entry_path_str = entry_path.to_string_lossy().to_string();
            if entry_path == Path::new("document.json") {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                doc_json = Some(buf);
            } else if entry_path == Path::new(THUMBNAIL_ENTRY) {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                thumbnail = Some(buf);
            } else if entry_path == Path::new(stamp::STAMP_ENTRY) {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                // A stamp that does not read is a stamp from a writer that
                // knew more: the file is taken as newer than this one.
                written_by = Some(serde_json::from_slice(&buf).unwrap_or(FileStamp {
                    format: u32::MAX,
                    app_version: String::new(),
                }));
            } else if entry_path_str.starts_with("assets/")
                || entry_path_str.starts_with("brep/")
                || entry_path_str.starts_with("mesh/")
            {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                blobs_by_path.insert(entry_path_str, buf);
            }
        }

        let json = doc_json.ok_or_else(|| {
            DocumentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "document.json not found in archive",
            ))
        })?;
        if let Some(why) = stamp::refusal(written_by.as_ref()) {
            return Err(DocumentError::Refused(why));
        }
        // An older format's document is brought up to this one's meaning
        // first; one of this format reads as it is.
        let from = written_by.as_ref().map_or(1, |s| s.format);
        let mut doc: Document = if from < CONTAINER_FORMAT {
            let mut value: serde_json::Value = serde_json::from_slice(&json)?;
            stamp::upgrade(&mut value, from).map_err(DocumentError::Refused)?;
            serde_json::from_value(value)?
        } else {
            serde_json::from_slice(&json)?
        };
        doc.thumbnail = thumbnail.map(std::sync::Arc::new);
        doc.written_by = written_by;
        // Meshes kept as entries of their own; a document saved before
        // carries them in its JSON.
        for geom in doc
            .imported_meshes
            .values_mut()
            .chain(doc.base_solids.values_mut())
            .chain(doc.mesh_starts.values_mut())
        {
            if let Some(path) = &geom.mesh_path
                && let Some(bytes) = blobs_by_path.remove(path)
            {
                match rmp_serde::from_slice::<TriMesh>(&bytes) {
                    Ok(mesh) => geom.mesh = std::sync::Arc::new(mesh),
                    Err(e) => tracing::warn!("a mesh in {path} did not read: {e}"),
                }
            }
        }
        doc.recover_local_meshes();
        doc.refresh_left_out();
        doc.number_unnumbered_notes();

        // Resolve any asset blobs that match an `AssetReference::path`.
        for (asset_id, asset) in &doc.assets {
            if let Some(bytes) = blobs_by_path.remove(&asset.path) {
                doc.asset_blobs
                    .insert(*asset_id, std::sync::Arc::new(bytes));
            }
        }

        // Restore shape-snapshot sidecars for imported geometry. Blobs are
        // ogeom native-format text ("ogeom" magic); any other snapshot is
        // unreadable, so it is dropped with a clear log line rather than
        // failing later with a parse error (the body's mesh still loads;
        // re-importing the source restores the solid).
        for (body_id, geom) in &doc.imported_meshes {
            if let Some(ref brep_path) = geom.brep_blob_path
                && let Some(bytes) = blobs_by_path.remove(brep_path)
            {
                if bytes.starts_with(b"ogeom") {
                    doc.imported_brep_blobs
                        .insert(*body_id, std::sync::Arc::new(bytes));
                } else {
                    tracing::warn!(
                        body = ?body_id,
                        "shape snapshot predates the ogeom kernel; \
                         re-import the STEP source to restore this body's solid"
                    );
                }
            }
            if let Some(ref col_path) = geom.face_colors_path
                && let Some(bytes) = blobs_by_path.remove(col_path)
                && let Some(parsed) = decode_face_colors_blob(&bytes)
            {
                doc.imported_brep_face_colors.insert(*body_id, parsed);
            }
        }
        for (body_id, geom) in &doc.base_solids {
            if let Some(ref brep_path) = geom.brep_blob_path
                && let Some(bytes) = blobs_by_path.remove(brep_path)
                && bytes.starts_with(b"ogeom")
            {
                doc.base_brep_blobs
                    .insert(*body_id, std::sync::Arc::new(bytes));
            }
            if let Some(ref col_path) = geom.face_colors_path
                && let Some(bytes) = blobs_by_path.remove(col_path)
                && let Some(parsed) = decode_face_colors_blob(&bytes)
            {
                doc.base_face_colors.insert(*body_id, parsed);
            }
        }
        let copies: Vec<BodyId> = doc
            .bodies
            .iter()
            .filter(|b| b.copy_of.is_some())
            .map(|b| b.id)
            .collect();
        for copy in copies {
            doc.refresh_copy(copy);
        }

        doc.rebuild_imported_body_index();
        Ok(doc)
    }

    fn write_archive<W: Write>(
        builder: &mut Builder<W>,
        doc: &Document,
        progress: Option<ArchiveProgress<'_>>,
    ) -> DocumentResult<()> {
        let json = {
            let _apart = MeshesApart::start();
            serde_json::to_vec_pretty(doc)?
        };
        // The document's own JSON is only known once it is built, so the
        // total the caller sees settles here and holds for the rest.
        let total = doc.archive_payload_bytes() + json.len() as u64;
        let mut packed = 0u64;
        let report = |packed: u64| {
            if let Some(progress) = progress {
                progress(packed.min(total), total);
            }
        };
        // The preview goes first: a reader after it alone stops at the
        // first entry.
        if let Some(png) = &doc.thumbnail {
            let mut header = Header::new_gnu();
            header.set_path(THUMBNAIL_ENTRY)?;
            header.set_size(png.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, png.as_slice())?;
        }
        // What wrote the file, before the document: a reader learns it
        // without unpacking the rest.
        let stamp = serde_json::to_vec_pretty(&FileStamp::current())?;
        let mut header = Header::new_gnu();
        header.set_path(stamp::STAMP_ENTRY)?;
        header.set_size(stamp.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &stamp[..])?;
        let mut header = Header::new_gnu();
        header.set_path("document.json")?;
        header.set_size(json.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &json[..])?;
        packed += json.len() as u64;
        report(packed);

        // Asset blobs go beside the document, so a reload still has the
        // original file every import came from.
        for (asset_id, asset) in &doc.assets {
            let Some(bytes) = doc.asset_blobs.get(asset_id) else {
                continue;
            };
            let mut header = Header::new_gnu();
            header.set_path(&asset.path)?;
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, &bytes[..])?;
            packed += bytes.len() as u64;
            report(packed);
        }

        for (body_id, geom) in &doc.imported_meshes {
            if !doc.imported_brep_blobs.contains_key(body_id) {
                continue;
            }
            let Some(brep_path) = geom.brep_blob_path.as_ref() else {
                continue;
            };
            let Some(colors_path) = geom.face_colors_path.as_ref() else {
                continue;
            };
            let Some(brep_bytes) = doc.imported_brep_blobs.get(body_id) else {
                continue;
            };
            let Some(colors) = doc.imported_brep_face_colors.get(body_id) else {
                continue;
            };
            let colors_bytes = encode_face_colors_blob(colors);

            let mut header = Header::new_gnu();
            header.set_path(brep_path)?;
            header.set_size(brep_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, brep_bytes.as_slice())?;

            let mut header = Header::new_gnu();
            header.set_path(colors_path)?;
            header.set_size(colors_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, &colors_bytes[..])?;
            packed += brep_bytes.len() as u64 + colors_bytes.len() as u64;
            report(packed);
        }
        // Meshes, each an entry of its own.
        for geom in doc
            .imported_meshes
            .values()
            .chain(doc.base_solids.values())
            .chain(doc.mesh_starts.values())
        {
            let Some(path) = geom.mesh_path.as_ref() else {
                continue;
            };
            let bytes = rmp_serde::to_vec(&*geom.mesh)
                .map_err(|e| DocumentError::Compression(format!("a mesh did not pack: {e}")))?;
            let mut header = Header::new_gnu();
            header.set_path(path)?;
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, &bytes[..])?;
            packed += bytes.len() as u64;
            report(packed);
        }
        // Base solids: what bodies' histories start from.
        for (body_id, geom) in &doc.base_solids {
            let (Some(brep_path), Some(colors_path), Some(brep_bytes)) = (
                geom.brep_blob_path.as_ref(),
                geom.face_colors_path.as_ref(),
                doc.base_brep_blobs.get(body_id),
            ) else {
                continue;
            };
            let colors = doc
                .base_face_colors
                .get(body_id)
                .cloned()
                .unwrap_or_default();
            let colors_bytes = encode_face_colors_blob(&colors);
            for (path, bytes) in [
                (brep_path, brep_bytes.as_slice()),
                (colors_path, &colors_bytes[..]),
            ] {
                let mut header = Header::new_gnu();
                header.set_path(path)?;
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append(&header, bytes)?;
                packed += bytes.len() as u64;
            }
            report(packed);
        }
        report(total);
        Ok(())
    }

    fn sync_brep_paths_for_archive(doc: &mut Document) {
        let copies: Vec<BodyId> = doc
            .bodies
            .iter()
            .filter(|b| b.copy_of.is_some() || b.link.is_some())
            .map(|b| b.id)
            .collect();
        for (body_id, geom) in doc.imported_meshes.iter_mut() {
            // A linked copy's shape is its source's, and a linked part's its
            // file's, derived again on load.
            if copies.contains(body_id) {
                geom.brep_blob_path = None;
                geom.face_colors_path = None;
                geom.mesh_path = None;
                continue;
            }
            geom.mesh_path = Some(format!("mesh/{}.bin", body_id.0));
            if doc.imported_brep_blobs.contains_key(body_id) {
                geom.brep_blob_path = Some(format!("brep/{}.bin", body_id.0));
                geom.face_colors_path = Some(format!("brep/{}.colors", body_id.0));
            }
        }
        for (body_id, geom) in doc.mesh_starts.iter_mut() {
            geom.mesh_path = Some(format!("mesh/{}.start.bin", body_id.0));
            geom.brep_blob_path = None;
            geom.face_colors_path = None;
        }
        for (body_id, geom) in doc.base_solids.iter_mut() {
            geom.mesh_path = Some(format!("mesh/{}.base.bin", body_id.0));
            if doc.base_brep_blobs.contains_key(body_id) {
                geom.brep_blob_path = Some(format!("brep/{}.base.bin", body_id.0));
                geom.face_colors_path = Some(format!("brep/{}.base.colors", body_id.0));
            }
        }
    }
}

fn encode_face_colors_blob(colors: &[[f32; 3]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + colors.len() * 12);
    v.extend_from_slice(&(colors.len() as u32).to_le_bytes());
    for c in colors {
        for comp in c {
            v.extend_from_slice(&comp.to_le_bytes());
        }
    }
    v
}

/// The compression a container held in memory starts with.
fn compression_of(bytes: &[u8]) -> Compression {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        Compression::Gzip
    } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        Compression::Zstd
    } else {
        Compression::None
    }
}

fn decode_face_colors_blob(data: &[u8]) -> Option<Vec<[f32; 3]>> {
    if data.len() < 4 {
        return None;
    }
    let n = u32::from_le_bytes(data.get(0..4)?.try_into().ok()?) as usize;
    let need = 4usize.saturating_add(n.saturating_mul(12));
    if data.len() < need {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o = 4 + i * 12;
        out.push([
            f32::from_le_bytes(data.get(o..o + 4)?.try_into().ok()?),
            f32::from_le_bytes(data.get(o + 4..o + 8)?.try_into().ok()?),
            f32::from_le_bytes(data.get(o + 8..o + 12)?.try_into().ok()?),
        ]);
    }
    Some(out)
}
