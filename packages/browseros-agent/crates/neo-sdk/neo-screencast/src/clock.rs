//! Neo: wall-clock timestamps for capture metadata.

use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn now_epoch_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}
