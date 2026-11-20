//! Journal entries kept compact.
//!
//! A feature payload is a `serde_json::Value` tree, many times the size of
//! its JSON text (about 15x for a large sketch). The journal keeps each
//! payload an op carries as that text, compressed (a fifth of the text
//! again), and builds the tree again only when undo or redo applies the
//! op: a few milliseconds for a sketch of thousands of elements. Every
//! other op is kept as it is.
//!
//! An edited feature's payload is packed on a thread of its own, so
//! closing a step does not wait for it: until its packed form lands the
//! step holds the payload itself, which undo and redo then apply as it
//! is. The packing thread lets go of the payload before its packed form
//! shows, and the step lets go of it as that form lands, so a payload is
//! held by the history no longer than its packing takes.
//!
//! Consecutive steps of one feature share their payloads: one step's
//! forward write is the next one's inverse, the same `Arc`. [`Packer`]
//! knows the packed form of each payload it packed or unpacked by the
//! `Arc`'s identity, held as a `Weak` so the address cannot be reused
//! while it is known, and the tree behind an `Arc` never changes, so a
//! payload is packed once however many steps hold it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use serde_json::Value;

use crate::op::DocumentOp;

/// zstd's fastest level: it adds little to writing the text.
const LEVEL: i32 = 1;

/// Payloads waiting for the packing thread at most: with this many
/// waiting, steps closing faster than it packs (a script's calls) pack
/// their own, so the payloads the history holds stay few.
const WAITING: usize = 2;

/// Payloads handed to the packing thread and not let go of yet.
static PACKING: AtomicUsize = AtomicUsize::new(0);

/// A payload as its compressed JSON text, shared by every step that holds
/// it: the payload itself until that text is written.
#[derive(Debug, Clone)]
pub(super) struct Packed(Arc<Slot>);

#[derive(Debug, Default)]
struct Slot {
    bytes: OnceLock<Box<[u8]>>,
    /// The payload while `bytes` is still to be written; taken out only
    /// once they are.
    held: Mutex<Option<Arc<Value>>>,
}

impl Slot {
    fn held(&self) -> std::sync::MutexGuard<'_, Option<Arc<Value>>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn pack_bytes(value: &Value) -> Box<[u8]> {
    let text = serde_json::to_vec(value).expect("a value writes");
    zstd::bulk::compress(&text, LEVEL)
        .expect("bytes in memory compress")
        .into_boxed_slice()
}

fn unpack_bytes(bytes: &[u8]) -> Value {
    // The bytes are ours, written from a value: they always read back,
    // to the same value (`float_roundtrip` keeps every double).
    let text = zstd::stream::decode_all(bytes).expect("a journal payload unpacks");
    serde_json::from_slice(&text).expect("a journal payload reads back")
}

impl Packed {
    /// `value` packed here and now.
    fn of(value: &Value) -> Self {
        let slot = Slot::default();
        let _ = slot.bytes.set(pack_bytes(value));
        Self(Arc::new(slot))
    }

    /// `value` held until the packing thread has packed it, or packed
    /// here when too many wait for it already.
    fn later(value: &Arc<Value>) -> Self {
        if PACKING.fetch_add(1, Ordering::AcqRel) >= WAITING {
            PACKING.fetch_sub(1, Ordering::AcqRel);
            return Self::of(value);
        }
        let slot = Arc::new(Slot {
            bytes: OnceLock::new(),
            held: Mutex::new(Some(Arc::clone(value))),
        });
        packing_thread()
            .send(Job::Pack(Arc::clone(&slot)))
            .expect("the packing thread runs");
        Self(slot)
    }

    /// The payload: the one held while it is not packed yet, else read
    /// back from its packed form.
    fn value(&self) -> Arc<Value> {
        if let Some(bytes) = self.0.bytes.get() {
            return Arc::new(unpack_bytes(bytes));
        }
        if let Some(value) = self.0.held().clone() {
            return value;
        }
        // Taken out only once the bytes were set.
        Arc::new(unpack_bytes(
            self.0.bytes.get().expect("packed once let go"),
        ))
    }
}

enum Job {
    Pack(Arc<Slot>),
    /// Answered once every job sent before it is done.
    Flush(SyncSender<()>),
}

/// The thread that packs payloads, one for the process, started with the
/// first payload it is handed.
fn packing_thread() -> &'static Sender<Job> {
    static JOBS: OnceLock<Sender<Job>> = OnceLock::new();
    JOBS.get_or_init(|| {
        let (jobs, received) = mpsc::channel();
        std::thread::Builder::new()
            .name("undo-packing".into())
            .spawn(move || pack_all(received))
            .expect("the packing thread starts");
        jobs
    })
}

fn pack_all(jobs: Receiver<Job>) {
    for job in jobs {
        match job {
            Job::Pack(slot) => {
                let value = slot.held().clone();
                if let Some(value) = value {
                    let bytes = pack_bytes(&value);
                    drop(value);
                    let _ = slot.bytes.set(bytes);
                    let held = slot.held().take();
                    drop(held);
                }
                PACKING.fetch_sub(1, Ordering::AcqRel);
            }
            Job::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

/// Wait until every payload handed to the packing thread so far is
/// packed.
pub(super) fn settle() {
    let (done, landed) = mpsc::sync_channel(1);
    packing_thread()
        .send(Job::Flush(done))
        .expect("the packing thread runs");
    landed.recv().expect("the packing thread answers");
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
#[derive(Debug)]
pub(super) struct Packer {
    known: Vec<(Weak<Value>, Packed)>,
    /// Edited payloads are packed on the packing thread, else here.
    background: bool,
}

impl Default for Packer {
    fn default() -> Self {
        Self {
            known: Vec::new(),
            background: true,
        }
    }
}

impl Packer {
    pub(super) fn set_background(&mut self, background: bool) {
        self.background = background;
    }

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
        let packed = if self.background {
            Packed::later(data)
        } else {
            Packed::of(data)
        };
        self.known.push((Arc::downgrade(data), packed.clone()));
        packed
    }

    fn unpacked(&mut self, packed: &Packed) -> Arc<Value> {
        // Still held: known already, by the `Arc` it was packed from.
        if let Some(value) = packed.0.held().clone() {
            return value;
        }
        self.forget_dropped();
        let value = packed.value();
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
                DocumentOp::AddFeature { data, .. } => {
                    *data = Arc::unwrap_or_clone(packed.value());
                }
                _ => {}
            }
        }
        op
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A payload not packed yet is applied as the very value held; once
    /// packed, the packing has let go of it and it reads back equal to
    /// it to the last bit.
    #[test]
    fn a_payload_is_held_until_packed_then_read_back_exactly() {
        let value = Arc::new(serde_json::json!({
            "at": [0.1, -0.0, 1e-300, f64::MAX / 3.0],
            "count": u64::MAX,
            "name": "élément \"quoted\"",
        }));
        let slot = Arc::new(Slot {
            bytes: OnceLock::new(),
            held: Mutex::new(Some(Arc::clone(&value))),
        });
        let packed = Packed(Arc::clone(&slot));
        assert!(Arc::ptr_eq(&packed.value(), &value), "the value held");

        let (jobs, received) = mpsc::channel();
        PACKING.fetch_add(1, Ordering::AcqRel);
        jobs.send(Job::Pack(slot)).expect("send");
        drop(jobs);
        pack_all(received);
        assert_eq!(Arc::strong_count(&value), 1, "let go once packed");
        let back = packed.value();
        assert!(!Arc::ptr_eq(&back, &value), "read back from its bytes");
        assert_eq!(back.to_string(), value.to_string());
    }
}
