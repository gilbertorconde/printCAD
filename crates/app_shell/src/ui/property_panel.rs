//! The property panel under the model tree: what the selected item is and
//! the values it carries, grouped the way the design shows them.

use core_document::{BodyId, Document, FeatureId, PropertyHints, Unit, format_length_mm};
use egui::RichText;
use ui_kit::sans;
use ui_kit::tokens::*;
use ui_kit::widgets::{QtyField, check_row, mono_label};
use uuid::Uuid;

use super::feature_tree::{TreeFeatureCommand, TreeItemId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PropertyTab {
    View,
    #[default]
    Data,
}

#[derive(Default)]
pub struct PropertyPanelResult {
    pub feature_command: Option<(FeatureId, TreeFeatureCommand)>,
    pub imported_visibility: Option<(Uuid, bool)>,
    /// The Visible row of a body was toggled.
    pub body_visibility: Option<(BodyId, bool)>,
    pub rename: Option<(TreeItemId, String)>,
    /// The body's look changed: a colour and opacity of its own, or back
    /// to the one it came with.
    pub body_display: Option<(BodyId, Option<core_document::BodyDisplay>)>,
    /// Edits of variables and configurations.
    pub commands: Vec<super::UiCommand>,
    /// One of a feature's numbers was set: to a value or a formula.
    pub parameter: Option<(
        FeatureId,
        core_document::Parameter,
        ui_kit::widgets::FormulaEdit,
    )>,
}

/// The suffix a field of kind `dim` shows.
pub(crate) fn unit_suffix(dim: core_document::expr::Dim) -> &'static str {
    use core_document::expr::Dim;
    match dim {
        Dim::LENGTH => "mm",
        Dim::ANGLE => "°",
        _ => "",
    }
}

/// A feature's numbers, each a field a formula may set.
fn parameter_rows(
    ui: &mut egui::Ui,
    document: &Document,
    registry: &core_document::DocumentService,
    feature: FeatureId,
    result: &mut PropertyPanelResult,
) {
    let Some(node) = document.get_feature_meta(feature) else {
        return;
    };
    let params = registry.parameters(node);
    if params.is_empty() {
        return;
    }
    group_header(ui, "Parameters");
    let width = ui.available_width();
    let slots = document.evaluated_slots(feature);
    for p in params {
        // Only the numbers the feature has now (a counterbore hole has no
        // countersink angle).
        let Some(raw) = node
            .data
            .pointer(&p.pointer)
            .and_then(serde_json::Value::as_f64)
        else {
            continue;
        };
        let slot = slots.iter().find(|s| s.key == p.key);
        let value = match slot.map(|s| &s.result) {
            Some(Ok(q)) => q.value,
            _ => raw / p.scale,
        };
        let error = slot
            .and_then(|s| s.result.as_ref().err())
            .map(String::as_str);
        let host = core_document::DocumentFormulas {
            document,
            dim: p.dim,
        };
        let hint = match &p.name {
            Some(name) => format!("{}.{name}", core_document::expr::quote_name(&node.name)),
            None => "Name it to read it from formulas".to_string(),
        };
        field_row(ui, width, &p.label, Some(&hint), |ui, field_width| {
            let field = ui_kit::widgets::FormulaField::new(
                ui.id().with(("parameter", feature, &p.key)),
                value,
                &host,
            )
            .formula(node.formulas.get(&p.key).map(String::as_str))
            .error(error)
            .unit(unit_suffix(p.dim))
            .decimals(if p.integer { 0 } else { 2 })
            .speed(if p.integer { 0.05 } else { 0.1 })
            .fill(field_width);
            if let Some(edit) = field.show(ui) {
                result.parameter = Some((feature, p.clone(), edit));
            }
        });
    }
}

/// One labelled row of an editable group, `width` wide: the label in a
/// column of its own, left-aligned and cut short when long, and `field`
/// given exactly the rest, so no row reaches past the panel. `width` is
/// measured once per group, before its rows, so one row never widens the
/// next.
fn field_row(
    ui: &mut egui::Ui,
    width: f32,
    label: &str,
    hint: Option<&str>,
    field: impl FnOnce(&mut egui::Ui, f32),
) {
    const INDENT: f32 = 24.0;
    const RIGHT: f32 = 8.0;
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let label_width = ((width - INDENT) * 0.42).max(60.0);
        ui.add_space(INDENT - gap);
        let shown = ui
            .allocate_ui_with_layout(
                egui::vec2(label_width, TREE_ROW),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(label_width);
                    ui.add(
                        egui::Label::new(RichText::new(label).font(sans(FONT_SM)).color(TEXT2))
                            .truncate(),
                    )
                },
            )
            .inner;
        if let Some(hint) = hint {
            shown.on_hover_text(hint);
        }
        let rest = width - INDENT - label_width - gap - RIGHT;
        field(ui, rest.max(40.0));
    });
}

/// The inputs a feature's bench lets be swapped (its profile), each a
/// dropdown of what it may read instead.
fn reference_rows(
    ui: &mut egui::Ui,
    document: &Document,
    registry: &core_document::DocumentService,
    feature: FeatureId,
    result: &mut PropertyPanelResult,
) {
    let references = registry.references(document, feature);
    if references.is_empty() {
        return;
    }
    group_header(ui, "Inputs");
    let width = ui.available_width();
    for reference in references {
        field_row(ui, width, &reference.label, None, |ui, field_width| {
            let mut picked = None;
            egui::ComboBox::from_id_salt(("reference", feature, &reference.key))
                .selected_text(RichText::new(&reference.current).font(sans(FONT_SM)))
                .width(field_width - 2.0 * ui.spacing().button_padding.x)
                .show_ui(ui, |ui| {
                    for (id, name) in &reference.choices {
                        let on = reference.selected == Some(*id);
                        if ui
                            .selectable_label(on, RichText::new(name).font(sans(FONT_SM)))
                            .clicked()
                            && !on
                        {
                            picked = Some(core_document::ReferenceChoice::Feature(*id));
                        }
                    }
                    if reference.takes_face {
                        ui.separator();
                        if ui
                            .selectable_label(
                                false,
                                RichText::new("The face selected in the view").font(sans(FONT_SM)),
                            )
                            .on_hover_text("Click a flat face of the solid first")
                            .clicked()
                        {
                            picked = Some(core_document::ReferenceChoice::SelectedFace);
                        }
                    }
                });
            if let Some(to) = picked {
                result.commands.push(super::UiCommand::SetFeatureReference {
                    feature,
                    key: reference.key.clone(),
                    to,
                });
            }
        });
    }
}

/// `data` (a feature's payload, one variant key over its fields) without
/// `fields`.
fn without_fields(data: &serde_json::Value, fields: &[String]) -> serde_json::Value {
    let mut data = data.clone();
    if let Some(inner) = data
        .as_object_mut()
        .and_then(|m| m.values_mut().next())
        .and_then(serde_json::Value::as_object_mut)
    {
        for field in fields {
            inner.remove(field);
        }
    }
    data
}

/// One value row.
struct PropRow {
    name: String,
    value: String,
    mono: bool,
    /// A default or empty value, drawn muted.
    dim: bool,
}

impl PropRow {
    fn text(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            mono: false,
            dim: false,
        }
    }

    fn mono(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            mono: true,
            dim: false,
        }
    }

    fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }
}

fn humanize_key(key: &str) -> String {
    let mut out = String::new();
    for (i, part) in key.split('_').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if i == 0 {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        } else {
            out.push_str(part);
        }
    }
    out
}

fn resolve_ref(document: &Document, raw: &str) -> Option<String> {
    let id = Uuid::parse_str(raw).ok()?;
    if let Some(node) = document.get_feature_meta(FeatureId(id)) {
        return Some(node.name.clone());
    }
    document
        .bodies()
        .iter()
        .find(|b| b.id == BodyId(id))
        .map(|b| b.name.clone())
}

fn number_row(name: &str, key: &str, n: f64, hints: &PropertyHints, unit: Unit) -> PropRow {
    let text = if key.ends_with("_deg") {
        format!("{n:.2} °")
    } else if hints.is_length(key) {
        format_length_mm(n as f32, unit, 2)
    } else if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{n:.3}")
    };
    PropRow::mono(name, text).dim(n == 0.0)
}

/// Flatten a feature payload into rows. An externally tagged enum yields
/// its variant as the group; nested single-key objects (inner enums) read
/// as `field = Variant` followed by `field · sub` rows.
fn flatten_feature_json(
    value: &serde_json::Value,
    document: &Document,
    hints: &PropertyHints,
    unit: Unit,
) -> (String, Vec<PropRow>) {
    let mut rows = Vec::new();
    let (group, fields) = match value {
        serde_json::Value::Object(map) if map.len() == 1 => {
            let (variant, inner) = map.iter().next().expect("one entry");
            (variant.clone(), inner.clone())
        }
        serde_json::Value::String(s) => return (s.clone(), rows),
        other => ("Data".to_string(), other.clone()),
    };
    if let serde_json::Value::Object(map) = fields {
        for (key, v) in map {
            flatten_field(
                &mut rows,
                &key,
                &humanize_key(&key),
                &v,
                document,
                hints,
                unit,
            );
        }
    }
    (group, rows)
}

fn flatten_field(
    rows: &mut Vec<PropRow>,
    key: &str,
    name: &str,
    v: &serde_json::Value,
    document: &Document,
    hints: &PropertyHints,
    unit: Unit,
) {
    match v {
        serde_json::Value::Null => rows.push(PropRow::text(name, "-").dim(true)),
        serde_json::Value::Bool(b) => rows.push(PropRow::mono(name, b.to_string()).dim(!b)),
        serde_json::Value::Number(n) => {
            rows.push(number_row(
                name,
                key,
                n.as_f64().unwrap_or(0.0),
                hints,
                unit,
            ));
        }
        serde_json::Value::String(s) => {
            if hints.is_reference(key)
                && let Some(resolved) = resolve_ref(document, s)
            {
                rows.push(PropRow::text(name, resolved));
            } else {
                rows.push(PropRow::text(name, s.clone()));
            }
        }
        serde_json::Value::Array(items) => {
            if items.iter().all(serde_json::Value::is_number) {
                let parts: Vec<String> = items
                    .iter()
                    .map(|n| format!("{:.2}", n.as_f64().unwrap_or(0.0)))
                    .collect();
                rows.push(PropRow::mono(name, format!("({})", parts.join(", "))));
            } else if hints.is_reference(key) {
                let names: Vec<String> = items
                    .iter()
                    .filter_map(|i| i.as_str())
                    .map(|s| resolve_ref(document, s).unwrap_or_else(|| s.to_string()))
                    .collect();
                rows.push(PropRow::text(name, names.join(", ")).dim(names.is_empty()));
            } else {
                rows.push(
                    PropRow::mono(name, format!("{} item(s)", items.len())).dim(items.is_empty()),
                );
            }
        }
        serde_json::Value::Object(map) => {
            if map.len() == 1
                && let Some((variant, inner)) = map.iter().next()
                && inner.is_object()
            {
                rows.push(PropRow::text(name, variant.clone()));
                if let serde_json::Value::Object(sub) = inner {
                    for (k, sv) in sub {
                        flatten_field(
                            rows,
                            k,
                            &format!("{name} · {}", humanize_key(k)),
                            sv,
                            document,
                            hints,
                            unit,
                        );
                    }
                }
            } else {
                for (k, sv) in map {
                    flatten_field(
                        rows,
                        k,
                        &format!("{name} · {}", humanize_key(k)),
                        sv,
                        document,
                        hints,
                        unit,
                    );
                }
            }
        }
    }
}

fn format_created(created_at: i64) -> String {
    // Civil date from a Unix timestamp in milliseconds.
    let secs = created_at / 1000;
    let days = secs.div_euclid(86_400);
    let day_secs = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        day_secs / 3600,
        (day_secs % 3600) / 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A body's name, or a dash for none.
fn body_label(document: &Document, body: BodyId) -> String {
    document
        .bodies()
        .iter()
        .find(|b| b.id == body)
        .map_or_else(|| "-".to_string(), |b| b.name.clone())
}

/// The rows of the Data tab for `selected`.
fn data_groups(
    document: &Document,
    registry: &core_document::DocumentService,
    selected: TreeItemId,
) -> Vec<(String, Vec<PropRow>)> {
    let unit = document.display_unit();
    match selected {
        TreeItemId::DocumentRoot => vec![(
            "Document".to_string(),
            vec![
                PropRow::text("Name", document.name()),
                PropRow::mono("Bodies", document.bodies().len().to_string()),
                PropRow::mono(
                    "Features",
                    document.feature_tree().all_nodes().count().to_string(),
                ),
                PropRow::text("Display unit", unit.short_label()),
            ],
        )],
        TreeItemId::Array(id) => {
            let Some(array) = document.copy_array(id) else {
                return Vec::new();
            };
            let repeated = match array.component.and_then(|c| document.component(c)) {
                Some(component) => component.name.clone(),
                None => body_label(document, array.source),
            };
            let kind = match &array.pattern {
                core_document::ArrayPattern::Linear { .. } => "Row",
                core_document::ArrayPattern::Polar { .. } => "Ring",
                core_document::ArrayPattern::Path { .. } => "Along a path",
                core_document::ArrayPattern::Points { .. } => "At points",
            };
            vec![(
                "Array".to_string(),
                vec![
                    PropRow::text("Label", &array.name),
                    PropRow::text("Repeats", repeated),
                    PropRow::text("Kind", kind),
                    PropRow::mono("Occurrences", array.count.to_string()),
                    PropRow::mono("Copies", array.instances.len().to_string()),
                ],
            )]
        }
        TreeItemId::Component(id) => {
            let Some(component) = document.component(id) else {
                return Vec::new();
            };
            vec![(
                "Component".to_string(),
                vec![
                    PropRow::text("Label", &component.name),
                    PropRow::text(
                        "Moves",
                        if component.flexible {
                            "Flexible: its joints move"
                        } else {
                            "Rigid: as one"
                        },
                    ),
                    PropRow::mono("Bodies", document.component_bodies(id).len().to_string()),
                ],
            )]
        }
        TreeItemId::Body(id) => {
            let Some(body) = document.bodies().iter().find(|b| b.id == id) else {
                return Vec::new();
            };
            let features = document
                .feature_tree()
                .all_nodes()
                .filter(|(_, n)| n.body == Some(id))
                .count();
            vec![(
                "Body".to_string(),
                vec![
                    PropRow::text("Label", &body.name),
                    PropRow::mono("Features", features.to_string()),
                    PropRow::text(
                        "Material",
                        body.material.as_ref().map_or("None".to_string(), |m| {
                            format!("{} ({:.2} g/cm³)", m.name, m.density)
                        }),
                    ),
                    PropRow::text("Frozen", if body.frozen { "Yes" } else { "No" }),
                    PropRow::text("Selectable", if body.unselectable { "No" } else { "Yes" }),
                    PropRow::text(
                        "Tip",
                        body.tip
                            .and_then(|t| document.get_feature_meta(t))
                            .map(|n| n.name.clone())
                            .unwrap_or_else(|| "Last feature".to_string()),
                    ),
                    PropRow::mono("Created", format_created(body.created_at)),
                ],
            )]
        }
        TreeItemId::Feature(id) => {
            let Some(node) = document.get_feature_meta(id) else {
                return Vec::new();
            };
            let position = node.body.map(|b| {
                let mut ids: Vec<_> = document
                    .feature_tree()
                    .all_nodes()
                    .filter(|(_, n)| n.body == Some(b))
                    .map(|(_, n)| (n.seq, n.id))
                    .collect();
                ids.sort();
                let idx = ids.iter().position(|(_, fid)| *fid == id).unwrap_or(0);
                format!("{} / {}", idx + 1, ids.len())
            });
            let mut base = vec![
                PropRow::text("Label", &node.name),
                PropRow::text(
                    "Kind",
                    super::feature_tree::feature_info(registry, node).family_label,
                ),
                PropRow::mono("Created", format_created(node.created_at)),
            ];
            if let Some(position) = position {
                base.push(PropRow::mono("History position", position));
            }
            // A variable set's and the configurations' own editors follow.
            let kind = node.workbench_id.as_str();
            if kind == core_document::VARIABLES_KIND || kind == core_document::CONFIGURATIONS_KIND {
                return vec![("Base".to_string(), base)];
            }
            let hints = registry.property_hints();
            // What the Inputs group offers is not repeated here.
            let covered: Vec<String> = registry
                .references(document, id)
                .into_iter()
                .flat_map(|r| r.fields)
                .collect();
            let data = without_fields(&node.data, &covered);
            let (group, rows) = flatten_feature_json(&data, document, &hints, unit);
            vec![("Base".to_string(), base), (group, rows)]
        }
        TreeItemId::Notes(body) => vec![(
            "Annotations".to_string(),
            vec![
                PropRow::text("On", body_label(document, body)),
                PropRow::mono("Notes", document.body_notes(body).len().to_string()),
            ],
        )],
        TreeItemId::Note(id) => {
            let Some((body, note)) = document.note(id) else {
                return Vec::new();
            };
            let (at, label) = document.note_placed(body, note);
            let point = |p: [f32; 3]| p.map(|c| format_length_mm(c, unit, 2)).join(", ");
            let formula = note.text.trim_start().starts_with('=');
            vec![(
                "Note".to_string(),
                vec![
                    PropRow::text("Text", &note.text),
                    PropRow::text("Shows", document.note_text(note)).dim(!formula),
                    PropRow::text("On", body_label(document, body)),
                    PropRow::text(
                        "Face",
                        if note.face.is_some() {
                            "Followed through rebuilds"
                        } else {
                            "-"
                        },
                    )
                    .dim(note.face.is_none()),
                    PropRow::mono("Point", point(at)),
                    PropRow::mono("Label", point(label)),
                ],
            )]
        }
        TreeItemId::ImportedObject(id) => {
            let Some(obj) = document.imported_object(id) else {
                return Vec::new();
            };
            let kind = match obj.kind {
                kernel_api::ImportedNodeKind::Assembly => "Assembly",
                kernel_api::ImportedNodeKind::Part => "Part",
                kernel_api::ImportedNodeKind::Instance => "Instance",
                kernel_api::ImportedNodeKind::Annotations => "Annotations",
                kernel_api::ImportedNodeKind::Annotation => "Annotation",
            };
            if let Some(annotation) = &obj.annotation {
                return vec![(
                    "Annotation".to_string(),
                    vec![
                        PropRow::text("Label", &obj.name),
                        PropRow::text("Kind", annotation.kind.label()),
                        PropRow::text("Text", &annotation.text),
                        PropRow::text(
                            "Describes",
                            annotation
                                .body
                                .and_then(|b| document.bodies().iter().find(|x| x.id == b))
                                .map(|b| b.name.clone())
                                .unwrap_or_else(|| "-".to_string()),
                        )
                        .dim(annotation.body.is_none()),
                        PropRow::mono("Lines", annotation.polylines.len().to_string()),
                    ],
                )];
            }
            // An instance shown as the part it places answers with the
            // part's layers.
            let layers = if obj.layers.is_empty() {
                document
                    .body_of_imported_object(id)
                    .and_then(|body| document.imported_object_for_body(body))
                    .and_then(|part| document.imported_object(part))
                    .map(|part| part.layers.clone())
                    .unwrap_or_default()
            } else {
                obj.layers.clone()
            };
            vec![(
                "Imported".to_string(),
                vec![
                    PropRow::text("Label", &obj.name),
                    PropRow::text("Kind", kind),
                    PropRow::text(
                        "Linked body",
                        obj.body_id
                            .and_then(|b| document.bodies().iter().find(|x| x.id == b))
                            .map(|b| b.name.clone())
                            .unwrap_or_else(|| "-".to_string()),
                    )
                    .dim(obj.body_id.is_none()),
                    PropRow::mono("Children", obj.children.len().to_string()),
                    PropRow::text(
                        "Layers",
                        if layers.is_empty() {
                            "-".to_string()
                        } else {
                            layers.join(", ")
                        },
                    )
                    .dim(layers.is_empty()),
                ],
            )]
        }
    }
}

/// What the panel shows: the item, its spelled-out description, and the
/// measure of its body when it has one.
pub struct PanelSubject<'a> {
    pub selected: TreeItemId,
    pub detail: Option<&'a str>,
    pub physical: Option<&'a super::Physical>,
}

/// Draw the panel body for `subject.selected`. `subject.detail` is the
/// tree's spelled-out description of the hovered (else selected) item.
pub fn draw_property_panel(
    ui: &mut egui::Ui,
    document: &Document,
    registry: &core_document::DocumentService,
    subject: PanelSubject<'_>,
    tab: &mut PropertyTab,
    rename_buffer: &mut Option<(TreeItemId, String)>,
    variables: &mut super::variables_view::VariablesState,
) -> PropertyPanelResult {
    let PanelSubject {
        selected,
        detail,
        physical,
    } = subject;
    let mut result = PropertyPanelResult::default();
    let selected_name = match selected {
        TreeItemId::DocumentRoot => document.name().to_string(),
        TreeItemId::Body(id) => document
            .bodies()
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.name.clone())
            .unwrap_or_default(),
        TreeItemId::Feature(id) => document
            .get_feature_meta(id)
            .map(|n| n.name.clone())
            .unwrap_or_default(),
        TreeItemId::ImportedObject(id) => document
            .imported_object(id)
            .map(|o| o.name.clone())
            .unwrap_or_default(),
        TreeItemId::Component(id) => document
            .component(id)
            .map(|c| c.name.clone())
            .unwrap_or_default(),
        TreeItemId::Notes(_) => "Annotations".to_string(),
        TreeItemId::Array(id) => document
            .copy_array(id)
            .map(|a| a.name.clone())
            .unwrap_or_default(),
        TreeItemId::Note(id) => document
            .note(id)
            .map(|(_, n)| n.text.clone())
            .unwrap_or_default(),
    };

    let (strip, _) = ui.allocate_exact_size(
        egui::Vec2::new(ui.available_width(), 28.0),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(strip, 0.0, BG1);
    ui.painter().hline(
        strip.x_range(),
        strip.top() + 0.5,
        egui::Stroke::new(1.0, BORDER),
    );
    ui.painter().hline(
        strip.x_range(),
        strip.bottom() - 0.5,
        egui::Stroke::new(1.0, BORDER),
    );
    let mut h = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(strip.shrink2(egui::Vec2::new(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    h.spacing_mut().item_spacing.x = 2.0;
    for (value, label) in [(PropertyTab::View, "View"), (PropertyTab::Data, "Data")] {
        if ui_kit::widgets::Tab::new(label, *tab == value)
            .height(strip.height() - 4.0)
            .show(&mut h)
            .selected
        {
            *tab = value;
        }
    }
    h.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        mono_label(ui, &selected_name, FONT_XS, TEXT3);
    });
    if let Some(detail) = detail {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(detail).font(sans(FONT_XS)).color(TEXT3));
        });
    }

    egui::ScrollArea::vertical()
        .id_salt("property_rows")
        .auto_shrink([false, false])
        .show(ui, |ui| match *tab {
            PropertyTab::Data => {
                // A variable set or the configurations: their editor after
                // the Base rows; any other feature: its numbers first.
                let kind = match selected {
                    TreeItemId::Feature(feature) => document
                        .get_feature_meta(feature)
                        .map(|n| (feature, n.workbench_id.as_str().to_string())),
                    _ => None,
                };
                let table = kind.as_ref().filter(|(_, k)| {
                    k == core_document::VARIABLES_KIND || k == core_document::CONFIGURATIONS_KIND
                });
                if table.is_none()
                    && let Some((feature, _)) = &kind
                {
                    reference_rows(ui, document, registry, *feature, &mut result);
                    parameter_rows(ui, document, registry, *feature, &mut result);
                }
                let mut groups = data_groups(document, registry, selected);
                if let Some(physical) = physical {
                    let body = match selected {
                        TreeItemId::Body(id) => Some(id),
                        TreeItemId::ImportedObject(id) => document.body_of_imported_object(id),
                        _ => None,
                    };
                    let material = body
                        .and_then(|b| document.bodies().iter().find(|x| x.id == b))
                        .and_then(|b| b.material.as_ref());
                    groups.push(physical_group(physical, document.display_unit(), material));
                }
                for (group, rows) in groups {
                    group_header(ui, &group);
                    for row in rows {
                        let editable_label =
                            row.name == "Label" && !matches!(selected, TreeItemId::DocumentRoot);
                        if editable_label {
                            label_row(ui, selected, &row.value, rename_buffer, &mut result);
                        } else {
                            value_row(ui, &row);
                        }
                    }
                }
                match table {
                    Some((set, k)) if k == core_document::VARIABLES_KIND => {
                        super::variables_view::variable_set_section(
                            ui,
                            variables,
                            document,
                            *set,
                            &mut result.commands,
                        );
                    }
                    Some(_) => super::variables_view::configurations_section(
                        ui,
                        variables,
                        document,
                        &mut result.commands,
                    ),
                    None => {}
                }
            }
            PropertyTab::View => view_rows(ui, document, selected, &mut result),
        });
    result
}

/// Volume, surface area and centre of mass (one row per axis, since a
/// value column holds one length), in the display unit.
fn physical_group(
    physical: &super::Physical,
    unit: Unit,
    material: Option<&core_document::Material>,
) -> (String, Vec<PropRow>) {
    let rows = match physical {
        super::Physical::Measuring => vec![PropRow::text("Measure", "Measuring…").dim(true)],
        super::Physical::Failed(why) => {
            vec![PropRow::text("Measure", format!("Could not measure: {why}")).dim(true)]
        }
        super::Physical::Ready(props) => {
            // A figure integrated over a tessellation reads as approximate.
            let about = if props.approximate { "≈ " } else { "" };
            let length = |mm: f64| {
                format!(
                    "{about}{}",
                    core_document::format_length_mm(mm as f32, unit, 2)
                )
            };
            vec![
                PropRow::mono(
                    "Volume",
                    props.volume_mm3.map_or_else(
                        || "encloses none".to_string(),
                        |v| format!("{about}{}", core_document::format_volume_mm3(v, unit, 2)),
                    ),
                ),
                PropRow::mono(
                    "Surface area",
                    format!(
                        "{about}{}",
                        core_document::format_area_mm2(props.area_mm2, unit, 2)
                    ),
                ),
                PropRow::mono("Centre X", length(props.centre_mm[0])),
                PropRow::mono("Centre Y", length(props.centre_mm[1])),
                PropRow::mono("Centre Z", length(props.centre_mm[2])),
            ]
            .into_iter()
            .chain(material.zip(props.volume_mm3).map(|(m, v)| {
                // mm³ to cm³, times g/cm³.
                PropRow::mono(
                    "Mass",
                    format!("{about}{}", format_mass(v / 1000.0 * f64::from(m.density))),
                )
            }))
            .chain(
                props
                    .inertia
                    .iter()
                    .flat_map(|inertia| inertia_rows(inertia, unit, material, about)),
            )
            .collect()
        }
    };
    ("Physical".to_string(), rows)
}

/// The inertia about the centre of mass: the tensor on the world's axes,
/// then the principal moments, smallest first, each with its axis. With a
/// material it is the mass's (g times the unit squared); without, the
/// volume's at unit density (the unit to the fifth).
fn inertia_rows(
    inertia: &kernel_api::Inertia,
    unit: Unit,
    material: Option<&core_document::Material>,
    about: &str,
) -> Vec<PropRow> {
    let per = f64::from(unit.mm_per_unit());
    let label = unit.short_label();
    let (scale, suffix) = match material {
        // mm⁵ times g/cm³ (a thousandth of g/mm³) is g·mm².
        Some(m) => (
            f64::from(m.density) / 1000.0 / (per * per),
            format!("g·{label}²"),
        ),
        None => (1.0 / per.powi(5), format!("{label}⁵")),
    };
    // What is a billionth of the largest moment is the integration's
    // rounding, not a product of inertia.
    let floor = inertia.principal[2].0.abs() * 1e-9;
    let moment = |v: f64| {
        let v = if v.abs() <= floor { 0.0 } else { v };
        format!("{about}{} {suffix}", format_figure(v * scale))
    };
    let t = inertia.about_centre;
    let mut rows = vec![
        PropRow::mono("Ixx", moment(t[0][0])),
        PropRow::mono("Iyy", moment(t[1][1])),
        PropRow::mono("Izz", moment(t[2][2])),
        PropRow::mono("Ixy", moment(t[0][1])),
        PropRow::mono("Ixz", moment(t[0][2])),
        PropRow::mono("Iyz", moment(t[1][2])),
    ];
    for (i, (value, axis)) in inertia.principal.iter().enumerate() {
        rows.push(PropRow::mono(format!("I{}", i + 1), moment(*value)));
        rows.push(PropRow::mono(
            format!("Axis {}", i + 1),
            // Two places and no minus on a zero, to fit the value column.
            axis.map(|c| format!("{:.2}", if c.abs() < 0.005 { 0.0 } else { c }))
                .join(" "),
        ));
    }
    rows
}

/// A figure to about four significant digits: plainly while it reads so,
/// in powers of ten when it is very large or small.
fn format_figure(v: f64) -> String {
    let size = v.abs();
    if size == 0.0 || (0.01..1e6).contains(&size) {
        format!("{v:.2}")
    } else {
        format!("{v:.3e}")
    }
}

/// A mass in grams, or kilograms from a thousand.
fn format_mass(grams: f64) -> String {
    if grams >= 1000.0 {
        format!("{:.3} kg", grams / 1000.0)
    } else {
        format!("{grams:.2} g")
    }
}

fn group_header(ui: &mut egui::Ui, title: &str) {
    let (rect, _) = ui.allocate_exact_size(
        egui::Vec2::new(ui.available_width(), 22.0),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(rect, 0.0, BG0);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, BORDER),
    );
    ui.painter().text(
        egui::pos2(rect.left() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title.to_uppercase(),
        ui_kit::sans_semibold(FONT_XS),
        TEXT2,
    );
}

fn row_frame(ui: &mut egui::Ui) -> (egui::Rect, egui::Rect, egui::Rect) {
    let (rect, _) = ui.allocate_exact_size(
        egui::Vec2::new(ui.available_width(), TREE_ROW),
        egui::Sense::hover(),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, BG2),
    );
    let split = rect.left() + rect.width() * 0.5;
    ui.painter()
        .vline(split, rect.y_range(), egui::Stroke::new(1.0, BORDER));
    let name = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 24.0, rect.top()),
        egui::pos2(split - 8.0, rect.bottom()),
    );
    let value = egui::Rect::from_min_max(
        egui::pos2(split + 8.0, rect.top()),
        egui::pos2(rect.right() - 8.0, rect.bottom()),
    );
    (rect, name, value)
}

fn value_row(ui: &mut egui::Ui, row: &PropRow) {
    let (_, name, value) = row_frame(ui);
    let painter = ui.painter().clone();
    painter.with_clip_rect(name).text(
        name.left_center(),
        egui::Align2::LEFT_CENTER,
        &row.name,
        sans(FONT_SM),
        TEXT2,
    );
    let font = if row.mono {
        ui_kit::mono(FONT_SM)
    } else {
        sans(FONT_SM)
    };
    painter.with_clip_rect(value).text(
        value.left_center(),
        egui::Align2::LEFT_CENTER,
        &row.value,
        font,
        if row.dim { TEXT3 } else { TEXT1 },
    );
}

/// The key under which a rename asks the Label row for the keyboard.
pub const FOCUS_LABEL: &str = "property_label_focus";

/// The Label row edits the item's name in place.
fn label_row(
    ui: &mut egui::Ui,
    selected: TreeItemId,
    current: &str,
    rename_buffer: &mut Option<(TreeItemId, String)>,
    result: &mut PropertyPanelResult,
) {
    let (_, name, value) = row_frame(ui);
    ui.painter().with_clip_rect(name).text(
        name.left_center(),
        egui::Align2::LEFT_CENTER,
        "Label",
        sans(FONT_SM),
        TEXT2,
    );
    let mut text = match rename_buffer {
        Some((item, buffer)) if *item == selected => buffer.clone(),
        _ => current.to_string(),
    };
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(value)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let resp = child.add(
        egui::TextEdit::singleline(&mut text)
            .desired_width(value.width())
            .frame(egui::Frame::NONE)
            .font(sans(FONT_SM)),
    );
    // Rename (F2 or a menu) puts the name up for editing here.
    let focus_id = egui::Id::new(FOCUS_LABEL);
    if ui
        .data_mut(|d| d.remove_temp::<bool>(focus_id))
        .unwrap_or(false)
    {
        resp.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
            let all = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            );
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), resp.id);
        }
    }
    if resp.changed() {
        *rename_buffer = Some((selected, text.clone()));
    }
    if resp.lost_focus()
        && let Some((item, buffer)) = rename_buffer.take()
        && item == selected
        && buffer != current
        && !buffer.trim().is_empty()
    {
        result.rename = Some((item, buffer));
    }
}

fn view_rows(
    ui: &mut egui::Ui,
    document: &Document,
    selected: TreeItemId,
    result: &mut PropertyPanelResult,
) {
    group_header(ui, "View");
    match selected {
        TreeItemId::Feature(id) => {
            let Some(node) = document.get_feature_meta(id) else {
                return;
            };
            ui.add_space(SPACE_1);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let mut visible = node.visible;
                if check_row(ui, &mut visible, "Visible").changed() {
                    result.feature_command = Some((id, TreeFeatureCommand::SetVisible(visible)));
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let mut suppressed = node.suppressed;
                if check_row(ui, &mut suppressed, "Suppressed").changed() {
                    result.feature_command = Some((id, TreeFeatureCommand::Suppress(suppressed)));
                }
            });
        }
        TreeItemId::Body(id) => {
            let Some(body) = document.bodies().iter().find(|b| b.id == id) else {
                return;
            };
            ui.add_space(SPACE_1);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let mut visible = !body.hidden;
                if check_row(ui, &mut visible, "Visible").changed() {
                    result.body_visibility = Some((id, visible));
                }
            });
        }
        TreeItemId::ImportedObject(id) => {
            let Some(obj) = document.imported_object(id) else {
                return;
            };
            ui.add_space(SPACE_1);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let mut visible = obj.visible;
                if check_row(ui, &mut visible, "Visible").changed() {
                    result.imported_visibility = Some((id, visible));
                }
            });
        }
        _ => {}
    }
    // A body's own look: the body row's, or the body an imported part is.
    let body = match selected {
        TreeItemId::Body(id) => Some(id),
        TreeItemId::ImportedObject(id) => document.body_of_imported_object(id),
        _ => None,
    };
    if let Some(body) = body
        && let Some(entry) = document.bodies().iter().find(|b| b.id == body)
    {
        display_rows(ui, body, entry.display, result);
    }
}

/// Custom colour on or off, and, on, the colour and how much of it shows.
fn display_rows(
    ui: &mut egui::Ui,
    body: BodyId,
    current: Option<core_document::BodyDisplay>,
    result: &mut PropertyPanelResult,
) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let mut on = current.is_some();
        if check_row(ui, &mut on, "Custom color").changed() {
            result.body_display = Some((body, on.then(|| current.unwrap_or_default())));
        }
    });
    let Some(mut display) = current else {
        return;
    };
    ui.horizontal(|ui| {
        ui.add_space(28.0);
        let mut color = egui::Color32::from_rgb(
            (display.color[0] * 255.0) as u8,
            (display.color[1] * 255.0) as u8,
            (display.color[2] * 255.0) as u8,
        );
        let mut changed = false;
        if ui.color_edit_button_srgba(&mut color).changed() {
            display.color = [
                color.r() as f32 / 255.0,
                color.g() as f32 / 255.0,
                color.b() as f32 / 255.0,
            ];
            changed = true;
        }
        ui.label(RichText::new("Opacity").font(sans(FONT_XS)).color(TEXT2));
        let mut opacity = display.opacity;
        if QtyField::new(&mut opacity)
            .range(0.05..=1.0)
            .speed(0.01)
            .decimals(2)
            .width(64.0)
            .show(ui)
        {
            display.opacity = opacity;
            changed = true;
        }
        if changed {
            result.body_display = Some((body, Some(display)));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys a solid-feature bench's payloads use, as its hints declare
    /// them.
    fn hints() -> PropertyHints {
        PropertyHints {
            length_keys: vec!["length", "length2", "depth", "depth2", "radius", "diameter"],
            reference_keys: vec!["sketch", "profile", "originals"],
        }
    }

    #[test]
    fn a_pad_flattens_into_its_variant_group() {
        let doc = Document::new("t");
        let value = serde_json::json!({
            "Pad": { "sketch": Uuid::new_v4().to_string(), "length": 20.0, "reversed": false, "taper_deg": 5.0 }
        });
        let (group, rows) = flatten_feature_json(&value, &doc, &hints(), Unit::Mm);
        assert_eq!(group, "Pad");
        let length = rows.iter().find(|r| r.name == "Length").unwrap();
        assert!(length.mono && length.value.contains("20.00"));
        let taper = rows.iter().find(|r| r.name == "Taper deg").unwrap();
        assert!(taper.value.ends_with('°'));
        let reversed = rows.iter().find(|r| r.name == "Reversed").unwrap();
        assert!(reversed.dim, "false booleans draw muted");
    }

    #[test]
    fn a_nested_enum_reads_as_variant_then_sub_rows() {
        let doc = Document::new("t");
        let value = serde_json::json!({
            "Hole": { "cut": { "Counterbore": { "diameter": 6.0, "depth": 2.0 } } }
        });
        let (_, rows) = flatten_feature_json(&value, &doc, &hints(), Unit::Mm);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["Cut", "Cut · Depth", "Cut · Diameter"],
            "keys sort alphabetically"
        );
        assert_eq!(rows[0].value, "Counterbore");
    }

    #[test]
    fn a_measure_reads_in_the_display_unit() {
        let props = kernel_api::PhysicalProperties {
            volume_mm3: Some(8_000.0),
            area_mm2: 2_400.0,
            centre_mm: [10.0, 20.0, 30.0],
            approximate: false,
            // A 20 mm cube: 8000 (20² + 20²) / 12 about each axis.
            inertia: Some(kernel_api::Inertia {
                about_centre: [
                    [533_333.33, -5e-10, 0.0],
                    [0.0, 533_333.33, 0.0],
                    [0.0, 0.0, 533_333.33],
                ],
                principal: [
                    (533_333.33, [1.0, 0.0, 0.0]),
                    (533_333.33, [0.0, 1.0, 0.0]),
                    (533_333.33, [0.0, 0.0, 1.0]),
                ],
            }),
        };
        let (title, rows) =
            physical_group(&crate::ui::Physical::Ready(Box::new(props)), Unit::Cm, None);
        assert_eq!(title, "Physical");
        let value = |name: &str| {
            rows.iter()
                .find(|r| r.name == name)
                .map(|r| r.value.clone())
                .expect("row present")
        };
        assert_eq!(value("Volume"), "8.00 cm³");
        assert_eq!(value("Surface area"), "24.00 cm²");
        assert_eq!(value("Centre X"), "1.00 cm");
        assert_eq!(value("Centre Y"), "2.00 cm");
        assert_eq!(value("Centre Z"), "3.00 cm");
        assert!(
            rows.iter().all(|r| r.name != "Mass"),
            "no material, no mass"
        );
        // 8 cm³ of PLA.
        let pla = core_document::Material {
            name: "PLA".into(),
            density: 1.25,
        };
        let (_, rows) = physical_group(
            &crate::ui::Physical::Ready(Box::new(props)),
            Unit::Cm,
            Some(&pla),
        );
        let mass = rows.iter().find(|r| r.name == "Mass").unwrap();
        assert_eq!(mass.value, "10.00 g");
        assert_eq!(value("Izz"), "5.33 cm⁵", "the volume's, at unit density");
        assert_eq!(value("Ixy"), "0.00 cm⁵", "rounding is no product");
        assert_eq!(value("Axis 3"), "0.00 0.00 1.00");
        let moment = |rows: &[PropRow], name: &str| {
            rows.iter().find(|r| r.name == name).unwrap().value.clone()
        };
        assert_eq!(moment(&rows, "I1"), "6.67 g·cm²", "the PLA's");

        let open = kernel_api::PhysicalProperties {
            volume_mm3: None,
            ..props
        };
        let (_, rows) = physical_group(&crate::ui::Physical::Ready(Box::new(open)), Unit::Mm, None);
        assert_eq!(rows[0].value, "encloses none");
    }

    #[test]
    fn created_timestamps_read_as_civil_dates() {
        assert_eq!(format_created(0), "1970-01-01 00:00");
        assert_eq!(format_created(1_700_000_000_000), "2023-11-14 22:13");
    }
}
