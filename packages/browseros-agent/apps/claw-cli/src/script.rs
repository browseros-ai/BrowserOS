//! What makes `run` the primary way to drive the browser from a shell.
//!
//! Over MCP, `run`'s win is collapsing agent turns. A shell already does that
//! with `&&`, so that half of the argument does not transfer. What survives
//! and decides it is fan-out (`Promise.all` means N pages cost one wait cycle
//! rather than N), saved helpers (which only work inside `run`), and output
//! volume. Measured on the same five-page task: 523 ms and 130 bytes in one
//! call against 2,356 ms and 4,410 bytes across fifteen.
//!
//! Two things follow for the CLI: the script should not have to open with a
//! preamble the CLI could write itself, and a run that hits the wall-clock cap
//! must be reported as the dangerous thing it is.

use serde_json::Value;

use crate::client::result::ToolOutput;

/// Wraps a script so `page` is already bound to the current page.
///
/// A parameter rather than a `const` is the point. A script written for MCP
/// may open with its own `const page = browser.page(2)`; inside an arrow that
/// shadows the parameter legally instead of throwing a redeclaration error, so
/// the wrapper can never break a script that was written against the tool.
/// Top-level `await` and top-level `return` both still work inside it.
#[must_use]
pub fn with_page(code: &str, page: u32) -> String {
    format!("return await (async (page) => {{\n{code}\n}})(browser.page({page}));")
}

/// Whether a run or evaluate result is the server's wall-clock kill.
///
/// Matched on the structured error rather than on the rendered text, because
/// the text also carries the logs and a substring search over those would
/// misfire on a script that logged the word.
#[must_use]
pub fn hit_the_cap(output: &ToolOutput) -> bool {
    output
        .structured
        .as_ref()
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .is_some_and(is_cap_message)
        || output
            .text
            .lines()
            .next()
            .is_some_and(|line| line.strip_prefix("error: ").is_some_and(is_cap_message))
}

fn is_cap_message(error: &str) -> bool {
    error.starts_with("run exceeded") && error.ends_with("ms")
}

/// The logs a killed run left behind, which are all the evidence there is.
///
/// `console.log` output survives the kill and is returned in a `logs:`
/// section, which is why logging is a correctness requirement in a long
/// script rather than a debugging nicety.
#[must_use]
pub fn surviving_logs(output: &ToolOutput) -> Option<String> {
    if let Some(logs) = output
        .structured
        .as_ref()
        .and_then(|value| value.get("logs"))
        .and_then(Value::as_array)
        && !logs.is_empty()
    {
        return Some(
            logs.iter()
                .filter_map(|line| line.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    let (_, rest) = output.text.split_once("logs:\n")?;
    Some(rest.to_string())
}

/// What to say when a run is killed at the cap.
///
/// The last two lines matter more than the exit code. A timed-out run is not a
/// no-op: side effects from before the kill persist, verified against a live
/// server, so an agent that assumes otherwise acts on a page in a state it did
/// not expect.
pub const CAP_ADVICE: &str = "[run hit the 30s wall-clock cap (not your --script-timeout; the server clamps).\n Anything the script already did is still done, so the browser may be mid-flow.\n Check with: browseros snapshot\n Split the work into bounded chunks rather than raising the timeout.]";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_wrapper_binds_page_as_a_parameter_so_a_local_const_can_shadow_it() {
        let wrapped = with_page("const page = browser.page(9);\nreturn page.id;", 3);
        assert!(
            wrapped.starts_with("return await (async (page) => {"),
            "{wrapped}"
        );
        assert!(wrapped.ends_with("})(browser.page(3));"), "{wrapped}");
        // The caller's own declaration is carried through untouched.
        assert!(
            wrapped.contains("const page = browser.page(9);"),
            "{wrapped}"
        );
    }

    #[test]
    fn a_wall_clock_kill_is_recognised_from_the_structured_error() {
        let output = ToolOutput {
            is_error: true,
            text: "error: run exceeded 30000ms".into(),
            structured: Some(json!({ "ok": false, "error": "run exceeded 30000ms", "logs": [] })),
            ..ToolOutput::default()
        };
        assert!(hit_the_cap(&output));
    }

    #[test]
    fn a_script_that_merely_logs_the_word_is_not_a_cap_hit() {
        let output = ToolOutput {
            is_error: false,
            text: "ok\nlogs:\nrun exceeded expectations".into(),
            structured: Some(json!({ "ok": true, "logs": ["run exceeded expectations"] })),
            ..ToolOutput::default()
        };
        assert!(!hit_the_cap(&output));
    }

    #[test]
    fn the_surviving_logs_are_recovered_from_either_shape() {
        let structured = ToolOutput {
            structured: Some(json!({ "logs": ["step 1", "step 2"] })),
            ..ToolOutput::default()
        };
        assert_eq!(
            surviving_logs(&structured).as_deref(),
            Some("step 1\nstep 2")
        );

        let text_only = ToolOutput {
            text: "error: run exceeded 30000ms\nlogs:\nstep 1\nstep 2".into(),
            ..ToolOutput::default()
        };
        assert_eq!(
            surviving_logs(&text_only).as_deref(),
            Some("step 1\nstep 2")
        );
    }

    #[test]
    fn a_run_with_no_logs_has_nothing_to_recover() {
        let output = ToolOutput {
            text: "error: run exceeded 30000ms".into(),
            structured: Some(json!({ "logs": [] })),
            ..ToolOutput::default()
        };
        assert_eq!(surviving_logs(&output), None);
    }
}
