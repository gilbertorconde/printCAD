//! Dev utility: write a minimal `.prtcad` for testing document opening.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: make_doc <path.prtcad>");
    let mut doc = core_document::Document::new("Two Window Test");
    let body = doc.create_body(Some("Shared body".into()));
    doc.rename_body(body, "Shared");
    doc.save_to_file(
        std::path::Path::new(&path),
        core_document::Compression::None,
    )
    .expect("save");
    println!("wrote {path}");
}
