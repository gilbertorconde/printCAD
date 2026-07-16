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

use kernel_api::{ImportedModel, Kernel, SolidBuildResult, SolidOp, TessellationSettings, TriMesh};
use kernel_occt::OcctKernel;
use tracing::info;
use uuid::Uuid;

/// Job submitted from the UI thread to the kernel worker.
pub enum KernelRequest {
    /// Read a STEP/STP file (BRep-only fast path) and return bodies for the UI
    /// to register; tessellation is scheduled separately per body.
    ImportStep {
        path: PathBuf,
        detail: TessellationSettings,
    },
    /// Rebuild a body's solid from its Part Design feature chain.
    /// `op_features` maps each op index to its owning feature id so a
    /// failure can be pinned on the culprit in the tree.
    BuildSolid {
        body_id: Uuid,
        ops: Vec<SolidOp>,
        op_features: Vec<Uuid>,
        detail: TessellationSettings,
    },
    /// Tessellate one body's BRep snapshot on the kernel thread. The blob
    /// is shared with the document, so sending it is a refcount bump.
    TessellateBody {
        body_id: Uuid,
        brep_blob: std::sync::Arc<Vec<u8>>,
        face_colors: Vec<[f32; 3]>,
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
        detail: TessellationSettings,
        elapsed: Duration,
    },
    StepFailed {
        path: PathBuf,
        error: String,
    },
    BodyTessellated {
        body_id: Uuid,
        mesh: TriMesh,
        elapsed: Duration,
    },
    SolidBuilt {
        body_id: Uuid,
        result: SolidBuildResult,
        elapsed: Duration,
    },
    SolidFailed {
        body_id: Uuid,
        failed_feature: Option<Uuid>,
        error: String,
    },
    BodyTessellateFailed {
        body_id: Uuid,
        error: String,
    },
}

/// UI-side handle to the worker thread. `in_flight` is incremented by
/// [`Self::request_step_import`] / [`Self::request_tessellate_body`] and
/// decremented by [`Self::drain`] so the status panel can show a spinner while
/// imports are pending.
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
        if self
            .tx
            .send(KernelRequest::ImportStep { path, detail })
            .is_ok()
        {
            self.in_flight = self.in_flight.saturating_add(1);
        }
    }

    /// Submit a Part Design solid rebuild. One response arrives per request.
    pub fn request_build_solid(
        &mut self,
        body_id: Uuid,
        ops: Vec<SolidOp>,
        op_features: Vec<Uuid>,
        detail: TessellationSettings,
    ) {
        if self
            .tx
            .send(KernelRequest::BuildSolid {
                body_id,
                ops,
                op_features,
                detail,
            })
            .is_ok()
        {
            self.in_flight = self.in_flight.saturating_add(1);
        }
    }

    pub fn request_tessellate_body(
        &mut self,
        body_id: Uuid,
        brep_blob: std::sync::Arc<Vec<u8>>,
        face_colors: Vec<[f32; 3]>,
        detail: TessellationSettings,
    ) {
        if self
            .tx
            .send(KernelRequest::TessellateBody {
                body_id,
                brep_blob,
                face_colors,
                detail,
            })
            .is_ok()
        {
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
                let k0 = Instant::now();
                let response = match kernel.import_step(&path, &detail) {
                    Ok(model) => {
                        let kernel_ms = k0.elapsed();
                        let r0 = Instant::now();
                        match std::fs::read(&path) {
                            Ok(raw_bytes) => {
                                let read_ms = r0.elapsed();
                                let worker_total = started.elapsed();
                                info!(
                                    path = %path.display(),
                                    kernel_import_ms = format!("{:.2}", kernel_ms.as_secs_f64() * 1000.0),
                                    read_asset_bytes_ms = format!("{:.2}", read_ms.as_secs_f64() * 1000.0),
                                    worker_total_ms = format!("{:.2}", worker_total.as_secs_f64() * 1000.0),
                                    "STEP BRep import worker timing (kernel thread)"
                                );
                                KernelResponse::StepImported {
                                    path,
                                    model,
                                    raw_bytes,
                                    detail,
                                    elapsed: worker_total,
                                }
                            }
                            Err(err) => KernelResponse::StepFailed {
                                path,
                                error: format!("read source bytes failed: {err}"),
                            },
                        }
                    }
                    Err(err) => KernelResponse::StepFailed {
                        path,
                        error: err.to_string(),
                    },
                };
                if tx.send(response).is_err() {
                    return;
                }
            }
            KernelRequest::BuildSolid {
                body_id,
                ops,
                op_features,
                detail,
            } => {
                let started = Instant::now();
                let resp = match kernel.execute_solid_chain(&ops, &detail) {
                    Ok(result) => KernelResponse::SolidBuilt {
                        body_id,
                        result,
                        elapsed: started.elapsed(),
                    },
                    Err(err) => KernelResponse::SolidFailed {
                        body_id,
                        failed_feature: op_features.get(err.op_index).copied(),
                        error: err.message,
                    },
                };
                if tx.send(resp).is_err() {
                    return;
                }
            }
            KernelRequest::TessellateBody {
                body_id,
                brep_blob,
                face_colors,
                detail,
            } => {
                let started = Instant::now();
                let resp = match kernel.tessellate_step_brep(&brep_blob, &face_colors, &detail) {
                    Ok(mesh) => KernelResponse::BodyTessellated {
                        body_id,
                        mesh,
                        elapsed: started.elapsed(),
                    },
                    Err(err) => KernelResponse::BodyTessellateFailed {
                        body_id,
                        error: err.to_string(),
                    },
                };
                if tx.send(resp).is_err() {
                    return;
                }
            }
        }
    }
}
