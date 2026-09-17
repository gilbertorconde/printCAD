//! The daemon's own settings.
//!
//! These belong to the daemon, not to one client: changing them changes what
//! every application sees, and what the device itself does. They are the same
//! settings the daemon's configuration file holds, so [`Config::save`] is what
//! makes a change outlive the daemon.
//!
//! Floating-point settings travel as the bits of an `f32` packed into a word,
//! which is how the daemon reads them back out.

use std::time::Duration;

use crate::request::{
    REQ_CFG_RESET, REQ_CFG_RESTORE, REQ_CFG_SAVE, REQ_GCFG_AXISMAP, REQ_GCFG_BNACTION,
    REQ_GCFG_BNMAP, REQ_GCFG_DEADZONE, REQ_GCFG_GRAB, REQ_GCFG_INVERT, REQ_GCFG_KBMAP,
    REQ_GCFG_LED, REQ_GCFG_REPEAT, REQ_GCFG_SENS, REQ_GCFG_SENS_AXIS, REQ_GCFG_SERDEV,
    REQ_GCFG_SOCKET, REQ_GCFG_SWAPYZ, REQ_SCFG_AXISMAP, REQ_SCFG_BNACTION, REQ_SCFG_BNMAP,
    REQ_SCFG_DEADZONE, REQ_SCFG_GRAB, REQ_SCFG_INVERT, REQ_SCFG_KBMAP, REQ_SCFG_LED,
    REQ_SCFG_REPEAT, REQ_SCFG_SENS, REQ_SCFG_SENS_AXIS, REQ_SCFG_SERDEV, REQ_SCFG_SOCKET,
    REQ_SCFG_SWAPYZ,
};
use crate::{Client, Error};

/// What the device does when a button is pressed, beyond reporting it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonAction {
    /// Nothing; the button is reported and that is all.
    #[default]
    None,
    /// Puts sensitivity back to where it started.
    SensitivityReset,
    SensitivityIncrease,
    SensitivityDecrease,
    /// Holds the three rotation axes at zero.
    DisableRotation,
    /// Holds the three translation axes at zero.
    DisableTranslation,
    /// Passes only the largest axis through, so a gesture stays square.
    DominantAxis,
}

impl ButtonAction {
    fn from_code(code: i32) -> Result<ButtonAction, Error> {
        Ok(match code {
            0 => ButtonAction::None,
            1 => ButtonAction::SensitivityReset,
            2 => ButtonAction::SensitivityIncrease,
            3 => ButtonAction::SensitivityDecrease,
            4 => ButtonAction::DisableRotation,
            5 => ButtonAction::DisableTranslation,
            6 => ButtonAction::DominantAxis,
            _ => return Err(Error::Protocol("unknown button action")),
        })
    }

    fn code(self) -> i32 {
        match self {
            ButtonAction::None => 0,
            ButtonAction::SensitivityReset => 1,
            ButtonAction::SensitivityIncrease => 2,
            ButtonAction::SensitivityDecrease => 3,
            ButtonAction::DisableRotation => 4,
            ButtonAction::DisableTranslation => 5,
            ButtonAction::DominantAxis => 6,
        }
    }
}

/// The device's lamp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LedMode {
    Off,
    On,
    /// Lit while an application is listening.
    #[default]
    Auto,
}

impl LedMode {
    fn from_code(code: i32) -> Result<LedMode, Error> {
        Ok(match code {
            0 => LedMode::Off,
            1 => LedMode::On,
            2 => LedMode::Auto,
            _ => return Err(Error::Protocol("unknown LED mode")),
        })
    }

    fn code(self) -> i32 {
        match self {
            LedMode::Off => 0,
            LedMode::On => 1,
            LedMode::Auto => 2,
        }
    }
}

/// Settings shared by every client of the daemon.
///
/// Reached through [`Client::config`]. Changes take effect at once and last
/// until the daemon restarts; [`Config::save`] writes them to its
/// configuration file.
pub struct Config<'a> {
    pub(crate) client: &'a mut Client,
}

impl Config<'_> {
    /// How far a reading travels before the daemon passes it on, as a scale
    /// applied to every axis.
    pub fn sensitivity(&mut self) -> Result<f32, Error> {
        let reply = self.client.request(REQ_GCFG_SENS, &[])?;
        Ok(f32::from_bits(reply[1] as u32))
    }

    pub fn set_sensitivity(&mut self, sensitivity: f32) -> Result<(), Error> {
        self.client
            .request(REQ_SCFG_SENS, &[sensitivity.to_bits() as i32])
            .map(drop)
    }

    /// Per-axis scale: three translations, then three rotations.
    pub fn axis_sensitivity(&mut self) -> Result<[f32; 6], Error> {
        let reply = self.client.request(REQ_GCFG_SENS_AXIS, &[])?;
        let mut out = [0.0f32; 6];
        for (axis, word) in out.iter_mut().zip(&reply[1..7]) {
            *axis = f32::from_bits(*word as u32);
        }
        Ok(out)
    }

    pub fn set_axis_sensitivity(&mut self, sensitivity: [f32; 6]) -> Result<(), Error> {
        let args = sensitivity.map(|value| value.to_bits() as i32);
        self.client.request(REQ_SCFG_SENS_AXIS, &args).map(drop)
    }

    /// How far an axis must move before the daemon reports it at all — the
    /// setting that keeps a resting puck from drifting.
    pub fn dead_zone(&mut self, axis: u32) -> Result<i32, Error> {
        let reply = self.client.request(REQ_GCFG_DEADZONE, &[axis as i32])?;
        Ok(reply[2])
    }

    pub fn set_dead_zone(&mut self, axis: u32, threshold: i32) -> Result<(), Error> {
        self.client
            .request(REQ_SCFG_DEADZONE, &[axis as i32, threshold])
            .map(drop)
    }

    /// Which axes read backwards: three translations, then three rotations.
    pub fn inverted(&mut self) -> Result<[bool; 6], Error> {
        let reply = self.client.request(REQ_GCFG_INVERT, &[])?;
        let mut out = [false; 6];
        for (axis, word) in out.iter_mut().zip(&reply[1..7]) {
            *axis = *word != 0;
        }
        Ok(out)
    }

    pub fn set_inverted(&mut self, inverted: [bool; 6]) -> Result<(), Error> {
        let args = inverted.map(i32::from);
        self.client.request(REQ_SCFG_INVERT, &args).map(drop)
    }

    /// Where a device axis ends up in the six the daemon reports. `-1` drops
    /// the axis.
    pub fn axis_map(&mut self, device_axis: u32) -> Result<i32, Error> {
        let reply = self
            .client
            .request(REQ_GCFG_AXISMAP, &[device_axis as i32])?;
        Ok(reply[2])
    }

    pub fn set_axis_map(&mut self, device_axis: u32, mapped_to: i32) -> Result<(), Error> {
        self.client
            .request(REQ_SCFG_AXISMAP, &[device_axis as i32, mapped_to])
            .map(drop)
    }

    /// Which button number a device button is reported as.
    pub fn button_map(&mut self, device_button: u32) -> Result<u32, Error> {
        let reply = self
            .client
            .request(REQ_GCFG_BNMAP, &[device_button as i32])?;
        Ok(reply[2].max(0) as u32)
    }

    pub fn set_button_map(&mut self, device_button: u32, mapped_to: u32) -> Result<(), Error> {
        self.client
            .request(REQ_SCFG_BNMAP, &[device_button as i32, mapped_to as i32])
            .map(drop)
    }

    /// What the daemon itself does when a button is pressed.
    pub fn button_action(&mut self, device_button: u32) -> Result<ButtonAction, Error> {
        let reply = self
            .client
            .request(REQ_GCFG_BNACTION, &[device_button as i32])?;
        ButtonAction::from_code(reply[2])
    }

    pub fn set_button_action(
        &mut self,
        device_button: u32,
        action: ButtonAction,
    ) -> Result<(), Error> {
        self.client
            .request(REQ_SCFG_BNACTION, &[device_button as i32, action.code()])
            .map(drop)
    }

    /// The key a button types, as a keysym; `0` for none. The daemon refuses
    /// this unless it was built to emulate a keyboard.
    pub fn key_map(&mut self, device_button: u32) -> Result<u32, Error> {
        let reply = self
            .client
            .request(REQ_GCFG_KBMAP, &[device_button as i32])?;
        Ok(reply[2].max(0) as u32)
    }

    pub fn set_key_map(&mut self, device_button: u32, keysym: u32) -> Result<(), Error> {
        self.client
            .request(REQ_SCFG_KBMAP, &[device_button as i32, keysym as i32])
            .map(drop)
    }

    /// Whether the daemon swaps the two axes that older devices disagree on.
    pub fn swap_yz(&mut self) -> Result<bool, Error> {
        let reply = self.client.request(REQ_GCFG_SWAPYZ, &[])?;
        Ok(reply[1] != 0)
    }

    pub fn set_swap_yz(&mut self, swap: bool) -> Result<(), Error> {
        self.client
            .request(REQ_SCFG_SWAPYZ, &[i32::from(swap)])
            .map(drop)
    }

    pub fn led(&mut self) -> Result<LedMode, Error> {
        let reply = self.client.request(REQ_GCFG_LED, &[])?;
        LedMode::from_code(reply[1])
    }

    pub fn set_led(&mut self, mode: LedMode) -> Result<(), Error> {
        self.client.request(REQ_SCFG_LED, &[mode.code()]).map(drop)
    }

    /// Whether the daemon takes the device for itself, keeping its motion out
    /// of the rest of the desktop.
    pub fn grab_device(&mut self) -> Result<bool, Error> {
        let reply = self.client.request(REQ_GCFG_GRAB, &[])?;
        Ok(reply[1] != 0)
    }

    pub fn set_grab_device(&mut self, grab: bool) -> Result<(), Error> {
        self.client
            .request(REQ_SCFG_GRAB, &[i32::from(grab)])
            .map(drop)
    }

    /// The serial port to look for a device on, for the ones that predate USB.
    pub fn serial_device(&mut self) -> Result<String, Error> {
        self.client.request_string(REQ_GCFG_SERDEV)
    }

    pub fn set_serial_device(&mut self, path: &str) -> Result<(), Error> {
        self.client.send_string(REQ_SCFG_SERDEV, path)
    }

    /// How often the daemon repeats the last reading while the puck is held.
    /// `None` is the default: a reading arrives only when it changes.
    pub fn repeat_interval(&mut self) -> Result<Option<Duration>, Error> {
        let reply = self.client.request(REQ_GCFG_REPEAT, &[])?;
        Ok(u64::try_from(reply[1]).ok().map(Duration::from_millis))
    }

    pub fn set_repeat_interval(&mut self, interval: Option<Duration>) -> Result<(), Error> {
        let msec = match interval {
            Some(interval) => i32::try_from(interval.as_millis())
                .map_err(|_| Error::Protocol("a repeat interval that long does not fit"))?,
            None => -1,
        };
        self.client.request(REQ_SCFG_REPEAT, &[msec]).map(drop)
    }

    /// The socket the daemon listens on. A change takes effect when it
    /// restarts.
    pub fn socket_path(&mut self) -> Result<String, Error> {
        self.client.request_string(REQ_GCFG_SOCKET)
    }

    pub fn set_socket_path(&mut self, path: &str) -> Result<(), Error> {
        self.client.send_string(REQ_SCFG_SOCKET, path)
    }

    /// Writes the current settings to the daemon's configuration file.
    pub fn save(&mut self) -> Result<(), Error> {
        self.client.request(REQ_CFG_SAVE, &[]).map(drop)
    }

    /// Reloads the settings from that file, undoing unsaved changes.
    pub fn restore(&mut self) -> Result<(), Error> {
        self.client.request(REQ_CFG_RESTORE, &[]).map(drop)
    }

    /// Puts every setting back to the daemon's default.
    pub fn reset(&mut self) -> Result<(), Error> {
        self.client.request(REQ_CFG_RESET, &[]).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_actions_survive_the_wire() {
        for action in [
            ButtonAction::None,
            ButtonAction::SensitivityReset,
            ButtonAction::SensitivityIncrease,
            ButtonAction::SensitivityDecrease,
            ButtonAction::DisableRotation,
            ButtonAction::DisableTranslation,
            ButtonAction::DominantAxis,
        ] {
            assert_eq!(ButtonAction::from_code(action.code()).unwrap(), action);
        }
        assert!(ButtonAction::from_code(7).is_err());
        assert!(ButtonAction::from_code(-1).is_err());
    }

    #[test]
    fn led_modes_survive_the_wire() {
        for mode in [LedMode::Off, LedMode::On, LedMode::Auto] {
            assert_eq!(LedMode::from_code(mode.code()).unwrap(), mode);
        }
        assert!(LedMode::from_code(3).is_err());
    }
}
