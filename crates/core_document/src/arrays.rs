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

use crate::{Body, BodyId, BodyPlacement, ComponentId, Document, FeatureId, op};

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Along a path: occurrence k moved from where `stations[0]` stands to
    /// where `stations[k]` does, turned as the path turns there when
    /// `align`. The stations are read from `along` whenever the array is
    /// made or edited, one per occurrence.
    Path {
        along: PathSource,
        stations: Vec<PathStation>,
        /// Millimetres between neighbours along the path; spread over its
        /// whole length when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spacing: Option<f32>,
        #[serde(default)]
        align: bool,
        /// The normal of the plane the path lies in, when it lies in one:
        /// an aligned copy turns about it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        normal: Option<[f32; 3]>,
    },
    /// At points: occurrence k moved by `points[k] - points[0]`, the
    /// points those `from` lays out, read whenever the array is made or
    /// edited.
    Points {
        from: FeatureId,
        points: Vec<[f32; 3]>,
    },
}

/// What a path array runs along.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PathSource {
    /// Chain `chain` of the curves a feature (a sketch) lays out.
    Sketch {
        feature: FeatureId,
        #[serde(default)]
        chain: usize,
    },
    /// The edge of `body`'s solid nearest `point`, in the body's own frame.
    Edge { body: BodyId, point: [f32; 3] },
}

/// A point on a path and the way it runs there, in the document's axes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathStation {
    pub point: [f32; 3],
    pub tangent: [f32; 3],
}

impl ArrayPattern {
    /// The motion, in the document's axes, that takes the source to
    /// occurrence `index` (0 is the source itself).
    pub fn motion(&self, index: u32) -> BodyPlacement {
        let k = index as f32;
        match self {
            Self::Linear { direction, spacing } => {
                let d = Vec3::from_array(*direction).normalize_or_zero();
                BodyPlacement::new(Quat::IDENTITY, d * *spacing * k)
            }
            Self::Polar {
                center,
                axis,
                angle,
            } => {
                let a = Vec3::from_array(*axis).normalize_or_zero();
                if a == Vec3::ZERO {
                    return BodyPlacement::IDENTITY;
                }
                let turn = Quat::from_axis_angle(a, (angle * k).to_radians());
                let c = Vec3::from_array(*center);
                BodyPlacement::new(turn, c - turn * c)
            }
            Self::Path {
                stations,
                align,
                normal,
                ..
            } => {
                let (Some(first), Some(at)) = (stations.first(), stations.get(index as usize))
                else {
                    return BodyPlacement::IDENTITY;
                };
                let from = Vec3::from_array(first.point);
                let to = Vec3::from_array(at.point);
                let turn = if *align {
                    turn_between(first.tangent, at.tangent, *normal)
                } else {
                    Quat::IDENTITY
                };
                BodyPlacement::new(turn, to - turn * from)
            }
            Self::Points { points, .. } => {
                let (Some(first), Some(at)) = (points.first(), points.get(index as usize)) else {
                    return BodyPlacement::IDENTITY;
                };
                BodyPlacement::new(
                    Quat::IDENTITY,
                    Vec3::from_array(*at) - Vec3::from_array(*first),
                )
            }
        }
    }

    /// Whether it says where anything goes: a direction or axis that is
    /// not zero, a station or a point.
    pub fn is_valid(&self) -> bool {
        let v = match self {
            Self::Linear { direction, .. } => direction,
            Self::Polar { axis, .. } => axis,
            Self::Path { stations, .. } => {
                return !stations.is_empty()
                    && stations.iter().all(|s| {
                        Vec3::from_array(s.point).is_finite()
                            && Vec3::from_array(s.tangent).is_finite()
                    });
            }
            Self::Points { points, .. } => {
                return !points.is_empty()
                    && points.iter().all(|p| Vec3::from_array(*p).is_finite());
            }
        };
        let v = Vec3::from_array(*v);
        v.is_finite() && v.length() > 1e-9
    }

    /// How many occurrences it has places for: as many as its stations or
    /// points, and any number for a row or a ring.
    pub fn places(&self) -> Option<u32> {
        match self {
            Self::Linear { .. } | Self::Polar { .. } => None,
            Self::Path { stations, .. } => Some(stations.len() as u32),
            Self::Points { points, .. } => Some(points.len() as u32),
        }
    }
}

/// The turn taking tangent `from` to `to`: about `normal` by the angle
/// between them as they lie in its plane, when there is one, else the
/// shortest.
fn turn_between(from: [f32; 3], to: [f32; 3], normal: Option<[f32; 3]>) -> Quat {
    let (a, b) = (Vec3::from_array(from), Vec3::from_array(to));
    match normal.map(Vec3::from_array).map(Vec3::normalize_or_zero) {
        Some(n) if n != Vec3::ZERO => {
            let (a, b) = (a - n * a.dot(n), b - n * b.dot(n));
            let angle = n.dot(a.cross(b)).atan2(a.dot(b));
            Quat::from_axis_angle(n, angle)
        }
        _ => Quat::from_rotation_arc(a.normalize_or_zero(), b.normalize_or_zero()),
    }
}

/// Where `count` occurrences stand along a path `length` long, as
/// fractions of it from its start: `spacing` millimetres apart, or spread
/// over the whole of it (round it evenly, when it is `closed`).
pub fn path_stations(
    length: f64,
    closed: bool,
    count: u32,
    spacing: Option<f32>,
) -> Result<Vec<f64>, ArrayError> {
    if !(1..=MAX_ARRAY_COUNT).contains(&count) {
        return Err(ArrayError::BadCount);
    }
    if !(length.is_finite() && length > 0.0) {
        return Err(ArrayError::BadPattern);
    }
    let n = f64::from(count);
    let step = match spacing {
        Some(s) if s.is_finite() && s > 0.0 => f64::from(s) / length,
        Some(_) => return Err(ArrayError::BadPattern),
        None if closed => 1.0 / n,
        None if count == 1 => 0.0,
        None => 1.0 / (n - 1.0),
    };
    // A closed path's last place is short of its start, which the source
    // holds.
    if step > 0.0 {
        let fits = (1.0 / step + 1e-9).floor() as u32 + u32::from(!closed);
        if count > fits {
            return Err(ArrayError::PathTooShort { fits });
        }
    }
    Ok((0..count).map(|k| (step * f64::from(k)).min(1.0)).collect())
}

/// One occurrence an array owns: which it is and the copy of its source,
/// with the copies of the rest of a component it repeats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArrayInstance {
    /// Its place in the array, from 1 (0 is the source).
    pub index: u32,
    /// The copy of the array's source.
    pub body: BodyId,
    /// The name its body is made with.
    pub name: String,
    /// The copies of the component's other bodies, each with the body it
    /// copies; empty for an array of one body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<ArrayMember>,
    /// The rigid component the occurrence's copies sit in, holding them
    /// together, when the array repeats a component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<ComponentId>,
}

/// A copy of one of a repeated component's bodies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArrayMember {
    /// The body of the component it copies.
    pub of: BodyId,
    pub body: BodyId,
    pub name: String,
}

impl ArrayInstance {
    /// Every copy of the occurrence, each with the body it copies: the
    /// source's first.
    pub fn copies(&self, source: BodyId) -> Vec<(BodyId, BodyId, &str)> {
        std::iter::once((source, self.body, self.name.as_str()))
            .chain(self.members.iter().map(|m| (m.of, m.body, m.name.as_str())))
            .collect()
    }

    /// Whether `body` is one of its copies.
    pub fn holds(&self, body: BodyId) -> bool {
        self.body == body || self.members.iter().any(|m| m.body == body)
    }
}

/// A body, or a component's bodies, repeated: the source, and linked
/// copies at each further occurrence the array does not skip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopyArray {
    pub id: ArrayId,
    pub name: String,
    /// The body repeated; for a component, its first body, which the
    /// copies of each occurrence count as theirs.
    pub source: BodyId,
    /// The component repeated, every body in it copied together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<ComponentId>,
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
    /// The instance one of whose copies is `body`.
    pub fn instance_of(&self, body: BodyId) -> Option<&ArrayInstance> {
        self.instances.iter().find(|i| i.holds(body))
    }

    /// Whether it repeats `body`: its source, or a body of the component
    /// it repeats that it has copies of.
    pub fn repeats(&self, body: BodyId) -> bool {
        self.source == body
            || self
                .instances
                .iter()
                .any(|i| i.members.iter().any(|m| m.of == body))
    }

    /// Every body its occurrences hold.
    pub fn copy_bodies(&self) -> impl Iterator<Item = BodyId> + '_ {
        self.instances
            .iter()
            .flat_map(|i| std::iter::once(i.body).chain(i.members.iter().map(|m| m.body)))
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
    #[error("has no direction: a direction or axis of zero length, or no path or points")]
    BadPattern,
    #[error("is more than the path has room for: {fits} fit")]
    PathTooShort { fits: u32 },
    #[error("is more than the {places} places the path or points give")]
    TooFewPlaces { places: u32 },
    #[error("is not a component of this document, or holds no body")]
    EmptyComponent,
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

    /// The arrays that repeat `body`, alone or with its component.
    pub fn arrays_of_source(&self, body: BodyId) -> Vec<ArrayId> {
        self.copy_arrays
            .iter()
            .filter(|a| a.repeats(body))
            .map(|a| a.id)
            .collect()
    }

    /// The array that repeats `component`.
    pub fn array_of_component(&self, component: ComponentId) -> Option<ArrayId> {
        self.copy_arrays
            .iter()
            .find(|a| a.component == Some(component))
            .map(|a| a.id)
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
        self.make_array(source, None, &base, pattern, count)
    }

    /// Repeat every body of `component` together, `count` times (the
    /// component counted) by `pattern`: each occurrence a rigid component
    /// of linked copies, one of each of its bodies.
    pub fn create_component_array(
        &mut self,
        component: ComponentId,
        pattern: ArrayPattern,
        count: u32,
    ) -> Result<ArrayId, ArrayError> {
        let base = self
            .component(component)
            .ok_or(ArrayError::EmptyComponent)?
            .name
            .clone();
        let bodies = self.component_bodies(component);
        let source = *bodies.first().ok_or(ArrayError::EmptyComponent)?;
        if bodies.iter().any(|b| self.array_instance(*b).is_some()) {
            return Err(ArrayError::SourceIsInstance);
        }
        self.make_array(source, Some(component), &base, pattern, count)
    }

    fn make_array(
        &mut self,
        source: BodyId,
        component: Option<ComponentId>,
        base: &str,
        pattern: ArrayPattern,
        count: u32,
    ) -> Result<ArrayId, ArrayError> {
        let names: Vec<&str> = self.copy_arrays.iter().map(|a| a.name.as_str()).collect();
        let name = crate::next_indexed_name(&format!("{base} array"), names.into_iter());
        let id = ArrayId::new();
        let array = CopyArray {
            id,
            name,
            source,
            component,
            pattern: pattern.clone(),
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
        let (id, pattern, count, index) =
            (array.id, array.pattern.clone(), array.count, instance.index);
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
        if let Some(places) = pattern.places()
            && count > places
        {
            return Err(ArrayError::TooFewPlaces { places });
        }
        skipped.retain(|i| (1..count).contains(i));
        skipped.sort_unstable();
        skipped.dedup();
        let name_of = |body: BodyId| {
            self.bodies
                .iter()
                .find(|b| b.id == body)
                .map(|b| b.name.clone())
        };
        let base = name_of(array.source).ok_or(ArrayError::NoSuchBody)?;
        // A component's bodies as they are now, the source first.
        let others: Vec<(BodyId, String)> = match array.component {
            None => Vec::new(),
            Some(component) => {
                let bodies = self.component_bodies(component);
                if !bodies.contains(&array.source) {
                    return Err(ArrayError::EmptyComponent);
                }
                bodies
                    .into_iter()
                    .filter(|b| *b != array.source)
                    .filter_map(|b| Some((b, name_of(b)?)))
                    .collect()
            }
        };
        let mut names: Vec<String> = self.bodies.iter().map(|b| b.name.clone()).collect();
        let mut fresh = |base: &str| {
            let name = crate::next_indexed_name(base, names.iter().map(String::as_str));
            names.push(name.clone());
            name
        };
        let mut instances = Vec::new();
        for index in 1..count {
            if skipped.contains(&index) {
                continue;
            }
            let kept = array.instances.iter().find(|i| i.index == index);
            let (body, name) = match kept {
                Some(kept) => (kept.body, kept.name.clone()),
                None => (BodyId::new(), fresh(&base)),
            };
            let members = others
                .iter()
                .map(|(of, base)| {
                    kept.and_then(|k| k.members.iter().find(|m| m.of == *of))
                        .cloned()
                        .unwrap_or_else(|| ArrayMember {
                            of: *of,
                            body: BodyId::new(),
                            name: fresh(base),
                        })
                })
                .collect();
            let group = array
                .component
                .map(|_| kept.and_then(|k| k.group).unwrap_or_default());
            instances.push(ArrayInstance {
                index,
                body,
                name,
                members,
                group,
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
        let kept: Vec<BodyId> = array.map(|a| a.copy_bodies().collect()).unwrap_or_default();
        let kept_groups: Vec<ComponentId> = array
            .map(|a| a.instances.iter().filter_map(|i| i.group).collect())
            .unwrap_or_default();
        if let Some(old) = &old {
            for body in old.copy_bodies().filter(|b| !kept.contains(b)) {
                self.drop_body(body);
            }
            let gone: Vec<ComponentId> = old
                .instances
                .iter()
                .filter_map(|i| i.group)
                .filter(|g| !kept_groups.contains(g))
                .collect();
            self.components.retain(|c| !gone.contains(&c.id));
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
        // The occurrences' components sit beside the one repeated.
        let parent = array
            .component
            .and_then(|c| self.component(c))
            .and_then(|c| c.parent);
        for instance in &array.instances {
            if let Some(group) = instance.group
                && self.component(group).is_none()
            {
                self.components.push(crate::Component {
                    id: group,
                    name: format!("{} {}", array.name, instance.index),
                    parent,
                    flexible: false,
                });
            }
            for (of, copy, name) in instance.copies(array.source) {
                if self.bodies.iter().any(|b| b.id == copy) {
                    continue;
                }
                let Some((root, mirror)) = self
                    .bodies
                    .iter()
                    .find(|b| b.id == of)
                    .map(|b| (b.copy_of.unwrap_or(b.id), b.mirror))
                else {
                    continue;
                };
                let mut body = Body::new(copy, name.to_string(), array.created_at);
                body.copy_of = Some(root);
                body.mirror = mirror;
                body.component = instance.group;
                self.bodies.push(body);
                self.refresh_copy(copy);
            }
        }
        self.place_array(id);
    }

    /// Every copy of the array placed where its occurrence is, from where
    /// the source sits.
    pub(crate) fn place_array(&mut self, id: ArrayId) {
        let Some(array) = self.copy_array(id) else {
            return;
        };
        let places: Vec<(BodyId, BodyPlacement)> = array
            .instances
            .iter()
            .flat_map(|i| {
                let motion = array.pattern.motion(i.index);
                i.copies(array.source)
                    .into_iter()
                    .map(move |(of, copy, _)| (of, copy, motion))
            })
            .map(|(of, copy, motion)| (copy, motion.after(&self.body_placement(of))))
            .collect();
        for (body, placement) in places {
            if let Some(entry) = self.bodies.iter_mut().find(|b| b.id == body) {
                entry.placement = placement;
            }
            self.place_geometry(body);
        }
    }

    /// What taking `body` away does to the arrays: an array of it goes
    /// with its copies (for a component, the array of its first body), an
    /// array of its component loses its copies of it, and an array that
    /// owned it skips its occurrence, the rest of that occurrence's copies
    /// with it.
    pub(crate) fn arrays_lose_body(&mut self, body: BodyId) {
        let gone: Vec<CopyArray> = self
            .copy_arrays
            .iter()
            .filter(|a| a.source == body)
            .cloned()
            .collect();
        self.copy_arrays.retain(|a| a.source != body);
        let mut drop: Vec<BodyId> = Vec::new();
        let mut groups: Vec<ComponentId> = Vec::new();
        for array in &gone {
            drop.extend(array.copy_bodies());
            groups.extend(array.instances.iter().filter_map(|i| i.group));
        }
        for array in &mut self.copy_arrays {
            if let Some(at) = array.instances.iter().position(|i| i.holds(body)) {
                let instance = array.instances.remove(at);
                drop.extend(instance.copies(array.source).into_iter().map(|c| c.1));
                groups.extend(instance.group);
                array.skipped.push(instance.index);
                array.skipped.sort_unstable();
            }
            for instance in &mut array.instances {
                instance.members.retain(|m| {
                    if m.of == body {
                        drop.push(m.body);
                    }
                    m.of != body
                });
            }
        }
        for copy in drop.into_iter().filter(|b| *b != body) {
            self.drop_body(copy);
        }
        self.components.retain(|c| !groups.contains(&c.id));
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

    fn quarter_turn_path(align: bool) -> ArrayPattern {
        // Round the circle of radius 10 about the origin: (10, 0) running
        // along +Y, then (0, 10) running along -X.
        ArrayPattern::Path {
            along: PathSource::Edge {
                body: BodyId::new(),
                point: [0.0; 3],
            },
            stations: vec![
                PathStation {
                    point: [10.0, 0.0, 0.0],
                    tangent: [0.0, 1.0, 0.0],
                },
                PathStation {
                    point: [0.0, 10.0, 0.0],
                    tangent: [-1.0, 0.0, 0.0],
                },
            ],
            spacing: None,
            align,
            normal: Some([0.0, 0.0, 1.0]),
        }
    }

    #[test]
    fn a_path_step_moves_between_stations_and_turns_with_the_path_when_aligned() {
        let kept = quarter_turn_path(false).motion(1);
        assert!(close(kept.point([11.0, 0.0, 0.0]), [1.0, 10.0, 0.0]));
        let turned = quarter_turn_path(true).motion(1);
        assert!(close(turned.point([10.0, 0.0, 0.0]), [0.0, 10.0, 0.0]));
        assert!(close(turned.point([11.0, 0.0, 0.0]), [0.0, 11.0, 0.0]));
        assert!(quarter_turn_path(true).motion(0).is_identity());
    }

    #[test]
    fn an_opposite_tangent_turns_about_the_paths_normal() {
        let pattern = ArrayPattern::Path {
            along: PathSource::Edge {
                body: BodyId::new(),
                point: [0.0; 3],
            },
            stations: vec![
                PathStation {
                    point: [0.0; 3],
                    tangent: [1.0, 0.0, 0.0],
                },
                PathStation {
                    point: [0.0; 3],
                    tangent: [-1.0, 0.0, 0.0],
                },
            ],
            spacing: None,
            align: true,
            normal: Some([0.0, 0.0, 1.0]),
        };
        let p = pattern.motion(1).point([0.0, 0.0, 5.0]);
        assert!(close(p, [0.0, 0.0, 5.0]), "stays in its plane: {p:?}");
    }

    #[test]
    fn a_point_step_moves_by_the_points_offset_from_the_first() {
        let pattern = ArrayPattern::Points {
            from: FeatureId(Uuid::new_v4()),
            points: vec![[1.0, 1.0, 0.0], [4.0, 5.0, 0.0]],
        };
        assert!(close(pattern.motion(1).point([0.0; 3]), [3.0, 4.0, 0.0]));
        assert_eq!(pattern.places(), Some(2));
    }

    #[test]
    fn stations_spread_over_an_open_path_reach_both_ends_and_round_a_closed_one_stop_short() {
        let open = path_stations(30.0, false, 4, None).unwrap();
        assert_eq!(open.len(), 4);
        assert!((open[3] - 1.0).abs() < 1e-12 && (open[1] - 1.0 / 3.0).abs() < 1e-12);
        let ring = path_stations(30.0, true, 4, None).unwrap();
        assert!((ring[3] - 0.75).abs() < 1e-12);
        let spaced = path_stations(30.0, false, 3, Some(10.0)).unwrap();
        assert!((spaced[2] - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(
            path_stations(30.0, false, 5, Some(10.0)),
            Err(ArrayError::PathTooShort { fits: 4 })
        );
        assert_eq!(
            path_stations(30.0, true, 4, Some(10.0)),
            Err(ArrayError::PathTooShort { fits: 3 })
        );
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
