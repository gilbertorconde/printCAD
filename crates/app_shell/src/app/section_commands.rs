//! The commands of sections: the document's saved ones (`doc.sections`,
//! `doc.save_section`, `doc.remove_section`, which a script run without a
//! window takes too) and the plane the view cuts at (`view.section`).

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, Document, ParamKind,
    SavedSection,
};
use glam::Vec3;
use serde_json::{Value, json};

use crate::PrintCadApp;
use crate::camera::section::{SectionAim, SectionAxis, SectionPlane, SectionToggle, Wedge};

pub(crate) fn doc_section_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("doc.sections", "The document's saved sections")
            .returns(
                "a list of {name, aim, origin, normal, offset, flipped, formula, uncut, wedge}",
            )
            .read_only(),
        plane_params(CommandSpec::new(
            "doc.save_section",
            "Keep a section with the document by name, to switch the view to later",
        ))
        .param(
            "name",
            ParamKind::String,
            "Its name; one already kept by it is replaced",
        )
        .optional("formula", ParamKind::String, "A formula setting its offset")
        .returns("nil"),
        CommandSpec::new("doc.remove_section", "Drop a saved section")
            .param("name", ParamKind::String, "Its name")
            .returns("nil"),
    ]
    .into_iter()
    .map(|spec| prose().explain(spec))
    .collect()
}

fn plane_params(spec: CommandSpec) -> CommandSpec {
    spec.optional("axis", ParamKind::String, "x, y or z: square to that axis")
        .optional("origin", ParamKind::List, "A point of the plane, {x, y, z}")
        .optional("normal", ParamKind::List, "The way it faces, {x, y, z}")
        .optional(
            "offset",
            ParamKind::Number,
            "mm along the normal; 0 when left out",
        )
        .optional("flipped", ParamKind::Bool, "Keep the other side")
        .optional("uncut", ParamKind::List, "Ids of bodies it leaves whole")
        .optional(
            "wedge_normal",
            ParamKind::List,
            "A wedge's second plane: the way it faces",
        )
        .optional(
            "wedge_origin",
            ParamKind::List,
            "A point of the second plane, {x, y, z}",
        )
        .optional(
            "wedge_flipped",
            ParamKind::Bool,
            "The second plane keeps its other side",
        )
}

/// The app's command of the view's plane.
pub(crate) fn view_section_command() -> CommandSpec {
    let spec = plane_params(CommandSpec::new(
        "view.section",
        "Cut the view at a plane, a saved section or none",
    ))
    .optional("name", ParamKind::String, "A saved section to show")
    .optional("off", ParamKind::Bool, "true takes the plane away")
    .returns("the plane the view cuts at, as doc.sections lists one, or nil")
    .read_only();
    prose().explain(spec)
}

/// The notes, related commands and examples of these commands.
pub(crate) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(
        &PROSE,
        "section_commands.md",
        include_str!("section_commands.md"),
        &[],
    )
}

fn vec3(a: &Args<'_>, name: &str) -> Result<Option<Vec3>, CommandError> {
    Ok(a.opt_vec3(name)?.map(|v| glam::DVec3::from(v).as_vec3()))
}

fn uncut(a: &Args<'_>) -> Result<Vec<BodyId>, CommandError> {
    let Some(value) = a.0.get("uncut").filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| CommandError::bad("uncut", "must be a list of body ids"))?
        .iter()
        .map(|v| {
            v.as_str()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(BodyId)
                .ok_or_else(|| CommandError::bad("uncut", "must be a list of body ids"))
        })
        .collect()
}

/// Whether the arguments give a plane.
fn gives_plane(a: &Args<'_>) -> bool {
    a.has("axis") || a.has("normal")
}

/// The plane the arguments give.
fn plane_of(a: &Args<'_>) -> Result<SectionPlane, CommandError> {
    let (normal, base, aim) = match (a.opt_string("axis")?, vec3(a, "normal")?) {
        (Some(axis), None) => {
            let axis = match axis {
                "x" | "X" => SectionAxis::X,
                "y" | "Y" => SectionAxis::Y,
                "z" | "Z" => SectionAxis::Z,
                _ => return Err(CommandError::bad("axis", "must be x, y or z")),
            };
            (axis.unit(), 0.0, SectionAim::Axis(axis))
        }
        (None, Some(normal)) => {
            let origin = vec3(a, "origin")?.unwrap_or(Vec3::ZERO);
            let normal = normal
                .try_normalize()
                .ok_or_else(|| CommandError::bad("normal", "must not be zero"))?;
            (normal, origin.dot(normal), SectionAim::Face)
        }
        (Some(_), Some(_)) => {
            return Err(CommandError::bad(
                "axis",
                "give an axis or a normal, not both",
            ));
        }
        (None, None) => return Err(CommandError::bad("axis", "or `normal` is required")),
    };
    Ok(SectionPlane {
        normal,
        base,
        offset: a.opt_number("offset")?.unwrap_or(0.0) as f32,
        flipped: a.opt_bool("flipped")?.unwrap_or(false),
        aim,
        formula: None,
        uncut: uncut(a)?,
        wedge: match vec3(a, "wedge_normal")? {
            Some(normal) => {
                let normal = normal
                    .try_normalize()
                    .ok_or_else(|| CommandError::bad("wedge_normal", "must not be zero"))?;
                let origin = vec3(a, "wedge_origin")?.unwrap_or(Vec3::ZERO);
                Some(Wedge {
                    normal,
                    base: origin.dot(normal),
                    offset: 0.0,
                    flipped: a.opt_bool("wedge_flipped")?.unwrap_or(false),
                })
            }
            None => None,
        },
    })
}

/// A saved section as the commands answer it.
fn answer(saved: &SavedSection) -> Value {
    let normal = Vec3::from_array(saved.normal).normalize_or_zero();
    json!({
        "name": saved.name,
        "aim": format!("{:?}", saved.aim).to_lowercase(),
        "origin": (normal * saved.base).to_array(),
        "normal": normal.to_array(),
        "offset": saved.offset,
        "flipped": saved.flipped,
        "formula": saved.formula,
        "uncut": saved.uncut.iter().map(|b| b.0.to_string()).collect::<Vec<_>>(),
        "wedge": saved.wedge.map(|w| {
            let normal = Vec3::from_array(w.normal).normalize_or_zero();
            json!({
                "origin": (normal * (w.base + w.offset)).to_array(),
                "normal": normal.to_array(),
                "flipped": w.flipped,
            })
        }),
    })
}

/// The saved sections' commands on `document`, or `None` for another
/// command (and for `doc.save_section` given no plane, which the view
/// answers).
pub(crate) fn document_section_command(
    id: &str,
    args: &CommandArgs,
    document: &mut Document,
) -> Option<CommandResult> {
    let a = Args(args);
    let answer = (|| match id {
        "doc.sections" => Ok(Value::Array(
            document.saved_sections().iter().map(answer).collect(),
        )),
        "doc.save_section" => {
            if !gives_plane(&a) {
                return Err(CommandError::failed(
                    "give a plane: `axis`, or `origin` and `normal`; there is no view to take one from",
                ));
            }
            let mut plane = plane_of(&a)?;
            plane.formula = a.opt_string("formula")?.map(str::to_string);
            document.save_section(plane.saved(a.string("name")?));
            Ok(Value::Null)
        }
        "doc.remove_section" => {
            let name = a.string("name")?;
            if document.remove_saved_section(name) {
                Ok(Value::Null)
            } else {
                Err(CommandError::bad("name", format!("no section `{name}`")))
            }
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    })();
    match answer {
        Err(CommandError::Unknown(_)) => None,
        answer => Some(answer),
    }
}

impl PrintCadApp {
    /// The clip pill's Sections menu: saving is one undo step under the
    /// section's name; showing and dropping a name the document lacks do
    /// nothing.
    pub(crate) fn saved_section_action(&mut self, action: crate::ui::SavedSectionAction) {
        use crate::ui::SavedSectionAction as Action;
        match action {
            Action::Save(name) => {
                let Some(plane) = &self.session.section else {
                    return;
                };
                self.session.document.save_section(plane.saved(&name));
                self.session
                    .journal
                    .label_next(format!("Save section {name}"));
                self.close_gesture();
            }
            Action::Show(name) => {
                if let Some(plane) = self
                    .session
                    .document
                    .saved_section(&name)
                    .and_then(SectionPlane::from_saved)
                {
                    self.set_section(Some(SectionToggle::Set(plane)));
                }
            }
            Action::Remove(name) => {
                if self.session.document.remove_saved_section(&name) {
                    self.session
                        .journal
                        .label_next(format!("Remove section {name}"));
                    self.close_gesture();
                }
            }
        }
    }

    /// `doc.save_section` given no plane: the one the view cuts at.
    pub(crate) fn save_view_section(&mut self, args: &CommandArgs) -> CommandResult {
        let a = Args(args);
        let plane = self
            .session
            .section
            .clone()
            .ok_or_else(|| CommandError::failed("the view cuts at no plane; give one"))?;
        let mut saved = plane.saved(a.string("name")?);
        if let Some(formula) = a.opt_string("formula")? {
            saved.formula = Some(formula.to_string());
        }
        self.session.document.save_section(saved);
        Ok(Value::Null)
    }

    /// `view.section`.
    pub(crate) fn view_section(&mut self, args: &CommandArgs) -> CommandResult {
        let a = Args(args);
        if a.opt_bool("off")? == Some(true) {
            self.set_section(None);
        } else if let Some(name) = a.opt_string("name")? {
            let plane = self
                .session
                .document
                .saved_section(name)
                .and_then(SectionPlane::from_saved)
                .ok_or_else(|| CommandError::bad("name", format!("no section `{name}`")))?;
            self.set_section(Some(SectionToggle::Set(plane)));
        } else if gives_plane(&a) {
            let plane = plane_of(&a)?;
            self.set_section(Some(SectionToggle::Set(plane)));
        }
        self.redraw_needed = true;
        Ok(self
            .session
            .section
            .as_ref()
            .map_or(Value::Null, |plane| answer(&plane.saved(""))))
    }
}
