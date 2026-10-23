use crate::api::mcp::dispatch::{ToolCall, ToolGuard, extract_page_id};
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;

pub fn guard(call: &ToolCall) -> BoxFuture<'_, Option<ToolResult>> {
    Box::pin(async move {
        let captcha = call.state.captcha.as_ref()?;
        if matches!(
            call.tool().name,
            "snapshot" | "diff" | "read" | "grep" | "screenshot" | "wait"
        ) {
            return None;
        }
        captcha
            .reservations()
            .busy_message(extract_page_id(call)?)
            .map(ToolResult::error)
    })
}

const _: ToolGuard = guard;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        api::mcp::test_support::tool_call,
        config::ServerMode,
        services::captcha::{
            CaptchaService,
            config::{CAPSOLVER_KEY_FILE_ENV, CaptchaConfig},
        },
    };
    use serde_json::json;

    pub(crate) fn enable_captcha(call: &mut ToolCall) -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), "dummy-test-key")?;
        call.state.captcha = CaptchaService::from_config(
            CaptchaConfig::from_lookup(|name| {
                (name == CAPSOLVER_KEY_FILE_ENV).then(|| file.path().to_string_lossy().into_owned())
            }),
            ServerMode::Hosted,
        );
        anyhow::ensure!(call.state.captcha.is_some(), "service disabled");
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_reserved_pages_refuse_writes_but_allow_read_only_tools()
    -> anyhow::Result<()> {
        for (tool, refused) in [
            ("act", true),
            ("navigate", true),
            ("evaluate", true),
            ("tabs", true),
            ("snapshot", false),
            ("diff", false),
            ("read", false),
            ("grep", false),
            ("screenshot", false),
            ("wait", false),
        ] {
            let mut call = tool_call(tool, json!({"page":42, "action":"close"})).await?;
            enable_captcha(&mut call)?;
            let service = call
                .state
                .captcha
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing service"))?;
            let _lease = service
                .reservations()
                .try_reserve(42, "cap_test")
                .map_err(|_| anyhow::anyhow!("busy"))?;
            let result = guard(&call).await;
            assert_eq!(result.is_some(), refused, "{tool}");
            if let Some(result) = result {
                assert!(result.is_error);
                assert_eq!(
                    serde_json::to_value(&result.content)?,
                    json!([{"type":"text", "text":service.reservations().busy_message(42)}])
                );
            }
            call.raw_args["page"] = json!(7);
            assert!(guard(&call).await.is_none(), "other page: {tool}");
            call.raw_args["page"] = json!(42);
            call.state.captcha = None;
            assert!(guard(&call).await.is_none(), "disabled: {tool}");
        }
        Ok(())
    }
}
