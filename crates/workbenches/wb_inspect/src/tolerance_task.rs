//! A tolerance task: accepted changes record the same arguments the
//! tolerance command consumes, a new tolerance's or an edit's by its
//! opening data.
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

/// One walk of the panel through `bench_api::panel::Form`, keeping the
/// tolerance task's look: optional values behind check boxes, choices
/// with a "None" first.
struct Walk<'e> {
    form: panel::Form<'e>,
}

impl Walk<'_> {
    fn overline(&mut self, text: &str) {
        self.form.push(panel::overline(text));
    }

    fn quantity(&self, label: &str) -> Quantity {
        Quantity {
            speed: Some(0.01),
            decimals: 3,
            ..panel::quantity("", label, 0.0)
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
        let none = (!required).then(|| (None, "None".to_string()));
        let options: Vec<(Option<T>, String)> = none
            .into_iter()
            .chain(options.iter().map(|(v, n)| (Some(*v), n.clone())))
            .collect();
        let shape = Select {
            label: label.into(),
            width: Some(CHOICE),
            ..Default::default()
        };
        let bound = self.form.choose(id, value, &options, shape);
        self.form.put(bound);
    }

    /// A check box named `{id}:on` with `label` beside it, showing
    /// whether `value` has one; ticking it gives `default`.
    fn presence<T>(&mut self, id: &str, label: &str, value: &mut Option<T>, default: T) -> Widget {
        let mut on = value.is_some();
        let shape = panel::check("", "", label, false);
        let bound = self.form.check(&format!("{id}:on"), &mut on, shape);
        if bound.set {
            *value = on.then_some(default);
        }
        bound.widget.into()
    }

    /// A check box that gives the value `default` when ticked, and the
    /// value beside it while it has one.
    fn optional(&mut self, id: &str, label: &str, value: &mut Option<f64>, default: f64) {
        let mut row = vec![self.presence(id, label, value, default)];
        if let Some(v) = value {
            let shape = Quantity {
                width: Some(BESIDE_FIELD),
                ..self.quantity("")
            };
            row.push(self.form.number(id, v, shape).widget.into());
        }
        self.form.push(Flow {
            children: row,
            no_wrap: true,
            ..Default::default()
        });
    }

    /// Three numbers under a check box, as [`Walk::optional`] gives one.
    fn optional_vector(
        &mut self,
        id: &str,
        label: &str,
        value: &mut Option<[f64; 3]>,
        default: [f64; 3],
    ) {
        let check = self.presence(id, label, value, default);
        self.form.push(check);
        if let Some(v) = value {
            let mut row = Vec::new();
            for (axis, x) in v.iter_mut().enumerate() {
                let shape = Quantity {
                    width: Some(VECTOR_FIELD),
                    ..self.quantity("")
                };
                row.push(
                    self.form
                        .number(&format!("{id}:{axis}"), x, shape)
                        .widget
                        .into(),
                );
            }
            self.form.push(Flow {
                children: row,
                ..Default::default()
            });
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

/// The panel's widgets for `t`, the event applied to it first.
pub(crate) fn walk(
    document: &Document,
    t: &mut Tolerance,
    event: Option<&PanelEvent>,
) -> Vec<Widget> {
    let mut w = Walk {
        form: panel::Form::new(event),
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
    let shape = Quantity {
        min: Some(0.0),
        ..w.quantity("Limit (mm)")
    };
    let mut limit = t.limit;
    let mut bound = w.form.number("limit", &mut limit, shape);
    if bound.set {
        t.limit = limit.max(0.0);
        bound.widget.value = t.limit;
    }
    w.form.put(bound);
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
    w.form.finish()
}
