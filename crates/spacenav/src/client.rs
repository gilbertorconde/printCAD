//! The socket connection to the daemon.

use std::collections::VecDeque;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::codec::{PACKET_BYTES, Words, decode_event, is_event, words_from_bytes};
use crate::config::Config;
use crate::request::{
    MAX_PROTO_VER, REQ_CHANGE_PROTO, REQ_DEV_NAME, REQ_DEV_NAXES, REQ_DEV_NBUTTONS, REQ_DEV_PATH,
    REQ_DEV_TYPE, REQ_DEV_USBID, REQ_GET_EVMASK, REQ_GET_SENS, REQ_SET_EVMASK, REQ_SET_NAME,
    REQ_SET_SENS, REQ_TAG, StringReader, check_reply, reply_status, request_packet, string_packets,
};
use crate::{DeviceInfo, Error, Event, EventMask};

/// Where the daemon listens unless it is configured otherwise.
pub const DEFAULT_SOCKET: &str = "/var/run/spnav.sock";

/// The daemon's configuration file, which may name a different socket.
const CONFIG_FILE: &str = "/etc/spnavrc";

/// How long to wait for the daemon to accept the protocol version. An older
/// daemon answers nothing at all, so this is also how long connecting takes
/// against one.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(300);

/// How long to wait for the answer to a request.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

/// The socket the daemon is expected to be listening on.
///
/// `SPNAV_SOCKET` wins, then a `socket = <path>` line in the daemon's
/// configuration file, then the compiled-in default.
#[must_use]
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SPNAV_SOCKET") {
        return PathBuf::from(path);
    }
    if let Some(path) = configured_socket(Path::new(CONFIG_FILE)) {
        return path;
    }
    PathBuf::from(DEFAULT_SOCKET)
}

/// Reads the `socket` setting out of a daemon configuration file.
fn configured_socket(config: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(config).ok()?;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "socket" && !value.trim().is_empty() {
            return Some(PathBuf::from(value.trim()));
        }
    }
    None
}

/// How the socket behaves on the next read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Blocking,
    Nonblocking,
    Deadline(Duration),
}

/// A connection to the daemon.
pub struct Client {
    stream: UnixStream,
    /// The protocol version in force: 1 supports requests, 0 only events.
    proto: i32,
    mode: Mode,
    /// A packet under construction; the socket is a byte stream, so a read can
    /// land in the middle of one.
    buf: [u8; PACKET_BYTES],
    filled: usize,
    /// Events that arrived while a request was waiting for its answer.
    queued: VecDeque<Event>,
    device: Option<DeviceInfo>,
}

impl Client {
    /// Connects to the daemon on the socket [`socket_path`] names.
    pub fn connect() -> Result<Client, Error> {
        Client::connect_at(&socket_path())
    }

    /// Connects to a daemon listening on a specific socket.
    pub fn connect_at(path: &Path) -> Result<Client, Error> {
        let stream = UnixStream::connect(path)?;
        let mut client = Client {
            stream,
            proto: 0,
            mode: Mode::Blocking,
            buf: [0u8; PACKET_BYTES],
            filled: 0,
            queued: VecDeque::new(),
            device: None,
        };
        client.negotiate_protocol()?;
        // A daemon with nothing plugged in refuses every device query, which
        // is a connection worth keeping: hotplug will announce the device.
        let _ = client.refresh_device();
        Ok(client)
    }

    /// The protocol version in force. Version 0 carries events but answers no
    /// requests.
    #[must_use]
    pub fn protocol_version(&self) -> i32 {
        self.proto
    }

    /// Tells the daemon what to call this client in its logs.
    pub fn set_name(&mut self, name: &str) -> Result<(), Error> {
        self.send_string(REQ_SET_NAME, name)
    }

    /// Scales this client's motion readings, leaving other clients alone.
    pub fn set_sensitivity(&mut self, sensitivity: f32) -> Result<(), Error> {
        self.request(REQ_SET_SENS, &[sensitivity.to_bits() as i32])
            .map(drop)
    }

    /// This client's motion scale.
    pub fn sensitivity(&mut self) -> Result<f32, Error> {
        let reply = self.request(REQ_GET_SENS, &[])?;
        Ok(f32::from_bits(reply[1] as u32))
    }

    /// Which events the daemon is sending.
    pub fn event_mask(&mut self) -> Result<EventMask, Error> {
        let reply = self.request(REQ_GET_EVMASK, &[])?;
        Ok(EventMask::from_bits(reply[1] as u32))
    }

    /// The daemon's own settings, shared by every client and by the device
    /// itself: sensitivities, dead zones, axis and button mapping, LED.
    pub fn config(&mut self) -> Config<'_> {
        Config { client: self }
    }

    /// Chooses which events the daemon sends.
    pub fn set_event_mask(&mut self, mask: EventMask) -> Result<(), Error> {
        self.request(REQ_SET_EVMASK, &[mask.bits() as i32])
            .map(drop)
    }

    /// What the daemon last reported about the connected device.
    #[must_use]
    pub fn device(&self) -> Option<&DeviceInfo> {
        self.device.as_ref()
    }

    /// Asks the daemon again what device it is driving.
    ///
    /// Returns `None` when no device is connected, which is the ordinary state
    /// of a running daemon with nothing plugged in.
    pub fn refresh_device(&mut self) -> Result<Option<&DeviceInfo>, Error> {
        self.require_requests()?;
        let name = match self.request_string(REQ_DEV_NAME) {
            Ok(name) => name,
            Err(Error::Refused) => {
                self.device = None;
                return Ok(None);
            }
            Err(err) => return Err(err),
        };
        let path = self.request_string(REQ_DEV_PATH).ok();
        let axes = self.request_count(REQ_DEV_NAXES);
        let buttons = self.request_count(REQ_DEV_NBUTTONS);
        let usb_id = self.request(REQ_DEV_USBID, &[]).ok().and_then(|reply| {
            let vendor = u16::try_from(reply[1]).ok()?;
            let product = u16::try_from(reply[2]).ok()?;
            (vendor != 0 || product != 0).then_some((vendor, product))
        });
        let device_type = self.request(REQ_DEV_TYPE, &[]).map_or(0, |reply| reply[1]);

        self.device = Some(DeviceInfo {
            name,
            path,
            axes,
            buttons,
            usb_id,
            device_type,
        });
        Ok(self.device.as_ref())
    }

    /// Waits for the next event.
    pub fn read_blocking(&mut self) -> Result<Event, Error> {
        if let Some(event) = self.queued.pop_front() {
            return Ok(event);
        }
        loop {
            self.set_mode(Mode::Blocking)?;
            // A blocking read only returns once it has a whole packet.
            let Some(words) = self.fill_packet()? else {
                continue;
            };
            if is_event(&words) {
                return decode_event(&words);
            }
        }
    }

    /// Waits for the next event, giving up after `timeout`.
    ///
    /// `None` means the wait expired. A packet that arrived only in part is
    /// kept, so the next call picks it up where this one left off.
    pub fn read_timeout(&mut self, timeout: Duration) -> Result<Option<Event>, Error> {
        if let Some(event) = self.queued.pop_front() {
            return Ok(Some(event));
        }
        loop {
            self.set_mode(Mode::Deadline(timeout))?;
            let Some(words) = self.fill_packet()? else {
                return Ok(None);
            };
            if is_event(&words) {
                return decode_event(&words).map(Some);
            }
        }
    }

    /// Takes the next event if one is already waiting.
    pub fn poll(&mut self) -> Result<Option<Event>, Error> {
        if let Some(event) = self.queued.pop_front() {
            return Ok(Some(event));
        }
        loop {
            self.set_mode(Mode::Nonblocking)?;
            let Some(words) = self.fill_packet()? else {
                return Ok(None);
            };
            if is_event(&words) {
                return decode_event(&words).map(Some);
            }
        }
    }

    /// The socket, for callers that run their own readiness loop.
    #[must_use]
    pub fn as_raw_fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    /// Offers protocol version 1 and settles for whatever the daemon answers.
    /// A daemon too old to understand the offer says nothing, and version 0
    /// is what remains.
    fn negotiate_protocol(&mut self) -> Result<(), Error> {
        let offer = REQ_TAG | REQ_CHANGE_PROTO | MAX_PROTO_VER;
        self.stream.write_all(&offer.to_ne_bytes())?;
        self.stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;

        let mut reply = [0u8; 4];
        let mut filled = 0;
        while filled < reply.len() {
            match self.stream.read(&mut reply[filled..]) {
                Ok(0) => return Err(Error::Disconnected),
                Ok(read) => filled += read,
                Err(err) if err.kind() == ErrorKind::Interrupted => {}
                Err(err) if would_block(&err) => break,
                Err(err) => return Err(Error::Io(err)),
            }
        }
        self.stream.set_read_timeout(None)?;
        self.mode = Mode::Blocking;

        self.proto = match filled {
            4 => i32::from_ne_bytes(reply) & 0xff,
            0 => 0,
            _ => return Err(Error::Protocol("a truncated protocol reply")),
        };
        Ok(())
    }

    fn require_requests(&self) -> Result<(), Error> {
        if self.proto < 1 {
            return Err(Error::Unsupported);
        }
        Ok(())
    }

    /// Sends a request and waits for its answer, queueing any events that
    /// arrive in between.
    pub(crate) fn request(&mut self, request: i32, args: &[i32]) -> Result<Words, Error> {
        self.require_requests()?;
        self.send(&request_packet(request, args))?;
        let reply = self.await_reply()?;
        reply_status(request, &reply)?;
        Ok(reply)
    }

    /// Sends a request whose answer is a string, arriving 24 bytes per packet.
    pub(crate) fn request_string(&mut self, request: i32) -> Result<String, Error> {
        self.require_requests()?;
        self.send(&request_packet(request, &[]))?;
        let mut reader = StringReader::default();
        loop {
            let reply = self.await_reply()?;
            check_reply(request, &reply)?;
            if let Some(text) = reader.push(&reply)? {
                return Ok(text);
            }
        }
    }

    /// Sends a string the daemon takes without answering.
    pub(crate) fn send_string(&mut self, request: i32, text: &str) -> Result<(), Error> {
        self.require_requests()?;
        for packet in string_packets(request, text) {
            self.send(&packet)?;
        }
        Ok(())
    }

    /// A count the daemon reports in the first word of its answer.
    fn request_count(&mut self, request: i32) -> u32 {
        self.request(request, &[])
            .map_or(0, |reply| reply[1].max(0) as u32)
    }

    fn send(&mut self, words: &Words) -> Result<(), Error> {
        self.stream
            .write_all(&crate::codec::bytes_from_words(words))?;
        Ok(())
    }

    /// Reads packets until one is a reply rather than an event.
    fn await_reply(&mut self) -> Result<Words, Error> {
        self.set_mode(Mode::Deadline(REQUEST_TIMEOUT))?;
        loop {
            let Some(words) = self.fill_packet()? else {
                return Err(Error::Timeout);
            };
            if !is_event(&words) {
                return Ok(words);
            }
            if let Ok(event) = decode_event(&words) {
                self.queued.push_back(event);
            }
        }
    }

    /// Reads toward a whole packet. `None` means the socket had nothing more
    /// to give before the current mode gave up.
    fn fill_packet(&mut self) -> Result<Option<Words>, Error> {
        while self.filled < PACKET_BYTES {
            match self.stream.read(&mut self.buf[self.filled..]) {
                Ok(0) => return Err(Error::Disconnected),
                Ok(read) => self.filled += read,
                Err(err) if err.kind() == ErrorKind::Interrupted => {}
                Err(err) if would_block(&err) => return Ok(None),
                Err(err) => return Err(Error::Io(err)),
            }
        }
        self.filled = 0;
        Ok(Some(words_from_bytes(&self.buf)))
    }

    fn set_mode(&mut self, mode: Mode) -> Result<(), Error> {
        if self.mode == mode {
            return Ok(());
        }
        match mode {
            Mode::Blocking => {
                self.stream.set_nonblocking(false)?;
                self.stream.set_read_timeout(None)?;
            }
            Mode::Nonblocking => {
                self.stream.set_read_timeout(None)?;
                self.stream.set_nonblocking(true)?;
            }
            Mode::Deadline(timeout) => {
                self.stream.set_nonblocking(false)?;
                self.stream.set_read_timeout(Some(timeout))?;
            }
        }
        self.mode = mode;
        Ok(())
    }
}

/// Both kinds of "nothing to read right now": a non-blocking socket and one
/// whose read timeout expired.
fn would_block(err: &std::io::Error) -> bool {
    matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(name: &str, body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(body.as_bytes()).unwrap();
        path
    }

    #[test]
    fn a_configured_socket_overrides_the_default() {
        let path = write_config(
            "spacenav-config-socket.test",
            "# a comment\nrepeat-interval = 0\nsocket = /run/elsewhere.sock\n",
        );
        assert_eq!(
            configured_socket(&path),
            Some(PathBuf::from("/run/elsewhere.sock"))
        );
        fs::remove_file(path).ok();
    }

    #[test]
    fn a_config_without_a_socket_line_says_nothing() {
        let path = write_config(
            "spacenav-config-plain.test",
            "sensitivity = 1.0\n# socket = /commented/out.sock\n",
        );
        assert_eq!(configured_socket(&path), None);
        fs::remove_file(path).ok();
    }

    #[test]
    fn a_missing_config_says_nothing() {
        assert_eq!(
            configured_socket(Path::new("/nonexistent/spacenav/config")),
            None
        );
    }
}
