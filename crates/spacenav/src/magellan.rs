//! The Magellan protocol, the other way a device reaches an application.
//!
//! Where the daemon backend reads a UNIX socket, this one rides the display
//! server: the driver owns a window named "Magellan Window", advertises it in
//! a property on the root window, and sends each listening application client
//! messages carrying the axes. 3Dconnexion's own driver speaks only this, and
//! spacenavd speaks it too when it has a display to connect to.
//!
//! The client here opens its own display connection and registers a window of
//! its own, so it reads events on a thread of its own without touching the
//! application's windowing.
//!
//! Only motion, buttons and device removal cross this protocol. There is no
//! device query and no configuration; for those, use the daemon backend.

use std::collections::VecDeque;
use std::os::fd::{AsRawFd, RawFd};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use crate::{Error, Event, Motion};

/// The name the driver's window carries, and the only way to be sure the
/// window a property points at is really the driver's.
const DAEMON_WINDOW_NAME: &[u8] = b"Magellan Window";

/// Tells the driver which window to send events to.
const CMD_APP_WINDOW: u16 = 27695;
/// Scales what it sends this application.
const CMD_APP_SENS: u16 = 27696;

/// How often a timed read looks for an event. The display connection has no
/// timed wait of its own, so the wait is in steps.
const POLL_STEP: Duration = Duration::from_millis(20);

fn io(err: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::Io(std::io::Error::other(err))
}

struct Atoms {
    motion: u32,
    button_press: u32,
    button_release: u32,
    device_disconnect: u32,
    command: u32,
}

impl Atoms {
    fn intern(conn: &RustConnection) -> Result<Atoms, Error> {
        let atom = |name: &[u8]| -> Result<u32, Error> {
            Ok(conn
                .intern_atom(true, name)
                .map_err(io)?
                .reply()
                .map_err(io)?
                .atom)
        };
        let atoms = Atoms {
            motion: atom(b"MotionEvent")?,
            button_press: atom(b"ButtonPressEvent")?,
            button_release: atom(b"ButtonReleaseEvent")?,
            device_disconnect: atom(b"DeviceDisconnectEvent")?,
            command: atom(b"CommandEvent")?,
        };
        // Interning with `only_if_exists` yields nothing when the driver has
        // never run on this display.
        if atoms.motion == 0 || atoms.button_press == 0 || atoms.command == 0 {
            return Err(Error::Protocol("no driver on this display"));
        }
        Ok(atoms)
    }
}

/// A connection to a driver speaking the Magellan protocol.
pub struct Client {
    conn: RustConnection,
    window: Window,
    driver_window: Window,
    atoms: Atoms,
    queued: VecDeque<Event>,
}

impl Client {
    /// Connects using the display the environment names.
    pub fn connect() -> Result<Client, Error> {
        Client::connect_on(None)
    }

    /// Connects to a named display.
    pub fn connect_on(display: Option<&str>) -> Result<Client, Error> {
        let (conn, screen_num) = x11rb::connect(display).map_err(io)?;
        let atoms = Atoms::intern(&conn)?;
        let root = conn.setup().roots[screen_num].root;
        let driver_window = find_driver_window(&conn, root, atoms.command)?;

        // A window of our own, never mapped: it exists only as an address for
        // the driver to send to.
        let window = conn.generate_id().map_err(io)?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
        )
        .map_err(io)?;

        let client = Client {
            conn,
            window,
            driver_window,
            atoms,
            queued: VecDeque::new(),
        };
        client.command(CMD_APP_WINDOW, [(window >> 16) as u16, window as u16])?;
        Ok(client)
    }

    /// Waits for the next event.
    pub fn read_blocking(&mut self) -> Result<Event, Error> {
        loop {
            if let Some(event) = self.queued.pop_front() {
                return Ok(event);
            }
            let event = self.conn.wait_for_event().map_err(io)?;
            self.absorb(event);
        }
    }

    /// Waits for the next event, giving up after `timeout`.
    ///
    /// The display connection offers no timed wait, so this looks every
    /// [`POLL_STEP`] until the deadline passes.
    pub fn read_timeout(&mut self, timeout: Duration) -> Result<Option<Event>, Error> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(event) = self.poll()? {
                return Ok(Some(event));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            std::thread::sleep(POLL_STEP.min(left));
        }
    }

    /// Takes the next event if one is already waiting.
    pub fn poll(&mut self) -> Result<Option<Event>, Error> {
        loop {
            if let Some(event) = self.queued.pop_front() {
                return Ok(Some(event));
            }
            match self.conn.poll_for_event().map_err(io)? {
                Some(event) => self.absorb(event),
                None => return Ok(None),
            }
        }
    }

    /// Scales what the driver sends this application, leaving others alone.
    pub fn set_sensitivity(&mut self, sensitivity: f32) -> Result<(), Error> {
        let bits = sensitivity.to_ne_bytes();
        let low = u16::from_ne_bytes([bits[0], bits[1]]);
        let high = u16::from_ne_bytes([bits[2], bits[3]]);
        self.command(CMD_APP_SENS, [low, high])
    }

    /// The display connection, for callers that run their own readiness loop.
    #[must_use]
    pub fn as_raw_fd(&self) -> RawFd {
        self.conn.stream().as_raw_fd()
    }

    /// Sends one command to the driver.
    fn command(&self, command: u16, args: [u16; 2]) -> Result<(), Error> {
        let mut data = [0u16; 10];
        data[0] = args[0];
        data[1] = args[1];
        data[2] = command;
        let event = ClientMessageEvent::new(16, self.window, self.atoms.command, data);
        self.conn
            .send_event(false, self.driver_window, EventMask::NO_EVENT, event)
            .map_err(io)?;
        self.conn.flush().map_err(io)?;
        Ok(())
    }

    /// Keeps the events this protocol carries and drops the rest.
    fn absorb(&mut self, event: XEvent) {
        let XEvent::ClientMessage(message) = event else {
            return;
        };
        if message.format != 16 {
            return;
        }
        if let Some(event) = decode(&self.atoms, message.type_, message.data.as_data16()) {
            self.queued.push_back(event);
        }
    }
}

/// Reads the driver's window out of the root window's property, and makes
/// sure it is really the driver's.
fn find_driver_window(conn: &RustConnection, root: Window, command: u32) -> Result<Window, Error> {
    let property = conn
        .get_property(false, root, command, AtomEnum::ANY, 0, 1)
        .map_err(io)?
        .reply()
        .map_err(io)?;
    let window = property
        .value32()
        .and_then(|mut values| values.next())
        .ok_or(Error::Protocol("no driver window on this display"))?;

    let name = conn
        .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 32)
        .map_err(io)?
        .reply()
        .map_err(io)?;
    if !name.value.starts_with(DAEMON_WINDOW_NAME) {
        return Err(Error::Protocol("the advertised window is not the driver's"));
    }
    Ok(window)
}

/// The whole protocol in one function: which message type carries what.
fn decode(atoms: &Atoms, message_type: u32, data: [u16; 10]) -> Option<Event> {
    let axis = |index: usize| data[index] as i16 as i32;
    if message_type == atoms.motion {
        Some(Event::Motion(Motion {
            translate: [axis(2), axis(3), axis(4)],
            rotate: [axis(5), axis(6), axis(7)],
            period_ms: u32::from(data[8]),
        }))
    } else if message_type == atoms.button_press || message_type == atoms.button_release {
        Some(Event::Button {
            index: u32::from(data[2]),
            pressed: message_type == atoms.button_press,
        })
    } else if message_type == atoms.device_disconnect {
        Some(Event::Device {
            added: false,
            id: 0,
            device_type: 0,
            usb_id: None,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atoms() -> Atoms {
        Atoms {
            motion: 100,
            button_press: 101,
            button_release: 102,
            device_disconnect: 103,
            command: 104,
        }
    }

    #[test]
    fn motion_carries_six_signed_axes_and_a_period() {
        let mut data = [0u16; 10];
        data[2..8].copy_from_slice(&[1i16 as u16, -2i16 as u16, 3, -4i16 as u16, 5, -6i16 as u16]);
        data[8] = 16;
        assert_eq!(
            decode(&atoms(), 100, data),
            Some(Event::Motion(Motion {
                translate: [1, -2, 3],
                rotate: [-4, 5, -6],
                period_ms: 16,
            }))
        );
    }

    #[test]
    fn presses_and_releases_are_told_apart_by_message_type() {
        let mut data = [0u16; 10];
        data[2] = 4;
        assert_eq!(
            decode(&atoms(), 101, data),
            Some(Event::Button {
                index: 4,
                pressed: true
            })
        );
        assert_eq!(
            decode(&atoms(), 102, data),
            Some(Event::Button {
                index: 4,
                pressed: false
            })
        );
    }

    #[test]
    fn a_vanished_device_reports_a_removal() {
        assert_eq!(
            decode(&atoms(), 103, [0u16; 10]),
            Some(Event::Device {
                added: false,
                id: 0,
                device_type: 0,
                usb_id: None,
            })
        );
    }

    #[test]
    fn other_messages_are_left_alone() {
        assert_eq!(decode(&atoms(), 999, [0u16; 10]), None);
    }
}
