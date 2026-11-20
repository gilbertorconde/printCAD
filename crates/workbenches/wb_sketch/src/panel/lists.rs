//! The element and constraint lists under one filter: their rows, the
//! rows kept between frames, and the text and tone of each row's value.

use super::*;

impl SketchWorkbench {
    /// [`Self::build_list_widgets`], declared again only when what they
    /// are declared from changed, a row is to be brought into view, or
    /// the last ones drawn were not given back ([`Self::keep_list_widgets`]).
    pub(super) fn list_widgets(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &std::sync::Arc<crate::SketchFeature>,
    ) -> Vec<Widget> {
        let lost = self
            .tab
            .active_sketch_id
            .map(|id| self.lost_external(ctx.document, ctx.kernel, id))
            .unwrap_or_default();
        let key = ListKey {
            sketch: self.active_key(ctx).without_label(),
            lost,
            palette: ctx.sketch_palette,
            selected: (*self.tab.selected).clone(),
            selected_constraints: self.tab.selected_constraints.clone(),
            hovered: self.tab.hovered,
            hover_from_panel: self.tab.hover_from_panel,
            hovered_constraint: self.tab.hovered_constraint,
            element_filter: self.tab.element_filter,
            constraint_filter: self.tab.constraint_filter.clone(),
            constraint_kind_filter: self.tab.constraint_kind_filter,
            diagnosis: self.tab.last_diagnosis.clone(),
            pending_focus: self.tab.pending_focus,
            renaming: self.tab.renaming_constraint,
        };
        let tab = &mut *self.tab;
        if tab.scroll_to_row.is_none()
            && let Some((kept, widgets)) = &mut tab.panel.lists
            && *kept == key
            && let Some(widgets) = widgets.take()
        {
            return widgets;
        }
        let kept = tab.panel.lists.take().and_then(|(_, widgets)| widgets);
        let kept_rows =
            kept.map_or_else(Vec::new, |mut widgets| take_items(&mut widgets, "elements"));
        let widgets = self.build_list_widgets(ctx, feature, kept_rows);
        self.tab.panel.lists = Some((key, None));
        widgets
    }

    /// Take back the list widgets out of the panel's `widgets` once drawn,
    /// for [`Self::list_widgets`] to declare again while their key holds.
    pub(super) fn keep_list_widgets(&mut self, mut widgets: Vec<Widget>) {
        let range = std::mem::take(&mut self.tab.panel.list_range);
        if let Some((_, kept @ None)) = &mut self.tab.panel.lists
            && range.end <= widgets.len()
            && !range.is_empty()
        {
            *kept = Some(widgets.drain(range).collect());
        }
    }

    /// One filter over both lists, then the elements and the constraints,
    /// each folding on its own.
    fn build_list_widgets(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &std::sync::Arc<crate::SketchFeature>,
        kept_rows: Vec<Item>,
    ) -> Vec<Widget> {
        let sketch = &feature.sketch;
        let choice = if self.tab.element_filter != ElementFilter::All {
            ListFilter::Elements(self.tab.element_filter)
        } else {
            ListFilter::Constraints(self.tab.constraint_kind_filter)
        };
        // What the rows light comes from what last frame left in hand: the
        // item the panel hovered, and the selection.
        let relations = self.frame_relations(feature);
        let panel_element = self.tab.hovered.filter(|_| self.tab.hover_from_panel);
        let hand = InHand {
            related: relations.related(
                self.tab.selected.iter().copied().chain(panel_element),
                self.tab
                    .selected_constraints
                    .iter()
                    .copied()
                    .chain(self.tab.hovered_constraint),
            ),
            relations,
            element: self.tab.hovered,
            constraint: self.tab.hovered_constraint,
        };
        let elements = self.element_items(ctx, feature, &hand, kept_rows);
        let mut constraint_children = Vec::new();
        let shown = self.listed_constraints(sketch);
        // While a filter narrows the list, what it shows can go at once.
        if !shown.is_empty() && shown.len() < sketch.constraints.len() {
            constraint_children.push(panel::hinted(
                "Delete every constraint the filter shows",
                panel::button(
                    "delete_shown",
                    &format!("Delete {} shown", shown.len()),
                    ButtonStyle::Small,
                    true,
                ),
            ));
        }
        let constraints = self.constraint_items(ctx, sketch, &hand, &shown);
        constraint_children.push(
            ItemList {
                id: "constraints".into(),
                items: constraints,
                empty: sketch
                    .constraints
                    .is_empty()
                    .then(|| "None yet. Select geometry and pick a constraint tool.".into()),
                ..Default::default()
            }
            .into(),
        );
        self.tab.pending_focus = None;
        vec![
            panel::gap(GapSize::Medium),
            FilterField {
                id: "find".into(),
                hint: "Filter".into(),
                kinds: ListFilter::ALL.iter().map(|(_, k)| k.to_string()).collect(),
                kind: ListFilter::ALL
                    .iter()
                    .position(|(f, _)| *f == choice)
                    .unwrap_or(0),
                by_bench: true,
            }
            .into(),
            panel::gap(GapSize::Small),
            section(
                "sketch_elements",
                "Elements",
                Some(sketch.geometry.len() - sketch.derived.len()),
                None,
                true,
                vec![
                    ItemList {
                        id: "elements".into(),
                        items: elements,
                        empty: sketch
                            .geometry
                            .is_empty()
                            .then(|| "No geometry yet. Draw with the tools above.".into()),
                        ..Default::default()
                    }
                    .into(),
                ],
            ),
            section(
                "sketch_constraints",
                "Constraints",
                Some(sketch.constraints.len()),
                None,
                true,
                constraint_children,
            ),
        ]
    }

    /// The element rows: the sketch's reference geometry, then its
    /// elements as the filter passes them, each with its size and a badge
    /// per constraint on it. A row whose [`ElementRowKey`] is the one
    /// `kept` was declared from is taken over as it stands.
    fn element_items(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        feature: &std::sync::Arc<crate::SketchFeature>,
        hand: &InHand,
        kept: Vec<Item>,
    ) -> Vec<Item> {
        let sketch = &feature.sketch;
        let mut kept = KeptRows {
            rows: (std::mem::take(&mut self.tab.panel.element_keys).into_iter())
                .zip(kept)
                .map(|(key, item)| key.map(|key| (key, item)))
                .collect(),
            next: 0,
            index: None,
        };
        self.tab.panel.elements.clear();
        let mut items = Vec::new();
        if sketch.geometry.is_empty() {
            return items;
        }
        let text = self.tab.constraint_filter.trim().to_lowercase();
        let named = |name: &str| text.is_empty() || name.to_lowercase().contains(&text);
        // The reference geometry every sketch has: pickable from here as
        // well as from the viewport, but never drawn into a shape.
        if self.tab.element_filter == ElementFilter::All {
            for (reference, id) in [
                (sketch::Reference::Origin, sketch::ORIGIN_ID),
                (sketch::Reference::XAxis, sketch::X_AXIS_ID),
                (sketch::Reference::YAxis, sketch::Y_AXIS_ID),
            ] {
                if !named(reference.label()) {
                    continue;
                }
                self.tab.panel.elements.push(ElementRow {
                    id,
                    badges: Vec::new(),
                });
                self.tab.panel.element_keys.push(None);
                items.push(Item {
                    icon: Some(
                        if reference.is_point() {
                            "point"
                        } else {
                            "line"
                        }
                        .into(),
                    ),
                    muted: true,
                    label: reference.label().into(),
                    value: Some("reference".into()),
                    value_tone: Tone::Muted,
                    selected: self.tab.selected.contains(&id),
                    related: hand.related.elements.contains(&id),
                    lit: hand.element == Some(id),
                    ..Default::default()
                });
            }
        }
        let pal = ctx.sketch_palette;
        let diagnosis = self.tab.last_diagnosis.clone().unwrap_or_default();
        let formulas = Formulas::of(ctx.document, self.tab.active_sketch_id);
        // Projected elements whose edge, face, element or datum is gone.
        let lost: std::collections::HashSet<Uuid> = self
            .tab
            .active_sketch_id
            .map(|sketch_id| self.lost_external(ctx.document, ctx.kernel, sketch_id))
            .unwrap_or_default()
            .into_iter()
            .collect();
        let external = sketch.external_ids();
        let locked = self.locked_elements(feature);
        let rows = self
            .frame
            .row_text(self.active_key(ctx).without_label(), || RowText::of(sketch));
        let by_id: std::collections::HashMap<Uuid, &Constraint> =
            sketch.constraints.iter().map(|c| (c.id, c)).collect();
        let anchors: std::collections::HashSet<Uuid> =
            sketch.texts.iter().map(|b| b.anchor).collect();
        for geom in &sketch.geometry {
            let id = geom.id();
            if sketch.derived.contains(&id) || !self.tab.element_filter.accepts(sketch, id) {
                continue;
            }
            if !text.is_empty() && !named(&rows.name(sketch, id)) {
                continue;
            }
            // A badge for each constraint on the element or its points.
            let on: Vec<&Constraint> = hand
                .relations
                .constraints_on(id)
                .iter()
                .filter_map(|c| by_id.get(c).copied())
                .collect();
            let selected = self.tab.selected.contains(&id);
            let key = ElementRowKey {
                geom: geom.clone(),
                ends: match geom {
                    sketch::GeometryElement::Line(l) => sketch
                        .point_position(l.start)
                        .zip(sketch.point_position(l.end))
                        .map(|(a, b)| [a, b]),
                    _ => None,
                },
                ordinal: rows.ordinal(id),
                style: (
                    external.contains(&id),
                    sketch.external.get(&id).map(|source| source.defining),
                    sketch.is_construction(id),
                    sketch.is_fully_constrained,
                    locked.contains(&id),
                    pal,
                ),
                lit: (
                    selected,
                    hand.related.elements.contains(&id),
                    hand.element == Some(id),
                    lost.contains(&id),
                ),
                badges: on
                    .iter()
                    .map(|c| BadgeKey {
                        kind: c.kind.clone(),
                        name: c.name.clone(),
                        lit: hand.constraint == Some(c.id)
                            || self.tab.selected_constraints.contains(&c.id),
                        tint: constraint_tint(c, &diagnosis, &formulas),
                    })
                    .collect(),
            };
            self.tab.panel.elements.push(ElementRow {
                id,
                badges: on.iter().map(|c| c.id).collect(),
            });
            // A text's point is named after its text, which the key leaves
            // out; a row to be brought into view is declared anew, and kept
            // by no key, as it is brought into view once.
            let reveal = self.reveal(id);
            let reused =
                (kept.take(id)).filter(|(was, _)| *was == key && !anchors.contains(&id) && !reveal);
            if let Some((_, item)) = reused {
                self.tab.panel.element_keys.push(Some(key));
                items.push(item);
                continue;
            }
            let style = overlay::element_style(sketch, id, &self.tab.selected, None, &locked, &pal);
            let color = style.color.map(|c| (c * 255.0) as u8);
            let detail = if external.contains(&id) {
                Some("projected")
            } else if sketch.is_construction(id) {
                Some("construction")
            } else {
                None
            };
            let badges = on
                .iter()
                .zip(&key.badges)
                .map(|(c, badge)| Badge {
                    icon: constraint_icon(&c.kind).into(),
                    hint: crate::constraint_row_label(c),
                    lit: badge.lit,
                    tone: badge.tint.0,
                    color: badge.tint.1,
                })
                .collect();
            let construction = if sketch.is_construction(id) {
                "Make normal geometry"
            } else {
                "Make construction"
            };
            items.push(Item {
                icon: Some(element_row_icon(geom, external.contains(&id)).into()),
                color: Some([color[0], color[1], color[2]]),
                label: rows.name(sketch, id),
                secondary: detail.map(String::from),
                value: Some(element_value(sketch, geom)),
                flag: lost.contains(&id).then(|| StatusLine {
                    text: "lost".into(),
                    tone: Tone::Warning,
                    detail: "What it was projected from is gone: it stays where it was".into(),
                }),
                badges,
                selected,
                related: key.lit.1,
                lit: key.lit.2,
                reveal,
                menu: vec![
                    action("construction", construction, "", false),
                    action("delete", "Delete", "", true),
                ],
                ..Default::default()
            });
            self.tab.panel.element_keys.push((!reveal).then_some(key));
        }
        items
    }

    /// The constraint rows the filter shows: each names the elements it
    /// ties, and a dimension's value is a field while the row is in hand.
    fn constraint_items(
        &mut self,
        ctx: &WorkbenchRuntimeContext,
        sketch: &Sketch,
        hand: &InHand,
        shown: &[Uuid],
    ) -> Vec<Item> {
        self.tab.panel.constraints.clear();
        let diagnosis = self.tab.last_diagnosis.clone().unwrap_or_default();
        let formulas = Formulas::of(ctx.document, self.tab.active_sketch_id);
        let listed: std::collections::HashSet<&Uuid> = shown.iter().collect();
        let text = self
            .frame
            .row_text(self.active_key(ctx).without_label(), || RowText::of(sketch));
        let mut items = Vec::new();
        for constraint in &sketch.constraints {
            if !listed.contains(&constraint.id) {
                continue;
            }
            let id = constraint.id;
            let selected = self.tab.selected_constraints.contains(&id);
            let operands = hand.relations.operands(id).to_vec();
            let tags = operands
                .iter()
                .map(|e| Tag {
                    text: text.name(sketch, *e),
                    selected: self.tab.selected.contains(e),
                    lit: hand.element == Some(*e),
                    ..Default::default()
                })
                .collect();
            // The value is plain text until the row is the one in hand.
            let live =
                selected || hand.constraint == Some(id) || self.tab.pending_focus == Some(id);
            let (value, value_tone, value_hint, field) = if live {
                dimension_cell(
                    sketch,
                    constraint,
                    self.tab.active_sketch_id,
                    self.tab.pending_focus == Some(id),
                )
            } else {
                match dimension_text(sketch, constraint) {
                    Some((text, tone)) => (Some(text), tone, None, None),
                    None => (None, Tone::Neutral, None, None),
                }
            };
            let renaming = self.tab.renaming_constraint == Some(id);
            let mut label = if renaming {
                constraint.name.clone().unwrap_or_default()
            } else {
                crate::constraint_row_label(constraint)
            };
            if constraint.parked && !renaming {
                label.push_str(" (parked)");
            }
            let mut menu = vec![action(
                "active",
                if constraint.active {
                    "Deactivate"
                } else {
                    "Activate"
                },
                "",
                false,
            )];
            if constraint.kind.is_dimensional() {
                menu.push(action(
                    "driving",
                    if constraint.driving {
                        "Make reference"
                    } else {
                        "Make driving"
                    },
                    "Reference dimensions are measured, not enforced",
                    false,
                ));
            }
            menu.push(action("rename", "Rename", "", false));
            menu.push(action(
                "park",
                if constraint.parked { "Unpark" } else { "Park" },
                "Parked symbols draw only while the parked layer shows",
                false,
            ));
            menu.push(action("delete", "Delete", "", true));
            self.tab
                .panel
                .constraints
                .push(ConstraintRow { id, tags: operands });
            let (tone, color) = constraint_tint(constraint, &diagnosis, &formulas);
            items.push(Item {
                icon: Some(constraint_icon(&constraint.kind).into()),
                tone,
                color,
                muted: !constraint.active,
                problem: diagnosis.conflicting.contains(&id) || diagnosis.redundant.contains(&id),
                label,
                value,
                value_tone,
                value_hint,
                field,
                tags,
                selected,
                related: hand.related.constraints.contains(&id),
                lit: hand.constraint == Some(id),
                hint: Some(sketch::constraint_label(&constraint.kind)),
                editing: renaming,
                reveal: self.reveal(id),
                menu,
                ..Default::default()
            });
        }
        items
    }

    /// Whether the row of `id` is the one to bring into view; it is
    /// brought once.
    fn reveal(&mut self, id: Uuid) -> bool {
        let reveal = self.tab.scroll_to_row == Some(id);
        if reveal {
            self.tab.scroll_to_row = None;
        }
        reveal
    }
}

/// The list's filter choices: a kind of constraint, or of element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListFilter {
    Constraints(ConstraintFilter),
    Elements(ElementFilter),
}

impl ListFilter {
    pub(super) const ALL: [(ListFilter, &'static str); 10] = [
        (ListFilter::Constraints(ConstraintFilter::All), "All"),
        (
            ListFilter::Constraints(ConstraintFilter::Selected),
            "Selected",
        ),
        (
            ListFilter::Constraints(ConstraintFilter::Related),
            "Related to selection",
        ),
        (
            ListFilter::Constraints(ConstraintFilter::Geometric),
            "Relations",
        ),
        (
            ListFilter::Constraints(ConstraintFilter::Dimensional),
            "Dimensions",
        ),
        (
            ListFilter::Constraints(ConstraintFilter::Reference),
            "Reference",
        ),
        (ListFilter::Constraints(ConstraintFilter::Named), "Named"),
        (ListFilter::Constraints(ConstraintFilter::Parked), "Parked"),
        (
            ListFilter::Elements(ElementFilter::Normal),
            "Normal geometry",
        ),
        (
            ListFilter::Elements(ElementFilter::Construction),
            "Construction",
        ),
    ];
}

/// The id of constraint `id`'s value field.
pub(super) fn dimension_field(id: Uuid) -> String {
    format!("dim:{id}")
}

/// What the lists name each element of a sketch by: its place among the
/// elements of its kind, worked out once per read of the sketch.
pub(crate) struct RowText {
    ordinals: std::collections::HashMap<Uuid, usize>,
}

impl RowText {
    pub(crate) fn of(sketch: &Sketch) -> Self {
        Self {
            ordinals: crate::style::ordinals(sketch),
        }
    }

    /// [`crate::style::element_name`] of `id`.
    fn name(&self, sketch: &Sketch, id: Uuid) -> String {
        match self.ordinals.get(&id) {
            Some(&n) => crate::style::name_with(sketch, id, Some(n)),
            None => crate::style::element_name(sketch, id),
        }
    }

    fn ordinal(&self, id: Uuid) -> Option<usize> {
        self.ordinals.get(&id).copied()
    }
}

/// An element's size as its row shows it: a line's length, a round's
/// radius, a point's place; else its kind.
fn element_value(sketch: &Sketch, geom: &sketch::GeometryElement) -> String {
    use sketch::GeometryElement as G;
    let at = |id| sketch.point_position(id);
    match geom {
        G::Point(p) => format!("{:.2}, {:.2}", p.position.x, p.position.y),
        G::Line(l) => match (at(l.start), at(l.end)) {
            (Some(a), Some(b)) => format!("{:.2} mm", (b.x - a.x).hypot(b.y - a.y)),
            _ => element_kind(geom).to_string(),
        },
        G::Arc(a) => format!("R {:.2} mm", a.radius),
        G::Circle(c) => format!("R {:.2} mm", c.radius),
        _ => element_kind(geom).to_string(),
    }
}

/// A dimension's value as plain text and its tone: a driving one as it
/// is set, a reference one measured and in parentheses.
fn dimension_text(sketch: &Sketch, constraint: &Constraint) -> Option<(String, Tone)> {
    let value = sketch::dimension_value(&constraint.kind)?;
    let unit = sketch::dimension_unit(&constraint.kind);
    let (decimals, suffix) = match unit {
        sketch::DimensionUnit::Angle => (1, "°"),
        sketch::DimensionUnit::Length => (2, " mm"),
        sketch::DimensionUnit::Ratio => (3, ""),
    };
    if constraint.driving && constraint.active {
        Some((format!("{value:.decimals$}{suffix}"), Tone::Neutral))
    } else {
        let measured = sketch::measured_value(sketch, &constraint.kind)?;
        Some((
            format!("({measured:.decimals$}{suffix})"),
            measured_tone(constraint),
        ))
    }
}

/// A measured value's tone: the accent while it counts, dim while not.
fn measured_tone(constraint: &Constraint) -> Tone {
    if constraint.active {
        Tone::Accent
    } else {
        Tone::Muted
    }
}

type Cell = (Option<String>, Tone, Option<String>, Option<Box<Widget>>);

/// The value cell of a constraint row in hand: a driving value as a field
/// formulas may set, or the measured reference value in parentheses.
fn dimension_cell(
    sketch: &Sketch,
    constraint: &Constraint,
    sketch_id: Option<FeatureId>,
    focus: bool,
) -> Cell {
    let Some(value) = sketch::dimension_value(&constraint.kind) else {
        return (None, Tone::Neutral, None, None);
    };
    let unit = sketch::dimension_unit(&constraint.kind);
    if constraint.driving && constraint.active {
        let (dim, speed, decimals) = match unit {
            sketch::DimensionUnit::Angle => (Dim::Angle, 1.0, 1),
            sketch::DimensionUnit::Length => (Dim::Length, 0.1, 2),
            sketch::DimensionUnit::Ratio => (Dim::Number, 0.01, 3),
        };
        let field = Quantity {
            id: dimension_field(constraint.id),
            value,
            dim,
            bind: sketch_id.map(|s| Bind {
                feature: s.0.to_string(),
                key: constraint.id.to_string(),
            }),
            decimals,
            speed: Some(speed),
            width: Some(96.0),
            focus,
            ..Default::default()
        };
        return (None, Tone::Neutral, None, Some(Box::new(field.into())));
    }
    let text = match sketch::measured_value(sketch, &constraint.kind) {
        Some(m) if unit == sketch::DimensionUnit::Angle => format!("({m:.1}°)"),
        Some(m) => format!("({m:.2})"),
        None => "(-)".to_string(),
    };
    (
        Some(text),
        measured_tone(constraint),
        Some("Measured value (reference dimension)".into()),
        None,
    )
}

/// What a constraint's colour reads besides the constraint: the
/// constraints a formula of the open sketch sets.
struct Formulas {
    set: std::collections::HashSet<Uuid>,
}

impl Formulas {
    fn of(document: &core_document::Document, sketch_id: Option<FeatureId>) -> Self {
        let node = sketch_id.and_then(|id| document.feature_tree().get_node(id));
        let set = (node.into_iter())
            .flat_map(|node| node.formulas.keys())
            .filter_map(|key| {
                Uuid::parse_str(key)
                    .ok()
                    .filter(|id| id.to_string() == *key)
            })
            .collect();
        Self { set }
    }
}

/// How a constraint's icon reads: a conflict, a redundancy, an inactive
/// one or a reference dimension by its tone; a dimension a formula sets
/// and the rest in the sketch palette's colours, which no tone names.
fn constraint_tint(
    constraint: &Constraint,
    diagnosis: &crate::Diagnosis,
    formulas: &Formulas,
) -> (Tone, Option<[u8; 3]>) {
    let palette = |color: Color32| (Tone::Neutral, Some([color.r(), color.g(), color.b()]));
    if diagnosis.conflicting.contains(&constraint.id) {
        (Tone::Bad, None)
    } else if diagnosis.redundant.contains(&constraint.id) {
        (Tone::Warning, None)
    } else if !constraint.active {
        (Tone::Muted, None)
    } else if constraint.kind.is_dimensional() && !constraint.driving {
        (Tone::Accent, None)
    } else if formulas.set.contains(&constraint.id) {
        palette(SKETCH_FORMULA)
    } else {
        palette(SKETCH_CONSTRAINT)
    }
}

#[cfg(all(test, feature = "egui"))]
mod kept_lists {
    use super::*;
    use crate::SketchFeature;
    use crate::sketch::{ConstraintKind, GeometryElement, Line, Point, Vec2D};
    use core_document::{Document, Workbench};

    type Change = dyn Fn(&mut SketchWorkbench, &mut WorkbenchRuntimeContext);

    /// The panel's widgets, as drawn and given back after.
    fn draw(wb: &mut SketchWorkbench, ctx: &mut WorkbenchRuntimeContext) -> Vec<Widget> {
        wb.on_frame(0.016, ctx);
        let widgets = wb.task_widgets(ctx);
        let drawn = widgets.clone();
        wb.keep_list_widgets(widgets);
        drawn
    }

    /// The lists declared from nothing kept.
    fn afresh(wb: &mut SketchWorkbench, ctx: &mut WorkbenchRuntimeContext) -> Vec<Widget> {
        wb.tab.panel.lists = None;
        draw(wb, ctx)
    }

    /// The element and constraint lists are declared once while nothing
    /// they show changes, and again, as declared afresh, after a
    /// selection, a constraint in hand, a filter or an edit.
    #[test]
    fn the_lists_are_declared_again_only_when_what_they_show_changes() {
        let mut sketch = Sketch::new("s");
        let lone = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(9.0, 9.0))));
        let [a, b] = [0.0, 5.0]
            .map(|x| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 1.0)))));
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let length = sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
        let mut doc = Document::new("t");
        let feature = SketchFeature::new(sketch, SketchPlane::xy());
        let id = doc.add_feature(feature, "s".into()).unwrap();
        let mut wb = SketchWorkbench::default();
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = Some(id);
        let first = draw(&mut wb, &mut ctx);
        // A widget planted in the kept lists comes back: they were not
        // declared again.
        let planted = panel::gap(GapSize::Large);
        if let Some((_, Some(kept))) = &mut wb.tab.panel.lists {
            kept.push(planted.clone());
        }
        let again = draw(&mut wb, &mut ctx);
        assert_eq!(again.len(), first.len() + 1);
        assert!(again.contains(&planted));
        let edit = |change: fn(&mut Sketch, [Uuid; 3])| {
            move |wb: &mut SketchWorkbench, ctx: &mut WorkbenchRuntimeContext| {
                let mut feature = wb.get_active_sketch(ctx).unwrap();
                change(&mut feature.sketch, [lone, b, line]);
                wb.store_sketch(ctx, feature);
            }
        };
        let moved = edit(|sketch, [_, b, _]| {
            if let Some(GeometryElement::Point(p)) = sketch.get_geometry_mut(b) {
                p.position = Vec2D::new(7.0, 3.0);
            }
        });
        let added = edit(|sketch, _| {
            sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(2.0, 2.0))));
        });
        let construction = edit(|sketch, [_, _, line]| sketch.set_construction(line, true));
        // Its points' rows read projected too.
        let projected = edit(|sketch, [_, _, line]| {
            let datum = sketch::ExternalReference::Datum {
                datum: Uuid::from_u128(7),
            };
            (sketch.external).insert(line, sketch::ExternalSource::of_reference(datum));
        });
        // The lone point gone, the other points' names count down.
        let removed = edit(|sketch, [lone, _, _]| {
            sketch.remove_geometry_cascade(&[lone]);
        });
        let changes: [(&str, &Change); 10] = [
            ("a selection", &move |wb, _| {
                wb.tab.selected.insert(line);
            }),
            ("a constraint in hand", &move |wb, _| {
                wb.tab.hovered_constraint = Some(length);
            }),
            ("an element hovered", &move |wb, _| {
                wb.tab.hovered = Some(a);
            }),
            ("a point moved", &moved),
            ("an element added", &added),
            ("an element made construction", &construction),
            ("a line projected", &projected),
            ("an element removed", &removed),
            ("an edit", &|wb, ctx| {
                let mut feature = wb.get_active_sketch(ctx).unwrap();
                feature.sketch.constraints.clear();
                wb.store_sketch(ctx, feature);
            }),
            ("a filter", &|wb, _| {
                wb.tab.constraint_filter = "length".into();
            }),
        ];
        for (what, change) in changes {
            change(&mut wb, &mut ctx);
            let kept = draw(&mut wb, &mut ctx);
            assert!(!kept.contains(&planted), "{what}");
            assert_eq!(kept, afresh(&mut wb, &mut ctx), "{what}");
            if let Some((_, Some(kept))) = &mut wb.tab.panel.lists {
                kept.push(planted.clone());
            }
        }
    }
}
