use rmcp::model::Tool;
use serde_json::json;

pub fn solve_captcha_tool() -> Tool {
    let schema = serde_json::Map::from_iter([
        ("type".into(), json!("object")),
        (
            "properties".into(),
            json!({"page": {"type": "integer", "minimum": 1}}),
        ),
        ("required".into(), json!(["page"])),
    ]);
    Tool::new(
        "solve_captcha",
        "Call when a snapshot shows a CAPTCHA or an action appears blocked. Detects, solves, and verifies the CAPTCHA on the page. Do not act on the page while this runs. Never click CAPTCHAs yourself.",
        schema,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tests_that_tool_schema_requires_a_positive_integer_page() {
        let tool = solve_captcha_tool();
        assert_eq!(tool.name, "solve_captcha");
        assert_eq!(tool.input_schema.get("type"), Some(&json!("object")));
        assert_eq!(tool.input_schema.get("required"), Some(&json!(["page"])));
        assert_eq!(
            tool.input_schema.get("properties"),
            Some(&json!({"page": {"type": "integer", "minimum": 1}}))
        );
    }
}
