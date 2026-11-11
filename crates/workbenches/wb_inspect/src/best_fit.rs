//! The best fit the toolbar runs away from the window: one at a time per
//! document, kept with what it was fitted from, and applied only while
//! the document still holds those inputs.
//!
//! The job lives with the tab's session (the bench suspends and resumes
//! it), so its answer lands only in the document that asked. A newer
//! request supersedes it and a cancel abandons it: the kernel call cannot
//! be stopped, so its thread runs to its end and fills a slot nothing
//! reads any more.

use std::sync::{Arc, Mutex};

use core_document::{BodyId, BodyPlacement, Document};
use kernel_api::{Registration, TriMesh};

/// Where the scan goes and how well it fitted, or why it could not.
pub(crate) type Answer = Result<(BodyPlacement, Registration), String>;

/// What a fit was computed from: both bodies' geometry in their own
/// frames (the reference's shape too, when it has one) and where they
/// sit. The answer places the scan from all of it. Geometry is told
/// apart by the shared data itself, held so it cannot be freed and its
/// address reused: a placement re-places the scene's mesh but keeps the
/// body's own, so undoing a move leaves the key as it was.
#[derive(Clone)]
pub(crate) struct FitKey {
    scan: Option<Arc<TriMesh>>,
    reference: Option<Arc<TriMesh>>,
    reference_shape: Option<Arc<Vec<u8>>>,
    scan_at: BodyPlacement,
    reference_at: BodyPlacement,
}

impl FitKey {
    pub fn of(document: &Document, scan: BodyId, reference: BodyId) -> Self {
        let mesh = |b| document.local_geometry(b).map(|(mesh, _)| mesh);
        Self {
            scan: mesh(scan),
            reference: mesh(reference),
            reference_shape: document.imported_brep_blob_arc(reference),
            scan_at: document.body_placement(scan),
            reference_at: document.body_placement(reference),
        }
    }
}

impl PartialEq for FitKey {
    fn eq(&self, other: &Self) -> bool {
        fn same<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        }
        same(&self.scan, &other.scan)
            && same(&self.reference, &other.reference)
            && same(&self.reference_shape, &other.reference_shape)
            && self.scan_at == other.scan_at
            && self.reference_at == other.reference_at
    }
}

/// Where a job's thread leaves its answer.
#[derive(Clone, Default)]
pub(crate) struct Slot(Arc<Mutex<Option<Answer>>>);

impl Slot {
    pub fn fill(&self, answer: Answer) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(answer);
        }
    }

    fn take(&self) -> Option<Answer> {
        self.0.lock().ok()?.take()
    }
}

/// A best fit asked for and not yet applied.
pub(crate) struct Job {
    pub scan: BodyId,
    pub reference: BodyId,
    pub search: bool,
    key: FitKey,
    slot: Slot,
}

/// What a finished job comes to.
pub(crate) enum Landed {
    /// The answer, fitted from what the document holds now.
    Fresh(Box<(Job, Answer)>),
    /// The scan or reference changed or moved while it ran.
    Stale,
}

/// The best fit running in a document, if one is.
#[derive(Default)]
pub(crate) struct Fitting {
    job: Option<Job>,
}

impl Fitting {
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Start a fit of `scan` onto `reference` from what `document` holds
    /// now, superseding one already running; the thread fills the slot.
    pub fn begin(
        &mut self,
        document: &Document,
        scan: BodyId,
        reference: BodyId,
        search: bool,
    ) -> Slot {
        let slot = Slot::default();
        self.job = Some(Job {
            scan,
            reference,
            search,
            key: FitKey::of(document, scan, reference),
            slot: slot.clone(),
        });
        slot
    }

    /// Abandon the running fit; whether there was one.
    pub fn cancel(&mut self) -> bool {
        self.job.take().is_some()
    }

    /// The running fit once its answer is in, judged against `document`.
    pub fn landed(&mut self, document: &Document) -> Option<Landed> {
        let answer = self.job.as_ref()?.slot.take()?;
        let job = self.job.take()?;
        Some(
            if FitKey::of(document, job.scan, job.reference) == job.key {
                Landed::Fresh(Box::new((job, answer)))
            } else {
                Landed::Stale
            },
        )
    }
}
