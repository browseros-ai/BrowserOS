//! One `run` call, parsed into domain types.

use std::time::Duration;

use super::RunMeta;
use crate::config::Defaults;
use crate::dispatch::{ErrorCode, OperationError};

/// One `run` call. The MCP layer builds it from the tool's arguments (the model's) and `_meta`
/// (the host's); nothing after it re-validates. A plain data bag.
#[derive(Debug)]
pub(crate) struct RunRequest {
    /// The body of the agent's async function.
    pub(crate) code: String,
    /// The run's budget.
    pub(crate) timeout: RunTimeout,
    /// The host's ids and data.
    pub(crate) meta: RunMeta,
}

/// A run's wall-clock budget: `Defaults::RUN_TIMEOUT` unless the model asks for another, at
/// most `Defaults::RUN_TIMEOUT_MAX` (decision 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RunTimeout(Duration);

impl RunTimeout {
    /// The model's `timeout`, in milliseconds, or the default. Out of range is the model's own
    /// mistake: `script / invalid_input`.
    pub(crate) fn from_millis(timeout: Option<u64>) -> Result<Self, OperationError> {
        let Some(millis) = timeout else {
            return Ok(Self(Defaults::RUN_TIMEOUT));
        };
        let budget = Duration::from_millis(millis);
        if budget.is_zero() || budget > Defaults::RUN_TIMEOUT_MAX {
            return Err(OperationError::script(
                ErrorCode::INVALID_INPUT,
                format_args!(
                    "run: timeout is in milliseconds, from 1 to {}; leave it out for {}",
                    Defaults::RUN_TIMEOUT_MAX.as_millis(),
                    Defaults::RUN_TIMEOUT.as_millis()
                ),
            ));
        }
        Ok(Self(budget))
    }

    /// The budget.
    #[must_use]
    pub(crate) fn get(self) -> Duration {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use browseros_trace::ErrorOwner;

    use super::RunTimeout;
    use crate::dispatch::ErrorCode;

    #[test]
    fn the_budget_defaults_to_five_minutes_and_stops_at_fifteen() -> Result<(), Box<dyn Error>> {
        assert_eq!(RunTimeout::from_millis(None)?.get(), Duration::from_mins(5));
        assert_eq!(
            RunTimeout::from_millis(Some(900_000))?.get(),
            Duration::from_mins(15)
        );
        for millis in [0, 900_001] {
            let error = RunTimeout::from_millis(Some(millis))
                .err()
                .ok_or("out of range")?;
            assert_eq!(
                (error.owner(), error.code()),
                (ErrorOwner::Script, ErrorCode::INVALID_INPUT),
                "{millis} ms is the model's mistake"
            );
        }
        Ok(())
    }
}
