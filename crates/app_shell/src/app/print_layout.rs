//! The Print layout task's shown layout, kept up to the parts and the bed
//! while the task is open; the packing is `doc_render::print_layout`.

use std::sync::Arc;

use doc_render::print_layout::{
    LayoutSettings, lay_out, layout_key, parts_to_print, piece_mesh, plates_mesh,
};
use glam::Vec3;
use kernel_api::TriMesh;

/// The layout the Print layout task shows: what it was made from, the
/// copies' meshes on the bed, and the plates' outlines.
#[derive(Debug, Clone)]
pub(crate) struct ShownLayout {
    pub key: u64,
    pub layout: doc_render::print_layout::Layout,
    /// Each copy's id in the scene and its mesh where it lies.
    pub preview: Vec<(uuid::Uuid, Arc<TriMesh>)>,
    /// Every plate's outline as lines, one mesh.
    pub beds: Arc<TriMesh>,
}

impl crate::PrintCadApp {
    /// Keep the shown layout up to the parts, their shapes and the bed
    /// while the Print layout task is open, and drop it once it closes. A
    /// layout shown for the first time frames the view on the plates.
    pub(crate) fn drive_print_layout(&mut self) {
        if !self.session.tools.print_layout_shown {
            self.session.tools.print_layout = None;
            return;
        }
        let document = &self.session.document;
        let parts = parts_to_print(document, &self.registry);
        let settings = LayoutSettings::of(&self.user_settings.printing);
        let key = layout_key(document, &parts, &settings);
        if self
            .session
            .tools
            .print_layout
            .as_ref()
            .is_some_and(|shown| shown.key == key)
        {
            return;
        }
        let first = self.session.tools.print_layout.is_none();
        let layout = lay_out(document, &parts, &settings);
        let preview = layout
            .pieces
            .iter()
            .filter_map(|piece| Some((uuid::Uuid::new_v4(), piece_mesh(document, piece)?)))
            .collect();
        let beds = Arc::new(plates_mesh(
            &self.user_settings.printing,
            layout.plates,
            settings.plate_step(),
        ));
        self.session.tools.print_layout = Some(ShownLayout {
            key,
            layout,
            preview,
            beds,
        });
        if first {
            self.frame_print_layout();
        }
    }

    /// Frame the view on the shown layout's plates.
    fn frame_print_layout(&mut self) {
        let Some((lo, hi)) = self
            .session
            .tools
            .print_layout
            .as_ref()
            .and_then(|shown| shown.beds.bounds())
        else {
            return;
        };
        let aabb = (Vec3::from_array(lo), Vec3::from_array(hi));
        let (center, radius) = crate::app::view_fit::aabb_fit_center_radius(aabb.0, aabb.1);
        self.session.view.camera.reset_to_fit(
            center,
            radius,
            Some(aabb),
            &self.user_settings.camera,
        );
    }
}
