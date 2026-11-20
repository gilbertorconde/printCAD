//! The start page's examples: documents shipped inside the app
//! (`assets/examples/`), each opened into an untitled tab the way a file
//! is opened.

use core_document::{Document, FeatureId};

use crate::PrintCadApp;
use crate::contract::Example;
use crate::contract::TreeItemId;

/// The example's file as shipped.
fn bytes(example: Example) -> &'static [u8] {
    match example {
        Example::SketcherBasics => include_bytes!("../../assets/examples/sketcher-basics.prtcad"),
        Example::DesignWorkflow => include_bytes!("../../assets/examples/design-workflow.prtcad"),
    }
}

/// The example's document as stored, not yet made ready to build.
pub(crate) fn document(example: Example) -> Result<Document, String> {
    Document::load_from_bytes(bytes(example).to_vec()).map_err(|err| err.to_string())
}

/// The feature an example opens on: the last in its build history.
fn opening_feature(document: &Document) -> Option<FeatureId> {
    document
        .feature_tree()
        .all_nodes()
        .max_by_key(|(id, node)| (node.seq, **id))
        .map(|(id, _)| *id)
}

impl PrintCadApp {
    /// Put `example` in place of the tab's blank document, as an untitled
    /// document with unsaved edits and no undo history: its body the
    /// target and its last feature selected, or opened for editing in the
    /// bench that claims it.
    pub(crate) fn open_example(&mut self, example: Example) {
        let document = match document(example) {
            Ok(document) => document,
            Err(err) => {
                app_log::error(format!("Example: {err}"));
                return;
            }
        };
        self.session.send_outbox();
        let name = self.session.document.name().to_string();
        self.session.document = crate::app::doc_io::opened(&self.registry, document);
        self.session.document.name_after_file(name);
        self.session.journal.reset(&mut self.session.document);
        self.session
            .connection
            .server
            .send(core_document::server::ClientMessage::Rebase);
        self.session.document.mark_dirty();
        let Some(feature) = opening_feature(&self.session.document) else {
            return;
        };
        if let Some(body) = self
            .session
            .document
            .get_feature_meta(feature)
            .and_then(|node| node.body)
        {
            self.apply_tree_selection(TreeItemId::Body(body));
        }
        if example.opens_for_editing() {
            self.apply_tree_activation(TreeItemId::Feature(feature));
        } else {
            self.apply_tree_selection(TreeItemId::Feature(feature));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const ALL: [Example; 2] = [Example::SketcherBasics, Example::DesignWorkflow];

    /// The example's file name under `assets/examples/`.
    fn file_name(example: Example) -> &'static str {
        match example {
            Example::SketcherBasics => "sketcher-basics.prtcad",
            Example::DesignWorkflow => "design-workflow.prtcad",
        }
    }

    /// The hook scene each example was written from.
    fn scene(example: Example) -> bench_fixtures::Scene {
        match example {
            Example::SketcherBasics => bench_fixtures::Scene::Sketch,
            Example::DesignWorkflow => bench_fixtures::Scene::Pocket,
        }
    }

    /// A new tab's document with one body and `example`'s scene on it, as
    /// the scene builder makes it, and the feature the builder names.
    fn built(example: Example) -> (Document, FeatureId) {
        let mut document = Document::new("Untitled");
        let body = document.create_body(None);
        let handles =
            bench_fixtures::open_sketch_scene(&mut document, Some(body), scene(example)).unwrap();
        (document, handles.feature)
    }

    /// The document's replicated state with its ids named by position
    /// (features by build order, bodies by order, any other id by where it
    /// first appears) and creation times left out: what two documents
    /// built alike share.
    fn canonical(document: &Document) -> Value {
        let mut names = vec![(document.id().to_string(), "document".to_string())];
        let mut features: Vec<_> = document.feature_tree().all_nodes().collect();
        features.sort_by_key(|(_, node)| node.seq);
        for (k, (id, _)) in features.iter().enumerate() {
            names.push((id.0.to_string(), format!("feature {k}")));
        }
        for (k, body) in document.bodies().iter().enumerate() {
            names.push((body.id.0.to_string(), format!("body {k}")));
        }
        let mut text = document.replicated_projection().to_string();
        for (id, name) in names {
            text = text.replace(&id, &name);
        }
        let mut value: Value = serde_json::from_str(&text).unwrap();
        fn tidy(value: &mut Value, ids: &mut Vec<String>) {
            match value {
                Value::Object(map) => {
                    map.remove("created_at");
                    map.values_mut().for_each(|v| tidy(v, ids));
                }
                Value::Array(list) => list.iter_mut().for_each(|v| tidy(v, ids)),
                Value::String(text) if uuid::Uuid::parse_str(text).is_ok() => {
                    let k = ids.iter().position(|id| id == text).unwrap_or_else(|| {
                        ids.push(text.clone());
                        ids.len() - 1
                    });
                    *text = format!("id {k}");
                }
                _ => {}
            }
        }
        tidy(&mut value, &mut Vec::new());
        value
    }

    /// Each body's measures once built, by body name.
    fn measures(document: Document) -> Value {
        const MEASURE: &str = r#"
            pc.doc.rebuild(); local m = {}
            for _, b in ipairs(pc.doc.bodies()) do
                local ok, got = pcall(pc.doc.measure, {body = b.id})
                m[b.name] = ok and {volume = got.volume, area = got.area,
                                    min = got.min, max = got.max} or "no solid"
            end
            return m
        "#;
        let mut registry = core_document::DocumentService::default();
        workbenches::register_all_workbenches(&mut registry).unwrap();
        let document = crate::app::doc_io::opened(&registry, document);
        let (_, got) =
            crate::headless::run_in_document(&mut registry, document, MEASURE, "measure");
        got.unwrap().unwrap()
    }

    /// Each shipped example is the document its scene builds: the same
    /// features with the same contents on one body, building to the same
    /// solid. `PRINTCAD_WRITE_EXAMPLES=1` writes the files from the scenes,
    /// which the next build embeds.
    #[test]
    fn each_example_is_the_document_its_scene_builds() {
        if std::env::var_os("PRINTCAD_WRITE_EXAMPLES").is_some() {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/examples");
            std::fs::create_dir_all(&dir).unwrap();
            for example in ALL {
                built(example)
                    .0
                    .save_to_file(
                        &dir.join(file_name(example)),
                        core_document::Compression::Zstd,
                    )
                    .unwrap();
            }
        }
        for example in ALL {
            let shipped = document(example).unwrap();
            let (fresh, opens) = built(example);
            assert_eq!(canonical(&shipped), canonical(&fresh), "{example:?}");
            assert_eq!(shipped.bodies().len(), 1, "{example:?}");
            assert_eq!(opening_feature(&fresh), Some(opens), "{example:?}");
            assert_eq!(measures(shipped), measures(fresh), "{example:?}");
        }
    }
}
