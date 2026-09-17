# spacenav

A small, dependency-free Rust client for [spacenavd], the free Unix daemon
that drives 6-degree-of-freedom input devices (3Dconnexion SpaceMouse,
SpaceNavigator, Spaceball and friends).

The daemon owns device detection, calibration, dead zones and per-device
configuration; a client just opens its UNIX socket and reads events.

```rust
let mut client = spacenav::Client::connect()?;
client.set_name("my-app")?;
client.set_event_mask(spacenav::EventMask::DEFAULT)?;

loop {
    match client.read_blocking()? {
        spacenav::Event::Motion(m) => println!("{:?} {:?}", m.translate, m.rotate),
        spacenav::Event::Button { index, pressed } => println!("button {index} {pressed}"),
        other => println!("{other:?}"),
    }
}
```

## What it does

- Speaks the daemon's protocol directly over `/var/run/spnav.sock`
  (`SPNAV_SOCKET`, then `socket = <path>` in `/etc/spnavrc`, then the default).
- Negotiates protocol version 1 and falls back to version 0 against an older
  daemon, where events still decode but requests are unavailable.
- Blocking (`read_blocking`) and non-blocking (`poll`) reads, plus
  `as_raw_fd` for `select`/`poll`/`epoll` loops.
- Queries the connected device: name, path, axis and button counts, USB id.
- No dependencies, no C library, no unsafe code.

## What it does not do

- No Windows or macOS backend.
- No daemon-less path: without spacenavd running, `connect` fails and the
  caller is expected to retry.
- No configuration writes — the daemon's own tools own the config file.

## Licence

MIT or Apache-2.0, at your option.

[spacenavd]: https://spacenav.sourceforge.net/
