//! One connection, whichever route the device is reachable by.
//!
//! The daemon's socket is tried first: it carries device details and
//! settings, and it works with no display server at all. Failing that — which
//! is what running the vendor's own driver looks like — the Magellan protocol
//! over the display server is tried instead.

use std::time::Duration;

use crate::{Client, DeviceInfo, Error, Event, EventMask};

/// Which route a [`Source`] took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The daemon's UNIX socket: everything this crate offers.
    Daemon,
    /// The Magellan protocol over a display server: motion, buttons and
    /// device removal, and nothing else.
    Magellan,
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Backend::Daemon => write!(f, "daemon"),
            Backend::Magellan => write!(f, "display server"),
        }
    }
}

/// A device connection over whichever protocol answered.
pub enum Source {
    Daemon(Client),
    /// Boxed because a display connection carries far more state than a
    /// socket does.
    #[cfg(feature = "magellan")]
    Magellan(Box<crate::magellan::Client>),
}

impl Source {
    /// Connects by the best route available.
    ///
    /// The error returned is the daemon's, since that is the one a machine
    /// without a device is expected to give.
    pub fn connect() -> Result<Source, Error> {
        match Client::connect() {
            Ok(client) => Ok(Source::Daemon(client)),
            Err(err) => Source::connect_magellan().ok_or(err),
        }
    }

    #[cfg(feature = "magellan")]
    fn connect_magellan() -> Option<Source> {
        crate::magellan::Client::connect()
            .ok()
            .map(|client| Source::Magellan(Box::new(client)))
    }

    #[cfg(not(feature = "magellan"))]
    fn connect_magellan() -> Option<Source> {
        None
    }

    /// Which protocol this connection speaks.
    #[must_use]
    pub fn backend(&self) -> Backend {
        match self {
            Source::Daemon(_) => Backend::Daemon,
            #[cfg(feature = "magellan")]
            Source::Magellan(_) => Backend::Magellan,
        }
    }

    /// Names this client in the daemon's log. The Magellan protocol has no
    /// such notion and takes it silently.
    pub fn set_name(&mut self, name: &str) -> Result<(), Error> {
        match self {
            Source::Daemon(client) => client.set_name(name),
            #[cfg(feature = "magellan")]
            Source::Magellan(_) => Ok(()),
        }
    }

    /// Chooses which events arrive. The Magellan protocol carries motion,
    /// buttons and device removal and offers no choice about it.
    pub fn set_event_mask(&mut self, mask: EventMask) -> Result<(), Error> {
        match self {
            Source::Daemon(client) => client.set_event_mask(mask),
            #[cfg(feature = "magellan")]
            Source::Magellan(_) => Ok(()),
        }
    }

    /// Scales this application's readings, leaving other applications alone.
    pub fn set_sensitivity(&mut self, sensitivity: f32) -> Result<(), Error> {
        match self {
            Source::Daemon(client) => client.set_sensitivity(sensitivity),
            #[cfg(feature = "magellan")]
            Source::Magellan(client) => client.set_sensitivity(sensitivity),
        }
    }

    /// What is known about the device. The Magellan protocol tells an
    /// application nothing about it, so this is `None` there.
    #[must_use]
    pub fn device(&self) -> Option<&DeviceInfo> {
        match self {
            Source::Daemon(client) => client.device(),
            #[cfg(feature = "magellan")]
            Source::Magellan(_) => None,
        }
    }

    /// Asks again what device is connected.
    pub fn refresh_device(&mut self) -> Result<Option<&DeviceInfo>, Error> {
        match self {
            Source::Daemon(client) => client.refresh_device(),
            #[cfg(feature = "magellan")]
            Source::Magellan(_) => Ok(None),
        }
    }

    /// The daemon's own settings, when connected to one.
    pub fn config(&mut self) -> Option<crate::Config<'_>> {
        match self {
            Source::Daemon(client) => Some(client.config()),
            #[cfg(feature = "magellan")]
            Source::Magellan(_) => None,
        }
    }

    /// Waits for the next event.
    pub fn read_blocking(&mut self) -> Result<Event, Error> {
        match self {
            Source::Daemon(client) => client.read_blocking(),
            #[cfg(feature = "magellan")]
            Source::Magellan(client) => client.read_blocking(),
        }
    }

    /// Waits for the next event, giving up after `timeout`.
    pub fn read_timeout(&mut self, timeout: Duration) -> Result<Option<Event>, Error> {
        match self {
            Source::Daemon(client) => client.read_timeout(timeout),
            #[cfg(feature = "magellan")]
            Source::Magellan(client) => client.read_timeout(timeout),
        }
    }

    /// Takes the next event if one is already waiting.
    pub fn poll(&mut self) -> Result<Option<Event>, Error> {
        match self {
            Source::Daemon(client) => client.poll(),
            #[cfg(feature = "magellan")]
            Source::Magellan(client) => client.poll(),
        }
    }

    /// The socket or display connection, for callers running their own
    /// readiness loop.
    #[must_use]
    pub fn as_raw_fd(&self) -> std::os::fd::RawFd {
        match self {
            Source::Daemon(client) => client.as_raw_fd(),
            #[cfg(feature = "magellan")]
            Source::Magellan(client) => client.as_raw_fd(),
        }
    }
}
