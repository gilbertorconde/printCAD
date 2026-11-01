//! Where the window's named widgets are, for scripted checks that click
//! by id rather than by pixel: with `PRINTCAD_WIDGET_MAP=<path>` set, each
//! frame whose widgets moved writes `{id: [left, top, right, bottom]}`
//! there, in physical pixels from the window's corner. Toolbar buttons are
//! `button:<tool or command id>`, toolbar groups `group:<group id>`,
//! workbench tabs `tab:<bench id>`, panel captions `panel:<group>` and
//! open tool menu rows `menu:<tool id>` (variants append `:<variant id>`).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::LazyLock;

static PATH: LazyLock<Option<PathBuf>> =
    LazyLock::new(|| std::env::var_os("PRINTCAD_WIDGET_MAP").map(PathBuf::from));

thread_local! {
    /// This frame's widgets, and what was last written.
    static MAP: RefCell<(BTreeMap<String, [f32; 4]>, String)> = RefCell::default();
}

/// Note where widget `id` was drawn this frame.
pub(crate) fn note(ctx: &egui::Context, id: impl FnOnce() -> String, rect: egui::Rect) {
    if PATH.is_none() {
        return;
    }
    let scale = ctx.pixels_per_point();
    let place = [rect.left(), rect.top(), rect.right(), rect.bottom()].map(|v| v * scale);
    MAP.with(|map| map.borrow_mut().0.insert(id(), place));
}

/// Write the frame's widgets when they differ from the last written, and
/// start the next frame's list.
pub(crate) fn publish() {
    let Some(path) = PATH.as_ref() else {
        return;
    };
    MAP.with(|map| {
        let (widgets, written) = &mut *map.borrow_mut();
        let json = serde_json::to_string_pretty(&*widgets).unwrap_or_default();
        widgets.clear();
        if json != *written && std::fs::write(path, &json).is_ok() {
            *written = json;
        }
    });
}
