//! The client against a daemon written for the test, so the whole connection
//! — handshake, requests, string transfers, events — runs with no hardware.

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use spacenav::{ButtonAction, Client, Error, Event, EventMask, LedMode, Motion};

// The protocol's own numbers, spelled out again here so the test pins them
// rather than agreeing with the client by construction.
const REQ_TAG: i32 = 0x7faa_0000;
const REQ_SET_NAME: i32 = 0x1000;
const REQ_SET_EVMASK: i32 = 0x1003;
const REQ_DEV_NAME: i32 = 0x2000;
const REQ_DEV_PATH: i32 = 0x2001;
const REQ_DEV_NAXES: i32 = 0x2002;
const REQ_DEV_NBUTTONS: i32 = 0x2003;
const REQ_DEV_USBID: i32 = 0x2004;
const REQ_DEV_TYPE: i32 = 0x2005;
const REQ_SET_SENS: i32 = 0x1001;
const REQ_GET_SENS: i32 = 0x1002;
const REQ_GET_EVMASK: i32 = 0x1004;
const REQ_SCFG_SENS: i32 = 0x3000;
const REQ_GCFG_SENS: i32 = 0x3001;
const REQ_SCFG_SENS_AXIS: i32 = 0x3002;
const REQ_GCFG_SENS_AXIS: i32 = 0x3003;
const REQ_SCFG_DEADZONE: i32 = 0x3004;
const REQ_GCFG_DEADZONE: i32 = 0x3005;
const REQ_SCFG_INVERT: i32 = 0x3006;
const REQ_GCFG_INVERT: i32 = 0x3007;
const REQ_SCFG_BNACTION: i32 = 0x300c;
const REQ_GCFG_BNACTION: i32 = 0x300d;
const REQ_SCFG_SWAPYZ: i32 = 0x3010;
const REQ_GCFG_SWAPYZ: i32 = 0x3011;
const REQ_SCFG_LED: i32 = 0x3012;
const REQ_GCFG_LED: i32 = 0x3013;
const REQ_SCFG_SERDEV: i32 = 0x3016;
const REQ_GCFG_SERDEV: i32 = 0x3017;
const REQ_SCFG_REPEAT: i32 = 0x3018;
const REQ_GCFG_REPEAT: i32 = 0x3019;
const REQ_CFG_SAVE: i32 = 0x3ffe;
const REQ_CHANGE_PROTO: i32 = 0x5500;
const CONT_BIT: i32 = 0x1_0000;

const UEV_MOTION: i32 = 0;
const UEV_PRESS: i32 = 1;

const DEVICE_NAME: &str = "Fake 6-degree-of-freedom navigation device";
const DEVICE_PATH: &str = "/dev/input/event-fake";

type Words = [i32; 8];

fn to_bytes(words: &Words) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    let (chunks, _) = bytes.as_chunks_mut::<4>();
    for (word, chunk) in words.iter().zip(chunks) {
        *chunk = word.to_ne_bytes();
    }
    bytes
}

fn from_bytes(bytes: &[u8; 32]) -> Words {
    let mut words = [0i32; 8];
    let (chunks, _) = bytes.as_chunks::<4>();
    for (word, chunk) in words.iter_mut().zip(chunks) {
        *word = i32::from_ne_bytes(*chunk);
    }
    words
}

fn motion(translate: [i32; 3], rotate: [i32; 3], period: i32) -> Words {
    [
        UEV_MOTION,
        translate[0],
        translate[1],
        translate[2],
        rotate[0],
        rotate[1],
        rotate[2],
        period,
    ]
}

/// What the daemon under test recorded about its client, and the settings it
/// is holding on everyone's behalf.
#[derive(Default)]
struct Seen {
    name: Option<String>,
    event_mask: Option<u32>,
    client_sensitivity: f32,
    config: Settings,
    saved: bool,
}

/// The daemon-wide settings, as the real one keeps them.
struct Settings {
    sensitivity: f32,
    axis_sensitivity: [f32; 6],
    dead_zone: [i32; 8],
    inverted: [i32; 6],
    button_action: [i32; 4],
    swap_yz: i32,
    led: i32,
    repeat_msec: i32,
    serial_device: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sensitivity: 1.0,
            axis_sensitivity: [1.0; 6],
            dead_zone: [0; 8],
            inverted: [0; 6],
            button_action: [0; 4],
            swap_yz: 0,
            led: 2,
            repeat_msec: -1,
            serial_device: String::new(),
        }
    }
}

struct Daemon {
    path: PathBuf,
    events: Sender<Words>,
    seen: Arc<Mutex<Seen>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Daemon {
    /// Starts a daemon on its own socket. With `interleave` set it writes a
    /// motion event just before every reply, which is how a real daemon
    /// behaves when the puck moves mid-request.
    fn start(tag: &str, interleave: bool) -> Daemon {
        let path =
            std::env::temp_dir().join(format!("spacenav-test-{tag}-{}.sock", std::process::id()));
        std::fs::remove_file(&path).ok();

        let listener = UnixListener::bind(&path).expect("bind the test socket");
        listener.set_nonblocking(true).unwrap();

        let (events, inbox) = channel();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let stop = Arc::new(AtomicBool::new(false));

        let handle = {
            let seen = Arc::clone(&seen);
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name(format!("fake-spacenavd-{tag}"))
                .spawn(move || serve(&listener, &inbox, &seen, &stop, interleave))
                .expect("spawn the test daemon")
        };

        Daemon {
            path,
            events,
            seen,
            stop,
            handle: Some(handle),
        }
    }

    fn client(&self) -> Client {
        Client::connect_at(&self.path).expect("connect to the test daemon")
    }

    fn push(&self, words: Words) {
        self.events.send(words).expect("queue a test event");
    }

    /// Stops serving and closes the connection, the way a daemon shutting down
    /// looks from the client's side.
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join the test daemon");
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.shutdown();
        std::fs::remove_file(&self.path).ok();
    }
}

fn serve(
    listener: &UnixListener,
    inbox: &Receiver<Words>,
    seen: &Arc<Mutex<Seen>>,
    stop: &Arc<AtomicBool>,
    interleave: bool,
) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => serve_client(stream, inbox, seen, stop, interleave),
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(_) => return,
        }
    }
}

fn serve_client(
    mut stream: UnixStream,
    inbox: &Receiver<Words>,
    seen: &Arc<Mutex<Seen>>,
    stop: &Arc<AtomicBool>,
    interleave: bool,
) {
    stream.set_nonblocking(true).unwrap();
    if !handshake(&mut stream, stop) {
        return;
    }

    let mut name = StringReader::default();
    let mut serial = StringReader::default();
    let mut buf = [0u8; 32];
    let mut filled = 0;

    while !stop.load(Ordering::SeqCst) {
        let mut idle = true;
        match stream.read(&mut buf[filled..]) {
            Ok(0) => return,
            Ok(read) => {
                idle = false;
                filled += read;
                if filled == buf.len() {
                    filled = 0;
                    let request = from_bytes(&buf);
                    if interleave {
                        write_packet(&mut stream, &motion([9, 9, 9], [9, 9, 9], 4));
                    }
                    answer(&mut stream, &request, seen, &mut name, &mut serial);
                }
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {}
            Err(err) if err.kind() == ErrorKind::Interrupted => {}
            Err(_) => return,
        }
        match inbox.try_recv() {
            Ok(event) => {
                idle = false;
                write_packet(&mut stream, &event);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return,
        }
        if idle {
            thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Answers the version offer, which is the first thing a client sends.
fn handshake(stream: &mut UnixStream, stop: &Arc<AtomicBool>) -> bool {
    let mut offer = [0u8; 4];
    let mut filled = 0;
    let deadline = Instant::now() + Duration::from_secs(2);
    while filled < offer.len() {
        if stop.load(Ordering::SeqCst) || Instant::now() > deadline {
            return false;
        }
        match stream.read(&mut offer[filled..]) {
            Ok(0) => return false,
            Ok(read) => filled += read,
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    assert_eq!(i32::from_ne_bytes(offer), REQ_TAG | REQ_CHANGE_PROTO | 1);
    stream
        .write_all(&(REQ_TAG | REQ_CHANGE_PROTO | 1).to_ne_bytes())
        .is_ok()
}

fn answer(
    stream: &mut UnixStream,
    request: &Words,
    seen: &Arc<Mutex<Seen>>,
    name: &mut StringReader,
    serial: &mut StringReader,
) {
    let mut reply = *request;
    match request[0] & 0xffff {
        REQ_SET_NAME => {
            if let Some(text) = name.push(request) {
                seen.lock().unwrap().name = Some(text);
            }
        }
        REQ_SET_EVMASK => {
            seen.lock().unwrap().event_mask = Some(request[1] as u32);
            reply[7] = 0;
            write_packet(stream, &reply);
        }
        REQ_DEV_NAME => write_string(stream, request[0], DEVICE_NAME),
        REQ_DEV_PATH => write_string(stream, request[0], DEVICE_PATH),
        REQ_DEV_NAXES => {
            reply[1] = 6;
            reply[7] = 0;
            write_packet(stream, &reply);
        }
        REQ_DEV_NBUTTONS => {
            reply[1] = 2;
            reply[7] = 0;
            write_packet(stream, &reply);
        }
        REQ_DEV_USBID => {
            reply[1] = 0x256f;
            reply[2] = 0xc62e;
            reply[7] = 0;
            write_packet(stream, &reply);
        }
        REQ_DEV_TYPE => {
            reply[1] = 0x206;
            reply[7] = 0;
            write_packet(stream, &reply);
        }
        REQ_SET_SENS => {
            seen.lock().unwrap().client_sensitivity = f32::from_bits(request[1] as u32);
            write_packet(stream, &reply);
        }
        REQ_GET_SENS => {
            reply[1] = seen.lock().unwrap().client_sensitivity.to_bits() as i32;
            write_packet(stream, &reply);
        }
        REQ_GET_EVMASK => {
            reply[1] = seen.lock().unwrap().event_mask.unwrap_or(0) as i32;
            write_packet(stream, &reply);
        }
        _ => {
            if !config(stream, request, &mut reply, seen, serial) {
                reply[7] = -1;
                write_packet(stream, &reply);
            }
        }
    }
}

/// The daemon-wide settings, which is most of the request surface.
fn config(
    stream: &mut UnixStream,
    request: &Words,
    reply: &mut Words,
    seen: &Arc<Mutex<Seen>>,
    serial: &mut StringReader,
) -> bool {
    let mut state = seen.lock().unwrap();
    let cfg = &mut state.config;
    match request[0] & 0xffff {
        REQ_SCFG_SENS => cfg.sensitivity = f32::from_bits(request[1] as u32),
        REQ_GCFG_SENS => reply[1] = cfg.sensitivity.to_bits() as i32,
        REQ_SCFG_SENS_AXIS => {
            for (axis, word) in cfg.axis_sensitivity.iter_mut().zip(&request[1..7]) {
                *axis = f32::from_bits(*word as u32);
            }
        }
        REQ_GCFG_SENS_AXIS => {
            for (word, axis) in reply[1..7].iter_mut().zip(&cfg.axis_sensitivity) {
                *word = axis.to_bits() as i32;
            }
        }
        REQ_SCFG_DEADZONE => cfg.dead_zone[request[1] as usize] = request[2],
        REQ_GCFG_DEADZONE => reply[2] = cfg.dead_zone[request[1] as usize],
        REQ_SCFG_INVERT => cfg.inverted.copy_from_slice(&request[1..7]),
        REQ_GCFG_INVERT => reply[1..7].copy_from_slice(&cfg.inverted),
        REQ_SCFG_BNACTION => cfg.button_action[request[1] as usize] = request[2],
        REQ_GCFG_BNACTION => reply[2] = cfg.button_action[request[1] as usize],
        REQ_SCFG_SWAPYZ => cfg.swap_yz = request[1],
        REQ_GCFG_SWAPYZ => reply[1] = cfg.swap_yz,
        REQ_SCFG_LED => cfg.led = request[1],
        REQ_GCFG_LED => reply[1] = cfg.led,
        REQ_SCFG_REPEAT => cfg.repeat_msec = request[1],
        REQ_GCFG_REPEAT => reply[1] = cfg.repeat_msec,
        REQ_CFG_SAVE => state.saved = true,
        // A string setting arrives in pieces and is answered with nothing.
        REQ_SCFG_SERDEV => {
            if let Some(text) = serial.push(request) {
                cfg.serial_device = text;
            }
            return true;
        }
        REQ_GCFG_SERDEV => {
            let text = cfg.serial_device.clone();
            drop(state);
            write_string(stream, request[0], &text);
            return true;
        }
        _ => return false,
    }
    drop(state);
    write_packet(stream, reply);
    true
}

fn write_packet(stream: &mut UnixStream, words: &Words) {
    stream.write_all(&to_bytes(words)).ok();
}

/// Sends a string the way the protocol carries one: 24 bytes per packet, with
/// the bytes still to come in the last word.
fn write_string(stream: &mut UnixStream, request: i32, text: &str) {
    let bytes = text.as_bytes();
    let mut sent = 0;
    loop {
        let remaining = bytes.len() - sent;
        let take = remaining.min(24);
        let mut raw = [0u8; 32];
        raw[4..4 + take].copy_from_slice(&bytes[sent..sent + take]);
        let mut words = from_bytes(&raw);
        words[0] = request;
        words[7] = remaining as i32 | if sent == 0 { 0 } else { CONT_BIT };
        write_packet(stream, &words);
        sent += take;
        if sent >= bytes.len() {
            return;
        }
    }
}

/// The receiving half of the same string transfer.
#[derive(Default)]
struct StringReader {
    text: Vec<u8>,
    expect: usize,
}

impl StringReader {
    fn push(&mut self, words: &Words) -> Option<String> {
        let remaining = (words[7] & 0xffff) as usize;
        if words[7] & CONT_BIT == 0 {
            self.text.clear();
            self.expect = remaining;
        }
        let take = remaining.min(24);
        let bytes = to_bytes(words);
        self.text.extend_from_slice(&bytes[4..4 + take]);
        self.expect -= take;
        (self.expect == 0).then(|| String::from_utf8_lossy(&self.text).into_owned())
    }
}

/// Waits for an event, so a test never hangs on one that does not arrive.
fn expect_event(client: &mut Client) -> Event {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(event) = client.poll().expect("poll the client") {
            return event;
        }
        assert!(Instant::now() < deadline, "no event arrived");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn connecting_negotiates_the_protocol_and_reads_the_device() {
    let daemon = Daemon::start("connect", false);
    let client = daemon.client();

    assert_eq!(client.protocol_version(), 1);
    let device = client.device().expect("the daemon reported a device");
    assert_eq!(device.name, DEVICE_NAME);
    assert_eq!(device.path.as_deref(), Some(DEVICE_PATH));
    assert_eq!(device.axes, 6);
    assert_eq!(device.buttons, 2);
    assert_eq!(device.usb_id, Some((0x256f, 0xc62e)));
    assert_eq!(device.device_type, 0x206);
}

#[test]
fn the_client_names_itself_and_chooses_its_events() {
    let daemon = Daemon::start("requests", false);
    let mut client = daemon.client();

    client.set_name("printCAD navigation").unwrap();
    client.set_event_mask(EventMask::INPUT).unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let seen = daemon.seen.lock().unwrap();
        if seen.name.is_some() && seen.event_mask.is_some() {
            assert_eq!(seen.name.as_deref(), Some("printCAD navigation"));
            assert_eq!(seen.event_mask, Some(EventMask::INPUT.bits()));
            return;
        }
        drop(seen);
        assert!(Instant::now() < deadline, "the daemon saw no requests");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn the_client_scales_its_own_readings() {
    let daemon = Daemon::start("sensitivity", false);
    let mut client = daemon.client();

    assert_eq!(client.sensitivity().unwrap(), 0.0);
    client.set_sensitivity(0.25).unwrap();
    assert_eq!(client.sensitivity().unwrap(), 0.25);

    client.set_event_mask(EventMask::ALL).unwrap();
    assert_eq!(client.event_mask().unwrap(), EventMask::ALL);
}

#[test]
fn the_daemons_own_settings_round_trip() {
    let daemon = Daemon::start("config", false);
    let mut client = daemon.client();
    let mut config = client.config();

    assert_eq!(config.sensitivity().unwrap(), 1.0);
    config.set_sensitivity(2.5).unwrap();
    assert_eq!(config.sensitivity().unwrap(), 2.5);

    let per_axis = [1.0, 0.5, 2.0, 1.5, 0.25, 3.0];
    config.set_axis_sensitivity(per_axis).unwrap();
    assert_eq!(config.axis_sensitivity().unwrap(), per_axis);

    config.set_dead_zone(3, 12).unwrap();
    assert_eq!(config.dead_zone(3).unwrap(), 12);
    assert_eq!(config.dead_zone(0).unwrap(), 0);

    let inverted = [true, false, true, false, false, true];
    config.set_inverted(inverted).unwrap();
    assert_eq!(config.inverted().unwrap(), inverted);

    config
        .set_button_action(1, ButtonAction::DominantAxis)
        .unwrap();
    assert_eq!(config.button_action(1).unwrap(), ButtonAction::DominantAxis);
    assert_eq!(config.button_action(0).unwrap(), ButtonAction::None);

    config.set_swap_yz(true).unwrap();
    assert!(config.swap_yz().unwrap());

    assert_eq!(config.led().unwrap(), LedMode::Auto);
    config.set_led(LedMode::Off).unwrap();
    assert_eq!(config.led().unwrap(), LedMode::Off);

    // No repeat is the default, and it is not simply zero: a daemon that
    // repeats every 0 ms would flood the socket.
    assert_eq!(config.repeat_interval().unwrap(), None);
    config
        .set_repeat_interval(Some(Duration::from_millis(20)))
        .unwrap();
    assert_eq!(
        config.repeat_interval().unwrap(),
        Some(Duration::from_millis(20))
    );
    config.set_repeat_interval(None).unwrap();
    assert_eq!(config.repeat_interval().unwrap(), None);

    config.set_serial_device("/dev/ttyS0").unwrap();
    assert_eq!(config.serial_device().unwrap(), "/dev/ttyS0");

    config.save().unwrap();
    assert!(daemon.seen.lock().unwrap().saved);
}

#[test]
fn motion_and_buttons_arrive_in_order() {
    let daemon = Daemon::start("events", false);
    let mut client = daemon.client();

    daemon.push(motion([10, -20, 30], [-1, 2, -3], 16));
    daemon.push([UEV_PRESS, 1, 1, 0, 0, 0, 0, 0]);

    assert_eq!(
        client.read_blocking().unwrap(),
        Event::Motion(Motion {
            translate: [10, -20, 30],
            rotate: [-1, 2, -3],
            period_ms: 16,
        })
    );
    assert_eq!(
        client.read_blocking().unwrap(),
        Event::Button {
            index: 1,
            pressed: true
        }
    );
}

#[test]
fn a_quiet_socket_polls_empty() {
    let daemon = Daemon::start("quiet", false);
    let mut client = daemon.client();

    assert_eq!(client.poll().unwrap(), None);
    daemon.push(motion([1, 0, 0], [0, 0, 0], 8));
    assert!(matches!(expect_event(&mut client), Event::Motion(_)));
    assert_eq!(client.poll().unwrap(), None);
}

#[test]
fn events_that_land_during_a_request_are_kept() {
    let daemon = Daemon::start("interleave", true);
    let mut client = daemon.client();

    // Connecting queries the device, and every one of those replies came in
    // behind a motion event.
    assert_eq!(client.device().map(|d| d.name.as_str()), Some(DEVICE_NAME));
    assert_eq!(
        client.poll().unwrap(),
        Some(Event::Motion(Motion {
            translate: [9, 9, 9],
            rotate: [9, 9, 9],
            period_ms: 4,
        }))
    );
}

#[test]
fn a_timed_read_gives_up_and_then_finds_the_event() {
    let daemon = Daemon::start("timeout", false);
    let mut client = daemon.client();

    assert_eq!(
        client.read_timeout(Duration::from_millis(20)).unwrap(),
        None
    );
    daemon.push(motion([0, 0, -5], [0, 0, 0], 8));
    assert!(matches!(
        client.read_timeout(Duration::from_secs(2)).unwrap(),
        Some(Event::Motion(_))
    ));
}

#[test]
fn a_daemon_that_stops_surfaces_as_a_disconnect() {
    let mut daemon = Daemon::start("disconnect", false);
    let mut client = daemon.client();
    daemon.shutdown();

    assert!(matches!(client.read_blocking(), Err(Error::Disconnected)));
}

#[test]
fn a_missing_socket_is_an_ordinary_failure() {
    let path = std::env::temp_dir().join("spacenav-test-absent.sock");
    std::fs::remove_file(&path).ok();
    assert!(matches!(Client::connect_at(&path), Err(Error::Io(_))));
}
