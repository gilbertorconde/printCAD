//! A dependency-free client for the spacenavd input daemon.
//!
//! spacenavd owns the 6-degree-of-freedom device — detection, calibration,
//! dead zones, axis inversion, LEDs — and publishes events on a UNIX socket.
//! This crate speaks that socket protocol directly: no C library, no unsafe
//! code, nothing but `std`.
//!
//! ```no_run
//! let mut client = spacenav::Client::connect()?;
//! client.set_name("my-app")?;
//! loop {
//!     match client.read_blocking()? {
//!         spacenav::Event::Motion(m) => println!("{:?} {:?}", m.translate, m.rotate),
//!         other => println!("{other:?}"),
//!     }
//! }
//! # Ok::<(), spacenav::Error>(())
//! ```
//!
//! Without the daemon running there is no socket to open and [`Client::connect`]
//! fails; treat that as an ordinary state and retry.

#![cfg(unix)]
#![forbid(unsafe_code)]

mod client;
mod codec;
mod config;
#[cfg(feature = "magellan")]
pub mod magellan;
mod request;
mod source;

pub use client::{Client, DEFAULT_SOCKET, socket_path};
pub use config::{ButtonAction, Config, LedMode};
pub use source::{Backend, Source};

use std::fmt;

/// What went wrong talking to the daemon.
#[derive(Debug)]
pub enum Error {
    /// The socket itself failed.
    Io(std::io::Error),
    /// The daemon closed the connection, usually because it stopped.
    Disconnected,
    /// A packet did not say what the protocol says it must.
    Protocol(&'static str),
    /// The daemon refused the request — most often because no device is
    /// plugged in.
    Refused,
    /// The daemon did not answer in time.
    Timeout,
    /// The daemon speaks protocol version 0, which carries events but has no
    /// way to ask it anything.
    Unsupported,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(err) => write!(f, "device socket: {err}"),
            Error::Disconnected => write!(f, "the device daemon closed the connection"),
            Error::Protocol(what) => write!(f, "malformed device packet: {what}"),
            Error::Refused => write!(f, "the device daemon refused the request"),
            Error::Timeout => write!(f, "the device daemon did not answer"),
            Error::Unsupported => {
                write!(f, "the device daemon is too old to answer requests")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

/// One reading of all six axes.
///
/// Values are raw daemon units — full deflection is roughly ±350 on the common
/// pucks, but the daemon's own sensitivity settings scale them, so treat the
/// range as a tuning constant rather than a guarantee.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Motion {
    /// Push along x, y and z.
    pub translate: [i32; 3],
    /// Twist about x, y and z.
    pub rotate: [i32; 3],
    /// Milliseconds since the previous motion reading.
    pub period_ms: u32,
}

/// Something the daemon reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The puck moved.
    Motion(Motion),
    /// A button went down or came up.
    Button { index: u32, pressed: bool },
    /// A device was plugged in or unplugged. Call
    /// [`Client::refresh_device`] to learn what it is.
    Device {
        added: bool,
        id: i32,
        /// The daemon's device-type code.
        device_type: i32,
        /// Vendor and product, absent on serial devices.
        usb_id: Option<(u16, u16)>,
    },
    /// The daemon's configuration changed.
    Config { item: i32 },
    /// An uncalibrated axis reading, sent only when asked for.
    RawAxis { index: u32, value: i32 },
    /// An unmapped button, sent only when asked for.
    RawButton { index: u32, pressed: bool },
}

/// Which events the daemon should send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventMask(u32);

impl EventMask {
    pub const MOTION: EventMask = EventMask(0x01);
    pub const BUTTON: EventMask = EventMask(0x02);
    pub const DEVICE: EventMask = EventMask(0x04);
    pub const CONFIG: EventMask = EventMask(0x08);
    pub const RAW_AXIS: EventMask = EventMask(0x10);
    pub const RAW_BUTTON: EventMask = EventMask(0x20);
    /// Motion and buttons — what a navigation client needs.
    pub const INPUT: EventMask = EventMask(Self::MOTION.0 | Self::BUTTON.0);
    /// Input plus hotplug, which is what a client gets on connecting.
    pub const DEFAULT: EventMask = EventMask(Self::INPUT.0 | Self::DEVICE.0);
    pub const ALL: EventMask = EventMask(0xffff);

    #[must_use]
    pub fn bits(self) -> u32 {
        self.0
    }

    /// Reads a mask back from the daemon's own representation.
    #[must_use]
    pub fn from_bits(bits: u32) -> EventMask {
        EventMask(bits & Self::ALL.0)
    }

    #[must_use]
    pub fn contains(self, other: EventMask) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for EventMask {
    type Output = EventMask;
    fn bitor(self, rhs: EventMask) -> EventMask {
        EventMask(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for EventMask {
    fn bitor_assign(&mut self, rhs: EventMask) {
        self.0 |= rhs.0;
    }
}

/// What the daemon knows about the device it is driving.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The device's own name, as the kernel reports it.
    pub name: String,
    /// The node the daemon reads.
    pub path: Option<String>,
    pub axes: u32,
    pub buttons: u32,
    /// Vendor and product, absent on serial devices.
    pub usb_id: Option<(u16, u16)>,
    /// The daemon's device-type code.
    pub device_type: i32,
}
