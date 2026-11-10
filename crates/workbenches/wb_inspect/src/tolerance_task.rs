//! A tolerance task keeps its opening data for Cancel. Accepted changes
//! record the same arguments the tolerance command consumes.
//!
//! Its panel is declared as widgets and drawn by `core_document::panel` as
//! a package's panel is. One walk serves both ends: without an event it
//! lists the widgets, with one it applies the event to the tolerance it
//! walks, so what is drawn and what a change does read the same code.

use bench_api::{Flow, PanelEvent, Quantity, Select, Widget, panel};

use crate::tolerance::{Characteristic as C, DatumRef, Material, Tolerance};
use core_document::{Document, FeatureId, WorkbenchFeature};

pub(crate) struct Task {
    pub id: FeatureId,
    pub opening: Option<Tolerance>,
}

pub(crate) fn args(t: &Tolerance, id: Option<FeatureId>) -> core_document::CommandArgs {
    let mut args = t.to_json().as_object().cloned().unwrap_or_default();
    for key in ["name", "scan", "angle_deg"] {
        args.remove(key);
    }
    args.retain(|_, v| !v.is_null());
    if let Some(a) = t.angle_deg {
        args.insert("angle".into(), serde_json::json!(a));
    }
    if let Some(id) = id {
        args.insert("tolerance".into(), serde_json::json!(id));
    }
    args.into_iter().collect()
}

/// The width of each of three numbers on a row, so the row fits the panel.
const VECTOR_FIELD: f32 = 56.0;
/// The width of a number beside its check box.
const BESIDE_FIELD: f32 = 72.0;
/// The width of a choice, so a long label and it fit the panel.
const CHOICE: f32 = 130.0;

/// One walk of the panel: the widgets so far and the event it applies.
struct Walk<'e> {
    widgets: Vec<Widget>,
    event: Option<&'e PanelEvent>,
}

impl Walk<'_> {
    fn overline(&mut self, text: &str) {
        self.widgets.push(panel::overline(text));
    }

    fn number_event(&self, id: &str) -> Option<f64> {
        match self.event {
            Some(PanelEvent::Number { id: e, value }) if e == id => Some(*value),
            _ => None,
        }
    }

    fn quantity(&self, id: &str, label: &str, value: f64) -> Quantity {
        Quantity {
            speed: Some(0.01),
            decimals: 3,
            ..panel::quantity(id, label, value)
        }
    }

    /// One of `options`, by its words, after `None` unless `required`;
    /// the choice made.
    fn choose<T: Copy + PartialEq>(
        &mut self,
        (id, label): (&str, &str),
        value: &mut Option<T>,
        options: &[(T, String)],
        required: bool,
    ) {
        let first = usize::from(!required);
        if let Some(PanelEvent::Choice { id: e, index }) = self.event
            && e.as_str() == id
        {
            *value = index
                .checked_sub(first)
                .and_then(|i| options.get(i))
                .map(|o| o.0);
        }
        let selected = value
            .and_then(|v| options.iter().position(|(o, _)| *o == v))
            .map_or(0, |i| i + first);
        let none = (!required).then(|| "None".to_string());
        self.widgets.push(
            Select {
                id: id.into(),
                label: label.into(),
                options: none
                    .into_iter()
                    .chain(options.iter().map(|(_, n)| n.clone()))
                    .collect(),
                selected,
                width: Some(CHOICE),
                ..Default::default()
            }
            .into(),
        );
    }

    /// A check box that gives the value `default` when ticked, and the
    /// value beside it while it has one.
    fn optional(&mut self, id: &str, label: &str, value: &mut Option<f64>, default: f64) {
        if let Some(PanelEvent::Toggle { id: e, on }) = self.event
            && *e == format!("{id}:on")
        {
            *value = on.then_some(default);
        }
        if let (Some(v), Some(now)) = (value.as_mut(), self.number_event(id)) {
            *v = now;
        }
        let mut row: Vec<Widget> = vec![check(id, label, value.is_some())];
        if let Some(v) = value {
            let field = Quantity {
                width: Some(BESIDE_FIELD),
                ..self.quantity(id, "", *v)
            };
            row.push(field.into());
        }
        self.widgets.push(
            Flow {
                children: row,
                no_wrap: true,
                ..Default::default()
            }
            .into(),
        );
    }

    /// Three numbers under a check box, as [`Walk::optional`] gives one.
    fn optional_vector(
        &mut self,
        id: &str,
        label: &str,
        value: &mut Option<[f64; 3]>,
        default: [f64; 3],
    ) {
        if let Some(PanelEvent::Toggle { id: e, on }) = self.event
            && *e == format!("{id}:on")
        {
            *value = on.then_some(default);
        }
        self.widgets.push(check(id, label, value.is_some()));
        if let Some(v) = value {
            for (axis, x) in v.iter_mut().enumerate() {
                if let Some(now) = self.number_event(&format!("{id}:{axis}")) {
                    *x = now;
                }
            }
            let row = v
                .iter()
                .enumerate()
                .map(|(axis, x)| {
                    Quantity {
                        width: Some(VECTOR_FIELD),
                        ..self.quantity(&format!("{id}:{axis}"), "", *x)
                    }
                    .into()
                })
                .collect();
            self.widgets.push(
                Flow {
                    children: row,
                    ..Default::default()
                }
                .into(),
            );
        }
    }

    fn material(&mut self, id: &str, label: &str, value: &mut Option<Material>) {
        let options = [
            (Material::Mmc, "MMC Ⓜ".to_string()),
            (Material::Lmc, "LMC Ⓛ".to_string()),
        ];
        self.choose((id, label), value, &options, false);
    }
}

fn check(id: &str, text: &str, on: bool) -> Widget {
    panel::check(&format!("{id}:on"), "", text, on).into()
}

/// The panel's widgets for `t`, the event applied to it first.
pub(crate) fn walk(
    document: &Document,
    t: &mut Tolerance,
    event: Option<&PanelEvent>,
) -> Vec<Widget> {
    let mut w = Walk {
        widgets: Vec::new(),
        event,
    };
    let named = |kind: Option<&str>| -> Vec<(FeatureId, String)> {
        document
            .feature_tree()
            .all_nodes()
            .filter(|(id, n)| match kind {
                None => crate::element_at(document, **id).is_some_and(|e| e.scan == t.scan),
                Some(kind) => n.workbench_id.as_str() == kind && n.body == Some(t.scan),
            })
            .map(|(id, n)| (*id, n.name.clone()))
            .collect()
    };
    let choices = named(None);
    w.overline("Characteristic");
    let mut characteristic = Some(t.characteristic);
    let kinds: Vec<(C, String)> = C::ALL.iter().map(|c| (*c, c.word().to_string())).collect();
    w.choose(("characteristic", ""), &mut characteristic, &kinds, true);
    if let Some(c) = characteristic {
        t.characteristic = c;
    }
    if let Some(v) = w.number_event("limit") {
        t.limit = v.max(0.0);
    }
    w.widgets.push(
        Quantity {
            min: Some(0.0),
            ..w.quantity("limit", "Limit (mm)", t.limit)
        }
        .into(),
    );
    let mut element = Some(t.element);
    w.choose(("element", "Element"), &mut element, &choices, false);
    if let Some(id) = element {
        t.element = id;
    }
    if t.characteristic == C::Angularity {
        w.optional("angle", "Basic angle (°)", &mut t.angle_deg, 90.0);
    }
    w.overline("Datum frame");
    let mut refs = t.references();
    for (i, label) in ["Primary", "Secondary", "Tertiary"].into_iter().enumerate() {
        let mut id = refs.get(i).map(|r| r.element);
        w.choose((&format!("datum:{i}"), label), &mut id, &choices, false);
        match (refs.get(i), id) {
            (Some(_), None) => refs.truncate(i),
            (Some(_), Some(id)) => refs[i].element = id,
            (None, Some(id)) => refs.push(DatumRef {
                element: id,
                modifier: None,
                size: None,
            }),
            _ => {}
        }
        if let Some(r) = refs.get_mut(i) {
            w.material(&format!("modifier:{i}"), "Datum modifier", &mut r.modifier);
            if r.modifier.is_some() {
                w.optional(
                    &format!("size:{i}"),
                    "Boundary size (mm)",
                    &mut r.size,
                    10.0,
                );
            }
        }
    }
    if refs != t.references() {
        t.datum = None;
        t.datums = refs;
    }
    if t.characteristic == C::Position {
        w.overline("True position");
        w.optional_vector("basic", "Basic offsets (mm)", &mut t.basic, [0.0; 3]);
        w.optional_vector(
            "direction",
            "Zone direction",
            &mut t.basic_direction,
            [0.0, 0.0, 1.0],
        );
        w.material("material", "Material", &mut t.material);
        if t.material.is_some() {
            w.optional(
                "material_size",
                "Material size (mm)",
                &mut t.material_size,
                10.0,
            );
        }
        let mut face = t.reference_face.map(f64::from);
        w.optional("face", "Nominal face index", &mut face, 0.0);
        t.reference_face = face.map(|v| v.max(0.0) as u32);
    }
    if t.characteristic.is_profile() || t.characteristic == C::Position {
        let comparisons = named(Some(crate::compare::KIND));
        w.choose(
            ("comparison", "Comparison"),
            &mut t.comparison,
            &comparisons,
            false,
        );
    }
    if t.characteristic.is_profile() {
        w.optional("unequal", "Zone centre offset (mm)", &mut t.unequal, 0.0);
    }
    if t.characteristic == C::LineProfile {
        let sections = named(Some(crate::section::KIND));
        w.choose(("section", "Section"), &mut t.section, &sections, false);
    }
    if matches!(t.characteristic, C::Symmetry | C::Position) {
        w.choose(
            ("opposed", "Opposed plane (slot)"),
            &mut t.opposed,
            &choices,
            false,
        );
    }
    if t.characteristic == C::CircularRunout {
        w.optional(
            "section_width",
            "Section width (mm)",
            &mut t.section_width,
            0.1,
        );
    }
    w.widgets
}
