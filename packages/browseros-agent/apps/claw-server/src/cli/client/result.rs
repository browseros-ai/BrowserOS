//! Turning one JSON-RPC reply into something the CLI can print and exit on.

use serde_json::Value;

use crate::cli::exit::Code;

/// A tool result, flattened out of the MCP envelope.
#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    pub text: String,
    pub structured: Option<Value>,
    pub is_error: bool,
    /// Base64 image blocks, which a terminal cannot render and which therefore
    /// become files.
    pub images: Vec<ImageBlock>,
}

#[derive(Debug, Clone)]
pub struct ImageBlock {
    pub data: String,
    pub mime_type: String,
}

/// The line the server appends when it issues a logical session handle.
///
/// Over HTTP with a transport session id the handle is withheld, so this should
/// never appear. It would appear over stdio. Filtering it costs nothing and
/// keeps stdout clean if the transport ever changes.
const HANDLE_PREFIX: &str = "[browseros-neo session:";

impl ToolOutput {
    /// Which exit code this result deserves.
    ///
    /// The three special cases are the ones a script must be able to branch on:
    /// a human pressing Stop in the cockpit, the browser link being down, and a
    /// `run` that hit the wall-clock cap.
    #[must_use]
    pub fn exit_code(&self) -> Code {
        if !self.is_error {
            return Code::Ok;
        }
        if self.is_operator_cancelled() {
            return Code::Stopped;
        }
        if self.is_browser_unavailable() {
            return Code::NoBrowser;
        }
        if self.is_timeout() {
            return Code::Timeout;
        }
        Code::ToolFailed
    }

    /// `operator_cancellation_result` sets this in structured content. Matching
    /// on the kind rather than on prose keeps the check stable if the sentence
    /// is ever reworded.
    #[must_use]
    pub fn is_operator_cancelled(&self) -> bool {
        self.structured
            .as_ref()
            .and_then(|value| value.get("cancellationKind"))
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.starts_with("cockpit."))
    }

    /// `browser_unavailable_message` has three variants and every one of them
    /// contains this clause, because every one of them advises retrying rather
    /// than relaunching.
    #[must_use]
    pub fn is_browser_unavailable(&self) -> bool {
        let text = self.text.to_ascii_lowercase();
        text.contains("browser link") || text.contains("no link to the browser")
    }

    #[must_use]
    pub fn is_timeout(&self) -> bool {
        let text = self.text.to_ascii_lowercase();
        text.contains("exceeded") && text.contains("ms")
    }
}

/// Flattens an MCP `CallToolResult` into [`ToolOutput`].
#[must_use]
pub fn from_call_result(result: &Value) -> ToolOutput {
    let mut output = ToolOutput {
        is_error: result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        structured: result.get("structuredContent").cloned(),
        ..ToolOutput::default()
    };

    let mut lines: Vec<String> = Vec::new();
    if let Some(blocks) = result.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        if text.trim_start().starts_with(HANDLE_PREFIX) {
                            continue;
                        }
                        lines.push(text.to_string());
                    }
                }
                Some("image") => {
                    if let (Some(data), mime) = (
                        block.get("data").and_then(Value::as_str),
                        block.get("mimeType").and_then(Value::as_str),
                    ) {
                        output.images.push(ImageBlock {
                            data: data.to_string(),
                            mime_type: mime.unwrap_or("image/png").to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    output.text = lines.join("\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_text_result_flattens_to_its_text() {
        let output = from_call_result(&json!({
            "content": [{ "type": "text", "text": "navigated (url) -> https://x" }]
        }));
        assert_eq!(output.text, "navigated (url) -> https://x");
        assert_eq!(output.exit_code(), Code::Ok);
    }

    #[test]
    fn the_stdio_session_handle_line_is_stripped() {
        let output = from_call_result(&json!({
            "content": [
                { "type": "text", "text": "opened page 3" },
                { "type": "text", "text": "[browseros-neo session: abc. Pass this exact value…]" }
            ]
        }));
        assert_eq!(output.text, "opened page 3");
    }

    #[test]
    fn a_cockpit_stop_is_its_own_exit_code() {
        let output = from_call_result(&json!({
            "isError": true,
            "content": [{ "type": "text", "text": "cancelled" }],
            "structuredContent": { "cancellationKind": "cockpit.operator-cancelled" }
        }));
        assert_eq!(output.exit_code(), Code::Stopped);
    }

    #[test]
    fn a_down_browser_link_is_exit_three_not_a_tool_failure() {
        let output = from_call_result(&json!({
            "isError": true,
            "content": [{ "type": "text", "text":
                "browser link lost 4s ago and the server is reconnecting; BrowserOS neo is running" }]
        }));
        assert_eq!(output.exit_code(), Code::NoBrowser);
    }

    #[test]
    fn an_image_block_never_lands_in_the_text() {
        let output = from_call_result(&json!({
            "content": [{ "type": "image", "data": "AAAA", "mimeType": "image/jpeg" }]
        }));
        assert!(output.text.is_empty());
        assert_eq!(output.images.len(), 1);
        assert_eq!(output.images[0].mime_type, "image/jpeg");
    }
}
