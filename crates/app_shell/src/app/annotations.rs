//! The annotations imported files carry, drawn over the scene: their lines
//! as screen-space overlays in the annotation colour, their text as a label
//! at each one's anchor. The one selected in the tree draws in the
//! selection paint. Annotations follow the body they describe: placed where
//! it is placed, hidden when it is hidden.

use glam::Vec3;

use crate::PrintCadApp;
use crate::app::frame::ViewportData;
use crate::ui::TreeItemId;

/// How thick an annotation's lines draw, in pixels.
const LINE_PX: f32 = 1.2;

impl PrintCadApp {
    /// Add the shown annotations to this frame's overlays and labels.
    pub(crate) fn annotation_overlays(&self, data: &mut ViewportData) {
        if !self.user_settings.rendering.show_annotations {
            return;
        }
        let document = &self.session.document;
        let annotations = document.imported_annotations();
        if annotations.is_empty() {
            return;
        }
        let selected = match self.session.tree_selection {
            Some(TreeItemId::ImportedObject(id)) => Some(id),
            _ => None,
        };
        let ink = ui_kit::tokens::ANNOTATION.to_normalized_gamma_f32();
        let ink = [ink[0], ink[1], ink[2]];
        let camera = &self.session.camera;
        for (node, annotation) in annotations {
            if !document.imported_annotation_effective_visible(node.id) {
                continue;
            }
            let placement = annotation
                .body
                .map(|body| document.body_placement(body))
                .filter(|placement| !placement.is_identity());
            let project = |p: [f32; 3]| {
                let p = placement.map_or(p, |placement| placement.point(p));
                camera.world_to_viewport(Vec3::from_array(p))
            };
            let color = if selected == Some(node.id) {
                self.user_settings.rendering.selection_color
            } else {
                ink
            };
            for line in &annotation.polylines {
                let points: Vec<Option<(f32, f32)>> = line.iter().map(|p| project(*p)).collect();
                for pair in points.windows(2) {
                    if let [Some(a), Some(b)] = pair {
                        data.overlays.push(core_document::ScreenSpaceOverlay::new(
                            [a.0, a.1],
                            [b.0, b.1],
                            color,
                            LINE_PX,
                        ));
                    }
                }
            }
            if annotation.text.is_empty() {
                continue;
            }
            if let Some((x, y)) = annotation.anchor.and_then(project) {
                data.labels.push(
                    core_document::ScreenSpaceLabel::new(
                        [x, y - ui_kit::tokens::SPACE_3],
                        annotation.text.clone(),
                        color,
                        ui_kit::tokens::FONT_SM,
                    )
                    .pill(),
                );
            }
        }
    }
}
