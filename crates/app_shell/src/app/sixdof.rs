//! Background reader for a 6-DoF mouse — a six-axis navigation puck.
//!
//! The daemon that owns the device publishes events on a UNIX socket, and the
//! vendor's own driver publishes them through the display server instead; one
//! thread here blocks on whichever answered, so the UI thread never waits on
//! it. The thread reconnects on its own, because a daemon with no device
//! plugged in — or no daemon at all — is an ordinary state, not an error
//! worth reporting.
//!
//! The daemon sends a reading only when the puck's deflection *changes*, so
//! the last reading is held until the next one arrives; letting go sends
//! zeroes, which is what stops the motion. The UI thread reads the held value
//! once per frame and integrates it over the frame's duration.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use sixdof::{EventMask, Source};

use crate::log_panel as app_log;

/// How long a blocked read waits before the thread checks whether it should
/// stop. Long enough that an idle device costs nothing measurable.
const READ_TIMEOUT: Duration = Duration::from_millis(250);

/// Button changes kept while waiting for the UI thread to take them. A frame
/// the app spent elsewhere should not leave a queue of stale presses behind.
const MAX_QUEUED_BUTTONS: usize = 32;

/// How long to wait before looking for the daemon again, and the ceiling that
/// backoff climbs to.
const RETRY_FIRST: Duration = Duration::from_millis(500);
const RETRY_MAX: Duration = Duration::from_secs(5);

/// The puck's current deflection, in the daemon's raw units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeviceMotion {
    /// Push along the device's x, y and z.
    pub translate: [f32; 3],
    /// Twist about the device's x, y and z.
    pub rotate: [f32; 3],
}

impl DeviceMotion {
    /// The six readings in the order the device reports them: three
    /// translations, then three rotations.
    pub fn axis_readings(&self) -> [f32; 6] {
        [
            self.translate[0],
            self.translate[1],
            self.translate[2],
            self.rotate[0],
            self.rotate[1],
            self.rotate[2],
        ]
    }

    /// Whether the puck is at rest, which is what lets the render loop sleep.
    pub fn is_idle(&self) -> bool {
        self.translate.iter().chain(&self.rotate).all(|v| *v == 0.0)
    }
}

/// A button going down or coming up, in the order the device reported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonEvent {
    pub index: u32,
    pub pressed: bool,
}

#[derive(Default)]
struct Shared {
    motion: DeviceMotion,
    buttons: Vec<ButtonEvent>,
    /// The connected device's name, for the status bar.
    device: Option<String>,
    /// How many buttons it has, so Preferences can offer a row per button.
    button_count: u32,
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// UI-side handle to the device thread.
pub struct SixDofWorker {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
}

impl SixDofWorker {
    /// Starts the reader thread. It runs for the life of the process, looking
    /// for the daemon until it finds one.
    ///
    /// `wake` is called when the puck starts or stops moving and on every
    /// button change — the moments a sleeping frame loop has to be told
    /// about. While the puck is deflected the loop keeps itself awake, so
    /// nothing is called for the readings in between.
    pub fn spawn(wake: impl Fn() + Send + 'static) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));

        let worker_shared = Arc::clone(&shared);
        let worker_stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("printcad-6dof-mouse".to_string())
            .spawn(move || worker_loop(&worker_shared, &worker_stop, &wake))
            .expect("failed to spawn the 6-DoF mouse thread");

        Self { shared, stop }
    }

    /// The puck's deflection right now.
    pub fn motion(&self) -> DeviceMotion {
        lock(&self.shared).motion
    }

    /// Button changes since the last call.
    pub fn take_buttons(&self) -> Vec<ButtonEvent> {
        std::mem::take(&mut lock(&self.shared).buttons)
    }

    /// The connected device's name, or `None` when there is none.
    pub fn device_name(&self) -> Option<String> {
        lock(&self.shared).device.clone()
    }

    /// How many buttons the connected device has; zero when there is none.
    pub fn button_count(&self) -> u32 {
        lock(&self.shared).button_count
    }
}

impl Drop for SixDofWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn worker_loop(shared: &Arc<Mutex<Shared>>, stop: &Arc<AtomicBool>, wake: &dyn Fn()) {
    let mut retry = RETRY_FIRST;
    while !stop.load(Ordering::SeqCst) {
        match Source::connect() {
            Ok(source) => {
                retry = RETRY_FIRST;
                serve(source, shared, stop, wake);
            }
            Err(err) => {
                // No device reachable by either route. Ordinary: say so only
                // in the trace log, and look again a little later each time.
                tracing::debug!(target: "printcad.input", "no 6-DoF mouse: {err}");
                sleep_until_stopped(retry, stop);
                retry = (retry * 2).min(RETRY_MAX);
            }
        }
    }
}

/// Reads one connection until it fails, then leaves the device state clean.
fn serve(mut source: Source, shared: &Arc<Mutex<Shared>>, stop: &Arc<AtomicBool>, wake: &dyn Fn()) {
    tracing::debug!(
        target: "printcad.input",
        backend = %source.backend(),
        "6-DoF mouse connected"
    );
    source.set_name("printCAD").ok();
    source
        .set_event_mask(EventMask::INPUT | EventMask::DEVICE)
        .ok();
    announce(&source, shared);

    while !stop.load(Ordering::SeqCst) {
        match source.read_timeout(READ_TIMEOUT) {
            Ok(None) => {}
            Ok(Some(sixdof::Event::Motion(motion))) => {
                let motion = DeviceMotion {
                    translate: motion.translate.map(|v| v as f32),
                    rotate: motion.rotate.map(|v| v as f32),
                };
                let mut state = lock(shared);
                // Starting and stopping are the edges the frame loop cannot
                // see for itself: one wakes it, the other gets it the frame
                // that brings the view to rest.
                let edge = state.motion.is_idle() != motion.is_idle();
                state.motion = motion;
                drop(state);
                if edge {
                    wake();
                }
            }
            Ok(Some(sixdof::Event::Button { index, pressed })) => {
                let mut state = lock(shared);
                if state.buttons.len() >= MAX_QUEUED_BUTTONS {
                    state.buttons.remove(0);
                }
                state.buttons.push(ButtonEvent { index, pressed });
                drop(state);
                wake();
            }
            Ok(Some(sixdof::Event::Device { .. })) => {
                source.refresh_device().ok();
                announce(&source, shared);
            }
            Ok(Some(_)) => {}
            Err(err) => {
                tracing::debug!(target: "printcad.input", "6-DoF mouse read failed: {err}");
                break;
            }
        }
    }

    let mut state = lock(shared);
    if state.device.take().is_some() {
        app_log::info("6-DoF mouse disconnected");
    }
    state.button_count = 0;
    let was_moving = !state.motion.is_idle();
    state.motion = DeviceMotion::default();
    state.buttons.clear();
    drop(state);
    if was_moving {
        // A view left drifting by a device that vanished mid-motion needs one
        // more frame to stop.
        wake();
    }
}

/// Records which device is connected, logging only when it changes. The
/// display-server protocol never names one, so the route stands in for it.
fn announce(source: &Source, shared: &Arc<Mutex<Shared>>) {
    let buttons = source.device().map_or(0, |device| device.buttons);
    let name = source.device().map_or_else(
        || match source.backend() {
            sixdof::Backend::Daemon => None,
            sixdof::Backend::Magellan => Some("device on the display server".to_string()),
        },
        |device| Some(device.name.clone()),
    );
    let mut state = lock(shared);
    state.button_count = buttons;
    if state.device == name {
        return;
    }
    match &name {
        Some(name) => app_log::info(format!("6-DoF mouse connected: {name}")),
        None => app_log::info("6-DoF mouse disconnected"),
    }
    state.device = name;
}

fn sleep_until_stopped(total: Duration, stop: &Arc<AtomicBool>) {
    let step = Duration::from_millis(100);
    let mut slept = Duration::ZERO;
    while slept < total && !stop.load(Ordering::SeqCst) {
        let nap = step.min(total - slept);
        thread::sleep(nap);
        slept += nap;
    }
}
