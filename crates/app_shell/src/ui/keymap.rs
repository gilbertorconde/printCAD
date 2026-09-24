//! Keyboard shortcuts: one keymap of the application's own commands and
//! every workbench's tools and actions, each with default keys the user can
//! change in Preferences › Keyboard.
//!
//! A workbench's keys apply while it is active and win over the
//! application's for the same key. While a text field has focus, keys that
//! would type text are left to it, as are the text editing keys (undo,
//! redo and the clipboard).

use core_document::{Chord, DocumentService, KeyCode, WorkbenchId};
use settings::KeyboardSettings;

use super::feature_tree::{TreeFeatureCommand, TreeItemId};
use super::{EditCommand, FileCommand, UiCommand};
use crate::orientation_cube::CameraSnapView;

/// An application command a key can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAction {
    New,
    Open,
    Save,
    SaveAs,
    Import,
    Export,
    SendToSlicer,
    Quit,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    Palette,
    Preferences,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    FitAll,
    FitSelection,
    PivotAtCursor,
    Isometric,
    Front,
    Top,
    Right,
    Rear,
    Bottom,
    Left,
    Orthographic,
    Perspective,
    ClippingPlane,
    Measure,
    PrintBed,
    Recompute,
    LogPanel,
    Delete,
    ToggleVisibility,
    ShadedWithEdges,
    Shaded,
    Wireframe,
    Console,
    RunScript,
    Record,
}

struct HostSpec {
    action: HostAction,
    id: &'static str,
    label: &'static str,
    group: &'static str,
    keys: &'static [&'static str],
    /// Does nothing without a document on screen.
    needs_document: bool,
    /// A text field's own key: never taken from one that has focus.
    text_owned: bool,
}

const fn spec(
    action: HostAction,
    id: &'static str,
    label: &'static str,
    group: &'static str,
    keys: &'static [&'static str],
) -> HostSpec {
    HostSpec {
        action,
        id,
        label,
        group,
        keys,
        needs_document: true,
        text_owned: false,
    }
}

const fn anywhere(mut spec: HostSpec) -> HostSpec {
    spec.needs_document = false;
    spec
}

const fn text_owned(mut spec: HostSpec) -> HostSpec {
    spec.text_owned = true;
    spec
}

/// The application's commands, in the order Preferences lists them.
const HOST: &[HostSpec] = {
    use HostAction::*;
    &[
        anywhere(spec(New, "file.new", "New", "File", &["Ctrl+N"])),
        anywhere(spec(Open, "file.open", "Open", "File", &["Ctrl+O"])),
        spec(Save, "file.save", "Save", "File", &["Ctrl+S"]),
        spec(SaveAs, "file.save_as", "Save as", "File", &["Ctrl+Shift+S"]),
        spec(Import, "file.import", "Import", "File", &["Ctrl+I"]),
        spec(Export, "file.export", "Export", "File", &["Ctrl+E"]),
        spec(
            SendToSlicer,
            "file.send_to_slicer",
            "Send to slicer",
            "File",
            &["Ctrl+P"],
        ),
        anywhere(spec(
            RunScript,
            "file.run_script",
            "Run script",
            "File",
            &[],
        )),
        anywhere(spec(
            Record,
            "app.record",
            "Record a script, or stop",
            "File",
            &[],
        )),
        anywhere(spec(Quit, "app.quit", "Quit", "File", &["Ctrl+Q"])),
        text_owned(spec(Undo, "edit.undo", "Undo", "Edit", &["Ctrl+Z"])),
        text_owned(spec(
            Redo,
            "edit.redo",
            "Redo",
            "Edit",
            &["Ctrl+Shift+Z", "Ctrl+Y"],
        )),
        text_owned(spec(Cut, "edit.cut", "Cut", "Edit", &["Ctrl+X"])),
        text_owned(spec(Copy, "edit.copy", "Copy", "Edit", &["Ctrl+C"])),
        text_owned(spec(Paste, "edit.paste", "Paste", "Edit", &["Ctrl+V"])),
        spec(
            Delete,
            "edit.delete",
            "Delete the selected item",
            "Edit",
            &["Delete"],
        ),
        spec(
            Recompute,
            "edit.recompute",
            "Recompute all",
            "Edit",
            &["Ctrl+R"],
        ),
        spec(FitAll, "view.fit_all", "Fit all", "View", &["F"]),
        spec(
            FitSelection,
            "view.fit_selection",
            "Fit selection",
            "View",
            &["Shift+F"],
        ),
        spec(
            ToggleVisibility,
            "view.toggle_visibility",
            "Show or hide the selected item",
            "View",
            &["Space"],
        ),
        spec(
            PivotAtCursor,
            "view.pivot",
            "Orbit around the point under the cursor",
            "View",
            &["H"],
        ),
        spec(
            Isometric,
            "view.isometric",
            "Isometric view",
            "View",
            &["0"],
        ),
        spec(Front, "view.front", "Front view", "View", &["1"]),
        spec(Top, "view.top", "Top view", "View", &["2"]),
        spec(Right, "view.right", "Right view", "View", &["3"]),
        spec(Rear, "view.rear", "Rear view", "View", &["4"]),
        spec(Bottom, "view.bottom", "Bottom view", "View", &["5"]),
        spec(Left, "view.left", "Left view", "View", &["6"]),
        spec(
            Orthographic,
            "view.orthographic",
            "Orthographic",
            "View",
            &["O"],
        ),
        spec(
            Perspective,
            "view.perspective",
            "Perspective",
            "View",
            &["P"],
        ),
        spec(
            ShadedWithEdges,
            "view.shaded_edges",
            "Shaded with edges",
            "View",
            &[],
        ),
        spec(Shaded, "view.shaded", "Shaded", "View", &[]),
        spec(Wireframe, "view.wireframe", "Wireframe", "View", &[]),
        spec(
            ClippingPlane,
            "view.clipping_plane",
            "Clipping plane",
            "View",
            &[],
        ),
        spec(Measure, "view.measure", "Measure", "View", &[]),
        spec(PrintBed, "view.print_bed", "Print bed", "View", &[]),
        anywhere(spec(NewTab, "tab.new", "New tab", "Tabs", &["Ctrl+T"])),
        anywhere(spec(
            CloseTab,
            "tab.close",
            "Close tab",
            "Tabs",
            &["Ctrl+W"],
        )),
        anywhere(spec(NextTab, "tab.next", "Next tab", "Tabs", &["Ctrl+Tab"])),
        anywhere(spec(
            PreviousTab,
            "tab.previous",
            "Previous tab",
            "Tabs",
            &["Ctrl+Shift+Tab"],
        )),
        anywhere(spec(
            Palette,
            "app.palette",
            "Command palette",
            "Application",
            &["Ctrl+K"],
        )),
        anywhere(spec(
            Preferences,
            "app.preferences",
            "Preferences",
            "Application",
            &["Ctrl+,"],
        )),
        anywhere(spec(LogPanel, "app.log", "Log panel", "Application", &[])),
        anywhere(spec(
            Console,
            "app.console",
            "Script console",
            "Application",
            &[],
        )),
    ]
};

/// The application's commands a key can run, as `(id, label, action)`,
/// for callers that name them rather than press them.
pub fn host_actions() -> impl Iterator<Item = (&'static str, &'static str, HostAction)> {
    HOST.iter().map(|spec| (spec.id, spec.label, spec.action))
}

/// What a binding runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Host(HostAction),
    /// A workbench tool, activated as a click on its button would.
    Tool,
    /// A workbench's registered keyboard action.
    Action,
    /// A script of the scripts folder.
    Script(std::path::PathBuf),
}

/// One entry of the keymap.
#[derive(Debug, Clone)]
pub struct Binding {
    /// The command's id, which the user's changes are saved under.
    pub id: String,
    pub label: String,
    /// The heading Preferences lists it under: a menu name, or the
    /// workbench's name.
    pub group: String,
    /// The workbench it belongs to; `None` for the application's own.
    pub scope: Option<WorkbenchId>,
    pub target: Target,
    pub defaults: Vec<Chord>,
    /// The keys in effect: the defaults, or the user's.
    pub keys: Vec<Chord>,
    pub needs_document: bool,
    pub text_owned: bool,
}

impl Binding {
    /// Whether the user changed its keys.
    pub fn is_changed(&self) -> bool {
        self.keys != self.defaults
    }

    /// Whether both can fire in the same place: both global, or both in
    /// the same workbench, or one global and one in a workbench.
    fn overlaps(&self, other: &Binding) -> bool {
        match (&self.scope, &other.scope) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    }
}

/// Two bindings on one key where both could fire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    pub key: Chord,
    pub other: String,
    /// Where the other binding lives: a menu name or a workbench's name.
    pub other_group: String,
    /// The other binding belongs to a workbench and this one is global (or
    /// the reverse): the workbench's wins while it is active, so this is a
    /// shadow rather than a tie.
    pub shadowed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// The application's commands, then every registered workbench's tools
    /// and actions in registration order, each with the user's keys where
    /// the user changed them.
    pub fn build(
        registry: &DocumentService,
        user: &KeyboardSettings,
        scripts: &[crate::script_library::ScriptEntry],
    ) -> Self {
        let mut bindings = Vec::new();
        for spec in HOST {
            bindings.push(Binding {
                id: spec.id.to_string(),
                label: spec.label.to_string(),
                group: spec.group.to_string(),
                scope: None,
                target: Target::Host(spec.action),
                defaults: parse_all(spec.keys.iter().copied()),
                keys: Vec::new(),
                needs_document: spec.needs_document,
                text_owned: spec.text_owned,
            });
        }
        for bench in registry.ids() {
            let group = registry
                .descriptor(bench)
                .map(|d| d.label.clone())
                .unwrap_or_else(|| bench.as_str().to_string());
            let mut add = |id: &str, label: &str, target: Target, defaults: &[Chord]| {
                bindings.push(Binding {
                    id: id.to_string(),
                    label: label.to_string(),
                    group: group.clone(),
                    scope: Some(bench.clone()),
                    target,
                    defaults: defaults.to_vec(),
                    keys: Vec::new(),
                    needs_document: true,
                    text_owned: false,
                });
            };
            for tool in registry.tools_for(bench).into_iter().flatten() {
                if tool.planned.is_none() {
                    add(&tool.id, &tool.label, Target::Tool, &tool.shortcuts);
                }
            }
            for action in registry.actions_for(bench).into_iter().flatten() {
                add(&action.id, &action.label, Target::Action, &action.shortcuts);
            }
        }
        for script in scripts {
            bindings.push(Binding {
                id: script.id.clone(),
                label: script.name.clone(),
                group: "Scripts".to_string(),
                scope: None,
                target: Target::Script(script.path.clone()),
                defaults: Vec::new(),
                keys: Vec::new(),
                needs_document: false,
                text_owned: false,
            });
        }
        for binding in &mut bindings {
            binding.keys = match user.bindings.get(&binding.id) {
                Some(keys) => parse_all(keys.iter().map(String::as_str)),
                None => binding.defaults.clone(),
            };
        }
        Self { bindings }
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub fn get(&self, id: &str) -> Option<&Binding> {
        self.bindings.iter().find(|b| b.id == id)
    }

    /// The binding `key` runs with `active` the active workbench: the
    /// workbench's own first, then the application's.
    pub fn lookup(&self, key: Chord, active: &WorkbenchId) -> Option<&Binding> {
        let has = |b: &&Binding| b.keys.contains(&key);
        self.bindings
            .iter()
            .filter(has)
            .find(|b| b.scope.as_ref() == Some(active))
            .or_else(|| self.bindings.iter().filter(has).find(|b| b.scope.is_none()))
    }

    /// The keys of every workbench tool and action that has one, by id,
    /// for `Workbench::shortcuts_changed`.
    pub fn workbench_keys(&self) -> std::collections::HashMap<String, Vec<Chord>> {
        self.bindings
            .iter()
            .filter(|b| b.scope.is_some() && !b.keys.is_empty())
            .map(|b| (b.id.clone(), b.keys.clone()))
            .collect()
    }

    /// The first key of a command, as menus show it.
    pub fn text(&self, id: &str) -> Option<String> {
        self.get(id)
            .and_then(|b| b.keys.first())
            .map(ToString::to_string)
    }

    /// Every other binding that shares a key with `id` and could fire in
    /// the same place.
    pub fn clashes(&self, id: &str) -> Vec<Clash> {
        let Some(this) = self.get(id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for other in self
            .bindings
            .iter()
            .filter(|b| b.id != id && this.overlaps(b))
        {
            for key in this.keys.iter().filter(|k| other.keys.contains(k)) {
                out.push(Clash {
                    key: *key,
                    other: other.label.clone(),
                    other_group: other.group.clone(),
                    shadowed: this.scope.is_some() != other.scope.is_some(),
                });
            }
        }
        out
    }
}

/// Give `binding` these keys in the user's settings: an entry when they
/// differ from its defaults, none when they are the defaults again.
pub fn set_keys(user: &mut KeyboardSettings, binding: &Binding, keys: Vec<Chord>) {
    if keys == binding.defaults {
        user.bindings.remove(&binding.id);
    } else {
        user.bindings.insert(
            binding.id.clone(),
            keys.iter().map(ToString::to_string).collect(),
        );
    }
}

/// The chords of a list of texts, leaving out any that do not parse.
fn parse_all<'a>(texts: impl Iterator<Item = &'a str>) -> Vec<Chord> {
    texts.filter_map(Chord::parse).collect()
}

/// The chord of a key egui reports, if a chord can name it.
pub fn chord_of(key: egui::Key, modifiers: egui::Modifiers) -> Option<Chord> {
    Some(Chord {
        ctrl: modifiers.ctrl || modifiers.command || modifiers.mac_cmd,
        shift: modifiers.shift,
        alt: modifiers.alt,
        key: key_code(key)?,
    })
}

fn key_code(key: egui::Key) -> Option<KeyCode> {
    use egui::Key as E;
    Some(match key {
        E::A => KeyCode::A,
        E::B => KeyCode::B,
        E::C => KeyCode::C,
        E::D => KeyCode::D,
        E::E => KeyCode::E,
        E::F => KeyCode::F,
        E::G => KeyCode::G,
        E::H => KeyCode::H,
        E::I => KeyCode::I,
        E::J => KeyCode::J,
        E::K => KeyCode::K,
        E::L => KeyCode::L,
        E::M => KeyCode::M,
        E::N => KeyCode::N,
        E::O => KeyCode::O,
        E::P => KeyCode::P,
        E::Q => KeyCode::Q,
        E::R => KeyCode::R,
        E::S => KeyCode::S,
        E::T => KeyCode::T,
        E::U => KeyCode::U,
        E::V => KeyCode::V,
        E::W => KeyCode::W,
        E::X => KeyCode::X,
        E::Y => KeyCode::Y,
        E::Z => KeyCode::Z,
        E::Num0 => KeyCode::Key0,
        E::Num1 => KeyCode::Key1,
        E::Num2 => KeyCode::Key2,
        E::Num3 => KeyCode::Key3,
        E::Num4 => KeyCode::Key4,
        E::Num5 => KeyCode::Key5,
        E::Num6 => KeyCode::Key6,
        E::Num7 => KeyCode::Key7,
        E::Num8 => KeyCode::Key8,
        E::Num9 => KeyCode::Key9,
        E::F1 => KeyCode::F1,
        E::F2 => KeyCode::F2,
        E::F3 => KeyCode::F3,
        E::F4 => KeyCode::F4,
        E::F5 => KeyCode::F5,
        E::F6 => KeyCode::F6,
        E::F7 => KeyCode::F7,
        E::F8 => KeyCode::F8,
        E::F9 => KeyCode::F9,
        E::F10 => KeyCode::F10,
        E::F11 => KeyCode::F11,
        E::F12 => KeyCode::F12,
        E::Escape => KeyCode::Escape,
        E::Enter => KeyCode::Enter,
        E::Space => KeyCode::Space,
        E::Delete => KeyCode::Delete,
        E::Backspace => KeyCode::Backspace,
        E::Tab => KeyCode::Tab,
        E::ArrowUp => KeyCode::ArrowUp,
        E::ArrowDown => KeyCode::ArrowDown,
        E::ArrowLeft => KeyCode::ArrowLeft,
        E::ArrowRight => KeyCode::ArrowRight,
        E::Home => KeyCode::Home,
        E::End => KeyCode::End,
        E::PageUp => KeyCode::PageUp,
        E::PageDown => KeyCode::PageDown,
        E::Insert => KeyCode::Insert,
        E::Period => KeyCode::Period,
        E::Comma => KeyCode::Comma,
        E::Minus => KeyCode::Minus,
        E::Equals => KeyCode::Equals,
        E::Slash => KeyCode::Slash,
        _ => return None,
    })
}

/// The first key pressed this frame, with its modifiers, taken out of the
/// input so nothing else acts on it. Preferences records a new binding
/// with it. A bare modifier is not a key.
pub fn take_any_chord(ctx: &egui::Context) -> Option<Chord> {
    ctx.input_mut(|i| {
        let mut found = None;
        i.events.retain(|event| {
            if found.is_some() {
                return true;
            }
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => match chord_of(*key, *modifiers) {
                    Some(chord) => {
                        found = Some(chord);
                        false
                    }
                    None => true,
                },
                _ => true,
            }
        });
        found
    })
}

/// Where the keyboard's input is going this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyFocus {
    /// A text field has focus.
    pub typing: bool,
    /// The active workbench is taking a typed number.
    pub numeric: bool,
    /// A document is on screen.
    pub have_document: bool,
}

/// Whether `chord` is a key that types a number.
fn types_number(chord: Chord) -> bool {
    use KeyCode::*;
    !chord.ctrl
        && !chord.alt
        && !chord.shift
        && matches!(
            chord.key,
            Key0 | Key1
                | Key2
                | Key3
                | Key4
                | Key5
                | Key6
                | Key7
                | Key8
                | Key9
                | Period
                | Comma
                | Minus
        )
}

/// The bindings whose keys were pressed this frame, taken out of the input
/// so no widget acts on them too.
pub fn take_pressed(
    ctx: &egui::Context,
    keymap: &Keymap,
    active: &WorkbenchId,
    focus: KeyFocus,
) -> Vec<Binding> {
    let KeyFocus {
        typing,
        numeric,
        have_document,
    } = focus;
    ctx.input_mut(|i| {
        let mut hits = Vec::new();
        i.events.retain(|event| {
            let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            let Some(chord) = chord_of(*key, *modifiers) else {
                return true;
            };
            let Some(binding) = keymap.lookup(chord, active) else {
                return true;
            };
            if typing && (chord.types_text() || binding.text_owned) {
                return true;
            }
            if numeric && types_number(chord) {
                return true;
            }
            if binding.needs_document && !have_document {
                return true;
            }
            if !repeat {
                hits.push(binding.clone());
            }
            false
        });
        hits
    })
}

/// What the application does for one of its commands.
pub enum HostOutcome {
    Command(UiCommand),
    OpenPalette,
    OpenPreferences,
    ToggleConsole,
    Nothing,
}

/// What an application binding reads of the moment it runs in.
pub struct HostState<'a> {
    pub active_tab: Option<uuid::Uuid>,
    /// The clipping plane is showing, so its key puts it away.
    pub section_on: bool,
    pub document: &'a core_document::Document,
    /// The tree's selected row, which Delete and Space act on.
    pub tree_selection: Option<TreeItemId>,
    /// A workbench is editing a feature, which keeps Delete for itself.
    pub editing: bool,
}

/// The command an application binding runs.
pub fn host_outcome(action: HostAction, state: &HostState<'_>) -> HostOutcome {
    use HostAction::*;
    use HostOutcome::Command as C;
    let HostState {
        active_tab,
        section_on,
        ..
    } = *state;
    match action {
        New => C(UiCommand::File(FileCommand::New)),
        Open => C(UiCommand::File(FileCommand::Open)),
        Save => C(UiCommand::File(FileCommand::Save)),
        SaveAs => C(UiCommand::File(FileCommand::SaveAs)),
        Import => C(UiCommand::File(FileCommand::ImportStep)),
        Export => C(UiCommand::File(FileCommand::Export)),
        SendToSlicer => C(UiCommand::File(FileCommand::SendToSlicer)),
        Quit => C(UiCommand::Quit),
        NewTab => C(UiCommand::NewTab),
        CloseTab => match active_tab {
            Some(tab) => C(UiCommand::CloseTab(tab)),
            None => HostOutcome::Nothing,
        },
        NextTab => C(UiCommand::CycleTab(1)),
        PreviousTab => C(UiCommand::CycleTab(-1)),
        Palette => HostOutcome::OpenPalette,
        Preferences => HostOutcome::OpenPreferences,
        Undo => C(UiCommand::Undo),
        Redo => C(UiCommand::Redo),
        Cut => C(UiCommand::Edit(EditCommand::Cut)),
        Copy => C(UiCommand::Edit(EditCommand::Copy)),
        Paste => C(UiCommand::Edit(EditCommand::Paste)),
        FitAll => C(UiCommand::FitView),
        FitSelection => C(UiCommand::FitSelection),
        PivotAtCursor => C(UiCommand::PivotAtCursor),
        Isometric => C(UiCommand::CameraSnap(CameraSnapView::FrontTopRight)),
        Front => C(UiCommand::CameraSnap(CameraSnapView::Front)),
        Top => C(UiCommand::CameraSnap(CameraSnapView::Top)),
        Right => C(UiCommand::CameraSnap(CameraSnapView::Right)),
        Rear => C(UiCommand::CameraSnap(CameraSnapView::Rear)),
        Bottom => C(UiCommand::CameraSnap(CameraSnapView::Bottom)),
        Left => C(UiCommand::CameraSnap(CameraSnapView::Left)),
        Orthographic => C(UiCommand::SetProjection(
            settings::ProjectionMode::Orthographic,
        )),
        Perspective => C(UiCommand::SetProjection(
            settings::ProjectionMode::Perspective,
        )),
        ClippingPlane => C(UiCommand::SetSection(
            (!section_on).then_some(crate::camera::section::SectionToggle::On),
        )),
        Measure => C(UiCommand::ToggleMeasure),
        PrintBed => C(UiCommand::TogglePrintBed),
        Recompute => C(UiCommand::RecomputeAll),
        LogPanel => C(UiCommand::ToggleLogPanel),
        Delete => match state.tree_selection {
            Some(
                item @ (TreeItemId::Feature(_)
                | TreeItemId::Body(_)
                | TreeItemId::ImportedObject(_)),
            ) if !state.editing => C(UiCommand::DeleteTreeItem(item)),
            _ => HostOutcome::Nothing,
        },
        ToggleVisibility => state
            .tree_selection
            .and_then(|item| toggle_visibility(state.document, item))
            .map_or(HostOutcome::Nothing, C),
        ShadedWithEdges => C(UiCommand::SetDrawStyle(settings::DrawStyle::ShadedEdges)),
        Shaded => C(UiCommand::SetDrawStyle(settings::DrawStyle::Shaded)),
        Wireframe => C(UiCommand::SetDrawStyle(settings::DrawStyle::Wireframe)),
        Console => HostOutcome::ToggleConsole,
        RunScript => C(UiCommand::File(FileCommand::RunScript)),
        Record => C(UiCommand::ToggleRecording),
    }
}

/// The command that shows `item` if it is hidden and hides it if not.
fn toggle_visibility(document: &core_document::Document, item: TreeItemId) -> Option<UiCommand> {
    match item {
        TreeItemId::Feature(feature) => {
            let visible = document.get_feature_meta(feature)?.visible;
            Some(UiCommand::TreeFeature {
                feature,
                command: TreeFeatureCommand::SetVisible(!visible),
            })
        }
        TreeItemId::Body(body) => {
            let hidden = document.bodies().iter().find(|b| b.id == body)?.hidden;
            Some(UiCommand::SetBodyVisible {
                body,
                visible: hidden,
            })
        }
        TreeItemId::ImportedObject(node) => {
            let visible = document.imported_object(node)?.visible;
            Some(UiCommand::SetImportedVisibility {
                node,
                visible: !visible,
            })
        }
        TreeItemId::DocumentRoot => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{ActionDescriptor, ToolDescriptor, Workbench, WorkbenchContext};

    struct Bench {
        id: &'static str,
        tool_key: &'static str,
    }

    impl Workbench for Bench {
        fn descriptor(&self) -> core_document::WorkbenchDescriptor {
            core_document::WorkbenchDescriptor::new(self.id, self.id, "")
        }

        fn configure(&self, context: &mut WorkbenchContext) {
            context.register_tool(
                ToolDescriptor::new_action(format!("{}.tool", self.id), "Tool", Some("t"))
                    .shortcut(self.tool_key),
            );
            context.register_tool(
                ToolDescriptor::new_action(format!("{}.planned", self.id), "Later", Some("t"))
                    .shortcut("J")
                    .planned("not built"),
            );
            context.register_action(
                ActionDescriptor::new(format!("{}.act", self.id), "Act").shortcut("Shift+M"),
            );
        }
    }

    fn registry() -> DocumentService {
        let mut registry = DocumentService::default();
        registry
            .register_workbench(Box::new(Bench {
                id: "wb.one",
                tool_key: "F",
            }))
            .unwrap();
        registry
            .register_workbench(Box::new(Bench {
                id: "wb.two",
                tool_key: "L",
            }))
            .unwrap();
        registry
    }

    fn key(text: &str) -> Chord {
        Chord::parse(text).unwrap()
    }

    #[test]
    fn every_default_key_of_the_application_parses() {
        for spec in HOST {
            assert_eq!(
                parse_all(spec.keys.iter().copied()).len(),
                spec.keys.len(),
                "{}",
                spec.id
            );
        }
        let ids: std::collections::HashSet<_> = HOST.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), HOST.len(), "application ids are unique");
    }

    #[test]
    fn a_workbench_key_wins_while_it_is_active_and_the_application_s_elsewhere() {
        let keymap = Keymap::build(&registry(), &KeyboardSettings::default(), &[]);
        let one = WorkbenchId::from("wb.one");
        let two = WorkbenchId::from("wb.two");
        assert_eq!(keymap.lookup(key("F"), &one).unwrap().id, "wb.one.tool");
        assert_eq!(keymap.lookup(key("F"), &two).unwrap().id, "view.fit_all");
        assert_eq!(keymap.lookup(key("L"), &two).unwrap().id, "wb.two.tool");
        assert!(keymap.lookup(key("L"), &one).is_none());
        assert_eq!(
            keymap.lookup(key("Shift+M"), &two).unwrap().target,
            Target::Action
        );
        assert!(
            keymap.get("wb.one.planned").is_none(),
            "a planned tool takes no key"
        );
    }

    #[test]
    fn the_user_s_keys_replace_the_defaults_and_an_empty_list_unbinds() {
        let mut user = KeyboardSettings::default();
        user.bindings.insert("file.save".into(), vec!["F5".into()]);
        user.bindings.insert("view.fit_all".into(), Vec::new());
        let keymap = Keymap::build(&registry(), &user, &[]);
        let one = WorkbenchId::from("wb.one");
        assert_eq!(keymap.lookup(key("F5"), &one).unwrap().id, "file.save");
        assert!(keymap.lookup(key("Ctrl+S"), &one).is_none());
        assert!(keymap.get("file.save").unwrap().is_changed());
        let two = WorkbenchId::from("wb.two");
        assert!(keymap.lookup(key("F"), &two).is_none());
        assert_eq!(keymap.text("file.save").as_deref(), Some("F5"));
    }

    /// The bindings a frame with these key presses runs, and the keys left
    /// in the input for widgets.
    fn press(
        keymap: &Keymap,
        keys: &[(egui::Key, egui::Modifiers, bool)],
        typing: bool,
        have_document: bool,
    ) -> (Vec<String>, usize) {
        let focus = KeyFocus {
            typing,
            numeric: false,
            have_document,
        };
        press_in(keymap, keys, focus)
    }

    fn press_in(
        keymap: &Keymap,
        keys: &[(egui::Key, egui::Modifiers, bool)],
        focus: KeyFocus,
    ) -> (Vec<String>, usize) {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        for (key, modifiers, repeat) in keys {
            raw.events.push(egui::Event::Key {
                key: *key,
                physical_key: None,
                pressed: true,
                repeat: *repeat,
                modifiers: *modifiers,
            });
        }
        let mut hits = Vec::new();
        let mut left = 0;
        let mut output = ctx.run_ui(raw, |ui| {
            hits = take_pressed(ui.ctx(), keymap, &WorkbenchId::from("wb.two"), focus)
                .into_iter()
                .map(|b| b.id)
                .collect();
            left = ui.ctx().input(|i| i.events.len());
        });
        output.textures_delta.clear();
        (hits, left)
    }

    #[test]
    fn pressed_keys_run_their_bindings_and_leave_the_input() {
        use egui::{Key, Modifiers};
        let keymap = Keymap::build(&registry(), &KeyboardSettings::default(), &[]);
        let (hits, left) = press(
            &keymap,
            &[
                (Key::L, Modifiers::NONE, false),
                (Key::S, Modifiers::CTRL, false),
            ],
            false,
            true,
        );
        assert_eq!(hits, ["wb.two.tool", "file.save"]);
        assert_eq!(left, 0);
        // A held key repeats without running again, and is still taken.
        let held = [
            (Key::L, Modifiers::NONE, false),
            (Key::L, Modifiers::NONE, true),
        ];
        let (hits, left) = press(&keymap, &held, false, true);
        assert_eq!(hits, ["wb.two.tool"]);
        assert_eq!(left, 0);
        // A key nothing is bound to stays for the widgets.
        let (hits, left) = press(&keymap, &[(Key::B, Modifiers::NONE, false)], false, true);
        assert!(hits.is_empty());
        assert_eq!(left, 1);
    }

    #[test]
    fn a_focused_text_field_keeps_its_typing_and_editing_keys() {
        use egui::{Key, Modifiers};
        let keymap = Keymap::build(&registry(), &KeyboardSettings::default(), &[]);
        let keys = [
            (Key::L, Modifiers::NONE, false),
            (Key::Z, Modifiers::CTRL, false),
            (Key::S, Modifiers::CTRL, false),
        ];
        let (hits, left) = press(&keymap, &keys, true, true);
        assert_eq!(hits, ["file.save"], "only the chord a field has no use for");
        assert_eq!(left, 2);
    }

    #[test]
    fn a_workbench_taking_a_number_keeps_the_number_keys() {
        use egui::{Key, Modifiers};
        let keymap = Keymap::build(&registry(), &KeyboardSettings::default(), &[]);
        let keys = [
            (Key::Num1, Modifiers::NONE, false),
            (Key::Minus, Modifiers::NONE, false),
            (Key::L, Modifiers::NONE, false),
        ];
        let numeric = KeyFocus {
            typing: false,
            numeric: true,
            have_document: true,
        };
        let (hits, left) = press_in(&keymap, &keys, numeric);
        assert_eq!(hits, ["wb.two.tool"], "letters still switch tools");
        assert_eq!(left, 2);
        let (hits, _) = press(&keymap, &keys[..1], false, true);
        assert_eq!(hits, ["view.front"]);
    }

    #[test]
    fn delete_and_space_act_on_the_tree_selection() {
        let mut document = core_document::Document::new("t");
        let body = document.create_body(None);
        let mut state = HostState {
            active_tab: None,
            section_on: false,
            document: &document,
            tree_selection: Some(TreeItemId::Body(body)),
            editing: false,
        };
        assert!(matches!(
            host_outcome(HostAction::ToggleVisibility, &state),
            HostOutcome::Command(UiCommand::SetBodyVisible { visible: false, .. })
        ));
        assert!(matches!(
            host_outcome(HostAction::Delete, &state),
            HostOutcome::Command(UiCommand::DeleteTreeItem(TreeItemId::Body(_)))
        ));
        state.editing = true;
        assert!(
            matches!(
                host_outcome(HostAction::Delete, &state),
                HostOutcome::Nothing
            ),
            "an open edit keeps Delete"
        );
        state.tree_selection = None;
        assert!(matches!(
            host_outcome(HostAction::ToggleVisibility, &state),
            HostOutcome::Nothing
        ));
    }

    #[test]
    fn document_commands_wait_for_a_document() {
        use egui::{Key, Modifiers};
        let keymap = Keymap::build(&registry(), &KeyboardSettings::default(), &[]);
        let keys = [
            (Key::S, Modifiers::CTRL, false),
            (Key::N, Modifiers::CTRL, false),
        ];
        let (hits, _) = press(&keymap, &keys, false, false);
        assert_eq!(hits, ["file.new"]);
    }

    #[test]
    fn clashes_are_ties_in_one_place_and_shadows_across_places() {
        let mut user = KeyboardSettings::default();
        user.bindings
            .insert("file.open".into(), vec!["Ctrl+N".into()]);
        let keymap = Keymap::build(&registry(), &user, &[]);
        let open = keymap.clashes("file.open");
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].other, "New");
        assert!(!open[0].shadowed);
        let tool = keymap.clashes("wb.one.tool");
        assert_eq!(tool.len(), 1, "{tool:?}");
        assert!(tool[0].shadowed, "the workbench's F shadows Fit all");
        assert!(keymap.clashes("wb.two.tool").is_empty(), "L is free");
    }

    #[test]
    fn a_script_is_a_command_the_user_can_give_a_key() {
        let script = crate::script_library::ScriptEntry {
            id: "script.make_block".into(),
            name: "make block".into(),
            path: "/tmp/make_block.lua".into(),
            about: None,
        };
        let mut user = KeyboardSettings::default();
        user.bindings
            .insert("script.make_block".into(), vec!["Ctrl+Shift+B".into()]);
        let keymap = Keymap::build(&registry(), &user, std::slice::from_ref(&script));
        let binding = keymap
            .lookup(key("Ctrl+Shift+B"), &WorkbenchId::from("wb.one"))
            .unwrap();
        assert_eq!(binding.target, Target::Script(script.path.clone()));
        assert_eq!(binding.group, "Scripts");
        assert!(!binding.needs_document);
    }

    #[test]
    fn a_key_back_to_its_default_leaves_no_trace_in_the_settings() {
        let registry = registry();
        let mut user = KeyboardSettings::default();
        let keymap = Keymap::build(&registry, &user, &[]);
        let save = keymap.get("file.save").unwrap().clone();
        set_keys(&mut user, &save, vec![key("F5")]);
        assert_eq!(user.bindings["file.save"], ["F5"]);
        set_keys(&mut user, &save, save.defaults.clone());
        assert!(user.bindings.is_empty());
    }
}
