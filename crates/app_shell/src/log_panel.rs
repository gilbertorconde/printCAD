use std::{
    fmt,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    /// Good news worth a moment's notice: shown like a warning, in green.
    Success,
    Warn,
    Error,
}

impl LogLevel {
    pub fn label(self) -> &'static str {
        match self {
            LogLevel::Info => "INFO",
            LogLevel::Success => "OK",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp_secs: u64,
    pub level: LogLevel,
    pub message: String,
}

const MAX_ENTRIES: usize = 500;

static LOG_BUFFER: OnceLock<Mutex<Vec<LogEntry>>> = OnceLock::new();

fn buffer() -> &'static Mutex<Vec<LogEntry>> {
    LOG_BUFFER.get_or_init(|| Mutex::new(Vec::with_capacity(128)))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn push(level: LogLevel, message: String) {
    let mut guard = buffer().lock().expect("log buffer mutex poisoned");
    guard.push(LogEntry {
        timestamp_secs: now_secs(),
        level,
        message,
    });
    if guard.len() > MAX_ENTRIES {
        let overflow = guard.len() - MAX_ENTRIES;
        guard.drain(0..overflow);
    }
}

pub fn entries() -> Vec<LogEntry> {
    buffer().lock().map(|v| v.clone()).unwrap_or_default()
}

pub fn clear() {
    if let Ok(mut guard) = buffer().lock() {
        guard.clear();
    }
}

pub fn info(message: impl Into<String>) {
    let msg = message.into();
    tracing::info!("{msg}");
    push(LogLevel::Info, msg);
}

/// Log good news the user should see: it shows over the view for a few
/// seconds, in green.
pub fn success(message: impl Into<String>) {
    let msg = message.into();
    tracing::info!("{msg}");
    push(LogLevel::Success, msg);
}

pub fn warn(message: impl Into<String>) {
    let msg = message.into();
    tracing::warn!("{msg}");
    push(LogLevel::Warn, msg);
}

pub fn error(message: impl Into<String>) {
    let msg = message.into();
    tracing::error!("{msg}");
    push(LogLevel::Error, msg);
}
