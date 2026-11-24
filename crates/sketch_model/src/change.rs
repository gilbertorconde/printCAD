//! What an edit changed in a sketch, element by element, and the patch
//! that writes only that into the sketch's stored data.
//!
//! [`SketchChange::between`] compares two sketches' elements and
//! constraints by id: what was added, taken away and changed, and whether
//! the ones both hold kept their order. [`SketchFeature::patch_over`]
//! turns it into a [`DataPatch`] for the document.

use core_document::patch::{DataPatch, PathStep};
use serde_json::Value;
use uuid::Uuid;

use crate::feature::SketchFeature;
use crate::sketch::{GeometryElement, IdMap, Sketch};

/// How one list (a sketch's elements, or its constraints) changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListChange {
    /// Ids only the later list holds, in its order.
    pub added: Vec<Uuid>,
    /// Ids only the earlier list held, in its order.
    pub removed: Vec<Uuid>,
    /// Ids both hold whose item differs, in the later list's order.
    pub changed: Vec<Uuid>,
    /// The ids both hold stand in another order in the later list.
    pub reordered: bool,
    /// An id stands twice in one of the lists, so items cannot be told
    /// apart by id: the rest of the change is not to be trusted.
    pub repeated: bool,
    /// Where the changed items, then the added ones, stand in the later
    /// list, when the earlier list's items all kept their places.
    at: Option<Vec<usize>>,
}

/// Id lists found free of repeats lately, each with its ids as a set, so
/// a list holding one of them in order, then ids added after, is checked
/// by those alone.
struct Unique {
    ids: Vec<Uuid>,
    set: IdMap<()>,
}

/// The [`Unique`] lists kept: a sketch's elements and constraints, and
/// another sketch's.
const UNIQUE: usize = 4;

thread_local! {
    static KNOWN_UNIQUE: std::cell::RefCell<Vec<Unique>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Whether an id stands twice in `before` followed by `added`.
fn repeats(before: impl ExactSizeIterator<Item = Uuid> + Clone, added: &[Uuid]) -> bool {
    KNOWN_UNIQUE.with_borrow_mut(|known| {
        let n = before.len();
        let at = (known.iter())
            .position(|k| k.ids.len() == n && k.ids.iter().copied().eq(before.clone()));
        let mut unique = match at {
            Some(at) => known.remove(at),
            None => {
                let mut unique = Unique {
                    ids: Vec::with_capacity(n + added.len()),
                    set: IdMap::default(),
                };
                unique.set.reserve(n + added.len());
                for key in before {
                    if unique.set.insert(key, ()).is_some() {
                        return true;
                    }
                    unique.ids.push(key);
                }
                unique
            }
        };
        for &key in added {
            if unique.set.insert(key, ()).is_some() {
                return true;
            }
            unique.ids.push(key);
        }
        known.insert(0, unique);
        known.truncate(UNIQUE);
        false
    })
}

impl ListChange {
    fn between<T: PartialEq>(before: &[T], after: &[T], id: impl Fn(&T) -> Uuid) -> Self {
        let mut change = Self::default();
        // Most edits keep every item where it stood and add after them.
        let mut at = Vec::new();
        let kept = after.len() >= before.len()
            && (before.iter().zip(after).enumerate()).all(|(k, (a, b))| {
                if a == b {
                    return true;
                }
                at.push(k);
                id(a) == id(b)
            });
        if kept {
            change.changed = at.iter().map(|&k| id(&after[k])).collect();
            let tail = &after[before.len()..];
            change.added = tail.iter().map(&id).collect();
            change.repeated = repeats(before.iter().map(&id), &change.added);
            at.extend(before.len()..after.len());
            change.at = Some(at);
            return change;
        }
        let mut was: IdMap<usize> = IdMap::default();
        was.reserve(before.len());
        for (at, item) in before.iter().enumerate() {
            change.repeated |= was.insert(id(item), at).is_some();
        }
        let mut last = None;
        let mut seen: IdMap<()> = IdMap::default();
        seen.reserve(after.len());
        for item in after {
            let key = id(item);
            change.repeated |= seen.insert(key, ()).is_some();
            match was.get(&key) {
                None => change.added.push(key),
                Some(&at) => {
                    if before[at] != *item {
                        change.changed.push(key);
                    }
                    if last.is_some_and(|last| at < last) {
                        change.reordered = true;
                    }
                    last = Some(at);
                }
            }
        }
        change.removed = before
            .iter()
            .map(&id)
            .filter(|key| !seen.contains_key(key))
            .collect();
        change
    }

    /// Nothing added, taken away, changed or moved.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && !self.reordered
    }
}

/// How a sketch's elements and constraints changed between two versions
/// of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SketchChange {
    pub geometry: ListChange,
    pub constraints: ListChange,
}

impl SketchChange {
    pub fn between(before: &Sketch, after: &Sketch) -> Self {
        Self {
            geometry: ListChange::between(&before.geometry, &after.geometry, GeometryElement::id),
            constraints: ListChange::between(&before.constraints, &after.constraints, |c| c.id),
        }
    }
}

/// The lists [`SketchFeature::patch_over`] writes item by item, by their
/// key in the stored sketch.
const LISTS: [&str; 2] = ["geometry", "constraints"];

impl SketchFeature {
    /// The patch that turns `stored`, the data `before` was read from,
    /// into what [`core_document::WorkbenchFeature::to_json`] writes of
    /// this sketch: only the elements and constraints that changed, and
    /// the other fields that differ. `None` when a patch cannot say it:
    /// a sketch with text (its letters are not stored as they are read),
    /// one whose ids repeat, or data that is not what `before` reads from.
    pub fn patch_over(&mut self, before: &SketchFeature, stored: &Value) -> Option<DataPatch> {
        let plain = |f: &SketchFeature| f.sketch.texts.is_empty() && f.sketch.derived.is_empty();
        if !plain(self) || !plain(before) {
            return None;
        }
        let stored_sketch = stored.get("sketch")?.as_object()?;
        let sizes = [
            before.sketch.geometry.len(),
            before.sketch.constraints.len(),
        ];
        for (key, size) in LISTS.into_iter().zip(sizes) {
            if stored_sketch.get(key)?.as_array()?.len() != size {
                return None;
            }
        }
        let change = SketchChange::between(&before.sketch, &self.sketch);
        if change.geometry.repeated || change.constraints.repeated {
            return None;
        }

        // Everything but the two lists, written as it is and compared
        // with what is stored.
        let geometry = std::mem::take(&mut self.sketch.geometry);
        let constraints = std::mem::take(&mut self.sketch.constraints);
        let rest = serde_json::to_value(&*self);
        self.sketch.geometry = geometry;
        self.sketch.constraints = constraints;
        let rest = rest.ok()?;

        let mut patch = DataPatch::new();
        let fields = rest.as_object()?;
        let stored_fields = stored.as_object()?;
        write_fields(&mut patch, &[], fields, stored_fields, &["sketch"]);
        let sketch_fields = fields.get("sketch")?.as_object()?;
        let sketch_path = [PathStep::key("sketch")];
        write_fields(
            &mut patch,
            &sketch_path,
            sketch_fields,
            stored_sketch,
            &LISTS,
        );

        let geometry_path = vec![PathStep::key("sketch"), PathStep::key("geometry")];
        write_list(
            &mut patch,
            geometry_path,
            &change.geometry,
            &self.sketch.geometry,
            GeometryElement::id,
        )?;
        let constraints_path = vec![PathStep::key("sketch"), PathStep::key("constraints")];
        write_list(
            &mut patch,
            constraints_path,
            &change.constraints,
            &self.sketch.constraints,
            |c| c.id,
        )?;
        Some(patch)
    }
}

/// Set each of `fields` that `stored` does not hold as it is, and take
/// away what `stored` holds beyond them, `skip` left alone.
fn write_fields(
    patch: &mut DataPatch,
    at: &[PathStep],
    fields: &serde_json::Map<String, Value>,
    stored: &serde_json::Map<String, Value>,
    skip: &[&str],
) {
    let path = |key: &str| {
        let mut path = at.to_vec();
        path.push(PathStep::key(key));
        path
    };
    for (key, value) in fields {
        if !skip.contains(&key.as_str()) && stored.get(key) != Some(value) {
            patch.set(path(key), value.clone());
        }
    }
    for key in stored.keys() {
        if !skip.contains(&key.as_str()) && !fields.contains_key(key) {
            patch.remove(path(key));
        }
    }
}

/// The writes that make the stored list at `list` hold `items`, as
/// `change` says they differ from it: the whole list when the items it
/// kept were moved.
fn write_list<T: serde::Serialize>(
    patch: &mut DataPatch,
    list: Vec<PathStep>,
    change: &ListChange,
    items: &[T],
    id: impl Fn(&T) -> Uuid,
) -> Option<()> {
    if change.reordered {
        patch.set(list, serde_json::to_value(items).ok()?);
        return Some(());
    }
    let item = |key: Uuid| {
        let mut path = list.clone();
        path.push(PathStep::item(key));
        path
    };
    for key in &change.removed {
        patch.remove(item(*key));
    }
    if change.changed.is_empty() && change.added.is_empty() {
        return Some(());
    }
    if let Some(at) = &change.at {
        let added = at.len() - change.added.len();
        for (n, &k) in at.iter().enumerate() {
            let written = serde_json::to_value(&items[k]).ok()?;
            if n >= added {
                let previous = k.checked_sub(1).map(|p| id(&items[p]).to_string());
                patch.insert(list.clone(), previous, written);
            } else {
                patch.set(item(id(&items[k])), written);
            }
        }
        return Some(());
    }
    let wanted: IdMap<()> = change
        .changed
        .iter()
        .chain(&change.added)
        .map(|key| (*key, ()))
        .collect();
    let added: IdMap<()> = change.added.iter().map(|key| (*key, ())).collect();
    // Each added item goes in after the one before it in `items`, which
    // is either kept or added before it: the kept ones are in order.
    let mut previous: Option<Uuid> = None;
    for value in items {
        let key = id(value);
        if wanted.contains_key(&key) {
            let written = serde_json::to_value(value).ok()?;
            if added.contains_key(&key) {
                patch.insert(list.clone(), previous.map(|p| p.to_string()), written);
            } else {
                patch.set(item(key), written);
            }
        }
        previous = Some(key);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Constraint, ConstraintKind, Point, Vec2D};
    use core_document::WorkbenchFeature;

    fn point(sketch: &mut Sketch, x: f64) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, 0.0))))
    }

    fn feature() -> SketchFeature {
        let mut sketch = Sketch::new("s");
        for x in 0..6 {
            point(&mut sketch, f64::from(x));
        }
        let first = sketch.geometry[0].id();
        sketch
            .constraints
            .push(Constraint::new(ConstraintKind::FixedPoint {
                point: first,
                position: Vec2D::new(0.0, 0.0),
            }));
        SketchFeature::from_sketch(sketch)
    }

    /// Applied to the stored data, the patch an edit makes is what
    /// writing the edited sketch whole stores, for edits of every kind:
    /// elements added at the end and in the middle, taken away, changed,
    /// constraints added and removed, other fields set, and the kept
    /// elements moved about.
    #[test]
    fn a_patch_stores_what_writing_the_whole_sketch_stores() {
        type Edit = fn(&mut SketchFeature);
        let edits: Vec<(&str, Edit)> = vec![
            ("append", |f| {
                point(&mut f.sketch, 9.0);
            }),
            ("insert", |f| {
                let extra = point(&mut f.sketch, 7.0);
                let made = f.sketch.geometry.pop().expect("made");
                assert_eq!(made.id(), extra);
                f.sketch.geometry.insert(0, made);
                let other = point(&mut f.sketch, 8.0);
                let made = f.sketch.geometry.pop().expect("made");
                assert_eq!(made.id(), other);
                f.sketch.geometry.insert(3, made);
            }),
            ("remove", |f| {
                let gone = f.sketch.geometry[2].id();
                f.sketch.geometry.retain(|g| g.id() != gone);
                f.sketch.geometry.remove(0);
                f.sketch.constraints.clear();
            }),
            ("change", |f| {
                if let GeometryElement::Point(p) = &mut f.sketch.geometry[4] {
                    p.position.x = 42.0;
                }
                f.sketch.name = "renamed".into();
                f.section_view = true;
                f.sketch.is_fully_constrained = !f.sketch.is_fully_constrained;
            }),
            ("change and append", |f| {
                if let GeometryElement::Point(p) = &mut f.sketch.geometry[2] {
                    p.position.y = -3.0;
                }
                point(&mut f.sketch, 11.0);
                point(&mut f.sketch, 12.0);
            }),
            ("reorder", |f| f.sketch.geometry.swap(1, 4)),
            ("constraint", |f| {
                let a = f.sketch.geometry[1].id();
                f.sketch
                    .constraints
                    .push(Constraint::new(ConstraintKind::FixedPoint {
                        point: a,
                        position: Vec2D::new(1.0, 0.0),
                    }));
                f.sketch.constraints.swap(0, 1);
            }),
            ("nothing", |_| {}),
        ];
        for (name, edit) in edits {
            let before = feature();
            let mut stored = before.to_json();
            let mut after = before.clone();
            edit(&mut after);
            let patch = after.patch_over(&before, &stored).expect(name);
            if name == "nothing" {
                assert!(patch.is_empty(), "nothing written");
            }
            if name == "append" {
                assert_eq!(patch.len(), 1, "one insert");
            }
            patch.apply(&mut stored).expect(name);
            assert_eq!(stored, after.to_json(), "{name}");
        }
    }

    /// What changed, by id, as a solve that wants the touched parts would
    /// read it.
    #[test]
    fn a_change_names_what_was_added_removed_and_changed() {
        let before = feature();
        let mut after = before.clone();
        let gone = after.sketch.geometry.remove(1).id();
        let added = point(&mut after.sketch, 5.5);
        let moved = after.sketch.geometry[2].id();
        if let GeometryElement::Point(p) = &mut after.sketch.geometry[2] {
            p.position.y = 1.0;
        }
        let change = SketchChange::between(&before.sketch, &after.sketch);
        assert_eq!(change.geometry.added, [added]);
        assert_eq!(change.geometry.removed, [gone]);
        assert_eq!(change.geometry.changed, [moved]);
        assert!(!change.geometry.reordered);
        assert!(change.constraints.is_empty());
    }

    /// An id standing twice is caught on a list already found free of
    /// repeats, where only what was added is looked at: an element added
    /// under an id the sketch holds; and on a list holding one twice, each
    /// time it is compared.
    #[test]
    fn a_repeated_id_is_caught_on_a_list_seen_before() {
        let before = feature();
        let mut after = before.clone();
        point(&mut after.sketch, 9.0);
        let change = SketchChange::between(&before.sketch, &after.sketch);
        assert!(!change.geometry.repeated);
        let mut again = after.clone();
        let copy = again.sketch.geometry[0].clone();
        again.sketch.geometry.push(copy);
        let change = SketchChange::between(&after.sketch, &again.sketch);
        assert!(change.geometry.repeated);
        assert!(again.patch_over(&after, &after.to_json()).is_none());

        let mut twice = before.clone();
        let copy = twice.sketch.geometry[1].clone();
        twice.sketch.geometry.push(copy);
        let mut more = twice.clone();
        point(&mut more.sketch, 3.0);
        for _ in 0..2 {
            assert!(
                SketchChange::between(&twice.sketch, &more.sketch)
                    .geometry
                    .repeated
            );
        }
    }

    /// Data that is not what the earlier sketch reads from is not patched.
    #[test]
    fn stored_data_of_another_shape_is_not_patched() {
        let before = feature();
        let mut stored = before.to_json();
        stored["sketch"]["geometry"]
            .as_array_mut()
            .expect("list")
            .pop();
        let mut after = before.clone();
        point(&mut after.sketch, 1.0);
        assert!(after.patch_over(&before, &stored).is_none());
    }
}
