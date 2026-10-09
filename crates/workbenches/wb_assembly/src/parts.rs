//! The parts list: every body, identical ones counted together, with the
//! size of its box in its own frame (what a print bed has to hold).
//!
//! What the list says of each part beyond that (its item number, whether
//! it is bought rather than made, the values of the columns added to the
//! list) is kept in the document as one body-less feature, the parts
//! table, by the id of a body of the part.

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use core_document::{
    BodyId, ComponentId, Document, DocumentResult, FeatureError, FeatureId, WorkbenchFeature,
    WorkbenchId,
};
use serde::{Deserialize, Serialize};

/// The feature kind the parts table is stored as.
pub const PARTS_KIND: &str = "wb.assembly.parts";

/// What the parts list keeps for its parts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PartsTable {
    /// The columns added to the list, in order.
    pub columns: Vec<String>,
    /// Each part's entry, by the id of one of its bodies.
    pub entries: BTreeMap<String, PartEntry>,
    /// Listed by component: each component's parts under it, nested as
    /// the components are.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub by_component: bool,
    /// What the parts are printed in: the filament's mass of a part
    /// whose body has no material of its own is at its density.
    pub material: PrintMaterial,
}

/// A printing material: its name and density, g/cm³.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintMaterial {
    pub name: String,
    pub density: f32,
}

impl Default for PrintMaterial {
    fn default() -> Self {
        Self {
            name: PRINT_MATERIALS[0].0.into(),
            density: PRINT_MATERIALS[0].1,
        }
    }
}

/// The filaments offered for the parts list, densities in g/cm³.
pub const PRINT_MATERIALS: &[(&str, f32)] = &[
    ("PLA", 1.24),
    ("PETG", 1.27),
    ("ABS", 1.04),
    ("ASA", 1.07),
    ("TPU", 1.21),
    ("Nylon", 1.14),
    ("PC", 1.20),
];

impl PrintMaterial {
    /// The offered material of `name`, or one of its own at `density`.
    pub fn named(name: &str, density: Option<f32>) -> Self {
        let preset = PRINT_MATERIALS
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name));
        match (preset, density) {
            (Some((n, d)), None) => Self {
                name: (*n).into(),
                density: *d,
            },
            (_, density) => Self {
                name: name.into(),
                density: density.unwrap_or(1.0),
            },
        }
    }
}

/// What the list keeps for one part.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PartEntry {
    /// Its item number; 0 before the list is numbered.
    pub number: u32,
    /// Bought rather than made: left out of exports and the slicer.
    pub bought: bool,
    /// Made, though a bench declares its kind bought
    /// (`WorkbenchDescriptor::bought_kinds`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub made: bool,
    /// Its value in each added column, by column.
    pub values: BTreeMap<String, String>,
    /// How many of it to print, when set; otherwise as many as the
    /// assembly holds, none of a bought part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub print: Option<u32>,
}

impl WorkbenchFeature for PartsTable {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(PARTS_KIND)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            core_document::DocumentError::Feature(FeatureError::Deserialization(e.to_string()))
        })
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        "Parts list"
    }
}

/// The document's parts table and its feature, when it has one.
pub fn table_of(document: &Document) -> Option<(FeatureId, PartsTable)> {
    document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == PARTS_KIND)
        .min_by_key(|(id, n)| (n.seq, **id))
        .and_then(|(id, n)| Some((*id, PartsTable::from_json(&n.data).ok()?)))
}

/// Write the parts table, adding its feature the first time.
pub fn store_table(document: &mut Document, table: &PartsTable) -> DocumentResult<FeatureId> {
    match table_of(document) {
        Some((id, _)) => {
            document.update_feature_data(id, table.to_json())?;
            document.clear_feature_dirty(id);
            Ok(id)
        }
        None => {
            let id = document.add_feature_in_body(table.clone(), "Parts list".into(), None)?;
            document.clear_feature_dirty(id);
            Ok(id)
        }
    }
}

impl PartsTable {
    /// The entry of the part `bodies` make, any of them keyed.
    pub fn entry(&self, bodies: &[BodyId]) -> Option<&PartEntry> {
        bodies
            .iter()
            .find_map(|b| self.entries.get(&b.0.to_string()))
    }

    /// The entry of the part `bodies` make, made under its first body
    /// when it has none.
    pub fn entry_mut(&mut self, bodies: &[BodyId]) -> &mut PartEntry {
        let key = bodies
            .iter()
            .map(|b| b.0.to_string())
            .find(|k| self.entries.contains_key(k))
            .or_else(|| bodies.first().map(|b| b.0.to_string()))
            .unwrap_or_default();
        self.entries.entry(key).or_default()
    }

    /// Mark the part `part` stands for bought or made.
    pub fn set_bought(&mut self, part: &Part, bought: bool) {
        let entry = self.entry_mut(&part.bodies);
        entry.bought = bought;
        entry.made = !bought && part.bought_kind;
    }

    /// Number every part that has no number, after the highest in use, in
    /// the order given.
    pub fn number(&mut self, parts: &[Part]) {
        let mut next = self.entries.values().map(|e| e.number).max().unwrap_or(0);
        for part in parts {
            let entry = self.entry_mut(&part.bodies);
            if entry.number == 0 {
                next += 1;
                entry.number = next;
            }
        }
    }
}

/// One part: the bodies that are the same shape, and its size.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub name: String,
    pub bodies: Vec<BodyId>,
    /// Its box along its own X, Y and Z, in millimetres; `None` for a body
    /// with no geometry yet.
    pub size_mm: Option<[f32; 3]>,
    /// A mesh body rather than a solid.
    pub mesh: bool,
    /// Its item number, once the list is numbered.
    pub number: Option<u32>,
    /// Bought rather than made.
    pub bought: bool,
    /// Of a kind a bench declares bought, so bought unless the table says
    /// it is made.
    pub bought_kind: bool,
    /// Its values in the list's added columns.
    pub values: BTreeMap<String, String>,
    /// How many of it to print: the count set in the table, else one per
    /// body, none when bought.
    pub print: u32,
    /// The density its filament mass is reckoned at, g/cm³: its body's
    /// material's, else the list's.
    pub density: f32,
    /// The volume of one piece, mm³, once measured.
    pub volume_mm3: Option<f64>,
}

impl Part {
    /// The mass of one piece, grams, once its volume is measured.
    pub fn mass_g(&self) -> Option<f64> {
        self.volume_mm3
            .map(|v| v * f64::from(self.density) / 1000.0)
    }
}

/// The bodies of bought parts: what an export of the model, or the
/// slicer, leaves out. `bought_kinds` are the feature kinds benches
/// declare bought.
pub fn bought_bodies(document: &Document, bought_kinds: &[WorkbenchId]) -> Vec<BodyId> {
    parts_list(document, bought_kinds)
        .into_iter()
        .filter(|p| p.bought)
        .flat_map(|p| p.bodies)
        .collect()
}

/// Every body, bodies of the same shape as one part, in name order. A
/// body with a feature of one of `bought_kinds` is a bought part unless
/// the table says it is made.
pub fn parts_list(document: &Document, bought_kinds: &[WorkbenchId]) -> Vec<Part> {
    let of_bought_kind: std::collections::HashSet<BodyId> = if bought_kinds.is_empty() {
        Default::default()
    } else {
        document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| bought_kinds.contains(&n.workbench_id))
            .filter_map(|(_, n)| n.body)
            .collect()
    };
    let mut parts: Vec<(u64, Part)> = Vec::new();
    for body in document.bodies() {
        let local = document.local_geometry(body.id);
        let size_mm = local
            .as_ref()
            .and_then(|(mesh, bounds)| bounds.or_else(|| mesh.bounds()))
            .map(|(lo, hi)| [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]);
        // The same shape is the same snapshot, or failing one the same
        // triangles; a body with neither is a part of its own.
        let mut hasher = DefaultHasher::new();
        match (document.imported_brep_blob(body.id), &local) {
            (Some(blob), _) => blob.hash(&mut hasher),
            (None, Some((mesh, _))) if !mesh.positions.is_empty() => {
                for p in &mesh.positions {
                    p.map(f32::to_bits).hash(&mut hasher);
                }
            }
            _ => body.id.hash(&mut hasher),
        }
        let key = hasher.finish();
        match parts.iter_mut().find(|(k, _)| *k == key) {
            Some((_, part)) => {
                part.bodies.push(body.id);
                part.bought_kind |= of_bought_kind.contains(&body.id);
            }
            None => parts.push((
                key,
                Part {
                    name: body.name.clone(),
                    bodies: vec![body.id],
                    size_mm,
                    mesh: document.is_mesh_body(body.id),
                    number: None,
                    bought: false,
                    bought_kind: of_bought_kind.contains(&body.id),
                    values: BTreeMap::new(),
                    print: 0,
                    density: body.material.as_ref().map_or(0.0, |m| m.density),
                    volume_mm3: None,
                },
            )),
        }
    }
    let table = table_of(document).map(|(_, t)| t).unwrap_or_default();
    let mut out: Vec<Part> = parts
        .into_iter()
        .map(|(_, mut p)| {
            p.bought = p.bought_kind;
            let mut print = None;
            if let Some(entry) = table.entry(&p.bodies) {
                p.number = (entry.number > 0).then_some(entry.number);
                p.bought = entry.bought || (p.bought_kind && !entry.made);
                p.values = entry.values.clone();
                print = entry.print;
            }
            p.print = print.unwrap_or(if p.bought { 0 } else { p.bodies.len() as u32 });
            if p.density <= 0.0 {
                p.density = table.material.density;
            }
            p
        })
        .collect();
    // Numbered parts by number, then the rest by name.
    out.sort_by_key(|p| (p.number.unwrap_or(u32::MAX), p.name.to_lowercase()));
    out
}

/// The volume a closed mesh encloses, mm³: the signed volumes of the
/// tetrahedra its triangles make with the origin, summed.
pub fn mesh_volume_mm3(mesh: &kernel_api::TriMesh) -> f64 {
    let p = |i: u32| mesh.positions.get(i as usize).map(|v| v.map(f64::from));
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|t| {
            let (a, b, c) = (p(t[0])?, p(t[1])?, p(t[2])?);
            Some(
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0,
            )
        })
        .sum::<f64>()
}

/// What a part's volume is measured from: its solid, or its mesh's
/// volume already worked out.
enum Source {
    Solid(std::sync::Arc<Vec<u8>>),
    Known(Option<f64>),
}

/// A measured volume: the body measured, at which revision of its
/// geometry, and the volume, `None` when it encloses none.
type Measured = (BodyId, u64, Option<f64>);

/// The parts' volumes, each measured once per revision of its geometry:
/// a solid by the kernel (away from the window for the panel), a mesh
/// body from its triangles.
#[derive(Default)]
pub struct Volumes {
    known: std::collections::HashMap<BodyId, (u64, Option<f64>)>,
    running: Option<std::sync::mpsc::Receiver<Vec<Measured>>>,
}

impl std::fmt::Debug for Volumes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Volumes")
            .field("known", &self.known.len())
            .field("running", &self.running.is_some())
            .finish()
    }
}

impl Volumes {
    /// Each part's volume, measured from its first body, where it is
    /// known at the geometry the body has now.
    pub fn fill(&self, document: &Document, parts: &mut [Part]) {
        for part in parts {
            part.volume_mm3 = part.bodies.first().and_then(|b| {
                let revision = document.imported_geometry(*b)?.revision;
                self.known
                    .get(b)
                    .filter(|(r, _)| *r == revision)
                    .and_then(|(_, v)| *v)
            });
        }
    }

    /// Whether a measuring is running.
    pub fn running(&self) -> bool {
        self.running.is_some()
    }

    /// What is still to be measured, with where to measure it from.
    fn wanted(&self, document: &Document, parts: &[Part]) -> Vec<(BodyId, u64, Source)> {
        parts
            .iter()
            .filter_map(|part| {
                let body = *part.bodies.first()?;
                let geometry = document.imported_geometry(body)?;
                if self
                    .known
                    .get(&body)
                    .is_some_and(|(r, _)| *r == geometry.revision)
                {
                    return None;
                }
                let source = match document.imported_brep_blob_arc(body) {
                    Some(blob) => Source::Solid(blob),
                    None => Source::Known(
                        document
                            .local_geometry(body)
                            .map(|(mesh, _)| mesh_volume_mm3(&mesh))
                            .filter(|v| *v > 0.0),
                    ),
                };
                Some((body, geometry.revision, source))
            })
            .collect()
    }

    /// Measure what is not yet known, here and now.
    pub fn measure_now(
        &mut self,
        document: &Document,
        parts: &[Part],
        kernel: Option<&dyn kernel_api::KernelQueries>,
    ) {
        for (body, revision, source) in self.wanted(document, parts) {
            let volume = measure(source, kernel);
            self.known.insert(body, (revision, volume));
        }
    }

    /// Take a finished measuring's volumes, and start one on its own
    /// thread for what is still unknown. True while one runs.
    pub fn refresh(
        &mut self,
        document: &Document,
        parts: &[Part],
        kernel: Option<&'static dyn kernel_api::KernelQueries>,
    ) -> bool {
        if let Some(answer) = &self.running {
            match answer.try_recv() {
                Ok(measured) => {
                    for (body, revision, volume) in measured {
                        self.known.insert(body, (revision, volume));
                    }
                    self.running = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.running = None,
            }
        }
        let wanted = self.wanted(document, parts);
        if wanted.is_empty() {
            return false;
        }
        let (send, answer) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("printcad-part-volumes".into())
            .spawn(move || {
                let measured = wanted
                    .into_iter()
                    .map(|(body, revision, source)| (body, revision, measure(source, kernel)))
                    .collect();
                let _ = send.send(measured);
            });
        if spawned.is_ok() {
            self.running = Some(answer);
        }
        self.running.is_some()
    }
}

/// A part's volume from its source; `None` when it encloses none or
/// there is no kernel to measure a solid with.
fn measure(source: Source, kernel: Option<&dyn kernel_api::KernelQueries>) -> Option<f64> {
    match source {
        Source::Known(volume) => volume,
        Source::Solid(blob) => kernel?.measure(&blob).ok()?.volume_mm3.filter(|v| *v > 0.0),
    }
}

/// A row of the list by component.
#[derive(Debug, Clone, PartialEq)]
pub enum LevelRow {
    /// A component, `depth` components down.
    Component {
        depth: usize,
        id: ComponentId,
        name: String,
    },
    /// Bodies of `parts[part]` sitting directly in the component above.
    Part {
        depth: usize,
        part: usize,
        bodies: Vec<BodyId>,
    },
}

/// `parts` (the whole list) by component: the top's components, each
/// followed by what it holds, then the top's own parts; in a component,
/// its components before its parts, parts in list order.
pub fn parts_by_component(document: &Document, parts: &[Part]) -> Vec<LevelRow> {
    fn level(
        document: &Document,
        parts: &[Part],
        at: Option<ComponentId>,
        depth: usize,
        out: &mut Vec<LevelRow>,
    ) {
        for component in document.components() {
            let parent = component
                .parent
                .filter(|p| document.component(*p).is_some());
            if parent == at && depth < 64 {
                out.push(LevelRow::Component {
                    depth,
                    id: component.id,
                    name: component.name.clone(),
                });
                level(document, parts, Some(component.id), depth + 1, out);
            }
        }
        for (i, part) in parts.iter().enumerate() {
            let bodies: Vec<BodyId> = part
                .bodies
                .iter()
                .copied()
                .filter(|b| document.component_of(*b) == at)
                .collect();
            if !bodies.is_empty() {
                out.push(LevelRow::Part {
                    depth,
                    part: i,
                    bodies,
                });
            }
        }
    }
    let mut out = Vec::new();
    level(document, parts, None, 0, &mut out);
    out
}

/// A value as a CSV field: quoted when it holds a comma or a quote.
fn field(text: &str) -> String {
    if text.contains([',', '"', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// The list as comma-separated values, a header line first: the item
/// number, part, quantity, size, kind, whether bought, then the added
/// `columns`.
pub fn parts_csv(parts: &[Part], columns: &[String]) -> String {
    let mut out = String::from(
        "Item,Part,Quantity,Size X (mm),Size Y (mm),Size Z (mm),Kind,Bought,Print,\
         Volume (cm³),Mass (g)",
    );
    for column in columns {
        out.push(',');
        out.push_str(&field(column));
    }
    out.push('\n');
    for part in parts {
        let size = part.size_mm.map_or_else(
            || ",,".to_string(),
            |s| format!("{:.2},{:.2},{:.2}", s[0], s[1], s[2]),
        );
        let kind = if part.mesh { "mesh" } else { "solid" };
        let number = part.number.map_or(String::new(), |n| n.to_string());
        let bought = if part.bought { "yes" } else { "" };
        let volume = part
            .volume_mm3
            .map_or(String::new(), |v| format!("{:.2}", v / 1000.0));
        let mass = part.mass_g().map_or(String::new(), |m| format!("{m:.2}"));
        out.push_str(&format!(
            "{number},{},{},{size},{kind},{bought},{},{volume},{mass}",
            field(&part.name),
            part.bodies.len(),
            part.print,
        ));
        for column in columns {
            out.push(',');
            out.push_str(&field(part.values.get(column).map_or("", String::as_str)));
        }
        out.push('\n');
    }
    out
}

/// The list by component as comma-separated values: a level column (0
/// at the top) before `parts_csv`'s, a component a row of its own.
pub fn levels_csv(parts: &[Part], rows: &[LevelRow], columns: &[String]) -> String {
    let flat = parts_csv(&[], columns);
    let mut out = format!("Level,{flat}");
    for row in rows {
        match row {
            LevelRow::Component { depth, name, .. } => {
                out.push_str(&format!("{depth},,{},1,,,,component,,,,", field(name)));
                out.push_str(&",".repeat(columns.len()));
                out.push('\n');
            }
            LevelRow::Part {
                depth,
                part,
                bodies,
            } => {
                let one = Part {
                    bodies: bodies.clone(),
                    ..parts[*part].clone()
                };
                let line = parts_csv(&[one], columns);
                let line = line.lines().nth(1).unwrap_or_default();
                out.push_str(&format!("{depth},{line}\n"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_by_component_nests_its_parts() {
        let mut document = Document::new("t");
        let [bolt, bolt2, lid, base] =
            ["Bolt", "Bolt 2", "Lid", "Base"].map(|n| document.create_body(Some(n.into())));
        for (body, shape) in [
            (bolt, "bolt"),
            (bolt2, "bolt"),
            (lid, "lid"),
            (base, "base"),
        ] {
            document.set_imported_brep_data(body, shape.as_bytes().to_vec(), Vec::new());
        }
        let top = document.create_component("Top".into(), None).unwrap();
        document.set_body_component(bolt, Some(top)).unwrap();
        document.set_body_component(lid, Some(top)).unwrap();
        let parts = parts_list(&document, &[]);
        let rows = parts_by_component(&document, &parts);
        let names: Vec<(usize, String, usize)> = rows
            .iter()
            .map(|r| match r {
                LevelRow::Component { depth, name, .. } => (*depth, name.clone(), 1),
                LevelRow::Part {
                    depth,
                    part,
                    bodies,
                } => (*depth, parts[*part].name.clone(), bodies.len()),
            })
            .collect();
        assert_eq!(
            names,
            [
                (0, "Top".to_string(), 1),
                (1, "Bolt".to_string(), 1),
                (1, "Lid".to_string(), 1),
                (0, "Base".to_string(), 1),
                (0, "Bolt".to_string(), 1),
            ]
        );
        let csv = levels_csv(&parts, &rows, &[]);
        let lines: Vec<&str> = csv.lines().collect();
        assert!(lines[0].starts_with("Level,Item,Part"));
        assert_eq!(lines[1], "0,,Top,1,,,,component,,,,");
        assert_eq!(
            lines[2], "1,,Bolt,1,,,,solid,,2,,",
            "the whole part's count to print"
        );
    }

    #[test]
    fn bodies_of_one_shape_are_one_part_counted() {
        let mut document = Document::new("t");
        let a = document.create_body(Some("Bolt".into()));
        let b = document.create_body(Some("Bolt 2".into()));
        let c = document.create_body(Some("Bracket, left".into()));
        document.set_imported_brep_data(a, b"bolt".to_vec(), Vec::new());
        document.set_imported_brep_data(b, b"bolt".to_vec(), Vec::new());
        document.set_imported_brep_data(c, b"bracket".to_vec(), Vec::new());
        let parts = parts_list(&document, &[]);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "Bolt");
        assert_eq!(parts[0].bodies, [a, b]);
        let csv = parts_csv(&parts, &[]);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[1], ",Bolt,2,,,,solid,,2,,");
        assert_eq!(lines[2], ",\"Bracket, left\",1,,,,solid,,1,,");
    }

    /// The table numbers the parts, keeps each part's bought mark and
    /// column values in the document, and the list reads them back in
    /// number order.
    #[test]
    fn the_parts_table_is_kept_in_the_document() {
        let mut document = Document::new("t");
        let a = document.create_body(Some("Zeta".into()));
        let b = document.create_body(Some("Alpha".into()));
        document.set_imported_brep_data(a, b"z".to_vec(), Vec::new());
        document.set_imported_brep_data(b, b"a".to_vec(), Vec::new());
        let mut table = PartsTable {
            columns: vec!["Supplier".into()],
            ..PartsTable::default()
        };
        let parts = parts_list(&document, &[]);
        table.number(&parts);
        table.entry_mut(&[a]).bought = true;
        table
            .entry_mut(&[a])
            .values
            .insert("Supplier".into(), "ACME, Inc".into());
        store_table(&mut document, &table).unwrap();
        let parts = parts_list(&document, &[]);
        assert_eq!(parts[0].name, "Alpha", "numbered first by name");
        assert_eq!(parts[0].number, Some(1));
        assert_eq!(parts[1].number, Some(2));
        assert!(parts[1].bought);
        assert_eq!(bought_bodies(&document, &[]), [a]);
        let csv = parts_csv(&parts, &table.columns);
        assert!(csv.starts_with("Item,Part,Quantity"), "{csv}");
        assert!(
            csv.lines()
                .nth(2)
                .unwrap()
                .ends_with(",yes,0,,,\"ACME, Inc\""),
            "{csv}"
        );
        // Stored once: a second store updates the same feature.
        let (id, _) = table_of(&document).unwrap();
        assert_eq!(store_table(&mut document, &table).unwrap(), id);
    }

    /// A body of a kind a bench declares bought is a bought part until
    /// the table says it is made, and bought again once it says so.
    #[test]
    fn a_bought_kind_is_bought_until_marked_made() {
        let mut document = Document::new("t");
        let screw = document.create_body(Some("Screw".into()));
        let bracket = document.create_body(Some("Bracket".into()));
        document.set_imported_brep_data(screw, b"screw".to_vec(), Vec::new());
        document.set_imported_brep_data(bracket, b"bracket".to_vec(), Vec::new());
        let screw_kind = WorkbenchId::from("acme.hardware.screw");
        document.add_feature_of_kind(
            screw_kind.clone(),
            "Screw".into(),
            Some(screw),
            Vec::new(),
            serde_json::json!({}),
            core_document::FeatureOrigin::default(),
        );
        let kinds = [screw_kind];
        assert_eq!(bought_bodies(&document, &kinds), [screw]);
        assert!(bought_bodies(&document, &[]).is_empty());

        let parts = parts_list(&document, &kinds);
        let part = parts.iter().find(|p| p.bodies == [screw]).unwrap();
        let mut table = PartsTable::default();
        table.set_bought(part, false);
        store_table(&mut document, &table).unwrap();
        assert!(bought_bodies(&document, &kinds).is_empty());

        table.set_bought(part, true);
        store_table(&mut document, &table).unwrap();
        assert_eq!(bought_bodies(&document, &kinds), [screw]);
    }

    /// A unit cube's triangles, outward.
    fn cube(size: f32) -> kernel_api::TriMesh {
        let corners: Vec<[f32; 3]> = (0..8)
            .map(|i| {
                [
                    (i & 1) as f32 * size,
                    ((i >> 1) & 1) as f32 * size,
                    ((i >> 2) & 1) as f32 * size,
                ]
            })
            .collect();
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        kernel_api::TriMesh {
            positions: corners,
            indices: quads
                .iter()
                .flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]])
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_closed_mesh_encloses_its_volume_wherever_it_sits() {
        let mut mesh = cube(10.0);
        assert!((mesh_volume_mm3(&mesh) - 1000.0).abs() < 1e-6);
        for p in &mut mesh.positions {
            p[0] += 250.0;
            p[2] -= 40.0;
        }
        assert!((mesh_volume_mm3(&mesh) - 1000.0).abs() < 1e-3);
    }

    /// A mesh body is measured from its triangles, weighed at the list's
    /// material unless its body has one, and printed once per body until
    /// a count is set.
    #[test]
    fn a_part_is_weighed_at_its_material_and_printed_as_counted() {
        let mut document = Document::new("t");
        let a = document.create_body(Some("Cube".into()));
        let b = document.create_body(Some("Steel cube".into()));
        for (body, size) in [(a, 10.0), (b, 20.0)] {
            document.set_imported_geometry(
                body,
                core_document::ImportedGeometry {
                    mesh: std::sync::Arc::new(cube(size)),
                    source_asset: None,
                    revision: 0,
                    bounds_mm: None,
                    brep_blob_path: None,
                    mesh_path: None,
                    face_colors_path: None,
                    health: None,
                },
            );
        }
        document.set_body_material(
            b,
            Some(core_document::Material {
                name: "Steel".into(),
                density: 7.85,
            }),
        );
        let table = PartsTable {
            material: PrintMaterial::named("petg", None),
            ..PartsTable::default()
        };
        store_table(&mut document, &table).unwrap();
        let mut parts = parts_list(&document, &[]);
        let mut volumes = Volumes::default();
        volumes.measure_now(&document, &parts, None);
        volumes.fill(&document, &mut parts);
        let cube = parts.iter().find(|p| p.bodies == [a]).unwrap();
        assert!((cube.volume_mm3.unwrap() - 1000.0).abs() < 1e-6);
        assert!(
            (cube.mass_g().unwrap() - 1.27).abs() < 1e-6,
            "1 cm³ of PETG"
        );
        let steel = parts.iter().find(|p| p.bodies == [b]).unwrap();
        assert!((steel.mass_g().unwrap() - 8.0 * 7.85).abs() < 1e-4);
        assert_eq!(cube.print, 1);

        let mut table = table_of(&document).unwrap().1;
        table.entry_mut(&[a]).print = Some(4);
        store_table(&mut document, &table).unwrap();
        let csv = parts_csv(&parts_list(&document, &[]), &[]);
        assert!(
            csv.lines()
                .any(|l| l.starts_with(",Cube,1,") && l.contains(",4,")),
            "{csv}"
        );
    }
}
