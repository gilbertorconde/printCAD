//! Background worker that owns the OCCT kernel.
//!
//! The OCCT kernel is fully CPU-bound (STEP parsing + BRepMesh). Running it
//! on the UI thread freezes the viewport for tens of seconds on big models;
//! moving it onto a dedicated worker keeps panning/orbiting smooth during
//! imports.
//!
//! The worker also performs the `std::fs::read(path)` that backs the
//! document's asset blob — that I/O previously sat on the UI thread right
//! after the kernel returned, and is naturally cheap to colocate with the
//! kernel call since the worker is already off the hot path.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use kernel_api::{ImportedModel, Kernel, TessellationSettings};
use kernel_occt::OcctKernel;

/// Job submitted from the UI thread to the kernel worker.
pub enum KernelRequest {
    /// Read a STEP/STP file from disk, tessellate every top-level body, and
    /// return both the imported model and the raw source bytes for asset
    /// persistence.
    ImportStep {
        path: PathBuf,
        detail: TessellationSettings,
    },
}

/// Result delivered from the worker back to the UI thread.
///
/// Each request emits exactly one response; the UI's `in_flight` counter is
/// decremented when one is drained, so spurious extras would skew the
/// "importing X..." indicator.
pub enum KernelResponse {
    StepImported {
        path: PathBuf,
        model: ImportedModel,
        raw_bytes: Vec<u8>,
        elapsed: Duration,
    },
    StepFailed {
        path: PathBuf,
        error: String,
    },
}

/// UI-side handle to the worker thread. `in_flight` is incremented by
/// [`Self::request_step_import`] and decremented by [`Self::drain`] so the
/// status panel can show a spinner while imports are pending.
pub struct KernelWorker {
    tx: Sender<KernelRequest>,
    rx: Receiver<KernelResponse>,
    in_flight: u32,
}

impl KernelWorker {
    /// Spawn the worker thread. The thread owns its own [`OcctKernel`] for
    /// the lifetime of the app; the channels disconnect when the UI side is
    /// dropped, which lets the worker exit cleanly.
    pub fn spawn() -> Self {
        let (req_tx, req_rx) = channel::<KernelRequest>();
        let (resp_tx, resp_rx) = channel::<KernelResponse>();

        thread::Builder::new()
            .name("printcad-kernel-worker".to_string())
            .spawn(move || worker_loop(req_rx, resp_tx))
            .expect("failed to spawn kernel worker thread");

        Self {
            tx: req_tx,
            rx: resp_rx,
            in_flight: 0,
        }
    }

    /// Submit a STEP import job. Returns immediately; the result will arrive
    /// via [`Self::drain`] some time later.
    pub fn request_step_import(&mut self, path: PathBuf, detail: TessellationSettings) {
        if self.tx.send(KernelRequest::ImportStep { path, detail }).is_ok() {
            self.in_flight = self.in_flight.saturating_add(1);
        }
    }

    /// Pop every response that has arrived since the last call. The caller
    /// is responsible for any document/UI bookkeeping the responses imply.
    pub fn drain(&mut self) -> Vec<KernelResponse> {
        let mut out = Vec::new();
        while let Ok(resp) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            out.push(resp);
        }
        out
    }

    /// Number of imports the worker is currently processing or has queued.
    /// Drives the bottom-panel spinner.
    pub fn in_flight(&self) -> u32 {
        self.in_flight
    }
}

fn worker_loop(rx: Receiver<KernelRequest>, tx: Sender<KernelResponse>) {
    let mut kernel = OcctKernel::new();
    while let Ok(request) = rx.recv() {
        match request {
            KernelRequest::ImportStep { path, detail } => {
                let started = Instant::now();
                let response = match kernel.import_step(&path, &detail) {
                    Ok(model) => match std::fs::read(&path) {
                        Ok(raw_bytes) => KernelResponse::StepImported {
                            path,
                            model,
                            raw_bytes,
                            elapsed: started.elapsed(),
                        },
                        Err(err) => KernelResponse::StepFailed {
                            path,
                            error: format!("read source bytes failed: {err}"),
                        },
                    },
                    Err(err) => KernelResponse::StepFailed {
                        path,
                        error: err.to_string(),
                    },
                };
                if tx.send(response).is_err() {
                    // UI thread has dropped the receiver; nothing left to do.
                    return;
                }
            }
        }
    }
}
