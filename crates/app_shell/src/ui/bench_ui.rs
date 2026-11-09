//! `PRINTCAD_BENCH_UI`: pointer steps played into the window once the
//! scene has settled, so a capture shows a panel in a state only clicks
//! reach (a fold opened, a row selected, a menu open). Steps are
//! `;`-separated, in logical points from the window's corner:
//! `click:x,y`, `right:x,y` (a secondary click), `move:x,y` and `wait:n`
//! (frames).

use std::collections::VecDeque;

use egui::{Event, PointerButton, Pos2, RawInput};

/// Frames between two steps, so each lands on a panel the last one drew.
const GAP: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Click(PointerButton, Pos2),
    Move(Pos2),
    Wait(u32),
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
}

impl BenchUi {
    /// The steps `PRINTCAD_BENCH_UI` names, if it is set.
    pub(crate) fn from_env() -> Option<Self> {
        let spec = std::env::var("PRINTCAD_BENCH_UI").ok()?;
        Some(Self {
            steps: parse(&spec),
            ..Self::default()
        })
    }

    /// Start playing once the scene has settled.
    pub(crate) fn arm(&mut self) {
        self.armed = true;
    }

    /// Steps still to play: a capture waits for them.
    pub(crate) fn pending(&self) -> bool {
        !self.steps.is_empty() || self.held.is_some() || self.gap > 0
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
            steps: parse("click:10,20; move:5,5;bogus;wait:2"),
            ..Default::default()
        };
        assert_eq!(ui.steps.len(), 3, "an unknown step is left out");
        let mut raw = RawInput::default();
        ui.feed(&mut raw);
        assert!(raw.events.is_empty(), "nothing before the scene settles");
        ui.arm();
        let mut frames = Vec::new();
        while ui.pending() {
            let mut raw = RawInput::default();
            ui.feed(&mut raw);
            frames.push(raw.events);
        }
        let events: Vec<&Event> = frames.iter().flatten().collect();
        let at = Pos2::new(10.0, 20.0);
        assert_eq!(
            events,
            [
                &Event::PointerMoved(at),
                &press(PointerButton::Primary, at, true),
                &press(PointerButton::Primary, at, false),
                &Event::PointerMoved(Pos2::new(5.0, 5.0)),
            ]
        );
        assert!(frames[1].len() == 1, "the release comes a frame later");
    }
}
