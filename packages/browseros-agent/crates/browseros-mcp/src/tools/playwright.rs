//! Playwright scripts enter the same bounded QuickJS runtime as the run SDK.

use super::run::{
    RunError, RunOutcome, RunOutput, ScriptSpec, default_timeout, execute_script,
    normalized_timeout_ms,
};
use crate::{
    framework::{ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args},
    pw,
};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

const DESCRIPTION: &str = "Execute Playwright JavaScript in the BrowserOS neo script runtime.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlaywrightArgs {
    /// Standard Playwright JavaScript body, no imports. Top-level await; `return` a JSON value.
    code: String,
    /// Max run time in ms. Default and hard cap 30000; larger values are clamped.
    #[serde(default = "default_timeout")]
    timeout: f64,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def_with_output::<PlaywrightArgs, RunOutput>(
        "playwright",
        DESCRIPTION,
        Some(super::open_world_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: PlaywrightArgs = parse_args(raw)?;
        let outcome = match execute_script(
            ScriptSpec {
                bootstrap_js: pw::FACADE_JS,
                code: args.code,
                timeout_ms: normalized_timeout_ms(args.timeout),
                helpers: false,
            },
            ctx,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(RunError::Syntax(message)) => {
                RunOutcome::failure(format!("playwright: syntax error - {message}"), Vec::new())
            }
            Err(RunError::Cancelled) => return Err(ToolError::Cancelled),
            Err(RunError::Engine(message)) => return Err(ToolError::message(message)),
        };
        Ok(Some(outcome.into_tool_result()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{framework::execute_tool, tools::run::tests::test_ctx};
    use serde_json::json;

    #[tokio::test]
    async fn playwright_exposes_globals_and_reports_stub_operations() -> anyhow::Result<()> {
        let ctx = test_ctx();
        let result = execute_tool(
            &definition(),
            json!({"code": "return typeof page + typeof expect"}),
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({"ok": true, "value": "objectfunction", "logs": []}))
        );

        let result = execute_tool(
            &definition(),
            json!({"code": "await page.goto('https://example.com')"}),
            &ctx,
        )
        .await?;
        assert!(result.is_error);
        let output = result
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("missing script output"))?;
        assert_eq!(output["ok"], false);
        assert!(
            output["error"]
                .as_str()
                .is_some_and(|error| error.contains("not implemented yet: page.goto"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn playwright_does_not_load_legacy_helpers() -> anyhow::Result<()> {
        let mut ctx = test_ctx();
        ctx.preloaded_helpers.push(crate::framework::HelperSource {
            name: "legacy".to_string(),
            source: "(() => { throw new Error('legacy helper evaluated'); })()".to_string(),
        });
        let result = execute_tool(
            &definition(),
            json!({"code": "return typeof helpers"}),
            &ctx,
        )
        .await?;
        assert_eq!(
            result.structured_content,
            Some(json!({"ok": true, "value": "undefined", "logs": []}))
        );
        Ok(())
    }

    #[derive(Default)]
    struct RecordingHook {
        authorized: std::sync::Mutex<Vec<Option<u32>>>,
        recorded: std::sync::Mutex<Vec<(String, Option<u32>, Value)>>,
    }

    impl crate::framework::InnerCallHook for RecordingHook {
        fn authorize<'a>(&'a self, page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async move {
                self.authorized
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(page);
                Ok(())
            })
        }

        fn record<'a>(
            &'a self,
            record: crate::framework::InnerCallRecord<'a>,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                assert!(record.is_error);
                assert!(!record.from_helper);
                assert!(record.secrets.is_empty());
                self.recorded
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((record.method.to_string(), record.page, record.args.clone()));
            })
        }

        fn on_page_created<'a>(&'a self, _page_id: u32) -> BoxFuture<'a, ()> {
            Box::pin(async { panic!("stub calls must not claim a page") })
        }
    }

    #[tokio::test]
    async fn playwright_bridge_routes_and_audits_page_first_methods() -> anyhow::Result<()> {
        let methods = [
            ("page.goto", Some(7)),
            ("locator.click", Some(7)),
            ("expect.toBeVisible", Some(7)),
            ("keyboard.press", Some(7)),
            ("mouse.click", Some(7)),
            ("frame.info", Some(7)),
            ("neo.snapshot", Some(7)),
            ("neo.read", Some(7)),
            ("neo.grep", Some(7)),
            ("neo.download", Some(7)),
            ("neo.page", Some(7)),
            ("context.newPage", None),
            ("context.pages", None),
            ("context.close", None),
            ("neo.pages", None),
            ("neo.cdp", None),
        ];
        let hook = std::sync::Arc::new(RecordingHook::default());
        let mut ctx = test_ctx();
        ctx.inner_call_hook = Some(hook.clone());
        let names = methods
            .iter()
            .map(|(method, _)| *method)
            .collect::<Vec<_>>();
        let code = format!(
            r#"
            const errors = [];
            for (const method of {}) {{
                try {{ await __browserosCall(method, '[7]', false); }}
                catch (error) {{ errors.push(error.message); }}
            }}
            return errors;
        "#,
            serde_json::to_string(&names)?
        );
        let result = execute_tool(&definition(), json!({"code": code}), &ctx).await?;
        assert!(!result.is_error);
        let output = result
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("missing output"))?;
        assert_eq!(
            output["value"],
            json!(
                names
                    .iter()
                    .map(|method| format!("not implemented yet: {method}"))
                    .collect::<Vec<_>>()
            )
        );
        assert_eq!(
            *hook
                .authorized
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            methods.iter().map(|(_, page)| *page).collect::<Vec<_>>()
        );
        assert_eq!(
            *hook
                .recorded
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            methods
                .iter()
                .map(|(method, page)| ((*method).to_string(), *page, json!([7])))
                .collect::<Vec<_>>()
        );
        Ok(())
    }
}
