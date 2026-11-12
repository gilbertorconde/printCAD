//! A bench's drawing side: the hooks that draw into the window with egui,
//! kept off `core_document::Workbench` so the core holds no drawing. A
//! bench implements [`BenchUi`] on its own type behind its `egui` feature;
//! the registry keeps the cast to it beside the bench ([`attach`]), and the
//! host reaches it through [`bench_ui`] without naming the bench.

use std::any::Any;

use core_document::{
    DocumentService, TaskOutcome, TaskRequest, Workbench, WorkbenchId, WorkbenchRuntimeContext,
};
use egui::Ui;

/// What a bench draws itself: the task panel, its Preferences page, the
/// left panel and the strip under the view. Every hook draws nothing by
/// default.
pub trait BenchUi {
    /// Draw the strip under the view, while
    /// [`Workbench::has_viewport_strip`].
    fn ui_viewport_strip(&mut self, _ui: &mut Ui, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Draw custom UI in the left panel, above the feature tree.
    /// Called every frame while this workbench is active.
    fn ui_left_panel(&mut self, _ui: &mut Ui, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Draw the task panel body and react to the host's accept/cancel.
    fn ui_task_panel(
        &mut self,
        _ui: &mut Ui,
        _ctx: &mut WorkbenchRuntimeContext,
        _request: TaskRequest,
    ) -> TaskOutcome {
        TaskOutcome::Open
    }

    /// Draw this bench's Preferences page, while it is shown
    /// ([`Workbench::has_settings`]). `filter` is the dialog's lowercase
    /// search text; rows that do not match it stay hidden
    /// (`ui_kit::widgets::pref_group` applies it). The settings it changes
    /// are saved through [`Workbench::settings_json`].
    fn ui_settings(&mut self, _ui: &mut Ui, _filter: &str) {}
}

/// The cast from a registered bench to its drawing side, made for its
/// concrete type: what the registry keeps as the bench's drawing side.
#[derive(Clone, Copy)]
struct Cast(fn(&mut dyn Any) -> Option<&mut dyn BenchUi>);

fn cast_to<T: BenchUi + 'static>(bench: &mut dyn Any) -> Option<&mut dyn BenchUi> {
    bench
        .downcast_mut::<T>()
        .map(|bench| bench as &mut dyn BenchUi)
}

/// Give every registered bench of type `T` its drawing side, so
/// [`bench_ui`] finds it. A bench left without one draws nothing.
pub fn attach<T: Workbench + BenchUi>(registry: &mut DocumentService) {
    let ids: Vec<WorkbenchId> = registry.ids().to_vec();
    for id in ids {
        let is_t = registry.workbench_mut(&id).is_ok_and(|bench| {
            let bench: &mut dyn Workbench = &mut **bench;
            (bench as &mut dyn Any).is::<T>()
        });
        if is_t {
            registry.set_drawing_side(&id, Box::new(Cast(cast_to::<T>)));
        }
    }
}

/// Bench `id`'s drawing side, when it is registered with one.
pub fn bench_ui<'a>(
    registry: &'a mut DocumentService,
    id: &WorkbenchId,
) -> Option<&'a mut dyn BenchUi> {
    let cast = *registry.drawing_side(id)?.downcast_ref::<Cast>()?;
    let bench: &mut dyn Workbench = &mut **registry.workbench_mut(id).ok()?;
    (cast.0)(bench as &mut dyn Any)
}
