//! The pin tool and the pins drawn over the scene. With the tool armed a
//! click on a part opens a card by the point asking for the pin's note
//! (Enter puts the pin down, Escape drops it); a click on a pin opens the
//! card on it. Each shown pin draws as a numbered dot where it stands on
//! its part as built now: dimmed once done, in the danger colour once
//! lost, in the selection paint while its tree row is selected. The cursor
//! over a pin shows its note.

use core_document::{BodyId, Document, Pin, PinAuthor, PinWant};
use glam::{Vec2, Vec3};
use serde_json::{Value, json};

use crate::PrintCadApp;
use crate::app::frame::ViewportData;
use crate::ui::TreeItemId;

/// How near the cursor a pin's dot must be to be under it, in logical
/// pixels.
const REACH: f32 = 9.0;

/// The pin card: the body, and the new pin waiting for its note or the
/// number of the pin it edits.
#[derive(Debug, Clone)]
pub(crate) struct PinCard {
    pub body: BodyId,
    pub editing: PinEditing,
}

#[derive(Debug, Clone)]
pub(crate) enum PinEditing {
    New(Box<Pin>),
    Existing(u32),
}

/// What the UI draws of the pin card.
#[derive(Debug, Clone, PartialEq)]
pub struct PinCardView {
    /// The pin's number, or the one a new pin will take.
    pub number: u32,
    pub new: bool,
    /// The note it opens with.
    pub note: String,
    pub done: bool,
    pub author: PinAuthor,
    pub lost: bool,
    /// Where the pin stands, in viewport pixels; `None` when off screen.
    pub at: Option<[f32; 2]>,
}

impl PrintCadApp {
    /// A click with the pin tool armed: on a pin, its card opens; on a
    /// part, a new pin waits on the card for its note, on the edge there
    /// or else the face; on nothing, the card goes. Whether to draw again.
    pub(crate) fn pin_click(&mut self) -> bool {
        if let Some((body, number)) = self.pin_under_cursor() {
            self.session.pin_card = Some(PinCard {
                body,
                editing: PinEditing::Existing(number),
            });
            return true;
        }
        let (Some(body), Some(point)) = (self.session.hovered_body, self.session.hovered_world_pos)
        else {
            return self.session.pin_card.take().is_some();
        };
        let body = BodyId(body);
        let want = match self.session.hovered_edge {
            Some(edge) if edge.body == body.0 => PinWant::Edge(Some(edge.edge)),
            _ => PinWant::Face,
        };
        let pin =
            self.session
                .document
                .new_pin(body, String::new(), PinAuthor::User, Some(point), want);
        self.session.pin_card = Some(PinCard {
            body,
            editing: PinEditing::New(Box::new(pin)),
        });
        true
    }

    /// Open the card on pin `number`, to edit its note.
    pub(crate) fn edit_pin(&mut self, number: u32) {
        if let Some((body, _)) = self.session.document.pin(number) {
            self.session.pin_card = Some(PinCard {
                body,
                editing: PinEditing::Existing(number),
            });
        }
    }

    /// The card's note saved: the new pin put down with it, or the pin
    /// edited given it. A new pin with nothing to say is not put down.
    pub(crate) fn save_pin(&mut self, note: String) {
        let Some(card) = self.session.pin_card.take() else {
            return;
        };
        let note = note.trim().to_string();
        match card.editing {
            PinEditing::New(mut pin) => {
                if note.is_empty() {
                    return;
                }
                pin.note = note;
                if let Some(number) = self.session.document.add_pin(card.body, *pin) {
                    self.session.journal.label_next(format!("Pin {number}"));
                }
            }
            PinEditing::Existing(number) => {
                let Some((_, pin)) = self.session.document.pin(number) else {
                    return;
                };
                let mut pin = pin.clone();
                pin.note = note;
                self.session.document.set_pin(pin);
                self.session
                    .journal
                    .label_next(format!("Edit pin {number}"));
            }
        }
        self.close_gesture();
    }

    /// Mark a pin done or open again, or show or hide it.
    pub(crate) fn change_pin(&mut self, number: u32, change: impl FnOnce(&mut Pin), label: &str) {
        let Some((_, pin)) = self.session.document.pin(number) else {
            return;
        };
        let mut pin = pin.clone();
        change(&mut pin);
        self.session.document.set_pin(pin);
        self.session
            .journal
            .label_next(format!("{label} pin {number}"));
        self.close_gesture();
    }

    /// What the UI draws of the pin card, while it is open.
    pub(crate) fn pin_card_view(&self) -> Option<PinCardView> {
        let card = self.session.pin_card.as_ref()?;
        let document = &self.session.document;
        let (number, new, pin, at) = match &card.editing {
            PinEditing::New(pin) => (
                document.next_pin_number(),
                true,
                pin.as_ref(),
                document.body_placement(card.body).point(pin.at),
            ),
            PinEditing::Existing(number) => {
                let (body, pin) = document.pin(*number)?;
                (*number, false, pin, document.pin_placed(body, pin).at)
            }
        };
        Some(PinCardView {
            number,
            new,
            note: pin.note.clone(),
            done: pin.done,
            author: pin.author,
            lost: !new && document.pin_placed(card.body, pin).lost,
            at: self
                .session
                .camera
                .world_to_viewport(Vec3::from_array(at))
                .map(|(x, y)| [x, y]),
        })
    }

    /// Every shown pin where it is drawn: its body, number, place in the
    /// viewport and whether it is lost.
    fn drawn_pins(&self) -> Vec<(BodyId, &Pin, [f32; 2], bool)> {
        let document = &self.session.document;
        let camera = &self.session.camera;
        document
            .pins()
            .filter(|(_, pin)| document.pin_effective_visible(pin.number))
            .filter_map(|(body, pin)| {
                let placed = document.pin_placed(body, pin);
                let (x, y) = camera.world_to_viewport(Vec3::from_array(placed.at))?;
                Some((body, pin, [x, y], placed.lost))
            })
            .collect()
    }

    /// The shown pin whose dot is nearest the cursor, within reach.
    fn pin_under_cursor(&self) -> Option<(BodyId, u32)> {
        let cursor = Vec2::from(self.cursor_in_viewport?);
        let reach = REACH * self.pixels_per_point();
        self.drawn_pins()
            .into_iter()
            .map(|(body, pin, at, _)| (Vec2::from(at).distance(cursor), body, pin.number))
            .filter(|(d, ..)| *d <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, body, number)| (body, number))
    }

    /// Add the shown pins, and the new one on the card, to this frame's
    /// marks and labels; the note of the pin under the cursor beside it.
    pub(crate) fn pin_overlays(&self, data: &mut ViewportData) {
        let tone = |c: egui::Color32| {
            let [r, g, b, _] = c.to_normalized_gamma_f32();
            [r, g, b]
        };
        let selected = match self.session.tree_selection {
            Some(TreeItemId::Pin(number)) => Some(number),
            _ => None,
        };
        let card = self.session.pin_card.as_ref().map(|c| &c.editing);
        let ppp = self.pixels_per_point();
        let hovered = self
            .pin_under_cursor()
            .filter(|_| self.session.pin_card.is_none());
        for (_, pin, at, lost) in self.drawn_pins() {
            let editing = matches!(card, Some(PinEditing::Existing(n)) if *n == pin.number);
            let color = if selected == Some(pin.number) || editing {
                self.user_settings.rendering.selection_color
            } else if lost {
                tone(ui_kit::tokens::DANGER)
            } else if pin.done {
                tone(ui_kit::tokens::TEXT3)
            } else {
                tone(ui_kit::tokens::ACCENT)
            };
            let (marks, labels) =
                core_document::numbered_pins([(at, pin.number as usize)], color, ppp);
            data.marks.extend(marks);
            data.labels.extend(labels);
            if hovered.is_some_and(|(_, n)| n == pin.number) {
                let mut words = pin.note.clone();
                if lost {
                    words.push_str(" (lost: what it was on is gone)");
                }
                data.labels.push(
                    core_document::ScreenSpaceLabel::new(
                        [at[0] + 24.0 * ppp, at[1] + 6.0 * ppp],
                        words,
                        color,
                        ui_kit::tokens::FONT_SM * ppp,
                    )
                    .pill(),
                );
            }
        }
        if let Some(view) = self.pin_card_view().filter(|v| v.new)
            && let Some(at) = view.at
        {
            let (marks, labels) = core_document::numbered_pins(
                [(at, view.number as usize)],
                self.user_settings.rendering.selection_color,
                ppp,
            );
            data.marks.extend(marks);
            data.labels.extend(labels);
        }
    }
}

/// A pin as the pin commands and an agent's context give it: its note
/// and state, where it stands now, and what it is on.
pub(crate) fn pin_json(document: &Document, body: BodyId, pin: &Pin) -> Value {
    let placed = document.pin_placed(body, pin);
    let mut out = json!({
        "number": pin.number,
        "note": pin.note,
        "author": pin.author.word(),
        "done": pin.done,
        "visible": !pin.hidden,
        "lost": placed.lost,
        "body": body.0.to_string(),
        "body_name": document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .map(|b| b.name.clone())
            .unwrap_or_default(),
        "on": match pin.on {
            core_document::PinSpot::Point => "point",
            core_document::PinSpot::Face(_) => "face",
            core_document::PinSpot::Edge { .. } => "edge",
        },
        "point": placed.at,
    });
    let Some(geometry) = document.imported_geometry(body).filter(|_| !placed.lost) else {
        return out;
    };
    let mesh = &geometry.mesh;
    if let Some(face) = placed.face {
        if let Some(surface) = mesh.face_surfaces.get(face as usize) {
            let mut described = crate::app::scripts::surface_json(*surface);
            described["words"] = json!(surface_words(surface));
            out["surface"] = described;
        }
        let maker = mesh
            .face_names
            .get(face as usize)
            .and_then(|name| document.face_maker(body, *name));
        if let Some(feature) = maker {
            out["feature"] = json!(feature.0.to_string());
            if let Some(node) = document.get_feature_meta(feature) {
                out["feature_name"] = json!(node.name);
            }
        }
    }
    if let Some(edge) = placed.edge
        && let Some(hit) = crate::app::edges::edge_hit(mesh, body.0, edge)
    {
        let mut described = json!({"length": hit.length_mm, "direction": hit.direction});
        if let Some(circle) = hit.circle {
            described["centre"] = json!(circle.center);
            described["radius"] = json!(circle.radius);
            described["normal"] = json!(circle.normal);
        }
        out["edge"] = described;
    }
    out
}

/// A face's surface in a few words: "cylinder, diameter 4.2, axis
/// (0, 0, 1) through (10, 5, 0)".
fn surface_words(surface: &kernel_api::FaceSurface) -> String {
    use kernel_api::FaceSurface as S;
    let v = |p: [f32; 3]| {
        let n = |x: f32| {
            let r = (x * 1000.0).round() / 1000.0;
            if r == 0.0 {
                "0".to_string()
            } else {
                r.to_string()
            }
        };
        format!("({}, {}, {})", n(p[0]), n(p[1]), n(p[2]))
    };
    let r = |x: f32| (x * 1000.0).round() / 1000.0;
    match *surface {
        S::Plane { origin, normal } => format!("plane facing {} through {}", v(normal), v(origin)),
        S::Cylinder {
            origin,
            axis,
            radius,
        } => format!(
            "cylinder, diameter {}, axis {} through {}",
            r(2.0 * radius),
            v(axis),
            v(origin)
        ),
        S::Cone {
            apex,
            axis,
            half_angle,
        } => format!(
            "cone, half angle {}°, axis {} from apex {}",
            r(half_angle.to_degrees()),
            v(axis),
            v(apex)
        ),
        S::Sphere { center, radius } => {
            format!("sphere, diameter {}, centre {}", r(2.0 * radius), v(center))
        }
        S::Torus {
            center,
            axis,
            major_radius,
            minor_radius,
        } => format!(
            "torus, radii {} and {}, axis {} through {}",
            r(major_radius),
            r(minor_radius),
            v(axis),
            v(center)
        ),
        S::Other => "free-form surface".to_string(),
    }
}
