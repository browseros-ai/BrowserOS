//! Ultrafast skills: a saved skill's verified flow, how it runs, and how it hands back.

pub mod brief;
pub mod compose;
pub mod journal;
pub mod lifecycle;
pub mod outcome;
pub mod prose;
pub mod store;
pub mod triage;
pub mod verify;

use crate::{clock::now_epoch_ms, db::entities::skill_flows};
use compose::Mode;
use futures_util::future::BoxFuture;
use outcome::Outcome;
use serde_json::{Map, Value};
use store::FlowDoc;

/// Runs a composed flow in the browser. The MCP service implements it with a real `run`
/// dispatch; tests implement it with canned outcomes.
pub trait FlowExec: Send + Sync {
    fn exec<'a>(
        &'a self,
        doc: &'a FlowDoc,
        inputs: &'a Map<String, Value>,
        mode: Mode,
        stop_after: Option<u32>,
    ) -> BoxFuture<'a, Outcome>;
}

/// Checks the caller's inputs against the flow's declared input types.
pub fn check_inputs(doc: &FlowDoc, raw: &Value) -> Result<Map<String, Value>, String> {
    let empty = Map::new();
    let given = match raw {
        Value::Object(map) => map,
        Value::Null => &empty,
        _ => return Err("inputs must be an object".to_string()),
    };
    for (name, kind) in &doc.meta.inputs {
        let Some(value) = given.get(name) else {
            return Err(format!("missing input `{name}` ({kind})"));
        };
        let ok = match kind.as_str() {
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            _ => value.is_string(),
        };
        if !ok {
            return Err(format!("input `{name}` must be a {kind}"));
        }
    }
    Ok(given.clone())
}

/// Where a skill's flow stands in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowStatus {
    None,
    Pending,
    Verified,
    Degraded,
    Broken,
    Off,
}

impl FlowStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pending => "pending",
            Self::Verified => "verified",
            Self::Degraded => "degraded",
            Self::Broken => "broken",
            Self::Off => "off",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw {
            "pending" => Self::Pending,
            "verified" => Self::Verified,
            "degraded" => Self::Degraded,
            "broken" => Self::Broken,
            "off" => Self::Off,
            _ => Self::None,
        }
    }

    /// Whether `run_skill` may run the flow.
    #[must_use]
    pub fn runs(self) -> bool {
        matches!(self, Self::Verified | Self::Degraded)
    }
}

/// A fresh lifecycle row for a skill entering the ultrafast path.
#[must_use]
pub fn new_row(skill_name: &str, status: FlowStatus) -> skill_flows::Model {
    skill_flows::Model {
        skill_name: skill_name.to_owned(),
        status: status.as_str().to_owned(),
        status_reason: None,
        flow_version: 0,
        attempts: 0,
        site_failures: 0,
        effects: None,
        write_approved: false,
        p50_ms: None,
        source_session_id: None,
        verified_at: None,
        repaired_at: None,
        updated_at: now_epoch_ms(),
    }
}

#[cfg(test)]
mod tests {
    use super::FlowStatus;

    #[test]
    fn status_round_trips_and_only_live_flows_run() {
        for status in [
            FlowStatus::None,
            FlowStatus::Pending,
            FlowStatus::Verified,
            FlowStatus::Degraded,
            FlowStatus::Broken,
            FlowStatus::Off,
        ] {
            assert_eq!(FlowStatus::parse(status.as_str()), status);
        }
        assert_eq!(FlowStatus::parse("garbage"), FlowStatus::None);
        assert!(FlowStatus::Verified.runs());
        assert!(FlowStatus::Degraded.runs());
        assert!(!FlowStatus::Broken.runs());
        assert!(!FlowStatus::Pending.runs());
    }
}
