//! One deployment's run budget, shared by execution, discovery and saved flows.
//! Hosts resolve configuration once; scripts may shorten this ceiling but cannot extend it.

use std::{num::NonZeroU32, time::Duration};

/// Immutable wall-clock ceiling; omitted request timeouts use this same duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunPolicy {
    wall_timeout: Duration,
}

impl Default for RunPolicy {
    fn default() -> Self {
        Self {
            wall_timeout: Duration::from_secs(60),
        }
    }
}

impl RunPolicy {
    #[must_use]
    pub fn from_seconds(seconds: NonZeroU32) -> Self {
        Self {
            wall_timeout: Duration::from_secs(u64::from(seconds.get())),
        }
    }

    #[must_use]
    pub fn wall_timeout(self) -> Duration {
        self.wall_timeout
    }

    /// Keep room for a saved flow's snapshot/handback before the outer script is dropped.
    /// Small deployment budgets reserve half rather than consuming all their execution time.
    #[must_use]
    pub fn handback_timeout(self) -> Duration {
        self.wall_timeout - Duration::from_secs(5).min(self.wall_timeout / 2)
    }

    pub fn resolve_timeout(self, requested_ms: Option<f64>) -> Result<Duration, String> {
        let maximum = self.wall_timeout.as_millis() as u64;
        let Some(requested_ms) = requested_ms else {
            return Ok(self.wall_timeout);
        };
        if !requested_ms.is_finite() || requested_ms <= 0.0 {
            return Err("run timeout must be a positive finite number of milliseconds".to_string());
        }
        if requested_ms.ceil() > maximum as f64 {
            return Err(format!(
                "run timeout exceeds the configured maximum of {maximum}ms; omit timeout or request at most {maximum}ms"
            ));
        }
        Ok(Duration::from_millis(requested_ms.ceil() as u64))
    }
}
