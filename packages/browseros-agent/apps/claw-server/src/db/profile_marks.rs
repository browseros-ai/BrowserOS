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

    fn of(row: &ToolDispatchRow) -> Option<Self> {
        let meta = serde_json::from_str::<Value>(row.result_meta.as_deref()?).ok()?;
        match meta.get(KEY)?.as_str()? {
            "profileStop" => Some(Self::Stopped),
            "profileLocked" => Some(Self::Refused),
            _ => None,
        }
    }
}

/// The strongest mark in a task: a stop outranks a refused choice, since the task page shows one
/// note and a stopped task needs the user's attention first.
pub(crate) fn task_mark(dispatches: &[ToolDispatchRow]) -> Option<ProfileMark> {
    let marks = dispatches.iter().filter_map(ProfileMark::of);
    let mut found = None;
    for mark in marks {
        if mark == ProfileMark::Stopped {
            return Some(mark);
        }
        found = Some(mark);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(meta: &str) -> ToolDispatchRow {
        ToolDispatchRow {
            id: 1,
            created_at: 0,
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

    #[test]
    fn a_stop_outranks_a_refused_choice_and_plain_failures_carry_none() {
        let plain = row(r#"{"isError":true}"#);
        let refused = row(r#"{"isError":true,"planWithheld":"profileLocked"}"#);
        let stopped = row(r#"{"isError":true,"planWithheld":"profileStop"}"#);
        assert_eq!(task_mark(std::slice::from_ref(&plain)), None);
        assert_eq!(
            task_mark(&[plain.clone(), refused.clone()]),
            Some(ProfileMark::Refused)
        );
        assert_eq!(
            task_mark(&[refused, stopped, plain]),
            Some(ProfileMark::Stopped)
        );
    }
}
