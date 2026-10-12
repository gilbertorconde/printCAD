//! Arrays of linked copies: a body repeated along a line or around an
//! axis, each repeat a linked copy the array owns. The array keeps the
//! copies in step with it: a new count adds or takes away copies, a new
//! spacing places them again, and moving the source carries them along.
//! Every edit of an array is one op (`SetCopyArray`) carrying the whole
//! array, its copies' ids resolved, so the op is its own inverse's shape
//! and replays exactly.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Body, BodyId, BodyPlacement, Document, op};

/// Unique identifier for an array in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct ArrayId(pub Uuid);

impl ArrayId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ArrayId {
    fn default() -> Self {
        Self::new()
    }
}

/// Where an array's occurrences go, in the document's axes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArrayPattern {
    /// Along `direction`, `spacing` millimetres apart.
    Linear { direction: [f32; 3], spacing: f32 },
    /// Turned about the axis through `center` along `axis`, `angle`
    /// degrees between neighbours.
    Polar {
        center: [f32; 3],
        axis: [f32; 3],
        angle: f32,
    },
}

impl ArrayPattern {
    /// The motion, in the document's axes, that takes the source to
    /// occurrence `index` (0 is the source itself).
    pub fn motion(&self, index: u32) -> BodyPlacement {
        let k = index as f32;
        match *self {
            Self::Linear { direction, spacing } => {
                let d = Vec3::from_array(direction).normalize_or_zero();
                BodyPlacement::new(Quat::IDENTITY, d * spacing * k)
            }
            Self::Polar {
                center,
                axis,
                angle,
            } => {
                let a = Vec3::from_array(axis).normalize_or_zero();
                if a == Vec3::ZERO {
                    return BodyPlacement::IDENTITY;
                }
                let turn = Quat::from_axis_angle(a, (angle * k).to_radians());
                let c = Vec3::from_array(center);
                BodyPlacement::new(turn, c - turn * c)
            }
        }
    }

    /// Whether it says where anything goes: a direction or axis that is
    /// not zero.
    pub fn is_valid(&self) -> bool {
        let v = match self {
            Self::Linear { direction, .. } => direction,
            Self::Polar { axis, .. } => axis,
        };
        let v = Vec3::from_array(*v);
        v.is_finite() && v.length() > 1e-9
    }
}

/// One copy an array owns: which occurrence it is and its body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArrayInstance {
    /// Its place in the array, from 1 (0 is the source).
    pub index: u32,
    pub body: BodyId,
    /// The name its body is made with.
    pub name: String,
}

/// A body repeated: the source, and a linked copy at each further
/// occurrence the array does not skip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopyArray {
    pub id: ArrayId,
    pub name: String,
    pub source: BodyId,
    pub pattern: ArrayPattern,
    /// Occurrences, the source counted: 3 is the source and two copies.
    pub count: u32,
    /// Occurrences left out, each from 1 below `count`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<u32>,
    /// The copies, one per occurrence neither the source nor skipped, in
    /// order.
    pub instances: Vec<ArrayInstance>,
    /// When the copies' bodies are made.
    pub created_at: i64,
}

impl CopyArray {
    /// The instance whose body is `body`.
    pub fn instance_of(&self, body: BodyId) -> Option<&ArrayInstance> {
        self.instances.iter().find(|i| i.body == body)
    }
}

/// The most occurrences an array holds.
pub const MAX_ARRAY_COUNT: u32 = 1000;

/// Why an array edit was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ArrayError {
    #[error("is not a body of this document")]
    NoSuchBody,
    #[error("is not an array of this document")]
    NoSuchArray,
    #[error("is a copy an array owns: make the array of its source")]
    SourceIsInstance,
    #[error("must be from 1 to {MAX_ARRAY_COUNT}")]
    BadCount,
    #[error("has no direction: a direction or axis of zero length")]
    BadPattern,
}

impl Document {
    /// Every array, in the order they were made.
    pub fn copy_arrays(&self) -> &[CopyArray] {
        &self.copy_arrays
    }

    pub fn copy_array(&self, id: ArrayId) -> Option<&CopyArray> {
        self.copy_arrays.iter().find(|a| a.id == id)
    }

    /// The array that owns `body` as one of its copies, and its instance.
    pub fn array_instance(&self, body: BodyId) -> Option<(&CopyArray, &ArrayInstance)> {
        self.copy_arrays
            .iter()
            .find_map(|a| Some((a, a.instance_of(body)?)))
    }

    /// The arrays that repeat `body`.
    pub fn arrays_of_source(&self, body: BodyId) -> Vec<ArrayId> {
        self.copy_arrays
            .iter()
            .filter(|a| a.source == body)
            .map(|a| a.id)
            .collect()
    }

    /// A spacing that leaves 10 mm between neighbours along `direction`:
    /// the body's extent that way, plus 10.
    pub fn spacing_clear_of(&self, body: BodyId, direction: [f32; 3]) -> f32 {
        let d = Vec3::from_array(direction).normalize_or_zero().abs();
        let extent = self
            .imported_geometry(body)
            .and_then(|g| g.bounds_mm.or_else(|| g.mesh.bounds()))
            .map_or(0.0, |(lo, hi)| {
                (Vec3::from_array(hi) - Vec3::from_array(lo)).dot(d)
            });
        extent + 10.0
    }

    /// Repeat `source` `count` times (itself counted) by `pattern`, each
    /// repeat a linked copy the array owns.
    pub fn create_copy_array(
        &mut self,
        source: BodyId,
        pattern: ArrayPattern,
        count: u32,
    ) -> Result<ArrayId, ArrayError> {
        let entry = self
            .bodies
            .iter()
            .find(|b| b.id == source)
            .ok_or(ArrayError::NoSuchBody)?;
        if self.array_instance(source).is_some() {
            return Err(ArrayError::SourceIsInstance);
        }
        let base = entry.name.clone();
        let names: Vec<&str> = self.copy_arrays.iter().map(|a| a.name.as_str()).collect();
        let name = crate::next_indexed_name(&format!("{base} array"), names.into_iter());
        let id = ArrayId::new();
        let array = CopyArray {
            id,
            name,
            source,
            pattern,
            count: 1,
            skipped: Vec::new(),
            instances: Vec::new(),
            created_at: crate::epoch_ms_now(),
        };
        let array = self.resolve_array(array, pattern, count, Vec::new())?;
        self.record_and_apply(op::DocumentOp::SetCopyArray {
            id,
            array: Some(array),
        });
        Ok(id)
    }

    /// Change an array's pattern, count and skipped occurrences: copies
    /// are made for new occurrences, those past the count or skipped are
    /// taken away, and every one is placed again. One op.
    pub fn set_copy_array(
        &mut self,
        id: ArrayId,
        pattern: ArrayPattern,
        count: u32,
        skipped: Vec<u32>,
    ) -> Result<(), ArrayError> {
        let current = self.copy_array(id).ok_or(ArrayError::NoSuchArray)?.clone();
        let array = self.resolve_array(current.clone(), pattern, count, skipped)?;
        if array != current {
            self.record_and_apply(op::DocumentOp::SetCopyArray {
                id,
                array: Some(array),
            });
        }
        Ok(())
    }

    /// Put an array back as `array` was, its copies' bodies with it, or
    /// take it away with its copies (`None`). What a task's Cancel uses.
    pub fn restore_copy_array(&mut self, id: ArrayId, array: Option<CopyArray>) {
        if self.copy_array(id) != array.as_ref() {
            self.record_and_apply(op::DocumentOp::SetCopyArray { id, array });
        }
    }

    /// Take an array away with every copy it owns; the source stays.
    pub fn remove_copy_array(&mut self, id: ArrayId) -> bool {
        if self.copy_array(id).is_none() {
            return false;
        }
        self.record_and_apply(op::DocumentOp::SetCopyArray { id, array: None });
        true
    }

    /// Skip the occurrence the copy `body` stands at: the copy goes, and
    /// `set_copy_array` with a shorter skip list makes it again.
    pub fn skip_array_instance(&mut self, body: BodyId) -> bool {
        let Some((array, instance)) = self.array_instance(body) else {
            return false;
        };
        let (id, pattern, count, index) = (array.id, array.pattern, array.count, instance.index);
        let mut skipped = array.skipped.clone();
        skipped.push(index);
        self.set_copy_array(id, pattern, count, skipped).is_ok()
    }

    /// `array` with the new values, a copy for each occurrence that needs
    /// one: the ones it has kept, new ids and names for the rest.
    fn resolve_array(
        &self,
        mut array: CopyArray,
        pattern: ArrayPattern,
        count: u32,
        mut skipped: Vec<u32>,
    ) -> Result<CopyArray, ArrayError> {
        if !(1..=MAX_ARRAY_COUNT).contains(&count) {
            return Err(ArrayError::BadCount);
        }
        if !pattern.is_valid() {
            return Err(ArrayError::BadPattern);
        }
        skipped.retain(|i| (1..count).contains(i));
        skipped.sort_unstable();
        skipped.dedup();
        let base = self
            .bodies
            .iter()
            .find(|b| b.id == array.source)
            .map(|b| b.name.clone())
            .ok_or(ArrayError::NoSuchBody)?;
        let mut names: Vec<String> = self.bodies.iter().map(|b| b.name.clone()).collect();
        let mut instances = Vec::new();
        for index in 1..count {
            if skipped.contains(&index) {
                continue;
            }
            if let Some(kept) = array.instances.iter().find(|i| i.index == index) {
                instances.push(kept.clone());
                continue;
            }
            let name = crate::next_indexed_name(&base, names.iter().map(String::as_str));
            names.push(name.clone());
            instances.push(ArrayInstance {
                index,
                body: BodyId::new(),
                name,
            });
        }
        array.pattern = pattern;
        array.count = count;
        array.skipped = skipped;
        array.instances = instances;
        Ok(array)
    }

    /// Apply `SetCopyArray`: copies the array no longer has go, the ones
    /// it gains are made, and every one is placed.
    pub(crate) fn apply_set_copy_array(&mut self, id: ArrayId, array: Option<&CopyArray>) {
        let at = self.copy_arrays.iter().position(|a| a.id == id);
        let old = at.map(|i| self.copy_arrays.remove(i));
        let kept: Vec<BodyId> = array
            .map(|a| a.instances.iter().map(|i| i.body).collect())
            .unwrap_or_default();
        for instance in old.iter().flat_map(|a| &a.instances) {
            if !kept.contains(&instance.body) {
                self.drop_body(instance.body);
            }
        }
        let Some(array) = array else {
            return;
        };
        if !self.bodies.iter().any(|b| b.id == array.source) {
            return;
        }
        let at = at
            .unwrap_or(self.copy_arrays.len())
            .min(self.copy_arrays.len());
        self.copy_arrays.insert(at, array.clone());
        let (root, mirror) = self
            .bodies
            .iter()
            .find(|b| b.id == array.source)
            .map(|b| (b.copy_of.unwrap_or(b.id), b.mirror))
            .expect("the source is there");
        for instance in &array.instances {
            if self.bodies.iter().any(|b| b.id == instance.body) {
                continue;
            }
            let mut body = Body::new(instance.body, instance.name.clone(), array.created_at);
            body.copy_of = Some(root);
            body.mirror = mirror;
            self.bodies.push(body);
            self.refresh_copy(instance.body);
        }
        self.place_array(id);
    }

    /// Every copy of the array placed where its occurrence is, from where
    /// the source sits.
    pub(crate) fn place_array(&mut self, id: ArrayId) {
        let Some(array) = self.copy_array(id) else {
            return;
        };
        let source = self.body_placement(array.source);
        let places: Vec<(BodyId, BodyPlacement)> = array
            .instances
            .iter()
            .map(|i| (i.body, array.pattern.motion(i.index).after(&source)))
            .collect();
        for (body, placement) in places {
            if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == body) {
                entry.placement = placement;
            }
            self.place_geometry(body);
        }
    }

    /// What taking `body` away does to the arrays: an array of it goes
    /// with its copies, and an array that owned it skips its occurrence.
    pub(crate) fn arrays_lose_body(&mut self, body: BodyId) {
        let gone: Vec<CopyArray> = self
            .copy_arrays
            .iter()
            .filter(|a| a.source == body)
            .cloned()
            .collect();
        self.copy_arrays.retain(|a| a.source != body);
        for array in gone {
            for instance in array.instances {
                self.drop_body(instance.body);
            }
        }
        for array in &mut self.copy_arrays {
            if let Some(at) = array.instances.iter().position(|i| i.body == body) {
                let instance = array.instances.remove(at);
                array.skipped.push(instance.index);
                array.skipped.sort_unstable();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
    }

    #[test]
    fn a_linear_step_moves_along_the_direction_by_the_spacing() {
        let pattern = ArrayPattern::Linear {
            direction: [0.0, 2.0, 0.0],
            spacing: 15.0,
        };
        assert!(pattern.motion(0).is_identity());
        assert!(close(
            pattern.motion(3).point([1.0, 0.0, 0.0]),
            [1.0, 45.0, 0.0]
        ));
    }

    #[test]
    fn a_polar_step_turns_about_the_axis_through_its_center() {
        let pattern = ArrayPattern::Polar {
            center: [10.0, 0.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            angle: 90.0,
        };
        let moved = pattern.motion(1);
        assert!(close(moved.point([20.0, 0.0, 0.0]), [10.0, 10.0, 0.0]));
        assert!(close(moved.point([10.0, 0.0, 5.0]), [10.0, 0.0, 5.0]));
        assert!(close(
            pattern.motion(2).point([20.0, 0.0, 0.0]),
            [0.0, 0.0, 0.0]
        ));
    }

    #[test]
    fn a_zero_direction_is_refused() {
        let pattern = ArrayPattern::Linear {
            direction: [0.0; 3],
            spacing: 1.0,
        };
        assert!(!pattern.is_valid());
    }
}
