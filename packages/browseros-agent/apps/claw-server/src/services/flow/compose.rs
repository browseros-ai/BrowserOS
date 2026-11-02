//! Builds the one `run` script that carries a flow: the prelude, then a call into it with the
//! step sources and inputs as JSON data. Sources are compiled inside the engine, one step at a
//! time, so nothing here has to parse JavaScript.

use super::store::FlowDoc;
use serde_json::{Map, Value, json};

/// Every flow script starts with this, so the run journal can tell flow runs from the agent's.
pub const FLOW_SCRIPT_MARKER: &str = "/*__ultrafast__*/";
const PRELUDE: &str = include_str!("prelude.js");
const VIEW_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The real run: writes happen.
    Live,
    /// Verification: the commit step proves its target and stops before writing.
    Verify,
    /// Compile every step and return, to report syntax errors per step.
    Parse,
    /// Replay up to a step to tell a site change from a bad input.
    Triage,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Verify => "verify",
            Self::Parse => "parse",
            Self::Triage => "triage",
        }
    }
}

#[must_use]
pub fn is_flow_script(code: &str) -> bool {
    code.trim_start().starts_with(FLOW_SCRIPT_MARKER)
}

#[must_use]
pub fn compose(
    doc: &FlowDoc,
    inputs: &Map<String, Value>,
    mode: Mode,
    stop_after: Option<u32>,
    policy: browseros_mcp::RunPolicy,
) -> String {
    let steps: Vec<Value> = doc
        .steps
        .iter()
        .map(|step| json!({ "n": step.n, "src": step.src, "commit": step.commit }))
        .collect();
    // Derive handback from the exact policy used by the underlying run dispatch.
    // A deployment override must never leave a flow waiting past its hard deadline.
    let options = json!({
        "steps": steps,
        "inputs": inputs,
        "mode": mode.as_str(),
        "stopAfter": stop_after,
        "deadlineMs": policy.handback_timeout().as_millis() as u64,
        "viewBytes": VIEW_BYTES,
    });
    format!("{FLOW_SCRIPT_MARKER}\n{PRELUDE}\nreturn await __ultrafast.run(browser, {options});\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::flow::store::{Effects, FlowMeta, FlowStep};

    #[test]
    fn sources_and_inputs_travel_as_data() -> anyhow::Result<()> {
        let doc = FlowDoc {
            meta: FlowMeta {
                name: "neo-x".to_string(),
                flow_version: 1,
                effects: Effects::Read,
                hosts: vec![],
                inputs: Default::default(),
                output: Default::default(),
                examples: vec![],
                source_session: None,
            },
            steps: vec![FlowStep {
                n: 1,
                commit: false,
                src: "async () => `</script>${'\"'}`".to_string(),
            }],
        };
        let inputs = json!({ "q": "a\"b\n`c`" })
            .as_object()
            .cloned()
            .unwrap_or_default();
        let script = compose(
            &doc,
            &inputs,
            Mode::Verify,
            Some(1),
            browseros_mcp::RunPolicy::default(),
        );
        assert!(is_flow_script(&script));
        let tail = script
            .rsplit_once("__ultrafast.run(browser, ")
            .map(|(_, tail)| tail.trim_end().trim_end_matches(");"))
            .unwrap_or_default();
        let options: Value = serde_json::from_str(tail)?;
        assert_eq!(options["steps"][0]["src"], doc.steps[0].src);
        assert_eq!(options["inputs"]["q"], "a\"b\n`c`");
        assert_eq!(options["mode"], "verify");
        assert_eq!(options["stopAfter"], 1);
        assert_eq!(options["deadlineMs"], 55000);
        for (seconds, expected) in [(1, 500), (120, 115000)] {
            let policy = browseros_mcp::RunPolicy::from_seconds(
                std::num::NonZeroU32::new(seconds)
                    .ok_or_else(|| anyhow::anyhow!("invalid test budget"))?,
            );
            let script = compose(&doc, &inputs, Mode::Live, None, policy);
            let tail = script
                .rsplit_once("__ultrafast.run(browser, ")
                .map(|(_, tail)| tail.trim_end().trim_end_matches(");"))
                .unwrap_or_default();
            let options: Value = serde_json::from_str(tail)?;
            assert_eq!(options["deadlineMs"], expected);
        }
        Ok(())
    }
}
