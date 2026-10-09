//! Parts linked from other printCAD files: inserting a file's bodies as
//! linked parts, reading each linked part's shape from its file (away from
//! the window), and marking a part whose file changed since it was read.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use core_document::{BodyId, Document, FileLink, ImportedGeometry};

use crate::PrintCadApp;
use crate::log_panel as app_log;

/// How often linked files are looked at for changes.
const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// A linked file being read: to insert its bodies, or to give the parts
/// linked from it their shapes.
pub(crate) struct LinkLoad {
    path: PathBuf,
    insert: bool,
    answer: Receiver<Result<Document, String>>,
}

/// What the links of a tab have under way.
#[derive(Default)]
pub(crate) struct Links {
    loads: Vec<LinkLoad>,
    /// Linked parts their file could not give a shape: not asked again
    /// until reloaded.
    failed: std::collections::HashSet<BodyId>,
    checked: Option<web_time::Instant>,
}

impl Links {
    pub(crate) fn busy(&self) -> bool {
        !self.loads.is_empty()
    }
}

/// The file's modified time, whole seconds.
pub(crate) fn stamp(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

fn read(path: PathBuf) -> Receiver<Result<Document, String>> {
    let (send, answer) = channel();
    let spawned = crate::platform::spawn("printcad-link", move || {
        let _ = send.send(Document::load_from_file(&path).map_err(|e| e.to_string()));
    });
    if let Err(err) = spawned {
        app_log::error(format!("Could not read the linked file: {err}"));
    }
    answer
}

/// A linked part's shape: its mesh, snapshot and face colours.
type Shape = (
    ImportedGeometry,
    Option<Arc<Vec<u8>>>,
    Option<Vec<[f32; 3]>>,
);

/// `body` of `source` as a linked part's shape: its own mesh, snapshot and
/// face colours.
fn shape_of(source: &Document, body: BodyId) -> Option<Shape> {
    let (mesh, bounds) = source.local_geometry(body)?;
    let health = source
        .imported_geometry(body)
        .and_then(|g| g.health.clone());
    Some((
        ImportedGeometry {
            mesh,
            source_asset: None,
            revision: 0,
            bounds_mm: bounds,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health,
        },
        source.imported_brep_blob_arc(body),
        source
            .imported_brep_face_colors(body)
            .map(<[[f32; 3]]>::to_vec),
    ))
}

impl PrintCadApp {
    /// Insert every visible body of the printCAD file at `path` as a part
    /// linked to it.
    pub(crate) fn insert_linked(&mut self, path: PathBuf) {
        app_log::info(format!("Reading {} to link its parts…", path.display()));
        let answer = read(path.clone());
        self.session.links.loads.push(LinkLoad {
            path,
            insert: true,
            answer,
        });
    }

    /// Read linked parts' files, land what was read, and look at the
    /// files for changes now and then.
    pub(crate) fn drive_links(&mut self) {
        let awaiting = self.session.document.links_awaiting_geometry();
        for (body, link) in &awaiting {
            let path = PathBuf::from(&link.path);
            let loading = self.session.links.loads.iter().any(|l| l.path == path);
            if !loading && !self.session.links.failed.contains(body) {
                let answer = read(path.clone());
                self.session.links.loads.push(LinkLoad {
                    path,
                    insert: false,
                    answer,
                });
            }
        }
        let mut done = Vec::new();
        for (i, load) in self.session.links.loads.iter().enumerate() {
            match load.answer.try_recv() {
                Ok(answer) => done.push((i, Some(answer))),
                Err(TryRecvError::Disconnected) => done.push((i, None)),
                Err(TryRecvError::Empty) => {}
            }
        }
        for (i, answer) in done.into_iter().rev() {
            let load = self.session.links.loads.remove(i);
            match answer {
                Some(Ok(source)) => self.land_link(&load, &source),
                Some(Err(why)) => {
                    app_log::error(format!("Could not read {}: {why}", load.path.display()));
                    self.fail_links_to(&load.path);
                }
                None => self.fail_links_to(&load.path),
            }
        }
        let due = self
            .session
            .links
            .checked
            .is_none_or(|t| t.elapsed() >= CHECK_EVERY);
        if due {
            self.session.links.checked = Some(web_time::Instant::now());
            let linked: Vec<(BodyId, FileLink)> = self
                .session
                .document
                .bodies()
                .iter()
                .filter_map(|b| Some((b.id, b.link.clone()?)))
                .collect();
            for (body, link) in linked {
                let now = stamp(Path::new(&link.path));
                self.session
                    .document
                    .set_link_stale(body, now != link.stamp);
            }
        }
    }

    /// Every part linked to `path` still waiting: not asked again.
    fn fail_links_to(&mut self, path: &Path) {
        for (body, link) in self.session.document.links_awaiting_geometry() {
            if Path::new(&link.path) == path {
                self.session.links.failed.insert(body);
            }
        }
    }

    /// A linked file read: its bodies inserted, or its parts' shapes given.
    fn land_link(&mut self, load: &LinkLoad, source: &Document) {
        let path = load.path.to_string_lossy().into_owned();
        if load.insert {
            let at = stamp(&load.path);
            let mut count = 0;
            for body in source.bodies() {
                if !source.imported_body_effective_visible(body.id) {
                    continue;
                }
                let Some((geometry, blob, colors)) = shape_of(source, body.id) else {
                    continue;
                };
                let id = self.session.document.create_linked_body(
                    body.name.clone(),
                    FileLink {
                        path: path.clone(),
                        body: body.id,
                        stamp: at,
                    },
                );
                self.session.document.set_body_placement(id, body.placement);
                self.session
                    .document
                    .set_linked_geometry(id, geometry, blob, colors);
                count += 1;
            }
            if count == 0 {
                app_log::warn(format!("{} has no body to link", load.path.display()));
                return;
            }
            self.session.journal.label_next("Insert linked parts");
            self.close_gesture();
            app_log::info(format!(
                "Linked {count} part{} from {}",
                if count == 1 { "" } else { "s" },
                load.path.display()
            ));
            return;
        }
        for (body, link) in self.session.document.links_awaiting_geometry() {
            if link.path != path {
                continue;
            }
            match shape_of(source, link.body) {
                Some((geometry, blob, colors)) => {
                    self.session
                        .document
                        .set_linked_geometry(body, geometry, blob, colors);
                }
                None => {
                    app_log::warn(format!(
                        "{} no longer has the body `{}` is linked to",
                        load.path.display(),
                        self.body_name(body)
                    ));
                    self.session.links.failed.insert(body);
                }
            }
        }
    }

    /// Read a linked part again from its file as it stands now.
    pub(crate) fn reload_link(&mut self, body: BodyId) {
        let Some(link) = self
            .session
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .and_then(|b| b.link.clone())
        else {
            return;
        };
        self.session.links.failed.remove(&body);
        let now = stamp(Path::new(&link.path));
        if self.session.document.reload_link(body, now) {
            self.session.journal.label_next("Reload linked part");
            self.close_gesture();
        }
    }

    /// Open a linked part's file in a tab of its own.
    pub(crate) fn open_link_source(&mut self, body: BodyId) {
        let Some(link) = self
            .session
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .and_then(|b| b.link.clone())
        else {
            return;
        };
        self.open_document_at(PathBuf::from(link.path));
    }
}
