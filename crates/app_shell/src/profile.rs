//! Sampled call stacks for `--features profile` builds: with
//! `PRINTCAD_PROFILE=<dir>` the process is sampled at 199 Hz and every five
//! seconds `<dir>/w<NNN>.txt` gets that window's frame times, the functions
//! that took the time (inclusive and self) and, for a slow window, `<dir>/w<NNN>.folded` the
//! whole stacks.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(5);
const HZ: i32 = 199;
/// A window's whole stacks are kept only when its frames were slow, and at
/// most this many such files.
const SLOW_P90_MS: f32 = 25.0;
const MAX_FOLDED: usize = 12;

struct State {
    dir: PathBuf,
    guard: pprof::ProfilerGuard<'static>,
    started: Instant,
    window: usize,
    folded_written: usize,
    frames: Vec<f32>,
    events: (f32, usize),
    notes: Vec<String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

pub fn start() {
    let Ok(dir) = std::env::var("PRINTCAD_PROFILE") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let _ = std::fs::create_dir_all(&dir);
    match pprof::ProfilerGuardBuilder::default()
        .frequency(HZ)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
    {
        Ok(guard) => {
            eprintln!("profiling into {}", dir.display());
            *STATE.lock().unwrap() = Some(State {
                dir,
                guard,
                started: Instant::now(),
                window: 0,
                folded_written: 0,
                frames: Vec::new(),
                events: (0.0, 0),
                notes: Vec::new(),
            });
        }
        Err(err) => eprintln!("profiler failed to start: {err}"),
    }
}

/// One rendered frame took `took`; a window that is over is written out.
pub fn frame(took: Duration) {
    let mut lock = STATE.lock().unwrap();
    let Some(state) = lock.as_mut() else { return };
    // An idle wake that renders nothing is not a frame.
    if took >= Duration::from_micros(500) {
        state.frames.push(took.as_secs_f32() * 1000.0);
    }
    if state.started.elapsed() >= WINDOW {
        write_window(state);
    }
}

/// One window event (input) took `took`.
pub fn event(took: Duration) {
    if let Some(state) = STATE.lock().unwrap().as_mut() {
        state.events.0 += took.as_secs_f32() * 1000.0;
        state.events.1 += 1;
    }
}

/// A line in the current window's report (what the document holds).
pub fn note(text: String) {
    if let Some(state) = STATE.lock().unwrap().as_mut()
        && state.notes.last() != Some(&text)
    {
        state.notes.push(text);
    }
}

pub fn finish() {
    if let Some(state) = STATE.lock().unwrap().as_mut() {
        write_window(state);
    }
}

fn write_window(state: &mut State) {
    let wall = state.started.elapsed().as_secs_f32();
    let report = state.guard.report().build();
    let fresh = pprof::ProfilerGuardBuilder::default()
        .frequency(HZ)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build();
    let mut out = String::new();
    let mut frames = std::mem::take(&mut state.frames);
    let notes = std::mem::take(&mut state.notes);
    let events = std::mem::take(&mut state.events);
    let mut p90 = 0.0;
    let _ = writeln!(
        out,
        "window {} wall {wall:.1}s frames {}",
        state.window,
        frames.len()
    );
    if !frames.is_empty() {
        let total: f32 = frames.iter().sum();
        let mut sorted = frames.clone();
        sorted.sort_by(f32::total_cmp);
        let at = |q: f32| sorted[((sorted.len() - 1) as f32 * q) as usize];
        p90 = at(0.9);
        let _ = writeln!(
            out,
            "frame ms: mean {:.1} p50 {:.1} p90 {:.1} max {:.1}, busy {:.0}%",
            total / frames.len() as f32,
            at(0.5),
            at(0.9),
            at(1.0),
            total / 10.0 / wall
        );
        frames.truncate(150);
        let _ = writeln!(
            out,
            "frames: {}",
            frames
                .iter()
                .map(|f| format!("{f:.0}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    let _ = writeln!(out, "events {} taking {:.0} ms", events.1, events.0);
    for note in &notes {
        let _ = writeln!(out, "note: {note}");
    }
    match report {
        Ok(report) => {
            let mut folded = String::new();
            let mut inclusive: HashMap<String, isize> = HashMap::new();
            let mut own: HashMap<String, isize> = HashMap::new();
            let mut threads: HashMap<String, isize> = HashMap::new();
            let mut samples = 0;
            for (frames, count) in &report.data {
                samples += count;
                *threads.entry(frames.thread_name.clone()).or_default() += count;
                // Outermost first.
                let names: Vec<String> = frames
                    .frames
                    .iter()
                    .rev()
                    .flat_map(|f| f.iter().rev().map(|s| short(&s.name())))
                    .filter(|n| !n.starts_with("std::") && !n.starts_with("core::"))
                    .collect();
                let mut seen = std::collections::HashSet::new();
                for name in &names {
                    if seen.insert(name.as_str()) {
                        *inclusive.entry(name.clone()).or_default() += count;
                    }
                }
                if let Some(leaf) = names.last() {
                    *own.entry(leaf.clone()).or_default() += count;
                }
                let _ = writeln!(folded, "{};{} {count}", frames.thread_name, names.join(";"));
            }
            let _ = writeln!(out, "samples {samples}");
            let mut by_thread: Vec<_> = threads.into_iter().collect();
            by_thread.sort_by_key(|(_, c)| -c);
            for (name, count) in by_thread {
                let _ = writeln!(out, "thread {name}: {count}");
            }
            for (title, table) in [("inclusive", inclusive), ("self", own)] {
                let mut rows: Vec<_> = table.into_iter().collect();
                rows.sort_by_key(|(_, c)| -c);
                let _ = writeln!(out, "\n== {title}");
                for (name, count) in rows.into_iter().take(60) {
                    let _ = writeln!(
                        out,
                        "{:6} {:5.1}%  {name}",
                        count,
                        100.0 * count as f32 / samples.max(1) as f32
                    );
                }
            }
            if (p90 >= SLOW_P90_MS || std::env::var_os("PRINTCAD_PROFILE_ALL").is_some())
                && state.folded_written < MAX_FOLDED
            {
                state.folded_written += 1;
                let _ = std::fs::write(
                    state.dir.join(format!("w{:03}.folded", state.window)),
                    folded,
                );
            }
        }
        Err(err) => {
            let _ = writeln!(out, "no report: {err}");
        }
    }
    let _ = std::fs::write(state.dir.join(format!("w{:03}.txt", state.window)), out);
    state.window += 1;
    state.started = Instant::now();
    if let Ok(guard) = fresh {
        state.guard = guard;
    }
}

/// A symbol without its generic arguments: `<a::B<T> as c::D>::f` reads
/// `a::B::f`.
fn short(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut depth = 0;
    let mut skipping_trait = false;
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '<' if i == 0 || (depth == 0 && out.ends_with("::") && name[..i].ends_with("::")) => {
                // A qualified path `<Type as Trait>`: keep the type.
                if i == 0 {
                    depth = 0;
                    i += 1;
                    let mut inner = 0;
                    while i < bytes.len() {
                        let c = bytes[i] as char;
                        if c == '<' {
                            inner += 1;
                        } else if c == '>' {
                            if inner == 0 {
                                break;
                            }
                            inner -= 1;
                        } else if inner == 0 && name[i..].starts_with(" as ") {
                            skipping_trait = true;
                        } else if inner == 0 && !skipping_trait {
                            out.push(c);
                        }
                        i += 1;
                    }
                    skipping_trait = false;
                } else {
                    depth += 1;
                }
            }
            '<' => depth += 1,
            '>' => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn short_names() {
        assert_eq!(super::short("<a::B<T> as c::D>::f"), "a::B::f");
        assert_eq!(super::short("x::y::<z>"), "x::y::");
        assert_eq!(super::short("a::b<c>::d"), "a::b::d");
    }
}
