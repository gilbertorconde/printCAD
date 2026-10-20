//! Write a printCAD workbench as a WebAssembly component.
//!
//! Implement [`Bench`] on a type and name it with [`bench!`]; build the
//! crate as a `cdylib` for `wasm32-wasip2`. The data the bench and the
//! app exchange is [`api`] (the `printcad-bench-api` crate); what the app offers
//! is [`host`]. `docs/PLUGINS.md` in the printCAD repository walks
//! through a package.
//!
//! The SDK's 0.1.x speaks the contract `printcad:workbench@0.1`, and a
//! patch release only adds: match the app's events with a wildcard arm
//! and fill the contract's structs with `..Default::default()`, and a
//! package keeps building on every later 0.1.x.
//!
//! ```ignore
//! use printcad_bench_sdk::{Bench, api::*, bench};
//!
//! #[derive(Default)]
//! struct Hello;
//!
//! impl Bench for Hello {
//!     fn describe(&self) -> Registration {
//!         Registration { label: "Hello".into(), ..Default::default() }
//!     }
//! }
//!
//! bench!(Hello);
//! ```

pub use bench_api as api;
pub use serde_json::{self, Value, json};

use api::*;

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        world: "workbench",
        path: "wit",
        pub_export_macro: true,
        export_macro_name: "export_workbench",
        default_bindings_module: "printcad_bench_sdk::bindings",
    });
}

/// A workbench. Every method has a default, so a bench implements what it
/// offers. One value of it lives in the bench's instance for as long as
/// the app runs; a job runs in an instance of its own, through
/// [`Bench::job`].
pub trait Bench: Default + 'static {
    /// What the bench is: label, icon, tools, actions, commands.
    fn describe(&self) -> Registration;

    /// How a feature of one of the package's kinds shows in the tree.
    fn feature_info(&self, node: &Node) -> FeatureInfo {
        FeatureInfo {
            kind_label: node.kind.clone(),
            builds_solid: true,
            ..Default::default()
        }
    }

    /// The numbers of a feature that formulas may set.
    fn parameters(&self, _node: &Node) -> Vec<Parameter> {
        Vec::new()
    }

    /// A feature's data made whole once formulas' values are in it.
    fn settle(&self, node: &Node) -> Value {
        node.data.clone()
    }

    /// Plans for the bodies whose features changed.
    fn rebuild(&mut self, _request: RebuildRequest) -> Vec<Rebuild> {
        Vec::new()
    }

    /// A command the bench registered. Edits go through [`host::call`].
    fn run_command(&mut self, id: &str, _args: Value) -> Result<Value, String> {
        Err(format!("no command `{id}`"))
    }

    /// Something the user did while the bench is active; `true` when it
    /// was used.
    fn input(&mut self, _input: &Input) -> bool {
        false
    }

    /// Everything the bench shows. Asked after events, document and
    /// selection changes, and after [`host::redraw`].
    fn frame(&mut self, _pointer: &Pointer) -> Frame {
        Frame::default()
    }

    /// A change to a widget of the task panel or the settings page.
    fn panel_event(&mut self, _slot: PanelSlot, _event: PanelEvent) {}

    /// The task's OK (`true`) or Cancel; the undo step's name when it
    /// closed keeping its edits.
    fn task_close(&mut self, _accept: bool) -> Option<String> {
        None
    }

    fn menu_items(&mut self, _scope: &MenuScope) -> Vec<MenuItem> {
        Vec::new()
    }

    fn menu_command(&mut self, _id: &str, _scope: &MenuScope) -> bool {
        false
    }

    /// Remove an owned feature itself; `false` leaves it to the app.
    fn delete_feature(&mut self, _id: &str) -> bool {
        false
    }

    /// The bench's Preferences page.
    fn settings_panel(&mut self) -> Vec<Widget> {
        Vec::new()
    }

    /// The bench's settings, for the app to keep.
    fn settings(&self) -> Option<Value> {
        None
    }

    fn apply_settings(&mut self, _settings: Value) {}

    /// The editing state of the document on screen, kept with its tab.
    fn suspend(&mut self) -> Option<Vec<u8>> {
        None
    }

    fn resume(&mut self, _state: Option<Vec<u8>>) {}

    /// Long work, started with [`host::start_job`], run in an instance of
    /// its own away from the window. It reaches no document; it reports
    /// with [`host::progress`] and stops when [`host::cancelled`].
    fn job(_entry: &str, _input: &str) -> Result<String, String> {
        Err("this bench has no jobs".into())
    }
}

/// What the app offers a bench.
pub mod host {
    use super::bindings::printcad::workbench::host as raw;
    use super::*;

    pub fn info(message: &str) {
        raw::log("info", message);
    }

    pub fn warn(message: &str) {
        raw::log("warn", message);
    }

    pub fn error(message: &str) {
        raw::log("error", message);
    }

    /// A feature, with its formulas' values in its data.
    pub fn feature(id: &str) -> Option<Node> {
        serde_json::from_str(&raw::feature(id)?).ok()
    }

    /// Every feature of the document, in history order.
    pub fn features() -> Vec<Node> {
        serde_json::from_str(&raw::features()).unwrap_or_default()
    }

    pub fn bodies() -> Vec<Body> {
        serde_json::from_str(&raw::bodies()).unwrap_or_default()
    }

    /// A body's triangles in world space.
    pub fn body_mesh(body: &str) -> Option<(Vec<[f32; 3]>, Vec<u32>)> {
        let mesh = raw::body_mesh(body)?;
        let positions = mesh.positions.as_chunks::<3>().0.to_vec();
        Some((positions, mesh.indices))
    }

    /// A body's solid in the kernel's native format.
    pub fn body_shape(body: &str) -> Option<Vec<u8>> {
        raw::body_shape(body)
    }

    /// A feature's closed loops (a sketch's) placed in world space: the
    /// plane, then the loops, holes inside their outlines. `None` when it
    /// closes no area.
    pub fn profile(feature: &str) -> Option<api::kernel_api::Profile> {
        serde_json::from_str(&raw::profile(feature)?).ok()
    }

    /// Run a document command (`api::calls`).
    pub fn call(command: &str, args: Value) -> Result<Value, String> {
        let answer = raw::call(command, &args.to_string())?;
        serde_json::from_str(&answer).map_err(|e| e.to_string())
    }

    pub fn request(request: Request) {
        if let Ok(json) = serde_json::to_string(&request) {
            raw::request(&json);
        }
    }

    /// Ask for the frame again.
    pub fn redraw() {
        raw::redraw();
    }

    /// Add a feature of an owned kind; its id.
    pub fn add_feature(
        kind: &str,
        name: &str,
        body: Option<&str>,
        data: Value,
    ) -> Result<String, String> {
        let answer = call(
            calls::ADD_FEATURE,
            json!({"kind": kind, "name": name, "body": body, "data": data}),
        )?;
        answer
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "no id came back".into())
    }

    pub fn set_feature_data(id: &str, data: Value) -> Result<(), String> {
        call(calls::SET_FEATURE_DATA, json!({"id": id, "data": data})).map(|_| ())
    }

    pub fn remove_feature(id: &str) -> Result<(), String> {
        call(calls::REMOVE_FEATURE, json!({"id": id})).map(|_| ())
    }

    /// Remove a body that holds nothing of another package's and takes no
    /// shape from an import, a link or a copy: the empty body a cancelled
    /// part leaves (its removal undoes), or one of the package's own parts.
    pub fn remove_body(id: &str) -> Result<(), String> {
        call(calls::REMOVE_BODY, json!({"id": id})).map(|_| ())
    }

    /// A new body; its id.
    pub fn create_body(name: Option<&str>) -> Result<String, String> {
        let answer = call(calls::CREATE_BODY, json!({"name": name}))?;
        answer
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "no id came back".into())
    }

    /// Start [`Bench::job`] with `entry` and `input`; its number, which
    /// `Event::JobFinished` names when it ends.
    pub fn start_job(entry: &str, input: &str) -> Result<u64, String> {
        call(calls::JOB_START, json!({"entry": entry, "input": input}))?
            .as_u64()
            .ok_or_else(|| "no job number came back".into())
    }

    pub fn cancel_job(job: u64) {
        let _ = call(calls::JOB_CANCEL, json!({ "job": job }));
    }

    /// Inside a job: how far it is.
    pub fn progress(done: u64, total: u64) {
        raw::progress(done, total);
    }

    /// Inside a job: whether the user stopped it.
    pub fn cancelled() -> bool {
        raw::cancelled()
    }

    /// Inside a job, with the `helper` capability: run a native helper.
    /// It reads `input` on its standard input and answers on its
    /// standard output. A `progress <done> <total>` line on its
    /// standard error moves the job's bar while it runs.
    pub fn helper(name: &str, input: &[u8]) -> Result<Vec<u8>, String> {
        raw::helper(name, input)
    }

    /// With the `helper` capability: start a native helper and keep it
    /// running to talk to a line at a time; its handle. It ends when
    /// closed, when the job that opened it ends, or with the package's
    /// instance.
    pub fn helper_open(name: &str) -> Result<u64, String> {
        raw::helper_open(name)
    }

    /// Send a session's helper one line (no line break in it).
    pub fn helper_send(handle: u64, line: &str) -> Result<(), String> {
        raw::helper_send(handle, line)
    }

    /// The helper's next line: waiting up to `wait_ms` for it inside a
    /// job, not at all outside one; `None` when none came. An error once
    /// the helper has ended.
    pub fn helper_receive(handle: u64, wait_ms: u32) -> Result<Option<String>, String> {
        raw::helper_receive(handle, wait_ms)
    }

    pub fn helper_close(handle: u64) {
        raw::helper_close(handle);
    }

    /// Keep a mesh for frames to draw by its handle (`Frame::kept`), with
    /// a colour per position or none.
    pub fn mesh_create(
        positions: &[[f32; 3]],
        indices: &[u32],
        colors: &[[f32; 3]],
    ) -> Result<u64, String> {
        raw::mesh_create(positions.as_flattened(), indices, colors.as_flattened())
    }

    /// Replace a kept mesh's positions from `first` on, and their colours
    /// when as many are given.
    pub fn mesh_update(
        handle: u64,
        first: u32,
        positions: &[[f32; 3]],
        colors: &[[f32; 3]],
    ) -> Result<(), String> {
        raw::mesh_update(
            handle,
            first,
            positions.as_flattened(),
            colors.as_flattened(),
        )
    }

    pub fn mesh_remove(handle: u64) {
        raw::mesh_remove(handle);
    }
}

/// A grid of heights kept as a mesh (simulated material, a terrain):
/// `cols` × `rows` points `pitch` apart in x and y from `origin`, each
/// raised by its height, two triangles a cell. Draw it with its
/// [`handle`](Self::handle) in `Frame::kept`; [`update`](Self::update)
/// sends only the rectangle that changed.
pub struct HeightField {
    handle: u64,
    origin: [f32; 3],
    cols: u32,
    rows: u32,
    pitch: f32,
}

impl HeightField {
    /// A field of `heights`, row by row from `origin`, and a colour per
    /// point or none.
    pub fn new(
        origin: [f32; 3],
        cols: u32,
        rows: u32,
        pitch: f32,
        heights: &[f32],
        colors: &[[f32; 3]],
    ) -> Result<Self, String> {
        if cols < 2 || rows < 2 || heights.len() != (cols * rows) as usize {
            return Err("a field has at least 2 × 2 points and a height for each".into());
        }
        let field = |handle| Self {
            handle,
            origin,
            cols,
            rows,
            pitch,
        };
        let shape = field(0);
        let positions: Vec<[f32; 3]> = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (c, r)))
            .zip(heights)
            .map(|((c, r), h)| shape.point(c, r, *h))
            .collect();
        let mut indices = Vec::with_capacity(((cols - 1) * (rows - 1) * 6) as usize);
        for r in 0..rows - 1 {
            for c in 0..cols - 1 {
                let at = r * cols + c;
                indices.extend([at, at + 1, at + cols, at + 1, at + cols + 1, at + cols]);
            }
        }
        host::mesh_create(&positions, &indices, colors).map(field)
    }

    pub fn handle(&self) -> u64 {
        self.handle
    }

    fn point(&self, col: u32, row: u32, height: f32) -> [f32; 3] {
        [
            self.origin[0] + col as f32 * self.pitch,
            self.origin[1] + row as f32 * self.pitch,
            self.origin[2] + height,
        ]
    }

    /// New heights (and colours, or none) for the `width` × `height`
    /// points from `col`, `row`, row by row.
    pub fn update(
        &self,
        col: u32,
        row: u32,
        width: u32,
        height: u32,
        heights: &[f32],
        colors: &[[f32; 3]],
    ) -> Result<(), String> {
        if col + width > self.cols
            || row + height > self.rows
            || heights.len() != (width * height) as usize
            || !(colors.is_empty() || colors.len() == heights.len())
        {
            return Err("the rectangle and its heights fit the field".into());
        }
        for r in 0..height {
            let line = (r * width) as usize..((r + 1) * width) as usize;
            let positions: Vec<[f32; 3]> = (0..width)
                .zip(&heights[line.clone()])
                .map(|(c, h)| self.point(col + c, row + r, *h))
                .collect();
            let colors = if colors.is_empty() {
                &[][..]
            } else {
                &colors[line]
            };
            host::mesh_update(self.handle, (row + r) * self.cols + col, &positions, colors)?;
        }
        Ok(())
    }
}

impl Drop for HeightField {
    fn drop(&mut self) {
        host::mesh_remove(self.handle);
    }
}

#[doc(hidden)]
pub mod adapter {
    use std::any::Any;
    use std::cell::RefCell;

    use super::bindings::exports::printcad::workbench::bench::Guest;
    use super::*;

    thread_local! {
        static BENCH: RefCell<Option<Box<dyn Any>>> = const { RefCell::new(None) };
    }

    fn with<B: Bench, R>(f: impl FnOnce(&mut B) -> R) -> R {
        BENCH.with(|cell| {
            let mut slot = cell.borrow_mut();
            let bench = slot.get_or_insert_with(|| Box::new(B::default()));
            f(bench.downcast_mut::<B>().expect("one bench per component"))
        })
    }

    /// A value the app sent. A kind of event or menu this build of the SDK
    /// does not know comes from a newer app within the same contract, and
    /// passes the bench by unread.
    fn read<T: serde::de::DeserializeOwned>(json: &str) -> Option<T> {
        match serde_json::from_str(json) {
            Ok(value) => Some(value),
            Err(e) if e.to_string().starts_with("unknown variant") => None,
            Err(e) => {
                host::error(&format!("the app sent what this bench cannot read: {e}"));
                None
            }
        }
    }

    fn write<T: serde::Serialize>(value: &T) -> String {
        serde_json::to_string(value).unwrap_or_default()
    }

    /// The component's exports, forwarded to `B`.
    pub struct Exports<B>(std::marker::PhantomData<B>);

    impl<B: Bench> Guest for Exports<B> {
        fn describe() -> String {
            with::<B, _>(|b| write(&b.describe()))
        }

        fn feature_info(node: String) -> String {
            match read::<Node>(&node) {
                Some(node) => with::<B, _>(|b| write(&b.feature_info(&node))),
                None => String::new(),
            }
        }

        fn parameters(node: String) -> String {
            match read::<Node>(&node) {
                Some(node) => with::<B, _>(|b| write(&b.parameters(&node))),
                None => "[]".into(),
            }
        }

        fn settle(node: String) -> String {
            match read::<Node>(&node) {
                Some(node) => with::<B, _>(|b| write(&b.settle(&node))),
                None => String::new(),
            }
        }

        fn rebuild(request: String) -> String {
            match read::<RebuildRequest>(&request) {
                Some(request) => with::<B, _>(|b| write(&b.rebuild(request))),
                None => "[]".into(),
            }
        }

        fn run_command(id: String, args: String) -> Result<String, String> {
            let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
            with::<B, _>(|b| b.run_command(&id, args)).map(|v| v.to_string())
        }

        fn input(input: String) -> bool {
            match read::<Input>(&input) {
                Some(input) => with::<B, _>(|b| b.input(&input)),
                None => false,
            }
        }

        fn frame(pointer: String) -> String {
            let pointer = read::<Pointer>(&pointer).unwrap_or_default();
            with::<B, _>(|b| write(&b.frame(&pointer)))
        }

        fn panel_event(slot: String, event: String) {
            let slot = match slot.as_str() {
                "settings" => PanelSlot::Settings,
                "transport" => PanelSlot::Transport,
                _ => PanelSlot::Task,
            };
            if let Some(event) = read::<PanelEvent>(&event) {
                with::<B, _>(|b| b.panel_event(slot, event));
            }
        }

        fn task_close(accept: bool) -> Option<String> {
            with::<B, _>(|b| b.task_close(accept))
        }

        fn menu_items(scope: String) -> String {
            match read::<MenuScope>(&scope) {
                Some(scope) => with::<B, _>(|b| write(&b.menu_items(&scope))),
                None => "[]".into(),
            }
        }

        fn menu_command(id: String, scope: String) -> bool {
            match read::<MenuScope>(&scope) {
                Some(scope) => with::<B, _>(|b| b.menu_command(&id, &scope)),
                None => false,
            }
        }

        fn delete_feature(id: String) -> bool {
            with::<B, _>(|b| b.delete_feature(&id))
        }

        fn settings_panel() -> String {
            with::<B, _>(|b| write(&b.settings_panel()))
        }

        fn settings() -> Option<String> {
            with::<B, _>(|b| b.settings()).map(|v| v.to_string())
        }

        fn apply_settings(settings: String) {
            if let Ok(value) = serde_json::from_str(&settings) {
                with::<B, _>(|b| b.apply_settings(value));
            }
        }

        fn suspend() -> Option<Vec<u8>> {
            with::<B, _>(|b| b.suspend())
        }

        fn resume(state: Option<Vec<u8>>) {
            with::<B, _>(|b| b.resume(state));
        }

        fn job_run(entry: String, input: String) -> Result<String, String> {
            B::job(&entry, &input)
        }
    }
}

/// Make `$bench` (a [`Bench`]) the component's workbench.
#[macro_export]
macro_rules! bench {
    ($bench:ty) => {
        type __PrintcadBenchExports = $crate::adapter::Exports<$bench>;
        $crate::bindings::export_workbench!(__PrintcadBenchExports with_types_in $crate::bindings);
    };
}
