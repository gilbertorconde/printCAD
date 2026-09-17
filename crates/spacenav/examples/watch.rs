//! Prints what the daemon reports, for checking a device by hand.
//!
//! ```text
//! cargo run -p spacenav --example watch
//! ```

fn main() {
    let mut client = match spacenav::Client::connect() {
        Ok(client) => client,
        Err(err) => {
            eprintln!(
                "cannot reach the daemon on {:?}: {err}",
                spacenav::socket_path()
            );
            std::process::exit(1);
        }
    };

    println!("protocol version {}", client.protocol_version());
    match client.device() {
        Some(device) => println!(
            "device {:?} ({} axes, {} buttons, usb {:?}, type {:#x}) at {:?}",
            device.name,
            device.axes,
            device.buttons,
            device.usb_id,
            device.device_type,
            device.path
        ),
        None => println!("no device connected"),
    }

    client.set_name("spacenav watch").ok();
    client.set_event_mask(spacenav::EventMask::DEFAULT).ok();

    loop {
        match client.read_blocking() {
            Ok(spacenav::Event::Device { added, .. }) => {
                println!("device {}", if added { "added" } else { "removed" });
                match client.refresh_device() {
                    Ok(Some(device)) => println!("  now {:?}", device.name),
                    Ok(None) => println!("  now nothing"),
                    Err(err) => println!("  query failed: {err}"),
                }
            }
            Ok(event) => println!("{event:?}"),
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        }
    }
}
