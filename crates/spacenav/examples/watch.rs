//! Prints what the daemon reports, for checking a device by hand.
//!
//! ```text
//! cargo run -p spacenav --example watch
//! ```

/// Reads the daemon's settings. Nothing here writes: these belong to every
/// application on the machine.
fn print_settings(client: &mut spacenav::Client) {
    let mut config = client.config();
    println!("daemon settings:");
    println!("  sensitivity      {:?}", config.sensitivity());
    println!("  per axis         {:?}", config.axis_sensitivity());
    println!("  inverted         {:?}", config.inverted());
    println!("  dead zone axis 0 {:?}", config.dead_zone(0));
    println!("  axis 0 maps to   {:?}", config.axis_map(0));
    println!("  button 0 maps to {:?}", config.button_map(0));
    println!("  button 0 action  {:?}", config.button_action(0));
    println!("  swap y and z     {:?}", config.swap_yz());
    println!("  led              {:?}", config.led());
    println!("  grab device      {:?}", config.grab_device());
    println!("  repeat           {:?}", config.repeat_interval());
    println!("  serial device    {:?}", config.serial_device());
    println!("  socket           {:?}", config.socket_path());
}

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

    print_settings(&mut client);

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
