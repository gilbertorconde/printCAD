//! `PRINTCAD_BENCH_UI`: pointer steps played into the window once the
//! scene has settled, so a capture shows a panel in a state only clicks
//! reach (a fold opened, a row selected, a menu open). Steps are
//! `;`-separated, in logical points from the window's corner:
//! `click:x,y`, `right:x,y` (a secondary click), `move:x,y` and `wait:n`
//! (frames), `view:fx,fy`: a click in the viewport at that fraction of
//! it, handed to the active bench as the viewport's own clicks are,
//! `type:text`, typed into the field with the keys, and `key:name`, a
//! key pressed (`Enter`, `Escape`, `Shift+D`).

use std::collections::VecDeque;

use egui::{Event, PointerButton, Pos2, RawInput};

/// Frames between two steps, so each lands on a panel the last one drew.
const GAP: u32 = 6;

#[derive(Debug, Clone, PartialEq)]
enum Step {
    Click(PointerButton, Pos2),
    Move(Pos2),
    Wait(u32),
    View(Pos2),
    Type(String),
    Key(egui::Modifiers, egui::Key),
}

#[derive(Debug, Default)]
pub(crate) struct BenchUi {
    steps: VecDeque<Step>,
    /// The window is ready for the steps.
    armed: bool,
    /// Frames left before the next step.
    gap: u32,
    /// A press to let go of on the next frame.
    held: Option<(PointerButton, Pos2)>,
    /// A viewport click for the app to make this frame.
    view: Option<Pos2>,
}

impl BenchUi {
    /// The steps `PRINTCAD_BENCH_UI` names, if it is set.
    pub(crate) fn from_env() -> Option<Self> {
        let spec = crate::app::bench_hooks::hooks().ui.as_deref()?;
        Some(Self {
            steps: parse(spec),
            ..Self::default()
        })
    }

    /// Start playing once the scene has settled.
    pub(crate) fn arm(&mut self) {
        self.armed = true;
    }

    /// Steps still to play: a capture waits for them.
    pub(crate) fn pending(&self) -> bool {
        !self.steps.is_empty() || self.held.is_some() || self.gap > 0 || self.view.is_some()
    }

    /// This frame's viewport click, as fractions of the viewport.
    pub(crate) fn take_view_click(&mut self) -> Option<Pos2> {
        self.view.take()
    }

    /// This frame's part of the steps, added to the window's input.
    pub(crate) fn feed(&mut self, raw: &mut RawInput) {
        if !self.armed {
            return;
        }
        if let Some((button, pos)) = self.held.take() {
            raw.events.push(press(button, pos, false));
            self.gap = GAP;
            return;
        }
        if self.gap > 0 {
            self.gap -= 1;
            return;
        }
        match self.steps.pop_front() {
            Some(Step::Move(pos)) => {
                raw.events.push(Event::PointerMoved(pos));
                self.gap = GAP;
            }
            Some(Step::Click(button, pos)) => {
                raw.events.push(Event::PointerMoved(pos));
                raw.events.push(press(button, pos, true));
                self.held = Some((button, pos));
            }
            Some(Step::Wait(frames)) => self.gap = frames,
            Some(Step::View(at)) => {
                self.view = Some(at);
                self.gap = GAP;
            }
            Some(Step::Type(text)) => {
                raw.events.push(Event::Text(text));
                self.gap = GAP;
            }
            Some(Step::Key(modifiers, key)) => {
                for pressed in [true, false] {
                    raw.events.push(Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    });
                }
                self.gap = GAP;
            }
            None => {}
        }
    }
}

fn press(button: PointerButton, pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Default::default(),
    }
}

/// A `key:` step: a key's name after any of `Shift+`, `Ctrl+`, `Alt+`.
fn key(spec: &str) -> Option<Step> {
    let mut modifiers = egui::Modifiers::NONE;
    let mut parts: Vec<&str> = spec.trim().split('+').collect();
    let name = parts.pop()?;
    for part in parts {
        match part {
            "Shift" => modifiers.shift = true,
            "Ctrl" => modifiers.ctrl = true,
            "Alt" => modifiers.alt = true,
            _ => return None,
        }
    }
    Some(Step::Key(modifiers, egui::Key::from_name(name)?))
}

fn parse(spec: &str) -> VecDeque<Step> {
    let point = |text: &str| {
        let (x, y) = text.split_once(',')?;
        Some(Pos2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
    };
    spec.split(';')
        .filter_map(|step| {
            let (kind, rest) = step.trim().split_once(':')?;
            match kind {
                "click" => Some(Step::Click(PointerButton::Primary, point(rest)?)),
                "right" => Some(Step::Click(PointerButton::Secondary, point(rest)?)),
                "move" => Some(Step::Move(point(rest)?)),
                "wait" => Some(Step::Wait(rest.trim().parse().ok()?)),
                "view" => Some(Step::View(point(rest)?)),
                "type" => Some(Step::Type(rest.to_string())),
                "key" => key(rest),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_play_one_at_a_time_once_armed() {
        let mut ui = BenchUi {
            steps: parse("click:10,20; move:5,5;bogus;wait:2;view:0.5,0.25;type:12;key:Enter"),
            ..Default::default()
        };
        assert_eq!(ui.steps.len(), 6, "an unknown step is left out");
        let mut raw = RawInput::default();
        ui.feed(&mut raw);
        assert!(raw.events.is_empty(), "nothing before the scene settles");
        ui.arm();
        let mut frames = Vec::new();
        let mut views = Vec::new();
        while ui.pending() {
            let mut raw = RawInput::default();
            ui.feed(&mut raw);
            frames.push(raw.events);
            views.extend(ui.take_view_click());
        }
        assert_eq!(views, [Pos2::new(0.5, 0.25)], "the viewport click, once");
        let events: Vec<&Event> = frames.iter().flatten().collect();
        let at = Pos2::new(10.0, 20.0);
        assert_eq!(
            events,
            [
                &Event::PointerMoved(at),
                &press(PointerButton::Primary, at, true),
                &press(PointerButton::Primary, at, false),
                &Event::PointerMoved(Pos2::new(5.0, 5.0)),
                &Event::Text("12".into()),
                &Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                },
                &Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: Default::default(),
                },
            ]
        );
        assert!(frames[1].len() == 1, "the release comes a frame later");
    }
}
