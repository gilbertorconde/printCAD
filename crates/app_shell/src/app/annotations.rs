//! The annotations imported files carry, the notes the user pinned to
//! bodies and a callout of each cosmetic thread, drawn over the scene:
//! their lines as screen-space overlays in the annotation colour, their
//! text as a label at each one's anchor. The one selected in the tree
//! draws in the selection paint. Annotations follow the body they
//! describe: placed where it is placed, hidden when it is hidden; a note
//! follows its face through a rebuild too, and a thread's callout stands
//! at its bore's mouth while its feature shows.

use glam::Vec3;

use crate::PrintCadApp;
use crate::app::frame::ViewportData;
use crate::ui::TreeItemId;

/// How thick an annotation's lines draw, in pixels.
const LINE_PX: f32 = 1.2;

/// The shortest a note's leader draws on screen, in pixels: a leader seen
/// end on would put the label over the point it marks.
const MIN_LEADER_PX: f32 = 28.0;

impl PrintCadApp {
    /// Add the shown annotations to this frame's overlays and labels.
    pub(crate) fn annotation_overlays(&self, data: &mut ViewportData) {
        if !self.user_settings.rendering.show_annotations {
            return;
        }
        self.note_overlays(data);
        self.thread_overlays(data);
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

    /// The user's notes: a leader from each note's point to its label.
    fn note_overlays(&self, data: &mut ViewportData) {
        let document = &self.session.document;
        let selected = match self.session.tree_selection {
            Some(TreeItemId::Note(id)) => Some(id),
            _ => None,
        };
        let ink = ui_kit::tokens::ANNOTATION.to_normalized_gamma_f32();
        let ink = [ink[0], ink[1], ink[2]];
        for (body, note) in document.notes() {
            if !document.note_effective_visible(note.id) {
                continue;
            }
            let (at, label) = document.note_placed(body, note);
            let color = if selected == Some(note.id) {
                self.user_settings.rendering.selection_color
            } else {
                ink
            };
            self.leader(data, (at, label), document.note_text(note), color);
        }
    }

    /// A callout of each cosmetic thread: a leader from its bore's rim to
    /// its designation.
    fn thread_overlays(&self, data: &mut ViewportData) {
        let document = &self.session.document;
        let selected = match self.session.tree_selection {
            Some(TreeItemId::Feature(id)) => Some(id),
            _ => None,
        };
        let ink = ui_kit::tokens::ANNOTATION.to_normalized_gamma_f32();
        let ink = [ink[0], ink[1], ink[2]];
        for thread in document.cosmetic_threads() {
            let shown = document.imported_body_effective_visible(thread.body)
                && document
                    .get_feature_meta(thread.feature)
                    .is_some_and(|node| node.visible);
            if !shown {
                continue;
            }
            let placement = document.body_placement(thread.body);
            let (at, label) = thread.leader();
            let color = if selected == Some(thread.feature) {
                self.user_settings.rendering.selection_color
            } else {
                ink
            };
            self.leader(
                data,
                (placement.point(at), placement.point(label)),
                thread.label(),
                color,
            );
        }
    }

    /// A leader from a point to a label reading `text` (both in world
    /// space), drawn no shorter than `MIN_LEADER_PX`.
    fn leader(
        &self,
        data: &mut ViewportData,
        (at, label): ([f32; 3], [f32; 3]),
        text: String,
        color: [f32; 3],
    ) {
        let camera = &self.session.camera;
        let project = |p: [f32; 3]| camera.world_to_viewport(Vec3::from_array(p));
        let (Some(from), Some(to)) = (project(at), project(label)) else {
            return;
        };
        let from = glam::Vec2::new(from.0, from.1);
        let mut to = glam::Vec2::new(to.0, to.1);
        if from.distance(to) < MIN_LEADER_PX {
            let away = (to - from)
                .try_normalize()
                .unwrap_or(glam::Vec2::new(1.0, -1.0).normalize());
            to = from + away * MIN_LEADER_PX;
        }
        data.overlays.push(core_document::ScreenSpaceOverlay::new(
            from.to_array(),
            to.to_array(),
            color,
            LINE_PX,
        ));
        data.marks.push(core_document::ScreenSpaceMark {
            pos: from.to_array(),
            color,
            alpha: 1.0,
            kind: core_document::MarkKind::Dot { radius: 2.5 },
        });
        data.labels.push(
            core_document::ScreenSpaceLabel::new(
                to.to_array(),
                text,
                color,
                ui_kit::tokens::FONT_SM,
            )
            .pill(),
        );
    }
}
