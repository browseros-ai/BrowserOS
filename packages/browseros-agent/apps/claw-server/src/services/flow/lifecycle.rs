//! How a flow's status moves. Sites change, so flows break; a break that the triage blames on
//! the site degrades the flow and asks for a one-step patch, while a strange input or a slow
//! page is not held against it. A flow that keeps failing is switched off before it costs more
//! than running the skill the normal way.

use super::FlowStatus;

/// Failing patches, or site-changed handovers without a passing patch, before a flow breaks.
pub const BREAK_AFTER: i64 = 3;

/// Why a flow handed back, decided by `triage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The page no longer looks the way the flow expects.
    SiteChanged,
    /// The flow still works on its original input; this input was the problem.
    InputOnly,
    /// A slow page, a captcha or a sign-in wall: nobody's fault, nothing to repair.
    Transient,
}

impl Cause {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SiteChanged => "site_changed",
            Self::InputOnly => "input_only",
            Self::Transient => "transient",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    RunDone,
    Handover(Cause),
    FlowPassed,
    FlowFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub status: FlowStatus,
    /// Failing submits since the flow last passed.
    pub attempts: i64,
    /// Site-changed handovers since the flow last passed.
    pub site_failures: i64,
}

#[must_use]
pub fn next(state: State, event: Event) -> State {
    let State {
        status,
        attempts,
        site_failures,
    } = state;
    match event {
        Event::RunDone if status.runs() => State {
            status: FlowStatus::Verified,
            attempts,
            site_failures: 0,
        },
        Event::RunDone | Event::Handover(Cause::InputOnly | Cause::Transient) => state,
        Event::Handover(Cause::SiteChanged) => match status {
            FlowStatus::Verified => State {
                status: FlowStatus::Degraded,
                attempts: 0,
                site_failures: site_failures + 1,
            },
            FlowStatus::Degraded => State {
                status: if site_failures + 1 >= BREAK_AFTER {
                    FlowStatus::Broken
                } else {
                    FlowStatus::Degraded
                },
                attempts,
                site_failures: site_failures + 1,
            },
            _ => state,
        },
        Event::FlowPassed => State {
            status: FlowStatus::Verified,
            attempts: 0,
            site_failures: 0,
        },
        Event::FlowFailed => {
            let attempts = attempts + 1;
            let exhausted = attempts >= BREAK_AFTER;
            let status = match status {
                FlowStatus::Degraded | FlowStatus::Broken if exhausted => FlowStatus::Broken,
                FlowStatus::Pending | FlowStatus::None | FlowStatus::Off if exhausted => {
                    FlowStatus::Off
                }
                other => other,
            };
            State {
                status,
                attempts,
                site_failures,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(status: FlowStatus, attempts: i64, site_failures: i64) -> State {
        State {
            status,
            attempts,
            site_failures,
        }
    }

    #[test]
    fn the_lifecycle_table() {
        use Cause::*;
        use Event::*;
        use FlowStatus::*;
        let rows = [
            (state(Verified, 0, 0), RunDone, state(Verified, 0, 0)),
            (state(Degraded, 1, 1), RunDone, state(Verified, 1, 0)),
            (
                state(Verified, 0, 0),
                Handover(InputOnly),
                state(Verified, 0, 0),
            ),
            (
                state(Degraded, 0, 1),
                Handover(Transient),
                state(Degraded, 0, 1),
            ),
            (
                state(Verified, 2, 0),
                Handover(SiteChanged),
                state(Degraded, 0, 1),
            ),
            (
                state(Degraded, 0, 1),
                Handover(SiteChanged),
                state(Degraded, 0, 2),
            ),
            (
                state(Degraded, 0, 2),
                Handover(SiteChanged),
                state(Broken, 0, 3),
            ),
            (state(Degraded, 2, 2), FlowPassed, state(Verified, 0, 0)),
            (state(Broken, 1, 3), FlowPassed, state(Verified, 0, 0)),
            (state(Pending, 0, 0), FlowFailed, state(Pending, 1, 0)),
            (state(Pending, 2, 0), FlowFailed, state(Off, 3, 0)),
            (state(Degraded, 2, 1), FlowFailed, state(Broken, 3, 1)),
            // A failed resubmit of a working flow leaves the working flow in place.
            (state(Verified, 2, 0), FlowFailed, state(Verified, 3, 0)),
            (
                state(Broken, 0, 3),
                Handover(SiteChanged),
                state(Broken, 0, 3),
            ),
        ];
        for (from, event, to) in rows {
            assert_eq!(next(from, event), to, "{from:?} + {event:?}");
        }
    }
}
