use crate::framework::{ToolCtx, ToolExecResult, ToolResult, error_result, parse_args};
use browseros_core::PageId;
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

const DESCRIPTION: &str = "\
Navigate a page: load a url, or go back/forward/reload. \
Returns a fresh snapshot of the resulting page \
(navigation invalidates refs, so old [ref=eN] handles no longer apply).";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum NavigateAction {
    #[default]
    Url,
    Back,
    Forward,
    Reload,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct NavigateArgs {
    /// Page id from `tabs`.
    page: u32,
    #[serde(default)]
    action: NavigateAction,
    /// Required when action is "url".
    url: Option<String>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<NavigateArgs>("navigate", DESCRIPTION, None, handler)
}

fn handler<'a>(
    raw: serde_json::Value,
    ctx: &'a ToolCtx,
    response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: NavigateArgs = parse_args(raw)?;
        let nav = ctx.session.nav(PageId(args.page));
        let action = match args.action {
            NavigateAction::Url => {
                let Some(url) = args.url.as_deref().filter(|url| !url.is_empty()) else {
                    return Ok(Some(error_result(
                        "navigate: url is required for action=\"url\".",
                    )));
                };
                nav.goto(url).await?;
                "url"
            }
            NavigateAction::Back => {
                nav.back().await?;
                "back"
            }
            NavigateAction::Forward => {
                nav.forward().await?;
                "forward"
            }
            NavigateAction::Reload => {
                nav.reload().await?;
                "reload"
            }
        };
        let origin = ctx
            .session
            .pages
            .refresh(PageId(args.page))
            .await?
            .map(|info| info.url);
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
        response.text(format!("navigated ({action}) -> {origin}"));
        response.data(json!({ "page": args.page, "url": origin }));
        response.include_snapshot(args.page);
        Ok(None)
    })
}
