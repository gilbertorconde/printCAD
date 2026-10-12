//! What wrote a `.prtcad`: every save stamps the container with the
//! application's version and the format number, the stamp reads back on
//! its own and with the document, and a container without one (or with
//! entries this build does not know) still loads.

use std::io::Read;

use core_document::{
    CONTAINER_FORMAT, Compression, Document, FileAge, FileStamp, file_age, set_writer_version,
};

/// The version this program's saves are stamped with.
const VERSION: &str = "0.8.1";

fn saved(compression: Compression) -> Vec<u8> {
    set_writer_version(VERSION);
    let mut doc = Document::new("Stamped");
    doc.create_body(Some("Body".into()));
    doc.set_thumbnail(Some(b"\x89PNG not really".to_vec()));
    doc.save_to_bytes(compression).expect("save")
}

/// Every entry of an uncompressed container, by path, in order.
fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = tar::Archive::new(bytes);
    archive
        .entries()
        .expect("entries")
        .map(|entry| {
            let mut entry = entry.expect("entry");
            let path = entry.path().expect("path").to_string_lossy().into_owned();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).expect("read");
            (path, data)
        })
        .collect()
}

/// A container of `entries`, in order.
fn pack(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_path(path).expect("path");
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, data.as_slice()).expect("append");
    }
    builder.into_inner().expect("finish")
}

#[test]
fn a_save_is_stamped_and_the_stamp_reads_back() {
    let expected = FileStamp {
        format: CONTAINER_FORMAT,
        app_version: VERSION.to_string(),
    };
    for (compression, ext) in [
        (Compression::None, "prtcad"),
        (Compression::Gzip, "prtcad.gz"),
        (Compression::Zstd, "prtcad.zst"),
    ] {
        let bytes = saved(compression);
        assert_eq!(
            Document::stamp_of_bytes(&bytes).expect("a container"),
            Some(expected.clone()),
            "{ext}"
        );
        let tmp = std::env::temp_dir().join(format!("printcad_stamp_{}.{ext}", std::process::id()));
        std::fs::write(&tmp, &bytes).expect("write");
        assert_eq!(
            Document::read_stamp(&tmp).expect("a container"),
            Some(expected.clone()),
            "{ext}"
        );
        // The preview still leads, for a reader of the first entry alone.
        assert!(Document::read_thumbnail(&tmp).is_some(), "{ext}");
        let _ = std::fs::remove_file(&tmp);

        let loaded = Document::load_from_bytes(bytes).expect("load");
        assert_eq!(loaded.written_by(), Some(&expected), "{ext}");
        assert_eq!(file_age(loaded.written_by(), VERSION), FileAge::Same);
    }
}

#[test]
fn a_container_from_before_stamps_loads_and_reads_as_older() {
    let stamped = entries(&saved(Compression::None));
    assert!(stamped.iter().any(|(path, _)| path == "meta.json"));
    let old: Vec<_> = stamped
        .into_iter()
        .filter(|(path, _)| path != "meta.json")
        .collect();
    let bytes = pack(&old);

    assert_eq!(Document::stamp_of_bytes(&bytes).expect("a container"), None);
    let loaded = Document::load_from_bytes(bytes).expect("an old file loads");
    assert_eq!(loaded.bodies().len(), 1);
    assert_eq!(loaded.written_by(), None);
    assert_eq!(file_age(loaded.written_by(), VERSION), FileAge::Older);
}

#[test]
fn a_newer_container_loads_what_it_can_and_reads_as_newer() {
    let mut newer = entries(&saved(Compression::None));
    for (path, data) in &mut newer {
        if path == "meta.json" {
            *data = br#"{"format": 99, "app_version": "9.0.0", "more": true}"#.to_vec();
        }
    }
    // An entry this build does not know is passed by.
    newer.push(("future/thing.bin".to_string(), vec![1, 2, 3]));
    let bytes = pack(&newer);

    let loaded = Document::load_from_bytes(bytes).expect("a newer file loads");
    assert_eq!(loaded.bodies().len(), 1);
    let stamp = loaded.written_by().expect("stamped");
    assert_eq!(stamp.format, 99);
    assert_eq!(file_age(Some(stamp), VERSION), FileAge::Newer);
}

#[test]
fn what_is_not_a_container_has_no_stamp_to_read() {
    assert!(Document::stamp_of_bytes(b"not an archive at all").is_err());
}
