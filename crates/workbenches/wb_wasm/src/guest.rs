//! A package's instance and the one way to call into it: under a budget,
//! with what the call may reach, and with a trap costing the guest its
//! state rather than the app anything.

use std::path::PathBuf;
use std::sync::Arc;

use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Store, StoreLimitsBuilder};
use wasmtime_wasi::{FsPerms, WasiCtxBuilder};

use crate::engine::{Budget, ENGINE, Running};
use crate::host::{Access, BenchExports, PackageInfo, State, Workbench};
use crate::jobs::JobBoard;

/// A trap or an overrun this many times in a session turns the bench off.
pub(crate) const STRIKES: u32 = 3;

/// A compiled package, ready to instantiate for the bench or for a job.
pub(crate) struct Loaded {
    pub component: Component,
    pub linker: Linker<State>,
    pub package: Arc<PackageInfo>,
    /// The package's own folder for files, the one it may reach.
    pub data_dir: PathBuf,
    pub memory_bytes: usize,
}

impl Loaded {
    pub(crate) fn new(
        component: Component,
        package: Arc<PackageInfo>,
        data_dir: PathBuf,
        memory_bytes: usize,
    ) -> Result<Self, String> {
        let mut linker = Linker::<State>::new(&ENGINE);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|e| format!("{e:#}"))?;
        Workbench::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|e| format!("{e:#}"))?;
        Ok(Self {
            component,
            linker,
            package,
            data_dir,
            memory_bytes,
        })
    }

    /// A fresh instance: its own memory, the package's data folder and
    /// nothing else of the file system, the network only when granted.
    pub(crate) fn instantiate(
        &self,
        jobs: Arc<JobBoard>,
    ) -> Result<(Store<State>, Workbench), String> {
        let _ = std::fs::create_dir_all(&self.data_dir);
        let mut wasi = WasiCtxBuilder::new();
        wasi.inherit_stderr();
        wasi.preopened_dir(&self.data_dir, "/data", FsPerms::ReadWrite)
            .map_err(|e| format!("cannot open the package's data folder: {e:#}"))?;
        if self.package.granted.network {
            wasi.inherit_network().allow_ip_name_lookup(true);
        }
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.memory_bytes)
            .instances(16)
            .build();
        let state = State::new(wasi.build(), limits, self.package.clone(), jobs);
        let mut store = Store::new(&ENGINE, state);
        store.limiter(|state| &mut state.limits);
        store.set_epoch_deadline(Budget::Long.ticks());
        let running = Running::start();
        let bindings = Workbench::instantiate(&mut store, &self.component, &self.linker)
            .map_err(|e| format!("{e:#}"));
        drop(running);
        Ok((store, bindings?))
    }
}

/// What one call left behind besides its answer.
#[derive(Debug, Default)]
pub(crate) struct Aftermath {
    pub requests: Vec<bench_api::Request>,
    pub redraw: bool,
}

/// The bench's own instance.
pub(crate) struct Guest {
    pub loaded: Arc<Loaded>,
    pub jobs: Arc<JobBoard>,
    store: Store<State>,
    bindings: Workbench,
    strikes: u32,
    /// Settings last given, put back into a fresh instance.
    pub settings: Option<String>,
}

impl Guest {
    pub(crate) fn new(loaded: Arc<Loaded>) -> Result<Self, String> {
        let jobs = JobBoard::new(loaded.clone());
        let (store, bindings) = loaded.instantiate(jobs.clone())?;
        Ok(Self {
            loaded,
            jobs,
            store,
            bindings,
            strikes: 0,
            settings: None,
        })
    }

    pub(crate) fn id(&self) -> &str {
        &self.loaded.package.id
    }

    /// Whether the guest misbehaved often enough to be turned off.
    pub(crate) fn disabled(&self) -> bool {
        self.strikes >= STRIKES
    }

    /// Call into the guest. `None` when it is turned off, trapped, or ran
    /// over `budget`; the reason is logged and a trapped instance is
    /// replaced by a fresh one.
    pub(crate) fn call<R>(
        &mut self,
        budget: Budget,
        access: Access,
        f: impl FnOnce(&BenchExports, &mut Store<State>) -> wasmtime::Result<R>,
    ) -> Option<(R, Aftermath)> {
        if self.disabled() {
            return None;
        }
        self.store.data_mut().access = access;
        self.store.set_epoch_deadline(budget.ticks());
        let running = Running::start();
        let result = f(self.bindings.printcad_workbench_bench(), &mut self.store);
        drop(running);
        let state = self.store.data_mut();
        state.access = Access::None;
        let aftermath = Aftermath {
            requests: std::mem::take(&mut state.requests),
            redraw: std::mem::take(&mut state.redraw),
        };
        match result {
            Ok(value) => Some((value, aftermath)),
            Err(error) => {
                self.strike(&error, budget);
                None
            }
        }
    }

    fn strike(&mut self, error: &wasmtime::Error, budget: Budget) {
        self.strikes += 1;
        let overran = matches!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(wasmtime::Trap::Interrupt)
        );
        let what = if overran {
            format!(
                "ran over its {} ms budget",
                budget.ticks() * crate::engine::TICK.as_millis() as u64
            )
        } else {
            format!("stopped with an error: {error:#}")
        };
        if self.disabled() {
            tracing::error!(
                target: "printcad.bench",
                package = %self.id(),
                "the workbench {what}; turned off for this session after {} failures",
                self.strikes
            );
            return;
        }
        tracing::error!(target: "printcad.bench", package = %self.id(), "the workbench {what}; restarting it");
        match self.loaded.instantiate(self.jobs.clone()) {
            Ok((store, bindings)) => {
                self.store = store;
                self.bindings = bindings;
                if let Some(settings) = self.settings.clone() {
                    let _ = self.call(Budget::Long, Access::None, |b, s| {
                        b.call_apply_settings(s, &settings)
                    });
                }
            }
            Err(e) => {
                self.strikes = STRIKES;
                tracing::error!(target: "printcad.bench", package = %self.id(), "cannot restart the workbench: {e}");
            }
        }
    }
}
