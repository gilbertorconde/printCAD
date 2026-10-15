mod cli;
mod conversion;
mod numbers;
mod parity;
mod reader;
mod resources;

pub fn fixture(
    name: &str,
) -> (
    project_archive::archive::FcstdArchive,
    project_archive::document_xml::FcDocument,
) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../project_archive/tests/data")
        .join(format!("{name}.FCStd"));
    let archive = project_archive::archive::FcstdArchive::open(&path).unwrap();
    let document = project_archive::document_xml::FcDocument::parse(&archive.document_xml).unwrap();
    (archive, document)
}
