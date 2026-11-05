//! A run's console output, bounded.

use browseros_trace::SecretSet;
use serde::{Deserialize, Serialize};

use crate::config::Defaults;

/// What a script logged, kept within `Defaults::MAX_LOG_LINES` and `Defaults::MAX_LOG_BYTES`.
/// A line past either limit is counted, not kept, so a script that logs in a loop cannot grow
/// its run's reply (or Neo's memory) without bound. Each kept line is masked whole before it is
/// cut: a cut through a known secret would leave a prefix no later mask recognizes.
#[derive(Debug, Default)]
pub(crate) struct RunLogs {
    lines: Vec<LogLine>,
    bytes: usize,
    dropped: usize,
}

/// One console line. A plain data bag. The run's reply carries it, masked, and so does its
/// stored replay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LogLine {
    /// The console method that wrote it.
    pub(crate) level: LogLevel,
    /// The formatted arguments.
    pub(crate) text: String,
}

/// Which console method wrote a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LogLevel {
    /// `console.log`.
    Log,
    /// `console.info`.
    Info,
    /// `console.warn`.
    Warn,
    /// `console.error`.
    Error,
    /// `console.debug`.
    Debug,
}

impl RunLogs {
    /// Keeps one line, masked and then cut to the bytes left, or counts it once a limit is
    /// reached.
    pub(crate) fn push(&mut self, level: LogLevel, mut text: String, secrets: &SecretSet) {
        let room = Defaults::MAX_LOG_BYTES.saturating_sub(self.bytes);
        if self.lines.len() >= Defaults::MAX_LOG_LINES || room == 0 {
            self.dropped += 1;
            return;
        }
        secrets.mask_str(&mut text);
        let text = cut_to(text, room);
        self.bytes += text.len();
        self.lines.push(LogLine { level, text });
    }

    /// The lines kept, oldest first, and how many were dropped.
    #[must_use]
    pub(crate) fn into_parts(self) -> (Vec<LogLine>, usize) {
        (self.lines, self.dropped)
    }
}

impl LogLevel {
    /// The bootstrap's level name; anything else is `log`.
    #[must_use]
    pub(crate) fn parse(level: &str) -> Self {
        match level {
            "info" => Self::Info,
            "warn" => Self::Warn,
            "error" => Self::Error,
            "debug" => Self::Debug,
            _ => Self::Log,
        }
    }
}

/// `text` within `room` bytes, cut at a character boundary.
fn cut_to(mut text: String, room: usize) -> String {
    if text.len() > room {
        let cut = (0..=room)
            .rev()
            .find(|index| text.is_char_boundary(*index))
            .unwrap_or(0);
        text.truncate(cut);
    }
    text
}

#[cfg(test)]
mod tests {
    use browseros_trace::{SecretSet, SecretSetConfig};

    use super::{LogLevel, RunLogs};
    use crate::config::Defaults;

    #[test]
    fn lines_past_the_line_limit_are_counted_not_kept() {
        let secrets = SecretSet::new(SecretSetConfig::default());
        let mut logs = RunLogs::default();
        for index in 0..Defaults::MAX_LOG_LINES + 5 {
            logs.push(LogLevel::Log, format!("line {index}"), &secrets);
        }
        let (lines, dropped) = logs.into_parts();
        assert_eq!(lines.len(), Defaults::MAX_LOG_LINES, "the first lines stay");
        assert_eq!(dropped, 5, "the rest are counted so the reply can say so");
        assert_eq!(lines.first().map(|line| line.text.as_str()), Some("line 0"));
    }

    #[test]
    fn text_past_the_byte_limit_is_cut_at_a_character() {
        let secrets = SecretSet::new(SecretSetConfig::default());
        let mut logs = RunLogs::default();
        logs.push(LogLevel::Log, "x".to_owned(), &secrets);
        logs.push(
            LogLevel::Warn,
            "é".repeat(Defaults::MAX_LOG_BYTES),
            &secrets,
        );
        logs.push(LogLevel::Log, "past the limit".to_owned(), &secrets);
        let (lines, dropped) = logs.into_parts();
        let cut = lines.get(1).map_or("", |line| line.text.as_str());
        assert_eq!(
            cut.len(),
            Defaults::MAX_LOG_BYTES - 2,
            "one byte of room is left, and half a two-byte character is never kept"
        );
        assert!(cut.chars().all(|char| char == 'é'), "cut on a boundary");
        assert_eq!(
            lines.len(),
            3,
            "the byte left still takes one more line, cut"
        );
        assert_eq!(dropped, 0);
    }
}
