//! What one frame's hooks read of the edited sketch, worked out once and
//! shared by every hook that asks while nothing it was worked out from has
//! changed: the sketch read from the document, the overlays and the
//! constraint glyphs.
//!
//! A memo holds while the tab state's stamp, the document, its edit count
//! and whether its formulas are worked out stay; the overlays and glyphs
//! also need the same settings and the same view. Any mutable borrow of
//! the tab state takes a fresh stamp ([`Stamped`]), and `on_frame` takes
//! one every frame, so a memo never outlives the frame it was made in:
//! what the document derives between frames (a solved formula, a
//! projection followed) is read afresh.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use core_document::WorkbenchRuntimeContext;

use crate::{SketchFeature, SketchOptions, glyphs::Glyph, overlay::Overlays};

static STAMPS: AtomicU64 = AtomicU64::new(1);

fn fresh_stamp() -> u64 {
    STAMPS.fetch_add(1, Ordering::Relaxed)
}

/// A value whose every mutable borrow takes a stamp no other borrow has
/// had, so what was worked out from it can tell whether it may have
/// changed since.
pub(crate) struct Stamped<T> {
    value: T,
    stamp: u64,
}

impl<T: Default> Default for Stamped<T> {
    fn default() -> Self {
        Self {
            value: T::default(),
            stamp: fresh_stamp(),
        }
    }
}

impl<T> From<T> for Stamped<T> {
    fn from(value: T) -> Self {
        Self {
            value,
            stamp: fresh_stamp(),
        }
    }
}

impl<T> Stamped<T> {
    pub(crate) fn stamp(&self) -> u64 {
        self.stamp
    }
}

impl<T> std::ops::Deref for Stamped<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> std::ops::DerefMut for Stamped<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.stamp = fresh_stamp();
        &mut self.value
    }
}

/// What the sketch read depends on.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct SketchKey {
    tab: u64,
    document: usize,
    seq: u64,
    evaluated: bool,
}

impl SketchKey {
    pub(crate) fn of(tab: u64, ctx: &WorkbenchRuntimeContext) -> Self {
        Self {
            tab,
            document: std::ptr::from_ref(&*ctx.document) as usize,
            seq: ctx.document.mutation_seq(),
            evaluated: !ctx.document.needs_evaluation(),
        }
    }
}

/// What the overlays and glyphs depend on beyond the sketch: the settings
/// and everything of the view and the pointer a hook's context carries.
#[derive(Clone, PartialEq)]
pub(crate) struct DrawKey {
    sketch: SketchKey,
    options: SketchOptions,
    camera: ([f32; 3], [f32; 3]),
    viewport: (u32, u32, u32, u32),
    pixels_per_point: f32,
    view_proj: Option<[[f32; 4]; 4]>,
    side_pane: Option<core_document::SidePaneView>,
    cursor: Option<(f32, f32)>,
    keys: (bool, bool, bool),
    palette: core_document::SketchPalette,
    kernel: Option<usize>,
    active: Option<core_document::FeatureId>,
    active_plane: Option<kernel_api::ProfilePlane>,
    selected_body: Option<uuid::Uuid>,
    selected_face: Option<core_document::FaceRef>,
    selected_edges: Vec<core_document::EdgeRef>,
    hovered_point: Option<[f32; 3]>,
    hovered_face: Option<core_document::FaceRef>,
    hovered_edge: Option<core_document::EdgeRef>,
    hovered_vertex: Option<core_document::VertexRef>,
    hovered_base_plane: Option<core_document::BasePlane>,
    undo_entry: Option<core_document::history::JournalEntryId>,
}

impl DrawKey {
    pub(crate) fn of(
        sketch: SketchKey,
        options: SketchOptions,
        ctx: &WorkbenchRuntimeContext,
    ) -> Self {
        Self {
            sketch,
            options,
            camera: (ctx.camera_position, ctx.camera_target),
            viewport: ctx.viewport,
            pixels_per_point: ctx.pixels_per_point,
            view_proj: ctx.view_proj,
            side_pane: ctx.side_pane.clone(),
            cursor: ctx.cursor_viewport_pos,
            keys: (ctx.ctrl_down, ctx.shift_down, ctx.alt_down),
            palette: ctx.sketch_palette,
            kernel: ctx
                .kernel
                .map(|k| std::ptr::from_ref(k).cast::<()>() as usize),
            active: ctx.active_document_object,
            active_plane: ctx.active_object_plane,
            selected_body: ctx.selected_body_id,
            selected_face: ctx.selected_face,
            selected_edges: ctx.selected_edges.clone(),
            hovered_point: ctx.hovered_world_pos,
            hovered_face: ctx.hovered_face,
            hovered_edge: ctx.hovered_edge,
            hovered_vertex: ctx.hovered_vertex,
            hovered_base_plane: ctx.hovered_base_plane,
            undo_entry: ctx.undo_entry,
        }
    }
}

/// One value kept with the key it was worked out under.
struct Slot<K, V>(RefCell<Option<(K, V)>>);

impl<K, V> Default for Slot<K, V> {
    fn default() -> Self {
        Self(RefCell::new(None))
    }
}

impl<K: PartialEq, V: Clone> Slot<K, V> {
    /// The value kept under `key`, or `make`'s, kept in its place.
    fn get(&self, key: K, make: impl FnOnce() -> V) -> V {
        if let Some((kept, value)) = &*self.0.borrow()
            && *kept == key
        {
            return value.clone();
        }
        let value = make();
        *self.0.borrow_mut() = Some((key, value.clone()));
        value
    }

    fn clear(&mut self) {
        *self.0.get_mut() = None;
    }
}

/// The frame's memos.
#[derive(Default)]
pub(crate) struct FrameMemo {
    active: Slot<SketchKey, Option<Arc<SketchFeature>>>,
    displayed: Slot<SketchKey, Option<Arc<SketchFeature>>>,
    overlays: Slot<DrawKey, Arc<Overlays>>,
    glyphs: Slot<DrawKey, Arc<Vec<Glyph>>>,
}

impl FrameMemo {
    pub(crate) fn active(
        &self,
        key: SketchKey,
        read: impl FnOnce() -> Option<SketchFeature>,
    ) -> Option<Arc<SketchFeature>> {
        self.active.get(key, || read().map(Arc::new))
    }

    pub(crate) fn displayed(
        &self,
        key: SketchKey,
        read: impl FnOnce() -> Option<Arc<SketchFeature>>,
    ) -> Option<Arc<SketchFeature>> {
        self.displayed.get(key, read)
    }

    pub(crate) fn overlays(&self, key: DrawKey, build: impl FnOnce() -> Overlays) -> Arc<Overlays> {
        self.overlays.get(key, || Arc::new(build()))
    }

    pub(crate) fn glyphs(
        &self,
        key: DrawKey,
        build: impl FnOnce() -> Vec<Glyph>,
    ) -> Arc<Vec<Glyph>> {
        self.glyphs.get(key, || Arc::new(build()))
    }

    /// Forget everything: the tab state is being moved.
    pub(crate) fn clear(&mut self) {
        self.active.clear();
        self.displayed.clear();
        self.overlays.clear();
        self.glyphs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{Document, EdgeRef, FaceRef, FeatureId, SidePaneView, VertexRef};

    struct NoKernel;
    impl kernel_api::KernelQueries for NoKernel {
        fn project_edge(
            &self,
            _brep: &[u8],
            _near: [f64; 3],
            _plane: &kernel_api::ProfilePlane,
        ) -> kernel_api::KernelResult<kernel_api::ProjectedEdge> {
            unreachable!("a key never asks the kernel")
        }
    }
    static KERNEL: NoKernel = NoKernel;

    type Change<'c> = &'c dyn Fn(&mut WorkbenchRuntimeContext);

    fn key(doc: &mut Document, tab: u64, options: SketchOptions, change: Change) -> DrawKey {
        let mut ctx =
            WorkbenchRuntimeContext::new(doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
        change(&mut ctx);
        DrawKey::of(SketchKey::of(tab, &ctx), options, &ctx)
    }

    /// Everything of the context, the settings, the tab state and the
    /// document the drawing reads keys it apart; the same inputs key it
    /// alike.
    #[test]
    fn every_input_of_the_drawing_keys_it_apart() {
        let mut doc = Document::new("t");
        let options = SketchOptions::default();
        let plain = key(&mut doc, 1, options, &|_| {});
        assert!(key(&mut doc, 1, options, &|_| {}) == plain);
        let face = FaceRef {
            point: [1.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            surface: None,
            name: 7,
        };
        let edge = EdgeRef {
            point: [1.0, 0.0, 0.0],
            direction: [1.0, 0.0, 0.0],
            length_mm: 2.0,
            body: uuid::Uuid::nil(),
            circle: None,
            faces: [1, 2],
        };
        let vertex = VertexRef {
            point: [1.0, 2.0, 3.0],
            body: uuid::Uuid::nil(),
        };
        let pane = SidePaneView {
            bodies: Vec::new(),
            view_proj: [[1.0; 4]; 4],
            offset: (400.0, 0.0),
        };
        let plane = kernel_api::ProfilePlane {
            origin: [1.0, 0.0, 0.0],
            ..Default::default()
        };
        let changes: &[(&str, Change)] = &[
            ("camera", &|c| c.camera_position[0] = 1.0),
            ("target", &|c| c.camera_target[1] = 1.0),
            ("viewport", &|c| c.viewport.2 = 640),
            ("scale", &|c| c.pixels_per_point = 2.0),
            ("view", &|c| c.view_proj = Some([[1.0; 4]; 4])),
            ("side pane", &|c| c.side_pane = Some(pane.clone())),
            ("cursor", &|c| c.cursor_viewport_pos = Some((3.0, 4.0))),
            ("ctrl", &|c| c.ctrl_down = true),
            ("shift", &|c| c.shift_down = true),
            ("alt", &|c| c.alt_down = true),
            ("palette", &|c| c.sketch_palette.geometry = [0.5, 0.25, 0.0]),
            ("kernel", &|c| c.kernel = Some(&KERNEL)),
            ("active", &|c| {
                c.active_document_object = Some(FeatureId(uuid::Uuid::nil()))
            }),
            ("active plane", &|c| c.active_object_plane = Some(plane)),
            ("body", &|c| c.selected_body_id = Some(uuid::Uuid::nil())),
            ("face", &|c| c.selected_face = Some(face)),
            ("edges", &|c| c.selected_edges = vec![edge]),
            ("hovered point", &|c| {
                c.hovered_world_pos = Some([1.0, 2.0, 3.0])
            }),
            ("hovered face", &|c| c.hovered_face = Some(face)),
            ("hovered edge", &|c| c.hovered_edge = Some(edge)),
            ("hovered vertex", &|c| c.hovered_vertex = Some(vertex)),
            ("base plane", &|c| {
                c.hovered_base_plane = Some(core_document::BasePlane::XY)
            }),
        ];
        for (what, change) in changes {
            assert!(key(&mut doc, 1, options, *change) != plain, "{what}");
        }
        let hidden = SketchOptions {
            constraints_hidden: !options.constraints_hidden,
            ..options
        };
        assert!(key(&mut doc, 1, hidden, &|_| {}) != plain, "settings");
        assert!(key(&mut doc, 2, options, &|_| {}) != plain, "tab state");
        assert!(
            key(&mut Document::new("u"), 1, options, &|_| {}) != plain,
            "another document"
        );
        let sketch = crate::sketch::Sketch::new("s");
        let feature = SketchFeature::new(sketch, crate::sketch::SketchPlane::xy());
        doc.add_feature(feature, "s".into()).unwrap();
        assert!(key(&mut doc, 1, options, &|_| {}) != plain, "an edit");
    }

    /// A slot works its value out once for as long as its key holds.
    #[test]
    fn a_slot_keeps_its_value_while_its_key_holds() {
        let slot: Slot<u32, u32> = Slot::default();
        let made = std::cell::Cell::new(0);
        let make = |v: u32| {
            made.set(made.get() + 1);
            v
        };
        assert_eq!(slot.get(1, || make(10)), 10);
        assert_eq!(slot.get(1, || make(11)), 10);
        assert_eq!(made.get(), 1);
        assert_eq!(slot.get(2, || make(12)), 12);
        assert_eq!(made.get(), 2);
    }

    /// Reading never takes a new stamp; a mutable borrow always does.
    #[test]
    fn only_a_mutable_borrow_takes_a_new_stamp() {
        let mut value: Stamped<Vec<u32>> = Stamped::default();
        let first = value.stamp();
        assert!(value.is_empty());
        assert_eq!(value.stamp(), first);
        value.push(1);
        let second = value.stamp();
        assert_ne!(second, first);
        let _ = &mut *value;
        assert_ne!(value.stamp(), second);
        assert_ne!(Stamped::<u32>::default().stamp(), value.stamp());
    }
}
