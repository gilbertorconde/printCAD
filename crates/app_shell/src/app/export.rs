//! Export: File › Export opens a dialog on a draft of the options (format,
//! which bodies, the mesh tolerance), a confirmed draft asks for a file
//! name, and the kernel writes the file on a thread of its own so a large
//! document never holds the window. The result lands in the log.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use kernel_api::{LinearDeflectionMode, TessellationSettings, TriMesh};
use kernel_ogeom::export::{ExportBody, ExportFormat, export};

use crate::PrintCadApp;
use crate::app_log;

/// The export dialog's options.
#[derive(Debug, Clone)]
pub(crate) struct ExportDraft {
    pub format: ExportFormat,
    /// Only the selected body, rather than every visible one.
    pub selected_only: bool,
    /// The mesh formats' tolerance.
    pub detail: TessellationSettings,
    /// One file per configuration, each named after it.
    pub every_configuration: bool,
    /// Bodies' surface textures pressed into the mesh formats.
    pub textures: bool,
    /// The print layout rather than the bodies where they sit: each part
    /// flat on the bed, as many as the parts list prints.
    pub layout: bool,
}

impl Default for ExportDraft {
    fn default() -> Self {
        Self {
            format: ExportFormat::ThreeMf,
            selected_only: false,
            every_configuration: false,
            textures: true,
            layout: false,
            // A print resolves far finer than a screen: a hundredth of a
            // millimetre off the true surface and a few degrees per facet.
            detail: TessellationSettings {
                linear_deflection_mode: LinearDeflectionMode::AbsoluteMm,
                chord_tolerance: 0.01,
                angular_tolerance_deg: 5.0,
                ..TessellationSettings::default()
            },
        }
    }
}

/// One body's share of an export, owned so the writer thread can hold it.
struct OwnedBody {
    name: String,
    brep: Option<Arc<Vec<u8>>>,
    /// Where the body sits, for its kernel shape; `None` when it has not
    /// moved.
    transform: Option<[[f64; 4]; 4]>,
    mesh: Arc<TriMesh>,
    /// The body's surface textures, to press into its mesh.
    pressing: Option<crate::app::textures::Pressing>,
}

impl OwnedBody {
    /// What a mesh file writes of the body's faces: its textures pressed
    /// in at the export's detail.
    fn finish(&self) -> Option<Finish<'_>> {
        let pressing = self.pressing.as_ref()?;
        Some(Box::new(move |mesh: TriMesh| {
            pressing.apply(&mesh, surface_texture::Detail::EXPORT)
        }))
    }
}

/// What a mesh file writes of one body's faces once meshed.
type Finish<'a> = Box<dyn Fn(TriMesh) -> TriMesh + Sync + 'a>;

/// Borrow owned bodies as the kernel's export takes them, their finishes
/// kept alive beside them.
fn borrowed<'a>(
    bodies: &'a [OwnedBody],
    finishes: &'a [Option<Finish<'a>>],
) -> Vec<ExportBody<'a>> {
    bodies
        .iter()
        .zip(finishes)
        .map(|(b, finish)| ExportBody {
            name: b.name.clone(),
            brep: b.brep.as_deref().map(Vec::as_slice),
            transform: b.transform,
            mesh: &b.mesh,
            finish: finish
                .as_deref()
                .map(|f| f as &(dyn Fn(TriMesh) -> TriMesh + Sync)),
        })
        .collect()
}

/// A finished export, for the log.
pub(crate) struct ExportOutcome {
    path: PathBuf,
    result: Result<kernel_ogeom::export::Exported, String>,
    /// The slicer command to open the file with once written, when the
    /// export was a send to the slicer.
    open_with: Option<String>,
}

/// `path` with the format's extension, unless it already names one the
/// format answers to.
pub(crate) fn with_extension(path: &Path, format: ExportFormat) -> PathBuf {
    if ExportFormat::of_path(path).map(ExportFormat::extension) == Some(format.extension()) {
        path.to_path_buf()
    } else {
        let mut name = path.as_os_str().to_owned();
        name.push(".");
        name.push(format.extension());
        PathBuf::from(name)
    }
}

impl PrintCadApp {
    /// Open the export dialog on the options used last.
    pub(crate) fn open_export_dialog(&mut self) {
        self.session.export_pending = Some(self.last_export.clone());
    }

    /// The walkthrough's second step: once the example has a solid, the
    /// export dialog opens over it, set for the whole document.
    pub(crate) fn open_export_when_ready(&mut self) {
        if !self.session.export_when_ready {
            return;
        }
        let draft = ExportDraft {
            selected_only: false,
            ..self.last_export.clone()
        };
        if !self.export_bodies(&draft).is_empty() {
            self.session.export_when_ready = false;
            self.session.export_pending = Some(draft);
        }
    }

    /// Ask where to write, once the dialog is confirmed.
    pub(crate) fn confirm_export(&mut self) {
        let Some(draft) = self.session.export_pending.take() else {
            return;
        };
        if self.export_bodies(&draft).is_empty() {
            app_log::warn(if draft.selected_only {
                "Nothing to export: select a body with geometry first"
            } else {
                "Nothing to export: no visible body has geometry"
            });
            return;
        }
        self.last_export = draft.clone();
        self.start_file_dialog(crate::app::doc_io::FileDialogKind::Export(draft.format));
    }

    /// Write the confirmed export to `path` on a thread of its own.
    pub(crate) fn start_export(&mut self, path: PathBuf) {
        let draft = self.last_export.clone();
        let configured = self
            .session
            .document
            .configurations()
            .is_some_and(|(_, t)| !t.rows.is_empty());
        if draft.every_configuration && configured {
            self.export_every_configuration(&path, &draft);
            return;
        }
        self.write_export(path, draft, None);
    }

    /// One file per configuration beside `path`, each named after it
    /// (`bracket-Large.3mf`): every configuration in turn is put in
    /// effect, built and written, and the one in effect before comes back.
    /// It runs as a script, which waits for each build, and is one undo
    /// step.
    fn export_every_configuration(&mut self, path: &Path, draft: &ExportDraft) {
        let base = path.with_extension("");
        let bodies = if draft.selected_only && !draft.layout {
            match self.session.selected_body {
                Some(body) => format!("{{\"{body}\"}}"),
                None => {
                    app_log::warn("Nothing to export: select a body with geometry first");
                    return;
                }
            }
        } else {
            "nil".to_string()
        };
        let source = every_configuration_script(
            &base.display().to_string(),
            draft.format.extension(),
            draft.detail.chord_tolerance,
            &bodies,
            draft.layout,
        );
        self.submit_script(
            scripting::Job::Script {
                name: "Export every configuration".to_string(),
                source,
            },
            crate::app::scripts::RunKind::File,
        );
    }

    /// Hand every visible body to the slicer: written to a file of the
    /// slicer's format in the temporary folder, then opened with the
    /// slicer command from Preferences. Laid out for printing when
    /// `laid_out`, or when Preferences say so.
    pub(crate) fn send_to_slicer(&mut self, laid_out: bool) {
        let printing = &self.user_settings.printing;
        let layout = laid_out || printing.slicer_layout;
        let draft = ExportDraft {
            format: match printing.slicer_format {
                settings::SlicerFormat::ThreeMf => ExportFormat::ThreeMf,
                settings::SlicerFormat::Stl => ExportFormat::Stl,
            },
            selected_only: false,
            detail: self.last_export.detail.clone(),
            every_configuration: false,
            // What goes to the slicer is what gets printed.
            textures: true,
            layout,
        };
        if self.export_bodies(&draft).is_empty() {
            app_log::warn("Nothing to send: no visible body has geometry");
            return;
        }
        let folder = crate::platform::temp_dir().join("printcad").join("slicer");
        if let Err(err) = std::fs::create_dir_all(&folder) {
            app_log::error(format!("Could not make {}: {err}", folder.display()));
            return;
        }
        let stem = self
            .session
            .current_file
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_stem_of(self.session.document.name()));
        let path = folder.join(format!("{stem}.{}", draft.format.extension()));
        let command = printing.slicer_command.clone();
        self.write_export(path, draft, Some(command));
    }

    /// Write `draft` to `path` on a thread of its own, then open it with
    /// `open_with` when given.
    fn write_export(&mut self, path: PathBuf, draft: ExportDraft, open_with: Option<String>) {
        if self.export_rx.is_some() {
            app_log::warn("An export is still being written");
            return;
        }
        let path = with_extension(&path, draft.format);
        let bodies = self.export_bodies(&draft);
        let (tx, rx) = channel();
        self.export_rx = Some(rx);
        app_log::info(format!(
            "Exporting {} {} to {}",
            bodies.len(),
            if bodies.len() == 1 { "body" } else { "bodies" },
            path.display()
        ));
        let started = crate::platform::spawn("printcad-export", move || {
            let finishes: Vec<_> = bodies.iter().map(OwnedBody::finish).collect();
            let borrowed = borrowed(&bodies, &finishes);
            let result = export(&borrowed, draft.format, &draft.detail)
                .map_err(|e| e.to_string())
                .and_then(|exported| {
                    crate::platform::write(&path, &exported.bytes)
                        .map(|()| exported)
                        .map_err(|e| format!("could not write the file: {e}"))
                });
            let _ = tx.send(ExportOutcome {
                path,
                result,
                open_with,
            });
        });
        if let Err(err) = started {
            crate::app_log::error(format!("Could not start the export: {err}"));
        }
    }

    /// Log a finished export, if one has finished.
    pub(crate) fn poll_export(&mut self) {
        let Some(outcome) = self
            .export_rx
            .as_ref()
            .and_then(|rx: &Receiver<ExportOutcome>| rx.try_recv().ok())
        else {
            return;
        };
        self.export_rx = None;
        let ExportOutcome {
            path,
            result,
            open_with,
        } = outcome;
        match result {
            Ok(exported) => {
                let mut line = format!(
                    "Exported {} to {}",
                    match exported.written {
                        1 => "1 body".to_string(),
                        n => format!("{n} bodies"),
                    },
                    path.display()
                );
                if exported.triangles > 0 {
                    line.push_str(&format!(" ({} triangles)", exported.triangles));
                }
                app_log::info(line);
                if !exported.skipped.is_empty() {
                    app_log::warn(format!(
                        "Left out, as meshes have no exact shape to write: {}",
                        exported.skipped.join(", ")
                    ));
                }
                if let Some(command) = open_with {
                    match open_in_slicer(&command, &path) {
                        Ok(program) => app_log::info(format!("Opened it with {program}")),
                        Err(err) => app_log::error(format!(
                            "Could not open it in the slicer: {err}. Set the slicer \
                             in Preferences › 3D printing."
                        )),
                    }
                }
            }
            Err(err) => app_log::error(format!("Export to {} failed: {err}", path.display())),
        }
    }

    /// The bodies the draft asks for: the selected one, or every visible
    /// body with geometry that is made, in the document's body order; or
    /// the print layout's copies.
    fn export_bodies(&self, draft: &ExportDraft) -> Vec<OwnedBody> {
        let document = &self.session.document;
        let textured = draft.textures && draft.format.is_mesh();
        if draft.layout {
            let layout = self.current_print_layout();
            return layout_bodies(document, &layout, textured);
        }
        let selected = self.session.selected_body;
        // What a bench says is not made (a bought part) stays out of an
        // export of everything.
        let not_made = self.registry.not_printed(document);
        bodies_to_export(
            document,
            |id| {
                if draft.selected_only {
                    selected == Some(id.0)
                } else {
                    document.imported_body_effective_visible(id) && !not_made.contains(&id)
                }
            },
            textured,
        )
    }

    /// The print layout as it stands: the one shown, when it is up to
    /// date, else made now.
    pub(crate) fn current_print_layout(&self) -> crate::app::print_layout::Layout {
        use crate::app::print_layout::{LayoutSettings, lay_out, layout_key, parts_to_print};
        let document = &self.session.document;
        let parts = parts_to_print(document, &self.registry);
        let settings = LayoutSettings::of(&self.user_settings.printing);
        let key = layout_key(document, &parts, &settings);
        match &self.session.print_layout {
            Some(shown) if shown.key == key => shown.layout.clone(),
            _ => lay_out(document, &parts, &settings),
        }
    }
}

/// The copies of a print layout, each its body's shape where the layout
/// puts it, named as the layout names them.
fn layout_bodies(
    document: &core_document::Document,
    layout: &crate::app::print_layout::Layout,
    textured: bool,
) -> Vec<OwnedBody> {
    let mut pictures = crate::app::textures::Pictures::new();
    layout
        .pieces
        .iter()
        .filter_map(|piece| {
            let pressing = textured
                .then(|| crate::app::textures::Pressing::of(document, piece.body, &mut pictures))
                .flatten();
            let brep = document.imported_brep_blob_arc(piece.body);
            let (local, _) = document.local_geometry(piece.body)?;
            if local.indices.is_empty() {
                return None;
            }
            Some(match brep {
                Some(brep) => OwnedBody {
                    name: piece.name.clone(),
                    brep: Some(brep),
                    transform: Some(piece.placement.rows()),
                    mesh: local,
                    pressing,
                },
                // A mesh body has only its triangles: pressed in its own
                // frame, then moved to the bed.
                None => {
                    let pressed = match &pressing {
                        Some(pressing) => pressing.apply(&local, surface_texture::Detail::EXPORT),
                        None => (*local).clone(),
                    };
                    OwnedBody {
                        name: piece.name.clone(),
                        brep: None,
                        transform: None,
                        mesh: Arc::new(piece.placement.mesh(&pressed)),
                        pressing: None,
                    }
                }
            })
        })
        .collect()
}

/// The script that writes every configuration: `base-<name>.<ext>` each,
/// `bodies` a Lua list of body ids or `nil` for every visible body, or
/// the print layout when `layout`.
pub(crate) fn every_configuration_script(
    base: &str,
    ext: &str,
    tolerance: f32,
    bodies: &str,
    layout: bool,
) -> String {
    format!(
        r#"-- Export every configuration
local base, ext, tolerance, bodies, layout = {base:?}, {ext:?}, {tolerance}, {bodies}, {layout}
local table = pc.config.list()
local was = table.active
for _, row in ipairs(table.rows) do
  pc.config.activate{{name = row.name}}
  pc.doc.rebuild()
  local file = base .. "-" .. row.name:gsub("[^%w%-_. ]", "-") .. "." .. ext
  pc.file.export{{path = file, tolerance = tolerance, bodies = bodies, layout = layout}}
  print("Wrote " .. file)
end
pc.config.activate{{name = was}}
"#
    )
}

/// A document name as a file name: letters, digits and a few marks kept,
/// anything else a dash.
fn file_stem_of(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if stem.trim_matches('-').is_empty() {
        "model".to_string()
    } else {
        stem
    }
}

/// A command line split into words: spaces separate, double quotes hold a
/// word with spaces together.
fn command_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in command.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// The program and arguments that open `file` with `command`: `{file}`
/// stands for the path, which otherwise goes last; an empty command hands
/// the file to the system's application for its type.
fn slicer_invocation(command: &str, file: &Path) -> (String, Vec<String>) {
    let mut words = command_words(command);
    if words.is_empty() {
        words.push(local_ipc::SYSTEM_OPENER.to_string());
    }
    let file = file.display().to_string();
    let program = words.remove(0);
    let named = words.iter().any(|w| w.contains("{file}"));
    let mut args: Vec<String> = words
        .into_iter()
        .map(|w| w.replace("{file}", &file))
        .collect();
    if !named {
        args.push(file);
    }
    (program, args)
}

/// Start the slicer on `file` beside the app, and say which program ran.
fn open_in_slicer(command: &str, file: &Path) -> std::io::Result<String> {
    let (program, args) = slicer_invocation(command, file);
    let mut child = local_ipc::background(&mut std::process::Command::new(&program))
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    // Collected when it exits, so it never lingers as a finished process.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(program)
}

/// Write `bodies` of `document` (every visible one when `None`), or the
/// copies of `layout` when given, to `path` now, on this thread: what a
/// script's export does.
pub(crate) fn export_document(
    document: &core_document::Document,
    path: PathBuf,
    format: ExportFormat,
    bodies: Option<Vec<core_document::BodyId>>,
    tolerance: Option<f32>,
    layout: Option<&crate::app::print_layout::Layout>,
) -> Result<(PathBuf, kernel_ogeom::export::Exported), String> {
    let owned = match layout {
        Some(layout) => layout_bodies(document, layout, format.is_mesh()),
        None => bodies_to_export(
            document,
            |id| match &bodies {
                Some(list) => list.contains(&id),
                None => document.imported_body_effective_visible(id),
            },
            format.is_mesh(),
        ),
    };
    if owned.is_empty() {
        return Err("there is nothing to export".to_string());
    }
    let mut detail = ExportDraft::default().detail;
    if let Some(tolerance) = tolerance {
        detail.chord_tolerance = tolerance;
    }
    let finishes: Vec<_> = owned.iter().map(OwnedBody::finish).collect();
    let borrowed = borrowed(&owned, &finishes);
    let path = with_extension(&path, format);
    let exported = export(&borrowed, format, &detail).map_err(|e| e.to_string())?;
    std::fs::write(&path, &exported.bytes).map_err(|e| format!("could not write the file: {e}"))?;
    Ok((path, exported))
}

/// The bodies of `document` that `take` accepts and that have geometry,
/// in tree order, their surface textures with them when `textured`.
fn bodies_to_export(
    document: &core_document::Document,
    take: impl Fn(core_document::BodyId) -> bool,
    textured: bool,
) -> Vec<OwnedBody> {
    let mut pictures = crate::app::textures::Pictures::new();
    let mut out: Vec<(usize, OwnedBody)> = document
        .imported_geometries()
        .filter(|(id, geometry)| !geometry.mesh.indices.is_empty() && take(**id))
        .map(|(id, geometry)| {
            let order = document
                .bodies()
                .iter()
                .position(|b| b.id == *id)
                .unwrap_or(usize::MAX);
            let name = document
                .bodies()
                .get(order)
                .map(|b| b.name.clone())
                .unwrap_or_else(|| format!("Body {}", &id.0.to_string()[..8]));
            (
                order,
                OwnedBody {
                    name,
                    brep: document.imported_brep_blob_arc(*id),
                    transform: {
                        let placement = document.body_placement(*id);
                        (!placement.is_identity()).then(|| placement.rows())
                    },
                    mesh: Arc::clone(&geometry.mesh),
                    pressing: textured
                        .then(|| crate::app::textures::Pressing::of(document, *id, &mut pictures))
                        .flatten(),
                },
            )
        })
        .collect();
    out.sort_by_key(|(order, body)| (*order, body.name.clone()));
    out.into_iter().map(|(_, body)| body).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 10 mm box built by the kernel, its top face textured.
    fn textured_box(textured: bool) -> core_document::Document {
        use kernel_api::{BooleanOp, Placement, PrimitiveKind, SolidOp};
        let built = kernel_ogeom::OgeomKernel::new()
            .execute_solid_chain(
                &[SolidOp::Primitive {
                    kind: PrimitiveKind::Box {
                        length: 10.0,
                        width: 10.0,
                        height: 10.0,
                    },
                    placement: Placement::default(),
                    op: BooleanOp::NewSolid,
                }],
                &TessellationSettings::default(),
            )
            .unwrap();
        let mut doc = core_document::Document::new("t");
        let body = doc.create_body(None);
        let mesh = Arc::new(built.mesh.clone());
        let top = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.faces)
            .find(|(t, _)| {
                t.iter()
                    .all(|i| (mesh.positions[*i as usize][2] - 10.0).abs() < 1e-4)
            })
            .map(|(_, f)| *f)
            .unwrap();
        doc.set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                bounds_mm: mesh.bounds(),
                mesh: Arc::clone(&mesh),
                source_asset: None,
                revision: 1,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        doc.set_imported_brep_data(body, built.brep_blob, Vec::new());
        if textured {
            doc.set_body_textures(
                body,
                vec![core_document::FaceTexture {
                    texture: surface_texture::Texture {
                        tile_mm: 2.0,
                        depth_mm: 0.5,
                        ..Default::default()
                    },
                    faces: vec![core_document::FaceKey::of(&mesh, top)],
                }],
            );
        }
        doc
    }

    /// An STL of a textured box carries the texture: many more triangles,
    /// and the top raised by up to the texture's depth.
    #[test]
    fn a_mesh_export_presses_the_texture_in() {
        let top_of = |bytes: &[u8]| {
            let mesh = ogeom_stl(bytes);
            mesh.1
        };
        let dir = std::env::temp_dir();
        let (plain_path, plain) = export_document(
            &textured_box(false),
            dir.join(format!("plain-{}.stl", uuid::Uuid::new_v4())),
            ExportFormat::Stl,
            None,
            None,
            None,
        )
        .unwrap();
        let (textured_path, textured) = export_document(
            &textured_box(true),
            dir.join(format!("textured-{}.stl", uuid::Uuid::new_v4())),
            ExportFormat::Stl,
            None,
            None,
            None,
        )
        .unwrap();
        let _ = std::fs::remove_file(plain_path);
        let _ = std::fs::remove_file(textured_path);
        assert!(
            textured.triangles > plain.triangles * 20,
            "{} vs {}",
            textured.triangles,
            plain.triangles
        );
        let (plain_top, textured_top) = (top_of(&plain.bytes), top_of(&textured.bytes));
        assert!((plain_top - 10.0).abs() < 1e-3, "{plain_top}");
        assert!(
            textured_top > 10.3 && textured_top <= 10.5 + 1e-3,
            "{textured_top}"
        );
    }

    /// An export of the print layout writes every copy where the layout
    /// puts it: three boxes side by side on the bed, the model untouched.
    #[test]
    fn a_layout_export_writes_each_copy_on_the_bed() {
        use crate::app::print_layout::{LayoutSettings, lay_out};
        let mut doc = textured_box(false);
        let body = doc.bodies()[0].id;
        doc.set_body_placement(
            body,
            core_document::BodyPlacement::new(
                glam::Quat::IDENTITY,
                glam::Vec3::new(500.0, 0.0, 40.0),
            ),
        );
        let parts = [core_document::PrintPart {
            name: "Box".into(),
            bodies: vec![body],
            count: 3,
        }];
        let settings = LayoutSettings {
            bed_mm: [100.0, 100.0, 100.0],
            origin_center: false,
            gap_mm: 4.0,
        };
        let layout = lay_out(&doc, &parts, &settings);
        let path = std::env::temp_dir().join(format!("layout-{}.stl", uuid::Uuid::new_v4()));
        let (path, exported) =
            export_document(&doc, path, ExportFormat::Stl, None, None, Some(&layout)).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(exported.written, 3);
        let bytes = &exported.bytes;
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for t in 0..count {
            for v in 0..3 {
                let at = 84 + t * 50 + 12 + v * 12;
                for k in 0..3 {
                    let c =
                        f32::from_le_bytes(bytes[at + k * 4..at + k * 4 + 4].try_into().unwrap());
                    lo[k] = lo[k].min(c);
                    hi[k] = hi[k].max(c);
                }
            }
        }
        // Three 10 mm boxes 4 mm apart, 2 mm in from the bed's corner.
        assert!(
            (lo[0] - 2.0).abs() < 1e-3 && (hi[0] - 40.0).abs() < 1e-3,
            "{lo:?} {hi:?}"
        );
        assert!(lo[2].abs() < 1e-3 && (hi[2] - 10.0).abs() < 1e-3);
        assert!(
            doc.body_placement(body).offset().x > 499.0,
            "the body stays put"
        );
    }

    /// A binary STL's triangle count and highest point.
    fn ogeom_stl(bytes: &[u8]) -> (usize, f32) {
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        let mut top = f32::MIN;
        for t in 0..count {
            let at = 84 + t * 50 + 12;
            for v in 0..3 {
                let z = at + v * 12 + 8;
                top = top.max(f32::from_le_bytes(bytes[z..z + 4].try_into().unwrap()));
            }
        }
        (count, top)
    }

    #[test]
    fn the_slicer_command_takes_the_file_where_it_says_or_last() {
        let file = Path::new("/tmp/printcad/slicer/part.3mf");
        let (program, args) = slicer_invocation("", file);
        assert_eq!(program, local_ipc::SYSTEM_OPENER);
        assert_eq!(args, ["/tmp/printcad/slicer/part.3mf"]);
        let (program, args) = slicer_invocation("\"/opt/My Slicer/run\" --single", file);
        assert_eq!(program, "/opt/My Slicer/run");
        assert_eq!(args, ["--single", "/tmp/printcad/slicer/part.3mf"]);
        let (_, args) = slicer_invocation("slice --load={file} --go", file);
        assert_eq!(args, ["--load=/tmp/printcad/slicer/part.3mf", "--go"]);
    }

    #[test]
    fn a_document_name_becomes_a_safe_file_name() {
        assert_eq!(file_stem_of("Bracket v2"), "Bracket-v2");
        assert_eq!(file_stem_of("a/b"), "a-b");
        assert_eq!(file_stem_of("   "), "model");
    }

    #[test]
    fn a_file_name_takes_the_format_s_extension_unless_it_has_it() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            with_extension(&p("/t/a.3mf"), ExportFormat::ThreeMf),
            p("/t/a.3mf")
        );
        assert_eq!(
            with_extension(&p("/t/a.STP"), ExportFormat::Step),
            p("/t/a.STP")
        );
        assert_eq!(with_extension(&p("/t/a"), ExportFormat::Stl), p("/t/a.stl"));
        assert_eq!(
            with_extension(&p("/t/a.stl"), ExportFormat::ThreeMf),
            p("/t/a.stl.3mf")
        );
    }
}
