//! The mass panel and the parts list.

use super::*;

/// What a field of the mass panel asks.
pub(super) enum MassEdit {
    Density(f32),
    /// A body's row clicked: the body selected.
    Select(BodyId),
    Again,
}

/// What a field of the parts list asks.
pub(super) enum PartsEdit {
    ByComponent(bool),
    /// A material picked, by its place among the presets; past them,
    /// Custom.
    Material(usize),
    Density(f64),
    /// Text typed into a cell.
    Cell {
        row: usize,
        column: usize,
        value: String,
    },
    Bought {
        row: usize,
        on: bool,
    },
    /// A row clicked: its part's first body selected.
    Select(usize),
    NewColumn(String),
    Number,
    AddColumn,
    RemoveColumn(usize),
    Copy,
    Save,
}

impl AssemblyWorkbench {
    /// The mass of the visible bodies at one density, their centre of mass,
    /// and each body's share.
    pub(super) fn mass_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.tab.measuring = None;
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        self.collect_mass(ctx);
        TaskOutcome::Open
    }

    /// The measuring's progress; then the density, the totals and each
    /// body's mass, a click on one selecting it.
    pub(super) fn mass_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        found: Option<&crate::MassReport>,
        density: f32,
        form: &mut Form<'_>,
    ) -> Option<MassEdit> {
        let mut edit = None;
        let Some(report) = found else {
            let (done, total) = self.mass_progress().unwrap_or((0, 0));
            form.push(Widget::Progress {
                label: format!("Measuring {done} of {total} bodies"),
                fraction: Some(if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                }),
                job: None,
            });
            return None;
        };
        let (field, typed) = f::ranged(
            form,
            "density",
            "Density, g/cm³",
            density,
            Dim::Number,
            Some((0.0, 100.0)),
        );
        form.push(field);
        if let Some(value) = typed {
            edit = Some(MassEdit::Density(value as f32));
        }
        let density = f64::from(density);
        let unit = ctx.document.display_unit();
        let line = |label: &str, value: String| Widget::Value {
            label: label.into(),
            value,
            mono: true,
        };
        form.push(line("Mass", mass_text(report.mass_g(density))));
        form.push(line(
            "Volume",
            core_document::format_volume_mm3(report.volume_mm3(), unit, 2),
        ));
        if let Some(c) = report.centre(density) {
            let f = |v: f64| core_document::format_length_mm(v as f32, unit, 2);
            form.push(line(
                "Centre of mass",
                format!("{}, {}, {}", f(c[0]), f(c[1]), f(c[2])),
            ));
        }
        // The principal moments about the centre of mass, in g times the
        // display unit squared.
        if let Some(inertia) = report.inertia(density) {
            let per = f64::from(unit.mm_per_unit());
            for (i, (moment, axis)) in inertia.principal.iter().enumerate() {
                form.push(line(
                    &format!("I{}", i + 1),
                    format!(
                        "{} g·{}² about {}",
                        figure(moment / (per * per)),
                        unit.short_label(),
                        axis.map(|c| format!("{:.2}", if c.abs() < 0.005 { 0.0 } else { c }))
                            .join(" ")
                    ),
                ));
            }
        }
        let (id, event) = f::raw(form, "bodies");
        if let Some(b) = f::picked(event).and_then(|i| report.bodies.get(i)) {
            edit = Some(MassEdit::Select(b.body));
        }
        form.push(Widget::Table {
            id,
            columns: vec!["Body".into(), "Mass".into()],
            rows: report
                .bodies
                .iter()
                .map(|b| {
                    let name = body_name(ctx, b.body);
                    vec![
                        if b.bought {
                            format!("{name} (bought)")
                        } else {
                            name
                        },
                        mass_text(b.mass_g(density)),
                    ]
                })
                .collect(),
            selected: None,
            editable: Vec::new(),
        });
        if report.skipped > 0 {
            form.push(w::text(format!(
                "{} visible bod{} without a closed solid left out",
                report.skipped,
                if report.skipped == 1 { "y" } else { "ies" }
            )));
        }
        let (again, pressed) = f::button(form, "again", "Measure again", ButtonStyle::Secondary);
        form.push(again);
        if pressed {
            edit = Some(MassEdit::Again);
        }
        edit
    }

    /// The density changed, a body clicked, or the bodies measured again.
    pub(super) fn mass_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        edit: MassEdit,
    ) -> Option<TaskOutcome> {
        match edit {
            MassEdit::Density(value) => {
                if let Some(Task::Mass { density, .. }) = &mut self.tab.task {
                    *density = value;
                }
            }
            MassEdit::Select(body) => {
                ctx.request(core_document::HostRequest::SelectBody(body));
            }
            MassEdit::Again => {
                if let Some(Task::Mass { density, .. }) = self.tab.task {
                    self.measure_mass(ctx, density);
                }
            }
        }
        None
    }

    /// Every part, how many of it and its size, with a copy for a
    /// spreadsheet; its item number, whether it is bought and the added
    /// columns, kept in the document.
    pub(super) fn parts_turn(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        request: TaskRequest,
    ) -> TaskOutcome {
        if request.accept || request.cancel {
            self.tab.task = None;
            return TaskOutcome::Cancelled;
        }
        let parts = crate::parts_list(ctx.document, &ctx.bought_kinds);
        self.volumes.refresh(ctx.document, &parts, ctx.kernel);
        TaskOutcome::Open
    }

    /// The rows of the parts list as it shows: by component when asked
    /// for and there are components.
    fn parts_rows(
        document: &core_document::Document,
        parts: &[crate::Part],
        by_component: bool,
    ) -> Vec<crate::parts::LevelRow> {
        if by_component {
            return crate::parts::parts_by_component(document, parts);
        }
        parts
            .iter()
            .enumerate()
            .map(|(i, p)| crate::parts::LevelRow::Part {
                depth: 0,
                part: i,
                bodies: p.bodies.clone(),
            })
            .collect()
    }

    /// The parts list with each part's volume, as far as it is measured.
    fn measured_parts(&self, ctx: &WorkbenchRuntimeContext) -> Vec<crate::Part> {
        let mut parts = crate::parts_list(ctx.document, &ctx.bought_kinds);
        self.volumes.fill(ctx.document, &mut parts);
        parts
    }

    pub(super) fn parts_walk(
        &self,
        ctx: &WorkbenchRuntimeContext,
        form: &mut Form<'_>,
    ) -> Option<PartsEdit> {
        use bench_api::Cell;
        let mut edit = None;
        let parts = self.measured_parts(ctx);
        let unit = ctx.document.display_unit();
        let pending = if self.volumes.running() { "…" } else { "-" };
        let table = crate::parts::table_of(ctx.document)
            .map(|(_, t)| t)
            .unwrap_or_default();
        let total: usize = parts.iter().map(|p| p.bodies.len()).sum();
        form.push(w::text(format!(
            "{} part{}, {total} bod{}",
            parts.len(),
            if parts.len() == 1 { "" } else { "s" },
            if total == 1 { "y" } else { "ies" }
        )));
        let components = !ctx.document.components().is_empty();
        if components {
            let (by_component, on) =
                f::toggle(form, "by_component", "By component", table.by_component);
            form.push(w::hinted(
                "Each component's parts under it, nested as the components are",
                by_component,
            ));
            if let Some(on) = on {
                edit = Some(PartsEdit::ByComponent(on));
            }
        }
        let text = |text: String, mono: bool, strong: bool| Cell::Text { text, mono, strong };
        let rows =
            Self::parts_rows(ctx.document, &parts, table.by_component && components)
                .iter()
                .map(|row| match row {
                    crate::parts::LevelRow::Component { depth, name, .. } => vec![
                        text(String::new(), false, false),
                        text(format!("{}{name}", "    ".repeat(*depth)), false, true),
                        text("1".into(), true, false),
                    ],
                    crate::parts::LevelRow::Part {
                        depth,
                        part,
                        bodies,
                    } => {
                        let part = &parts[*part];
                        let mut cells =
                            vec![
                        text(part.number.map_or("-".to_string(), |n| n.to_string()), true, false),
                        Cell::Link {
                            text: format!("{}{}", "    ".repeat(*depth), part.name),
                        },
                        text(bodies.len().to_string(), true, false),
                        Cell::Edit {
                            text: part.print.to_string(),
                        },
                        text(
                            part.size_mm.map_or_else(
                                || "-".to_string(),
                                |s| format!("{:.1} × {:.1} × {:.1}", s[0], s[1], s[2]),
                            ),
                            true,
                            false,
                        ),
                        text(
                            part.volume_mm3.map_or_else(
                                || pending.to_string(),
                                |v| core_document::format_volume_mm3(v, unit, 2),
                            ),
                            true,
                            false,
                        ),
                        text(
                            part.mass_g().map_or_else(|| pending.to_string(), mass_text),
                            true,
                            false,
                        ),
                        Cell::Check {
                            on: part.bought,
                            hint: Some(
                                "Bought rather than made: left out of exports and the slicer"
                                    .into(),
                            ),
                        },
                        text(part.threads_text(), false, false),
                    ];
                        cells.extend(table.columns.iter().map(|column| Cell::Edit {
                            text: part.values.get(column).cloned().unwrap_or_default(),
                        }));
                        cells
                    }
                })
                .collect();
        let mut columns: Vec<String> = PARTS_COLUMNS.into_iter().map(String::from).collect();
        columns.extend(table.columns.iter().cloned());
        let (id, event) = f::raw(form, "parts");
        match event {
            Some(PanelEvent::CellText {
                row, column, value, ..
            }) => {
                edit = Some(PartsEdit::Cell {
                    row: *row,
                    column: *column,
                    value: value.clone(),
                })
            }
            Some(PanelEvent::CellCheck { row, on, .. }) => {
                edit = Some(PartsEdit::Bought { row: *row, on: *on })
            }
            Some(PanelEvent::Select { index, .. }) => edit = Some(PartsEdit::Select(*index)),
            _ => {}
        }
        form.push(Widget::Sheet { id, columns, rows });
        let preset = crate::parts::PRINT_MATERIALS
            .iter()
            .position(|(name, density)| {
                *name == table.material.name && *density == table.material.density
            });
        let mut options: Vec<String> = crate::parts::PRINT_MATERIALS
            .iter()
            .map(|(name, density)| format!("{name} ({density} g/cm³)"))
            .collect();
        options.push("Custom".into());
        let (material, picked) = f::select(
            form,
            "print_material",
            bench_api::Select {
                label: "Printed in".into(),
                selected: preset.unwrap_or(options.len() - 1),
                options,
                ..Default::default()
            },
        );
        form.push(w::hinted(
            "The filament the parts print in; a body with a material of its own is \
             weighed at its own",
            material,
        ));
        if let Some(index) = picked {
            edit = Some(PartsEdit::Material(index));
        }
        if preset.is_none() {
            let (density, typed) = f::ranged(
                form,
                "print_density",
                "Density, g/cm³",
                table.material.density,
                Dim::Number,
                Some((0.01, 100.0)),
            );
            form.push(density);
            if let Some(value) = typed {
                edit = Some(PartsEdit::Density(value));
            }
        }
        form.push(w::text(print_summary(&parts, &table.material.name, unit)));
        let (number, pressed) =
            f::button(form, "number", "Number the parts", ButtonStyle::Secondary);
        form.push(w::hinted(
            "Give every part without an item number the next one, in list order",
            number,
        ));
        if pressed {
            edit = Some(PartsEdit::Number);
        }
        let (id, event) = f::raw(form, "new_column");
        if let Some(PanelEvent::Text { value, .. }) = event {
            edit = Some(PartsEdit::NewColumn(value.clone()));
        }
        let (add, pressed) = f::button(form, "add_column", "Add column", ButtonStyle::Small);
        if pressed {
            edit = Some(PartsEdit::AddColumn);
        }
        form.push(w::row(vec![
            Widget::TextField {
                id,
                label: "New column".into(),
                value: self.tab.parts_column.clone(),
            },
            add,
        ]));
        if !table.columns.is_empty() {
            let mut removes = Vec::new();
            for (i, column) in table.columns.iter().enumerate() {
                let (remove, pressed) = f::button(
                    form,
                    &format!("remove_column:{i}"),
                    &format!("Remove {column}"),
                    ButtonStyle::Small,
                );
                removes.push(remove);
                if pressed {
                    edit = Some(PartsEdit::RemoveColumn(i));
                }
            }
            form.push(w::row(removes));
        }
        let (copy, pressed) = f::button(form, "copy", "Copy as CSV", ButtonStyle::Secondary);
        if pressed {
            edit = Some(PartsEdit::Copy);
        }
        let (save, pressed) = f::button(form, "save", "Save as CSV", ButtonStyle::Secondary);
        if pressed {
            edit = Some(PartsEdit::Save);
        }
        form.push(w::row(vec![
            w::hinted("For a spreadsheet: every column of the list", copy),
            w::hinted("Write the list to a file a spreadsheet opens", save),
        ]));
        edit
    }

    /// A change to the parts list, kept as `asm.parts_table` keeps it; a
    /// part clicked, selected; the list copied or saved as CSV.
    pub(super) fn parts_edit(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        edit: PartsEdit,
    ) -> Option<TaskOutcome> {
        let parts = self.measured_parts(ctx);
        let mut table = crate::parts::table_of(ctx.document)
            .map(|(_, t)| t)
            .unwrap_or_default();
        let before = table.clone();
        let components = !ctx.document.components().is_empty();
        let rows = Self::parts_rows(ctx.document, &parts, table.by_component && components);
        let part_at = |row: usize| match rows.get(row)? {
            crate::parts::LevelRow::Part { part, bodies, .. } => Some((&parts[*part], bodies)),
            crate::parts::LevelRow::Component { .. } => None,
        };
        const FIXED: usize = PARTS_COLUMNS.len();
        let edit_copy = matches!(edit, PartsEdit::Copy);
        match edit {
            PartsEdit::ByComponent(on) => table.by_component = on,
            PartsEdit::Material(index) => {
                table.material = match crate::parts::PRINT_MATERIALS.get(index) {
                    Some((name, _)) => crate::parts::PrintMaterial::named(name, None),
                    None => crate::parts::PrintMaterial {
                        name: "Custom".into(),
                        density: table.material.density,
                    },
                };
            }
            PartsEdit::Density(value) => {
                if value > 0.0 {
                    table.material.density = value as f32;
                }
            }
            PartsEdit::Cell { row, column, value } if column == PRINT_COLUMN => {
                let (part, _) = part_at(row)?;
                let value = value.trim();
                let count = if value.is_empty() {
                    None
                } else {
                    Some(value.parse::<u32>().ok()?)
                };
                table.entry_mut(&part.bodies).print = count;
            }
            PartsEdit::Select(row) => {
                let (_, bodies) = part_at(row)?;
                ctx.request(core_document::HostRequest::SelectBody(*bodies.first()?));
            }
            PartsEdit::Bought { row, on } => {
                let (part, _) = part_at(row)?;
                table.set_bought(part, on);
            }
            PartsEdit::Cell { row, column, value } => {
                let (part, _) = part_at(row)?;
                let column = table.columns.get(column.checked_sub(FIXED)?)?.clone();
                table.entry_mut(&part.bodies).values.insert(column, value);
            }
            PartsEdit::NewColumn(value) => self.tab.parts_column = value,
            PartsEdit::Number => table.number(&parts),
            PartsEdit::AddColumn => {
                let name = self.tab.parts_column.trim().to_string();
                if !name.is_empty() && !table.columns.contains(&name) {
                    table.columns.push(name);
                    self.tab.parts_column.clear();
                }
            }
            PartsEdit::Copy | PartsEdit::Save => {
                let csv = if table.by_component && components {
                    crate::parts::levels_csv(&parts, &rows, &table.columns)
                } else {
                    crate::parts_csv(&parts, &table.columns)
                };
                if edit_copy {
                    ctx.request(core_document::HostRequest::CopyText(csv));
                    ctx.log_info("Parts list copied");
                } else {
                    ctx.request(core_document::HostRequest::SaveFile {
                        name: "parts.csv".into(),
                        kind: "Comma-separated values".into(),
                        extension: "csv".into(),
                        contents: csv.into_bytes(),
                    });
                }
            }
            PartsEdit::RemoveColumn(i) => {
                let column = table.columns.get(i)?.clone();
                table.columns.retain(|c| *c != column);
                for entry in table.entries.values_mut() {
                    entry.values.remove(&column);
                }
            }
        }
        if table != before {
            let args = crate::commands::object(serde_json::json!({
                "table": serde_json::to_value(&table).unwrap_or_default(),
            }));
            match crate::commands::run("asm.parts_table", &args, ctx) {
                Ok(_) => {
                    ctx.record("asm.parts_table", args, serde_json::Value::Null);
                    ctx.request(core_document::HostRequest::JournalLabel(
                        "Edit parts list".into(),
                    ));
                }
                Err(why) => ctx.log_warn(format!("Could not keep the parts list: {why}")),
            }
        }
        None
    }
}

/// A mass in grams, or kilograms from a thousand.
/// The parts list's own columns, before the ones added to it.
const PARTS_COLUMNS: [&str; 9] = [
    "No.",
    "Part",
    "Qty",
    "Print",
    "Size (mm)",
    "Volume",
    "Mass",
    "Bought",
    "Threads",
];

/// The parts list's column of how many of a part to print.
const PRINT_COLUMN: usize = 3;

/// What the print takes: how many pieces, their volume and the
/// filament's mass, as far as the parts are measured.
fn print_summary(parts: &[crate::Part], material: &str, unit: core_document::Unit) -> String {
    let printed: Vec<&crate::Part> = parts.iter().filter(|p| p.print > 0).collect();
    let pieces: u32 = printed.iter().map(|p| p.print).sum();
    if pieces == 0 {
        return "Nothing to print".into();
    }
    let s = if pieces == 1 { "" } else { "s" };
    if printed.iter().any(|p| p.volume_mm3.is_none()) {
        return format!("To print: {pieces} piece{s}, measuring");
    }
    let volume: f64 = printed
        .iter()
        .map(|p| p.volume_mm3.unwrap_or(0.0) * f64::from(p.print))
        .sum();
    let mass: f64 = printed
        .iter()
        .map(|p| p.mass_g().unwrap_or(0.0) * f64::from(p.print))
        .sum();
    format!(
        "To print: {pieces} piece{s}, {}, {} of {material}",
        core_document::format_volume_mm3(volume, unit, 2),
        mass_text(mass)
    )
}

fn mass_text(grams: f64) -> String {
    if grams >= 1000.0 {
        format!("{:.3} kg", grams / 1000.0)
    } else {
        format!("{grams:.2} g")
    }
}

/// A figure to about four significant digits: plainly while it reads so,
/// in powers of ten when it is very large or small.
fn figure(v: f64) -> String {
    if v == 0.0 || (0.01..1e6).contains(&v.abs()) {
        format!("{v:.2}")
    } else {
        format!("{v:.3e}")
    }
}
