//! The radial menu: a right press held still over the view opens the
//! active workbench's slices round the cursor; moving toward one and
//! letting go starts it, letting go in the middle starts nothing. A quick
//! right click is still the view's menu and a right drag still pans.
//!
//! The slices are, in order of who decides: the user's own for the bench
//! (Preferences › Radial menu, `UserSettings::radial`), the bench's
//! (`WorkbenchDescriptor::radial`, a package's `Registration::radial`), or
//! its most used tools (`UserSettings::tool_uses`). Each is a tool of the
//! bench, started as its button would, or any command, run with no
//! arguments.

use std::time::{Duration, Instant};

use core_document::{CommandArgs, RADIAL_SLOTS, ToolDescriptor, WorkbenchId};
use glam::Vec2;
use winit::event_loop::ActiveEventLoop;

use crate::PrintCadApp;
use crate::app_log;

/// How long a right press stays still before the menu opens.
pub(crate) const HOLD: Duration = Duration::from_millis(300);

/// The middle, in logical pixels, where letting go starts nothing.
const DEAD_ZONE: f32 = 24.0;

/// A right press waiting to become the menu.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Press {
    pub at: Instant,
    /// Where it went down, in the view's physical pixels.
    pub centre: Vec2,
}

/// One slice: what it starts and how it shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Slice {
    pub id: String,
    pub label: String,
    pub icon: &'static str,
}

/// The open menu.
#[derive(Debug, Clone)]
pub(crate) struct Radial {
    /// Its middle, in the view's physical pixels.
    pub centre: Vec2,
    pub slices: Vec<Slice>,
}

impl Radial {
    /// The slice the cursor at `cursor` (view physical pixels) points to,
    /// with `scale` physical pixels a logical one: slices go clockwise
    /// from the top, each the same share of the turn.
    pub(crate) fn pointed(&self, cursor: Vec2, scale: f32) -> Option<usize> {
        pointed(self.slices.len(), cursor - self.centre, scale)
    }
}

fn pointed(count: usize, offset: Vec2, scale: f32) -> Option<usize> {
    if count == 0 || offset.length() < DEAD_ZONE * scale {
        return None;
    }
    // Clockwise from the top, with screen Y down.
    let turn = offset.x.atan2(-offset.y).rem_euclid(std::f32::consts::TAU);
    let share = std::f32::consts::TAU / count as f32;
    Some(((turn + share / 2.0) / share) as usize % count)
}

/// The ids the menu offers for `bench`: the user's, the bench's, or its
/// tools by how often each was started, then in their own order.
pub(crate) fn chosen_ids(
    bench: &WorkbenchId,
    declared: &[String],
    tools: &[ToolDescriptor],
    settings: &settings::UserSettings,
) -> Vec<String> {
    if let Some(own) = settings.radial.get(bench.as_str()) {
        return own.iter().take(RADIAL_SLOTS).cloned().collect();
    }
    if !declared.is_empty() {
        return declared.iter().take(RADIAL_SLOTS).cloned().collect();
    }
    let mut ranked: Vec<&ToolDescriptor> = tools.iter().filter(|t| t.planned.is_none()).collect();
    let uses = |t: &ToolDescriptor| settings.tool_uses.get(&t.id).copied().unwrap_or(0);
    ranked.sort_by_key(|t| std::cmp::Reverse(uses(t)));
    ranked
        .into_iter()
        .take(RADIAL_SLOTS)
        .map(|t| t.id.clone())
        .collect()
}

impl PrintCadApp {
    /// The slices for the active bench, each named and drawn as its tool
    /// or command is; an id that is neither is left out.
    fn radial_slices(&self) -> Vec<Slice> {
        let bench = &self.session.active_workbench.0;
        let tools = self
            .registry
            .tools_for(bench)
            .map(<[_]>::to_vec)
            .unwrap_or_default();
        let declared = self
            .registry
            .descriptor(bench)
            .map(|d| d.radial.clone())
            .unwrap_or_default();
        let specs = crate::app::scripts::command_specs(&self.registry);
        chosen_ids(bench, &declared, &tools, &self.user_settings)
            .into_iter()
            .filter_map(|id| match tools.iter().find(|t| t.id == id) {
                Some(tool) if tool.planned.is_none() => Some(Slice {
                    label: tool.label.clone(),
                    icon: tool.icon.unwrap_or("more"),
                    id,
                }),
                Some(_) => None,
                None => specs.iter().find(|s| s.id == id).map(|spec| Slice {
                    label: spec.summary.clone(),
                    icon: "console",
                    id,
                }),
            })
            .collect()
    }

    /// A right press on the view: the menu waits for it to stay still.
    pub(crate) fn radial_pressed(&mut self, centre: Vec2) {
        self.radial_press = Some(Press {
            at: Instant::now(),
            centre,
        });
    }

    /// The cursor moved to `cursor` in the view: a press that moved is a
    /// pan, never the menu.
    pub(crate) fn radial_moved(&mut self, cursor: Vec2) {
        let scale = self.session.camera.pixels_per_point();
        let reach = self.user_settings.camera.click_drag_threshold_px * scale;
        if self
            .radial_press
            .is_some_and(|press| press.centre.distance(cursor) > reach)
        {
            self.radial_press = None;
        }
        if self.radial.is_some() {
            self.redraw_needed = true;
        }
    }

    /// Open the menu once its press has been held still long enough; the
    /// view lets go of the pan the press began.
    pub(crate) fn radial_tick(&mut self) {
        let Some(press) = self.radial_press else {
            return;
        };
        if press.at.elapsed() < HOLD {
            return;
        }
        self.radial_press = None;
        let slices = self.radial_slices();
        if slices.is_empty() {
            return;
        }
        self.session.camera.let_go_of_pan();
        self.radial = Some(Radial {
            centre: press.centre,
            slices,
        });
        self.redraw_needed = true;
    }

    /// The right button let go: start the slice pointed at, if the menu is
    /// open. Whether the release was the menu's.
    pub(crate) fn radial_released(&mut self, event_loop: &ActiveEventLoop) -> bool {
        self.radial_press = None;
        let Some(radial) = self.radial.take() else {
            return false;
        };
        self.redraw_needed = true;
        let cursor = self
            .cursor_in_viewport
            .map_or(radial.centre, |(x, y)| Vec2::new(x, y));
        let scale = self.session.camera.pixels_per_point();
        let Some(slice) = radial.pointed(cursor, scale).map(|i| &radial.slices[i]) else {
            return true;
        };
        let bench = self.session.active_workbench.0.clone();
        let is_tool = self
            .registry
            .tools_for(&bench)
            .is_ok_and(|tools| tools.iter().any(|t| t.id == slice.id));
        let (id, args) = if is_tool {
            let mut args = CommandArgs::new();
            args.insert("id".into(), slice.id.clone().into());
            ("app.tool".to_string(), args)
        } else {
            (slice.id.clone(), CommandArgs::new())
        };
        if let Err(err) = self.execute_command(&id, args, event_loop) {
            app_log::warn(format!("{}: {err}", slice.label));
        }
        true
    }

    /// Count each tool started since the last frame, for the slices of a
    /// bench that names none.
    pub(crate) fn count_tool_uses(&mut self) {
        let now: Vec<String> = self
            .session
            .active_tool
            .active_ids
            .iter()
            .cloned()
            .collect();
        let started: Vec<String> = now
            .iter()
            .filter(|id| !self.tools_last_frame.contains(*id))
            .cloned()
            .collect();
        self.tools_last_frame = now;
        if started.is_empty() {
            return;
        }
        for id in started {
            *self.user_settings.tool_uses.entry(id).or_default() += 1;
        }
        if let Err(err) = self.settings_store.save(&self.user_settings) {
            app_log::warn(format!("Tool counts not saved: {err}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slices go clockwise from the top; the middle points at none.
    #[test]
    fn a_direction_points_at_its_slice() {
        let at = |x: f32, y: f32| pointed(8, Vec2::new(x, y), 1.0);
        assert_eq!(at(0.0, -50.0), Some(0), "up");
        assert_eq!(at(50.0, -50.0), Some(1), "up and right");
        assert_eq!(at(50.0, 0.0), Some(2), "right");
        assert_eq!(at(0.0, 50.0), Some(4), "down");
        assert_eq!(at(-50.0, 0.0), Some(6), "left");
        assert_eq!(at(5.0, 5.0), None, "the middle");
        assert_eq!(
            pointed(3, Vec2::new(52.0, 30.0), 1.0),
            Some(1),
            "three slices"
        );
    }

    /// The user's slices win over the bench's, which win over its most
    /// used tools.
    #[test]
    fn the_user_then_the_bench_then_use_choose_the_slices() {
        let tool = |id: &str| ToolDescriptor::new_action(id, id, None::<String>);
        let tools = [tool("b.one"), tool("b.two"), tool("b.three")];
        let bench = WorkbenchId::from("b");
        let mut settings = settings::UserSettings::default();
        settings.tool_uses.insert("b.three".into(), 4);
        settings.tool_uses.insert("b.two".into(), 1);
        assert_eq!(
            chosen_ids(&bench, &[], &tools, &settings),
            ["b.three", "b.two", "b.one"]
        );
        let declared = vec!["b.one".to_string()];
        assert_eq!(chosen_ids(&bench, &declared, &tools, &settings), ["b.one"]);
        settings
            .radial
            .insert("b".into(), vec!["b.two".into(), "file.save".into()]);
        assert_eq!(
            chosen_ids(&bench, &declared, &tools, &settings),
            ["b.two", "file.save"]
        );
    }
}
