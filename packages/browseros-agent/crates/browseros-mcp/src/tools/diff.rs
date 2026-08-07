use crate::{
    format::diff::format_diff_result,
    framework::{
        ToolCtx, ToolExecResult, ToolResult, parse_args, pending_dialog_result, text_result,
    },
};
use browseros_core::PageId;
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

const DESCRIPTION: &str = "\
Show what changed on the page since the last snapshot/diff - a cheap way to see \
an action's effect without re-dumping the whole tree.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct DiffArgs {
    page: u32,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<DiffArgs>(
        "diff",
        DESCRIPTION,
        Some(super::read_only_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: DiffArgs = parse_args(raw)?;
        if let Some(result) = pending_dialog_result(ctx, PageId(args.page)) {
            return Ok(Some(result));
        }
        let diff = ctx.session.observe(PageId(args.page)).await.diff().await?;
        let origin = diff.after_url.as_deref().map(ToString::to_string);
        let origin = match origin {
            Some(origin) => origin,
            None => ctx
                .session
                .pages
                .get_info(PageId(args.page))
                .await
                .map(|info| info.url)
                .unwrap_or_else(|| "unknown".to_string()),
        };
        let formatted = format_diff_result(&diff, &origin, ctx).await;
        Ok(Some(text_result(
            formatted.text,
            Some(formatted.structured),
        )))
    })
}
