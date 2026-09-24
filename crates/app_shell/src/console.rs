//! What the script console shows: the lines typed and what they printed,
//! answered or raised. The script host writes it, the console panel reads
//! it each frame, as the log panel does with the log.

use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// What the user ran.
    Input,
    /// What the run printed.
    Printed,
    /// The value a console line came to.
    Value,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleLine {
    pub kind: LineKind,
    pub text: String,
}

const MAX_LINES: usize = 2000;

static LINES: OnceLock<Mutex<Vec<ConsoleLine>>> = OnceLock::new();

fn lines() -> &'static Mutex<Vec<ConsoleLine>> {
    LINES.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn push(kind: LineKind, text: impl Into<String>) {
    let Ok(mut guard) = lines().lock() else {
        return;
    };
    for line in text.into().lines() {
        guard.push(ConsoleLine {
            kind,
            text: line.to_string(),
        });
    }
    if guard.len() > MAX_LINES {
        let overflow = guard.len() - MAX_LINES;
        guard.drain(0..overflow);
    }
}

pub fn entries() -> Vec<ConsoleLine> {
    lines().lock().map(|v| v.clone()).unwrap_or_default()
}

pub fn clear() {
    if let Ok(mut guard) = lines().lock() {
        guard.clear();
    }
}
