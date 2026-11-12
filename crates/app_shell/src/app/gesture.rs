//! What the host's view holds between a press and its release, and the one
//! place that ends it all: focus loss, a tab leaving the screen, Escape
//! and a task closing each go through [`PrintCadApp::end_gestures`], which
//! lets go of every host drag and, where the release will never come,
//! cancels the active bench's gesture too.

use core_document::ArrayId;
use glam::Vec2;
use winit::event::{ElementState, MouseButton, WindowEvent};

use super::box_select::BoxDrag;
use super::marking::MarkGesture;
use super::radial::{Press, Radial};
use super::scalar_arrow::{HandleEvent, HandleStep, Held, handle_event};
use crate::PrintCadApp;

/// A left-button gesture of the host's: one at a time, since the press
/// that starts one is taken by it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HostGesture {
    /// A selection box being dragged over empty space.
    Box(BoxDrag),
    /// A stroke marking (or rubbing out) a scan's surface.
    Mark(MarkGesture),
    /// A scalar handle held: an array row's spacing or the clipping
    /// plane's offset.
    Handle(Held<HandleTarget>),
}

/// What a held scalar handle drags.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HandleTarget {
    /// The spacing of the row the array task lays out.
    Array(ArrayId),
    /// The clipping plane's offset, with the formula that set it at the
    /// press, which a cancel puts back.
    Section(Option<String>),
}

/// Why the gestures end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GestureEnd {
    /// Escape: a handle puts back the press's value. The bench keeps the
    /// key for its own gesture.
    Escape,
    /// Focus loss or the tab leaving the screen, where no release comes: a
    /// handle puts back the press's value and the active bench's gesture
    /// is cancelled.
    Lost,
    /// A task closed: everything is let go and nothing is set, the task
    /// having settled what the handle dragged.
    TaskClosed,
}

/// Every host gesture of a tab: the left button's, and the radial menu's
/// right press, which may be held beside it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Gestures {
    pub held: Option<HostGesture>,
    /// A right press waiting to open the radial menu.
    pub radial_press: Option<Press>,
    /// The radial menu, open while its press is held.
    pub radial: Option<Radial>,
}

/// What ending the gestures did.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ended {
    /// Something was held.
    pub any: bool,
    /// The value a cancelled handle puts back on what it dragged.
    pub set: Option<(HandleTarget, f32)>,
}

impl Gestures {
    /// A step of the held handle's gesture: a press takes hold (in place of
    /// whatever was held), moves drag, a release keeps and a cancel puts
    /// back.
    pub(super) fn handle(&mut self, event: HandleEvent<HandleTarget>) -> HandleStep<HandleTarget> {
        let mut held = match self.held.take() {
            Some(HostGesture::Handle(held)) => Some(held),
            other => {
                self.held = other;
                None
            }
        };
        let step = handle_event(&mut held, event);
        if let Some(held) = held {
            self.held = Some(HostGesture::Handle(held));
        }
        step
    }

    /// The handle held, if any.
    pub(crate) fn handle_held(&self) -> Option<&HandleTarget> {
        match &self.held {
            Some(HostGesture::Handle(held)) => Some(&held.of),
            _ => None,
        }
    }

    pub(crate) fn boxed(&self) -> Option<&BoxDrag> {
        match &self.held {
            Some(HostGesture::Box(drag)) => Some(drag),
            _ => None,
        }
    }

    pub(crate) fn boxed_mut(&mut self) -> Option<&mut BoxDrag> {
        match &mut self.held {
            Some(HostGesture::Box(drag)) => Some(drag),
            _ => None,
        }
    }

    /// Take the box, its button let go.
    pub(crate) fn take_box(&mut self) -> Option<BoxDrag> {
        match self.held.take() {
            Some(HostGesture::Box(drag)) => Some(drag),
            other => {
                self.held = other;
                None
            }
        }
    }

    pub(crate) fn mark(&self) -> Option<&MarkGesture> {
        match &self.held {
            Some(HostGesture::Mark(gesture)) => Some(gesture),
            _ => None,
        }
    }

    pub(crate) fn mark_mut(&mut self) -> Option<&mut MarkGesture> {
        match &mut self.held {
            Some(HostGesture::Mark(gesture)) => Some(gesture),
            _ => None,
        }
    }

    /// Take the marking stroke, its button let go or its tool put away.
    pub(crate) fn take_mark(&mut self) -> Option<MarkGesture> {
        match self.held.take() {
            Some(HostGesture::Mark(gesture)) => Some(gesture),
            other => {
                self.held = other;
                None
            }
        }
    }

    /// End every gesture: nothing is held afterwards.
    pub(crate) fn end(&mut self, why: GestureEnd) -> Ended {
        let radial = self.radial_press.take().is_some() | self.radial.take().is_some();
        let step = self.handle(match why {
            GestureEnd::TaskClosed => HandleEvent::Gone,
            GestureEnd::Escape | GestureEnd::Lost => HandleEvent::Cancel,
        });
        let other = self.held.take().is_some();
        Ended {
            any: radial || step.taken || other,
            set: step.set,
        }
    }
}

impl PrintCadApp {
    /// End every host gesture of the tab on screen, and on
    /// [`GestureEnd::Lost`] the active bench's pointer gesture and the
    /// camera's holds. Whether anything host side was held.
    pub(crate) fn end_gestures(&mut self, why: GestureEnd) -> bool {
        let ended = self.session.gestures.end(why);
        if let Some((target, value)) = ended.set {
            self.set_handle_value(target, value, true);
        }
        if why == GestureEnd::Lost {
            self.input.side_gesture = false;
            self.session.view.camera.forget_released([false; 3]);
            if let Some(side) = self.session.view.side_camera.as_mut() {
                side.forget_released([false; 3]);
            }
            let wb_id = self.active_workbench_id();
            let params = self.interaction_ctx_params();
            if let Some(((), outcome)) = self.with_workbench_ctx(&wb_id, params, |wb, ctx| {
                wb.cancel_pointer_gesture(ctx);
            }) {
                self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Interaction);
                self.redraw_needed = true;
            }
        }
        self.redraw_needed |= ended.any;
        ended.any
    }

    /// The view's scalar handles (the array task's spacing and the
    /// clipping plane's offset): a press on one takes hold of it, moves
    /// drag it and the release lets go. Whether it took the event.
    pub(crate) fn handle_input(&mut self, event: &WindowEvent) -> bool {
        self.let_go_of_a_row_gone();
        let cursor = self.input.cursor_in_viewport.map(|(x, y)| Vec2::new(x, y));
        let event = match event {
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Pressed,
                ..
            } => HandleEvent::Press(cursor.and_then(|at| {
                self.array_handle()
                    .filter(|h| h.is_under(at))
                    .map(|h| h.held(at))
                    .or_else(|| {
                        let formula = self.session.view.section.as_ref()?.formula.clone();
                        self.section_handle()
                            .filter(|h| h.is_under(at))
                            .map(|h| h.held(at, formula))
                    })
            })),
            WindowEvent::CursorMoved { .. } => HandleEvent::Move(cursor),
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Released,
                ..
            } => HandleEvent::Release,
            _ => return false,
        };
        self.handle_event(event)
    }

    pub(super) fn handle_event(&mut self, event: HandleEvent<HandleTarget>) -> bool {
        let cancel = event == HandleEvent::Cancel;
        let step = self.session.gestures.handle(event);
        if let Some((target, value)) = step.set {
            self.set_handle_value(target, value, cancel);
        }
        self.redraw_needed |= step.taken;
        step.taken
    }

    /// Set what a handle drags: a cancel puts back the formula the press
    /// found too.
    fn set_handle_value(&mut self, target: HandleTarget, value: f32, cancel: bool) {
        match target {
            HandleTarget::Array(array) => self.set_array_spacing(array, value),
            HandleTarget::Section(formula) => self.set_section_offset(value, formula, cancel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::scalar_arrow::{ScalarDrag, UNBOUNDED};

    fn held_nothing(gestures: &Gestures) -> bool {
        gestures.held.is_none() && gestures.radial_press.is_none() && gestures.radial.is_none()
    }

    fn hold(of: HandleTarget) -> HostGesture {
        HostGesture::Handle(Held {
            of,
            drag: ScalarDrag {
                press: Vec2::ZERO,
                pixels_per_unit: Vec2::X,
                value: 5.0,
                step: 0.1,
                stops: UNBOUNDED,
            },
        })
    }

    #[test]
    fn every_end_leaves_nothing_held_and_only_a_cancel_puts_a_handle_back() {
        let array = ArrayId::new();
        let section = HandleTarget::Section(Some("Spreadsheet.cut".into()));
        let boxed = HostGesture::Box(BoxDrag::at(Vec2::new(3.0, 4.0)));
        let mark = HostGesture::Mark(MarkGesture {
            erase: false,
            points: vec![Vec2::ONE],
        });
        let press = Press {
            at: std::time::Instant::now(),
            centre: Vec2::ONE,
        };
        let put_back = |target: HandleTarget| Some((target, 5.0));
        use GestureEnd::*;
        // Each case: what is held, the radial press beside it, why it ends,
        // and what ending it says.
        let cases = [
            (None, None, Lost, false, None),
            (Some(boxed.clone()), None, Escape, true, None),
            (Some(mark), None, Lost, true, None),
            (None, Some(press), Escape, true, None),
            (Some(boxed), Some(press), TaskClosed, true, None),
            (
                Some(hold(HandleTarget::Array(array))),
                None,
                Lost,
                true,
                put_back(HandleTarget::Array(array)),
            ),
            (
                Some(hold(section.clone())),
                Some(press),
                Escape,
                true,
                put_back(section.clone()),
            ),
            (Some(hold(section)), None, TaskClosed, true, None),
        ];
        for (held, radial_press, why, any, set) in cases {
            let mut gestures = Gestures {
                held: held.clone(),
                radial_press,
                radial: None,
            };
            let ended = gestures.end(why);
            assert_eq!(ended, Ended { any, set }, "{held:?} {why:?}");
            assert!(held_nothing(&gestures), "{held:?} {why:?}");
        }
    }

    #[test]
    fn a_handle_press_and_the_box_or_stroke_share_the_slot() {
        let mut gestures = Gestures {
            held: Some(HostGesture::Box(BoxDrag::at(Vec2::ZERO))),
            ..Gestures::default()
        };
        // A handle's moves and release pass a box by, which stays held.
        let array = ArrayId::new();
        assert!(!gestures.handle(HandleEvent::Release).taken);
        assert!(gestures.take_mark().is_none());
        assert!(gestures.boxed().is_some());
        // A press on a handle takes the slot.
        let HostGesture::Handle(held) = hold(HandleTarget::Array(array)) else {
            unreachable!()
        };
        assert!(gestures.handle(HandleEvent::Press(Some(held))).taken);
        assert!(gestures.take_box().is_none());
        assert_eq!(gestures.handle_held(), Some(&HandleTarget::Array(array)));
        assert!(gestures.handle(HandleEvent::Release).taken);
        assert!(held_nothing(&gestures));
    }
}
