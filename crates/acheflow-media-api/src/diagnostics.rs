//! Bounded in-memory application diagnostics; callers supply safe event text.
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub level: String,
    pub source: String,
    pub message: String,
    pub repetitions: u32,
}

#[derive(Default)]
pub struct LogBuffer(Mutex<VecDeque<LogEntry>>);

fn bounded(value: &str, limit: usize) -> String {
    let mut output = String::new();
    for c in value.chars().filter(|c| !c.is_control() || *c == '\t') {
        if output.len() + c.len_utf8() > limit {
            break;
        }
        output.push(c);
    }
    output
}

impl LogBuffer {
    pub fn push(&self, timestamp_ms: u64, level: &str, source: &str, message: &str) {
        let mut entries = self.0.lock().unwrap();
        let entry = LogEntry {
            timestamp_ms,
            level: bounded(level, 16),
            source: bounded(source, 128),
            message: bounded(&redact(message), 2048),
            repetitions: 1,
        };
        if let Some(last) = entries.back_mut()
            && last.level == entry.level
            && last.source == entry.source
            && last.message == entry.message
        {
            last.timestamp_ms = timestamp_ms;
            last.repetitions = last.repetitions.saturating_add(1);
            return;
        }
        entries.push_back(entry);
        while entries.len() > 500
            || entries
                .iter()
                .map(|e| e.message.len() + e.source.len() + e.level.len())
                .sum::<usize>()
                > 128 * 1024
        {
            entries.pop_front();
        }
    }

    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.0.lock().unwrap().iter().cloned().collect()
    }
}

fn redact(message: &str) -> String {
    let mut hide_next = false;
    message
        .split_whitespace()
        .map(|word| {
            if hide_next {
                hide_next = false;
                return "[redacted]";
            }
            let lower = word.to_ascii_lowercase();
            if lower == "bearer" {
                hide_next = true;
                return "Bearer";
            }
            if word.contains("://") {
                return "[URL]";
            }
            if [
                "access_token",
                "refresh_token",
                "client_secret",
                "code_verifier",
                "authorization_code",
                "password",
            ]
            .iter()
            .any(|key| lower.contains(key))
            {
                hide_next = word.ends_with([':', '=']);
                return "[redacted]";
            }
            word
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_history_redacts_urls_and_credential_fields() {
        let logs = LogBuffer::default();
        logs.push(1,"error","network","Failed https://user:private@host/path?access_token=private and rtmp://host/live/private Bearer private client_secret=private");
        let message = &logs.snapshot()[0].message;
        assert!(!message.contains("private"));
        assert!(message.contains("[URL]"));
        assert!(message.contains("[redacted]"));
    }

    #[test]
    fn history_is_bounded_and_repeated_events_are_coalesced() {
        let logs = LogBuffer::default();
        for n in 0..2000 {
            logs.push(n, "info", "server", &format!("{n}: {}", "я".repeat(4096)));
        }
        let entries = logs.snapshot();
        assert!(entries.len() <= 500);
        assert!(
            entries
                .iter()
                .map(|e| e.message.len() + e.source.len() + e.level.len())
                .sum::<usize>()
                <= 128 * 1024
        );
        assert!(entries.iter().all(|e| e.message.len() <= 2048));
        logs.push(2001, "error", "chat", "Disconnected\0\n");
        logs.push(2002, "error", "chat", "Disconnected\0\n");
        let entries = logs.snapshot();
        let last = entries.last().unwrap();
        assert_eq!(last.message, "Disconnected");
        assert_eq!(last.repetitions, 2);
        assert_eq!(last.timestamp_ms, 2002);
    }
}
