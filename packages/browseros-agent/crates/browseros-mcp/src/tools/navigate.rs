use crate::framework::{ToolCtx, ToolExecResult, ToolResult, error_result, parse_args};
use browseros_core::PageId;
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

const DESCRIPTION: &str = "\
Navigate a page: load a url, or go back/forward/reload. \
Returns a fresh snapshot of the resulting page \
(navigation invalidates refs, so old [ref=eN] handles no longer apply). \
Returns once the page has stopped changing, not merely once it has loaded, \
so do not pause afterwards to let it catch up. \
A url the browser cannot load (unknown host, refused connection) is an error; \
an HTTP error status is reported with the page; \
a url that is a file downloads it and leaves the page as it was. \
back/forward with no history entry in that direction leave the page as it was and say so.";

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
#[serde(deny_unknown_fields)]
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
        let nav = ctx.browser.nav(PageId(args.page)).await?;
        let mut status = None;
        let action = match args.action {
            NavigateAction::Url => {
                let Some(url) = args.url.as_deref().filter(|url| !url.is_empty()) else {
                    return Ok(Some(error_result(
                        "navigate: url is required for action=\"url\".",
                    )));
                };
                let outcome = nav.goto(url).await?;
                if outcome.download {
                    response.text(format!(
                        "{url} is a file, so the browser downloaded it; the page did not change."
                    ));
                    response.data(json!({ "page": args.page, "download": true }));
                    return Ok(None);
                }
                status = outcome.status;
                "url"
            }
            NavigateAction::Back | NavigateAction::Forward => {
                let back = matches!(args.action, NavigateAction::Back);
                let moved = if back {
                    nav.back().await?
                } else {
                    nav.forward().await?
                };
                let direction = if back { "back" } else { "forward" };
                if !moved {
                    response.text(format!(
                        "There is no {direction} history entry; the page did not change."
                    ));
                    response.data(json!({ "page": args.page, "navigated": false }));
                    return Ok(None);
                }
                direction
            }
            NavigateAction::Reload => {
                nav.reload().await?;
                "reload"
            }
        };
        let origin = ctx
            .browser
            .refresh_page(PageId(args.page))
            .await?
            .map(|info| info.url);
        let origin = match origin {
            Some(origin) => origin,
            None => ctx
                .browser
                .refresh_page(PageId(args.page))
                .await?
                .map(|info| info.url)
                .unwrap_or_else(|| "unknown".to_string()),
        };
        let http_error = status
            .filter(|status| *status >= 400)
            .map(|status| format!(" (HTTP {status})"))
            .unwrap_or_default();
        response.text(format!("navigated ({action}) -> {origin}{http_error}"));
        let mut data = json!({ "page": args.page, "url": origin });
        if let Some(status) = status {
            data["status"] = json!(status);
        }
        response.data(data);
        response.include_snapshot(args.page);
        Ok(None)
    })
}
