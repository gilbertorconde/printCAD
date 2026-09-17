//! A daemon that answers like the real one, for driving an application
//! without hardware.
//!
//! ```text
//! cargo run -p spacenav --example fake_daemon -- /tmp/fake.sock 0 0 -200 0 0 0 3000
//! SPNAV_SOCKET=/tmp/fake.sock cargo run -p app_shell
//! ```
//!
//! Every client that connects gets one scripted gesture: a second of quiet,
//! then the six axis values held for the given number of milliseconds, then
//! zeroes — the shape of a real puck being pushed and released.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;
use std::time::Duration;

const REQ_TAG: i32 = 0x7faa_0000;
const REQ_SET_NAME: i32 = 0x1000;
const REQ_DEV_NAME: i32 = 0x2000;
const REQ_DEV_PATH: i32 = 0x2001;
const REQ_DEV_NAXES: i32 = 0x2002;
const REQ_DEV_NBUTTONS: i32 = 0x2003;
const REQ_CHANGE_PROTO: i32 = 0x5500;
const CONT_BIT: i32 = 0x1_0000;
const UEV_MOTION: i32 = 0;

const DEVICE_NAME: &str = "Fake navigation device";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let socket = args.first().map_or("/tmp/fake-spnav.sock", String::as_str);
    let axes: Vec<i32> = args
        .iter()
        .skip(1)
        .take(6)
        .filter_map(|a| a.parse().ok())
        .collect();
    let axes: [i32; 6] = axes.try_into().unwrap_or([0, 0, -200, 0, 0, 0]);
    let hold = Duration::from_millis(args.get(7).and_then(|a| a.parse().ok()).unwrap_or(3000));

    std::fs::remove_file(socket).ok();
    let listener = UnixListener::bind(socket).expect("bind the socket");
    println!("listening on {socket}; gesture {axes:?} held for {hold:?}");

    for stream in listener.incoming().flatten() {
        thread::spawn(move || serve(stream, axes, hold));
    }
}

fn serve(mut stream: UnixStream, axes: [i32; 6], hold: Duration) {
    let mut offer = [0u8; 4];
    if stream.read_exact(&mut offer).is_err() {
        return;
    }
    let version = i32::from_ne_bytes(offer) & 0xff;
    if stream
        .write_all(&(REQ_TAG | REQ_CHANGE_PROTO | version.min(1)).to_ne_bytes())
        .is_err()
    {
        return;
    }
    println!("client connected, protocol {version}");

    // Requests are answered on this thread; the gesture plays on another, so
    // one cannot hold up the other.
    let mut motion_stream = stream.try_clone().expect("clone the socket");
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(1));
        let mut moving = [UEV_MOTION, 0, 0, 0, 0, 0, 0, 16];
        moving[1..7].copy_from_slice(&axes);
        if send(&mut motion_stream, &moving).is_err() {
            return;
        }
        println!("gesture started");
        thread::sleep(hold);
        send(&mut motion_stream, &[UEV_MOTION, 0, 0, 0, 0, 0, 0, 16]).ok();
        println!("gesture released");
    });

    let mut buf = [0u8; 32];
    while stream.read_exact(&mut buf).is_ok() {
        let request = words(&buf);
        let mut reply = request;
        match request[0] & 0xffff {
            REQ_SET_NAME => continue,
            REQ_DEV_NAME => send_string(&mut stream, request[0], DEVICE_NAME).ok(),
            REQ_DEV_PATH => send_string(&mut stream, request[0], "/dev/null").ok(),
            REQ_DEV_NAXES => {
                reply[1] = 6;
                send(&mut stream, &reply).ok()
            }
            REQ_DEV_NBUTTONS => {
                reply[1] = 2;
                send(&mut stream, &reply).ok()
            }
            _ => send(&mut stream, &reply).ok(),
        };
    }
    println!("client gone");
}

fn words(bytes: &[u8; 32]) -> [i32; 8] {
    let mut out = [0i32; 8];
    let (chunks, _) = bytes.as_chunks::<4>();
    for (word, chunk) in out.iter_mut().zip(chunks) {
        *word = i32::from_ne_bytes(*chunk);
    }
    out
}

fn send(stream: &mut UnixStream, words: &[i32; 8]) -> std::io::Result<()> {
    let mut bytes = [0u8; 32];
    let (chunks, _) = bytes.as_chunks_mut::<4>();
    for (word, chunk) in words.iter().zip(chunks) {
        *chunk = word.to_ne_bytes();
    }
    stream.write_all(&bytes)
}

/// A string travels 24 bytes per packet, the remaining length in the last
/// word.
fn send_string(stream: &mut UnixStream, request: i32, text: &str) -> std::io::Result<()> {
    let bytes = text.as_bytes();
    let mut sent = 0;
    loop {
        let remaining = bytes.len() - sent;
        let take = remaining.min(24);
        let mut raw = [0u8; 32];
        raw[4..4 + take].copy_from_slice(&bytes[sent..sent + take]);
        let mut packet = words(&raw);
        packet[0] = request;
        packet[7] = remaining as i32 | if sent == 0 { 0 } else { CONT_BIT };
        send(stream, &packet)?;
        sent += take;
        if sent >= bytes.len() {
            return Ok(());
        }
    }
}
