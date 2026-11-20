//! Journal entries kept compact.
//!
//! A feature payload is a `serde_json::Value` tree, many times the size of
//! its JSON text (about 15x for a large sketch). The journal keeps each
//! payload an op carries as that text, compressed (a fifth of the text
//! again), and builds the tree again only when undo or redo applies the
//! op: a few milliseconds for a sketch of thousands of elements. Every
//! other op is kept as it is.
//!
//! Consecutive steps of one feature share their payloads: one step's
//! forward write is the next one's inverse, the same `Arc`. [`Packer`]
//! knows the packed form of each payload it packed or unpacked by the
//! `Arc`'s identity, held as a `Weak` so the address cannot be reused
//! while it is known, and the tree behind an `Arc` never changes, so a
//! payload is packed once however many steps hold it.

use std::sync::{Arc, Weak};

use serde_json::Value;

use crate::op::DocumentOp;

/// zstd's fastest level: it adds little to writing the text.
const LEVEL: i32 = 1;

/// A payload as its compressed JSON text, shared by every step that holds
/// it.
#[derive(Debug, Clone)]
pub(super) struct Packed(Arc<[u8]>);

impl Packed {
    fn of(value: &Value) -> Self {
        let text = serde_json::to_vec(value).expect("a value writes");
        let packed = zstd::bulk::compress(&text, LEVEL).expect("bytes in memory compress");
        Self(Arc::from(packed))
    }

    fn value(&self) -> Value {
        // The bytes are ours, written from a value: they always read
        // back, to the same value (`float_roundtrip` keeps every double).
        let text = zstd::stream::decode_all(&*self.0).expect("a journal payload unpacks");
        serde_json::from_slice(&text).expect("a journal payload reads back")
    }
}

/// One op of a journal entry: the op with its payload taken out (`Null`
/// in its place), and that payload packed.
#[derive(Debug, Clone)]
pub(super) struct Kept {
    op: DocumentOp,
    payload: Option<Packed>,
}

/// The packed form of the payloads alive somewhere (the document, the outbox),
/// by identity: what packs a payload already packed without writing it
/// again.
#[derive(Debug, Default)]
pub(super) struct Packer {
    known: Vec<(Weak<Value>, Packed)>,
}

impl Packer {
    /// Forget the payloads no one holds any more.
    fn forget_dropped(&mut self) {
        self.known.retain(|(value, _)| value.strong_count() > 0);
    }

    fn packed(&mut self, data: &Arc<Value>) -> Packed {
        self.forget_dropped();
        if let Some((_, packed)) =
            (self.known.iter()).find(|(value, _)| std::ptr::eq(value.as_ptr(), Arc::as_ptr(data)))
        {
            return packed.clone();
        }
        let packed = Packed::of(data);
        self.known.push((Arc::downgrade(data), packed.clone()));
        packed
    }

    fn unpacked(&mut self, packed: &Packed) -> Arc<Value> {
        self.forget_dropped();
        let value = Arc::new(packed.value());
        self.known.push((Arc::downgrade(&value), packed.clone()));
        value
    }

    pub(super) fn pack(&mut self, mut op: DocumentOp) -> Kept {
        let payload = match &mut op {
            DocumentOp::UpdateFeatureData { data, .. } => {
                let packed = self.packed(data);
                *data = Arc::new(Value::Null);
                Some(packed)
            }
            DocumentOp::AddFeature { data, .. } => Some(Packed::of(&std::mem::take(data))),
            _ => None,
        };
        Kept { op, payload }
    }

    pub(super) fn unpack(&mut self, kept: &Kept) -> DocumentOp {
        let mut op = kept.op.clone();
        if let Some(packed) = &kept.payload {
            match &mut op {
                DocumentOp::UpdateFeatureData { data, .. } => *data = self.unpacked(packed),
                DocumentOp::AddFeature { data, .. } => *data = packed.value(),
                _ => {}
            }
        }
        op
    }
}
