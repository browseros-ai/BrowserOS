//! Read the script's value from the same MCP text an agent receives.

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;

use crate::mcp;

/// Returns the Result value or Error text; unrelated content is reported as a non-run reply.
pub fn run_value(result: &CallToolResult) -> Result<Value, String> {
    let [content] = result.content.as_slice() else {
        return Err("not a run reply: expected one text block".to_owned());
    };
    let text = ContentBlock::as_text(content)
        .ok_or_else(|| "not a run reply: expected text".to_owned())?;
    mcp::parse_text(&text.text)
        .ok_or_else(|| "not a run reply: invalid sections or JSON value".to_owned())?
        .value
}

#[cfg(test)]
mod tests {
    use rmcp::model::{CallToolResult, ContentBlock};
    use serde_json::json;

    use super::run_value;

    #[test]
    fn reads_success_errors_and_unrelated_content() {
        assert_eq!(
            run_value(&CallToolResult::success(vec![ContentBlock::text(
                "### Result\n{\"a\":1}\n### Console\nhello"
            )])),
            Ok(json!({ "a": 1 }))
        );
        assert_eq!(
            run_value(&CallToolResult::error(vec![ContentBlock::text(
                "### Error\nscript / script_error: failed\n### Console\nhello"
            )])),
            Err("script / script_error: failed".to_owned())
        );
        for content in [
            vec![],
            vec![ContentBlock::text("42")],
            vec![ContentBlock::text("### Result\ninvalid")],
        ] {
            assert!(
                run_value(&CallToolResult::success(content))
                    .is_err_and(|error| error.starts_with("not a run reply: "))
            );
        }
    }
}
