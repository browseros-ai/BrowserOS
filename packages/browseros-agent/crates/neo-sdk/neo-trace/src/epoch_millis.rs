//! Wall-clock time as every record stores it. Operations, frames and screenshots all carry
//! this one clock (the dispatcher's), so a viewer can line them up without guessing.

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EpochMillis(i64);

impl EpochMillis {
    /// This moment. A clock set before 1970 reads as 0 rather than failing a record.
    #[must_use]
    pub fn now() -> Self {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(elapsed) => Self(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)),
            Err(_) => Self(0),
        }
    }

    /// A stored or given value.
    #[must_use]
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// The value in milliseconds.
    #[must_use]
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// `millis` later (or earlier when negative), saturating at the ends of `i64`.
    #[must_use]
    pub const fn plus_millis(self, millis: i64) -> Self {
        Self(self.0.saturating_add(millis))
    }
}
