//! What a task page says when the plan turned an agent's profile choice away. The audit
//! observer marks those dispatches in `result_meta` (see `observers::audit`); task detail reads
//! the marks back, so the app can explain a stopped task or a refused choice without a new table.

use crate::db::audit_log::ToolDispatchRow;
use serde_json::Value;

/// The `result_meta` key the mark is written under.
pub(crate) const KEY: &str = "planWithheld";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProfileMark {
    /// The task was set to work in a profile, and the plan no longer has profile choice, so the
    /// call stopped instead of running in the Default profile.
    Stopped,
    /// An agent asked to list, load or pick a profile on a plan without it.
    Refused,
}

impl ProfileMark {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "profileStop",
            Self::Refused => "profileLocked",
        }
    }

    fn of(meta: &Value) -> Option<Self> {
        match meta.get(KEY)?.as_str()? {
            "profileStop" => Some(Self::Stopped),
            "profileLocked" => Some(Self::Refused),
            _ => None,
        }
    }
}

/// What the task page says now. The newest word wins: a stop stands only until a later call
/// succeeds (ultrafast came back and the task carried on), while a refused choice stays true of
/// the task. A stop still standing outranks a refused choice, since the page shows one note.
pub(crate) fn task_mark(dispatches: &[ToolDispatchRow]) -> Option<ProfileMark> {
    let mut rows = dispatches.iter().collect::<Vec<_>>();
    rows.sort_by_key(|row| (row.created_at, row.id));
    let mut stopped = false;
    let mut refused = false;
    for meta in rows
        .iter()
        .filter_map(|row| serde_json::from_str::<Value>(row.result_meta.as_deref()?).ok())
    {
        match ProfileMark::of(&meta) {
            Some(ProfileMark::Stopped) => stopped = true,
            Some(ProfileMark::Refused) => refused = true,
            None if meta.get("isError").and_then(Value::as_bool) == Some(false) => stopped = false,
            None => {}
        }
    }
    if stopped {
        Some(ProfileMark::Stopped)
    } else {
        refused.then_some(ProfileMark::Refused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(at: i64, meta: &str) -> ToolDispatchRow {
        ToolDispatchRow {
            id: at,
            created_at: at,
            agent_id: String::new(),
            slug: String::new(),
            agent_label: String::new(),
            session_id: String::new(),
            tool_name: "tabs".into(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            args_json: None,
            result_meta: Some(meta.into()),
            duration_ms: None,
            tool_input_token_estimate: 0,
            tool_output_token_estimate: 0,
            token_estimator_version: 1,
            dispatch_id: None,
            parent_dispatch_id: None,
            has_screenshot: false,
        }
    }

    const OK: &str = r#"{"isError":false}"#;
    const FAILED: &str = r#"{"isError":true}"#;
    const REFUSED: &str = r#"{"isError":true,"planWithheld":"profileLocked"}"#;
    const STOPPED: &str = r#"{"isError":true,"planWithheld":"profileStop"}"#;

    #[test]
    fn plain_failures_carry_no_mark_and_a_refused_choice_stays() {
        assert_eq!(task_mark(&[row(1, FAILED)]), None);
        assert_eq!(
            task_mark(&[row(1, REFUSED), row(2, OK)]),
            Some(ProfileMark::Refused)
        );
    }

    /// A task that stopped and then carried on (ultrafast came back) is no longer stopped; one
    /// that ran and then stopped is.
    #[test]
    fn the_newest_call_decides_whether_a_task_is_stopped() {
        assert_eq!(task_mark(&[row(1, STOPPED), row(2, OK)]), None);
        assert_eq!(
            task_mark(&[row(1, OK), row(2, STOPPED)]),
            Some(ProfileMark::Stopped)
        );
        // Order is by time, not by the order rows arrive in.
        assert_eq!(
            task_mark(&[row(2, STOPPED), row(1, OK)]),
            Some(ProfileMark::Stopped)
        );
        assert_eq!(
            task_mark(&[row(1, REFUSED), row(2, STOPPED), row(3, FAILED)]),
            Some(ProfileMark::Stopped)
        );
    }
}
