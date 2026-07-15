use std::collections::VecDeque;
use std::sync::Mutex;

use lazy_static::lazy_static;
use serde::Serialize;

const MAX_BACKLOG: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoggingState {
    Disabled,
    Enabled,
}

lazy_static! {
    static ref LOG_BUFFER: Mutex<VecDeque<LogEntry>> = Mutex::new(VecDeque::new());
    static ref LOG_STATE: Mutex<LoggingState> = Mutex::new(LoggingState::Disabled);
}

#[allow(clippy::expect_used)]
pub fn set_state(state: LoggingState) {
    *LOG_STATE.lock().expect("log lock poisoned") = state;
}

#[allow(clippy::expect_used)]
pub fn state() -> LoggingState {
    *LOG_STATE.lock().expect("log lock poisoned")
}

#[allow(clippy::expect_used)]
pub fn info(message: String, target: Option<String>) {
    match state() {
        LoggingState::Enabled => push(LogLevel::Info, message, target),
        LoggingState::Disabled => println!("{}", message),
    }
}

pub fn warn(message: String, target: Option<String>) {
    match state() {
        LoggingState::Enabled => push(LogLevel::Warn, message, target),
        LoggingState::Disabled => eprintln!("{}", message),
    }
}

#[allow(clippy::expect_used)]
pub fn error(message: String, target: Option<String>) {
    match state() {
        LoggingState::Enabled => push(LogLevel::Error, message, target),
        LoggingState::Disabled => eprintln!("{}", message),
    }
}

#[allow(clippy::expect_used)]
pub fn push(level: LogLevel, message: String, target: Option<String>) {
    if let LoggingState::Disabled = state() {
        return;
    }

    let entry = LogEntry {
        level,
        message,
        target,
    };

    let mut buffer = LOG_BUFFER.lock().expect("log lock poisoned");

    buffer.push_back(entry);

    if buffer.len() > MAX_BACKLOG {
        buffer.pop_front();
    }
}

#[allow(clippy::expect_used)]
pub fn drain() -> Vec<LogEntry> {
    if let LoggingState::Disabled = state() {
        return Vec::new();
    }

    let mut buffer = LOG_BUFFER.lock().expect("log lock poisoned");
    buffer.drain(..).collect()
}

#[allow(clippy::expect_used)]
pub fn clear() {
    let mut buffer = LOG_BUFFER.lock().expect("log lock poisoned");

    buffer.clear();
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use super::*;

    fn reset() {
        set_state(LoggingState::Disabled);
        clear();
    }

    #[test]
    #[serial]
    fn test_push_and_drain() {
        reset();
        set_state(LoggingState::Enabled);
        push(LogLevel::Info, "info".to_string(), None);
        push(
            LogLevel::Warn,
            "warning".to_string(),
            Some("build".to_string()),
        );
        push(
            LogLevel::Error,
            "error".to_string(),
            Some("fetch".to_string()),
        );

        let entries = drain();
        assert_eq!(entries.len(), 3);
        assert!(matches!(entries[0].level, LogLevel::Info));
        assert_eq!(entries[0].message, "info");
        assert!(entries[0].target.is_none());
        assert!(matches!(entries[1].level, LogLevel::Warn));
        assert_eq!(entries[1].message, "warning");
        assert_eq!(entries[1].target.as_deref(), Some("build"));
        assert!(matches!(entries[2].level, LogLevel::Error));

        assert!(drain().is_empty());
    }

    #[test]
    #[serial]
    fn test_push_noop_when_disabled() {
        reset();
        set_state(LoggingState::Disabled);
        push(LogLevel::Info, "should not appear".to_string(), None);
        assert!(drain().is_empty());
    }
}
