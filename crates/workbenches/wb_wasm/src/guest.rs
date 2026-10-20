//! A package's instance and the one way to call into it: under a budget,
//! with what the call may reach, and with a trap costing the guest its
//! state rather than the app anything.

use std::path::PathBuf;
use std::sync::Arc;

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::engine::{ENGINE, Running};
use crate::exports::{Aftermath, Answer, Budget, Exports, Fault, STRIKES};
use crate::host::{Access, PackageInfo, Reach};
use crate::jobs::JobBoard;

wasmtime::component::bindgen!({
    world: "workbench",
    path: "../../bench_api/wit",
});

pub(crate) use exports::printcad::workbench::bench::Guest as BenchExports;

/// The store's data: WASI, limits, and the host's side of the instance.
pub(crate) struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    pub limits: StoreLimits,
    pub reach: Reach,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl printcad::workbench::host::Host for State {
    fn log(&mut self, level: String, message: String) {
        self.reach.log(level, message)
    }

    fn feature(&mut self, id: String) -> Option<String> {
        self.reach.feature(id)
    }

    fn features(&mut self) -> String {
        self.reach.features()
    }

    fn bodies(&mut self) -> String {
        self.reach.bodies()
    }

    fn body_mesh(&mut self, body: String) -> Option<printcad::workbench::host::Mesh> {
        let (positions, indices) = self.reach.body_mesh(body)?;
        Some(printcad::workbench::host::Mesh { positions, indices })
    }

    fn body_shape(&mut self, body: String) -> Option<Vec<u8>> {
        self.reach.body_shape(body)
    }

    fn profile(&mut self, feature: String) -> Option<String> {
        self.reach.profile(feature)
    }

    fn call(&mut self, command: String, args: String) -> Result<String, String> {
        self.reach.call(command, args)
    }

    fn request(&mut self, request: String) {
        self.reach.request(request)
    }

    fn redraw(&mut self) {
        self.reach.redraw()
    }

    fn progress(&mut self, done: u64, total: u64) {
        self.reach.progress(done, total)
    }

    fn cancelled(&mut self) -> bool {
        self.reach.cancelled()
    }

    fn helper(&mut self, name: String, input: Vec<u8>) -> Result<Vec<u8>, String> {
        self.reach.helper(name, input)
    }

    fn mesh_create(
        &mut self,
        positions: Vec<f32>,
        indices: Vec<u32>,
        colors: Vec<f32>,
    ) -> Result<u64, String> {
        self.reach.meshes.create(&positions, indices, &colors)
    }

    fn mesh_update(
        &mut self,
        handle: u64,
        first: u32,
        positions: Vec<f32>,
        colors: Vec<f32>,
    ) -> Result<(), String> {
        let updated = self.reach.meshes.update(handle, first, &positions, &colors);
        self.reach.redraw();
        updated
    }

    fn mesh_remove(&mut self, handle: u64) {
        self.reach.meshes.remove(handle);
        self.reach.redraw();
    }
}

/// The exports of one instance with its store, for one call.
struct Bound<'a> {
    exports: &'a BenchExports,
    store: &'a mut Store<State>,
}

fn fault(error: wasmtime::Error) -> Fault {
    if matches!(
        error.downcast_ref::<wasmtime::Trap>(),
        Some(wasmtime::Trap::Interrupt)
    ) {
        Fault::Overran
    } else {
        Fault::Trapped(format!("{error:#}"))
    }
}

impl Exports for Bound<'_> {
    fn describe(&mut self) -> Answer<String> {
        self.exports.call_describe(&mut *self.store).map_err(fault)
    }
    fn feature_info(&mut self, node: &str) -> Answer<String> {
        self.exports
            .call_feature_info(&mut *self.store, node)
            .map_err(fault)
    }
    fn parameters(&mut self, node: &str) -> Answer<String> {
        self.exports
            .call_parameters(&mut *self.store, node)
            .map_err(fault)
    }
    fn settle(&mut self, node: &str) -> Answer<String> {
        self.exports
            .call_settle(&mut *self.store, node)
            .map_err(fault)
    }
    fn rebuild(&mut self, request: &str) -> Answer<String> {
        self.exports
            .call_rebuild(&mut *self.store, request)
            .map_err(fault)
    }
    fn run_command(&mut self, id: &str, args: &str) -> Answer<Result<String, String>> {
        self.exports
            .call_run_command(&mut *self.store, id, args)
            .map_err(fault)
    }
    fn input(&mut self, input: &str) -> Answer<bool> {
        self.exports
            .call_input(&mut *self.store, input)
            .map_err(fault)
    }
    fn frame(&mut self, pointer: &str) -> Answer<String> {
        self.exports
            .call_frame(&mut *self.store, pointer)
            .map_err(fault)
    }
    fn panel_event(&mut self, slot: &str, event: &str) -> Answer<()> {
        self.exports
            .call_panel_event(&mut *self.store, slot, event)
            .map_err(fault)
    }
    fn task_close(&mut self, accept: bool) -> Answer<Option<String>> {
        self.exports
            .call_task_close(&mut *self.store, accept)
            .map_err(fault)
    }
    fn menu_items(&mut self, scope: &str) -> Answer<String> {
        self.exports
            .call_menu_items(&mut *self.store, scope)
            .map_err(fault)
    }
    fn menu_command(&mut self, id: &str, scope: &str) -> Answer<bool> {
        self.exports
            .call_menu_command(&mut *self.store, id, scope)
            .map_err(fault)
    }
    fn delete_feature(&mut self, id: &str) -> Answer<bool> {
        self.exports
            .call_delete_feature(&mut *self.store, id)
            .map_err(fault)
    }
    fn settings_panel(&mut self) -> Answer<String> {
        self.exports
            .call_settings_panel(&mut *self.store)
            .map_err(fault)
    }
    fn settings(&mut self) -> Answer<Option<String>> {
        self.exports.call_settings(&mut *self.store).map_err(fault)
    }
    fn apply_settings(&mut self, settings: &str) -> Answer<()> {
        self.exports
            .call_apply_settings(&mut *self.store, settings)
            .map_err(fault)
    }
    fn suspend(&mut self) -> Answer<Option<Vec<u8>>> {
        self.exports.call_suspend(&mut *self.store).map_err(fault)
    }
    fn resume(&mut self, state: Option<&[u8]>) -> Answer<()> {
        self.exports
            .call_resume(&mut *self.store, state)
            .map_err(fault)
    }
}

/// A compiled package, ready to instantiate for the bench or for a job.
pub(crate) struct Loaded {
    pub component: Component,
    pub linker: Linker<State>,
    pub package: Arc<PackageInfo>,
    /// The package's own folder for files, the one it may reach.
    pub data_dir: PathBuf,
    pub memory_bytes: usize,
    pub meshes: Arc<crate::kept::KeptMeshes>,
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
            meshes: Arc::default(),
        })
    }

    /// A fresh instance: its own memory, the package's data folder and
    /// nothing else of the file system, the network only when granted.
    pub(crate) fn instantiate(
        &self,
        jobs: Arc<JobBoard>,
    ) -> Result<(Store<State>, Workbench), String> {
        let mut wasi = WasiCtxBuilder::new();
        wasi.inherit_stderr();
        let _ = std::fs::create_dir_all(&self.data_dir);
        wasi.preopened_dir(&self.data_dir, "/data", FsPerms::ReadWrite)
            .map_err(|e| format!("cannot open the package's data folder: {e:#}"))?;
        if self.package.granted.network {
            wasi.inherit_network().allow_ip_name_lookup(true);
        }
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.memory_bytes)
            .instances(16)
            .build();
        let state = State {
            wasi: wasi.build(),
            table: ResourceTable::new(),
            limits,
            reach: Reach::new(self.package.clone(), jobs, self.meshes.clone()),
        };
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
        f: impl FnOnce(&mut dyn Exports) -> Answer<R>,
    ) -> Option<(R, Aftermath)> {
        if self.disabled() {
            return None;
        }
        self.store.data_mut().reach.access = access;
        self.store.set_epoch_deadline(budget.ticks());
        let running = Running::start();
        let result = f(&mut Bound {
            exports: self.bindings.printcad_workbench_bench(),
            store: &mut self.store,
        });
        drop(running);
        let state = &mut self.store.data_mut().reach;
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

    fn strike(&mut self, error: &Fault, budget: Budget) {
        self.strikes += 1;
        let what = match error {
            Fault::Overran => format!(
                "ran over its {} ms budget",
                budget.ticks() * crate::engine::TICK.as_millis() as u64
            ),
            Fault::Trapped(error) => format!("stopped with an error: {error}"),
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
        // The meshes it kept went with its state.
        self.loaded.meshes.clear();
        match self.loaded.instantiate(self.jobs.clone()) {
            Ok((store, bindings)) => {
                self.store = store;
                self.bindings = bindings;
                if let Some(settings) = self.settings.clone() {
                    let _ = self.call(Budget::Long, Access::None, |b| b.apply_settings(&settings));
                }
            }
            Err(e) => {
                self.strikes = STRIKES;
                tracing::error!(target: "printcad.bench", package = %self.id(), "cannot restart the workbench: {e}");
            }
        }
    }
}
