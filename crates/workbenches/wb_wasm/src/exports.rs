//! What a bench offers the host (`bench` in the WIT world), as the host
//! calls it.

/// A trap or an overrun this many times in a session turns the bench off.
pub(crate) const STRIKES: u32 = 3;

/// How long a call may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Budget {
    /// Drawing and input: a frame must never wait on a bench.
    Frame,
    /// Commands, panel changes, rebuild plans, loading.
    Long,
}

/// Why a call into the guest gave no answer.
#[derive(Debug)]
pub(crate) enum Fault {
    /// It ran over its budget.
    Overran,
    /// It trapped, or the runtime refused it.
    Trapped(String),
}

pub(crate) type Answer<T> = Result<T, Fault>;

/// The guest's exports, one method each; the JSON they take and give is
/// the `bench_api` type the WIT world names beside each.
pub(crate) trait Exports {
    fn describe(&mut self) -> Answer<String>;
    fn feature_info(&mut self, node: &str) -> Answer<String>;
    fn parameters(&mut self, node: &str) -> Answer<String>;
    fn settle(&mut self, node: &str) -> Answer<String>;
    fn rebuild(&mut self, request: &str) -> Answer<String>;
    fn run_command(&mut self, id: &str, args: &str) -> Answer<Result<String, String>>;
    fn input(&mut self, input: &str) -> Answer<bool>;
    fn frame(&mut self, pointer: &str) -> Answer<String>;
    fn panel_event(&mut self, slot: &str, event: &str) -> Answer<()>;
    fn task_close(&mut self, accept: bool) -> Answer<Option<String>>;
    fn menu_items(&mut self, scope: &str) -> Answer<String>;
    fn menu_command(&mut self, id: &str, scope: &str) -> Answer<bool>;
    fn delete_feature(&mut self, id: &str) -> Answer<bool>;
    fn settings_panel(&mut self) -> Answer<String>;
    fn settings(&mut self) -> Answer<Option<String>>;
    fn apply_settings(&mut self, settings: &str) -> Answer<()>;
    fn suspend(&mut self) -> Answer<Option<Vec<u8>>>;
    fn resume(&mut self, state: Option<&[u8]>) -> Answer<()>;
}

/// What one call left behind besides its answer.
#[derive(Debug, Default)]
pub(crate) struct Aftermath {
    pub requests: Vec<bench_api::Request>,
    pub redraw: bool,
}
