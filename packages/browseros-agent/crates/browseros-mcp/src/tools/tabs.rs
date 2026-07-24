use crate::framework::{
    ToolCtx, ToolExecResult, ToolResult, error_result, page_json, parse_args, text_result,
};
use browseros_core::{PageId, pages::NewPageOptions};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

const DESCRIPTION: &str = "\
Manage browser tabs: list open pages (with their page ids), show the active page, \
open a new page (snapshot attached), or close one. \
Use the returned page id with snapshot/act/navigate.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum TabsAction {
    #[default]
    List,
    Active,
    New,
    Close,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TabsArgs {
    #[serde(default)]
    action: TabsAction,
    /// URL for action="new" (defaults to about:blank).
    url: Option<String>,
    /// Open without stealing focus for action="new".
    #[serde(default = "super::default_true")]
    background: bool,
    /// Page id for action="close".
    page: Option<u32>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<TabsArgs>(
        "tabs",
        DESCRIPTION,
        Some(super::open_world_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: serde_json::Value,
    ctx: &'a ToolCtx,
    response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: TabsArgs = parse_args(raw)?;
        let result = match args.action {
            TabsAction::List => {
                let pages = ctx.session.pages.list().await?;
                let lines = pages.iter().map(format_page_line).collect::<Vec<_>>();
                text_result(
                    if lines.is_empty() {
                        "(no open pages)".to_string()
                    } else {
                        lines.join("\n")
                    },
                    Some(json!({
                        "pages": pages.iter().map(|page| json!({
                            "page": page.page_id.0,
                            "url": page.url,
                            "title": page.title,
                        })).collect::<Vec<_>>()
                    })),
                )
            }
            TabsAction::Active => {
                let Some(page) = ctx.session.pages.get_active().await? else {
                    return Ok(Some(error_result("tabs active: no active page found.")));
                };
                text_result(
                    format!("Active page: {}", format_page_line(&page)),
                    Some(json!({ "action": "active", "page": page_json(&page) })),
                )
            }
            TabsAction::New => {
                let page = ctx
                    .session
                    .pages
                    .new_page(
                        args.url.as_deref().unwrap_or("about:blank"),
                        NewPageOptions {
                            background: Some(args.background),
                            window_id: ctx.defaults.default_window_id.clone(),
                            tab_group_id: ctx.defaults.default_tab_group_id.clone(),
                        },
                    )
                    .await?;
                response.text(format!("opened page {}", page.0));
                // Claw-server hooks key ownership/grouping off this "page" field.
                response.data(json!({ "page": page.0 }));
                response.include_snapshot(page.0);
                return Ok(None);
            }
            TabsAction::Close => {
                let Some(page) = args.page else {
                    return Ok(Some(error_result("tabs close: page is required.")));
                };
                ctx.session.pages.close(PageId(page)).await?;
                text_result(format!("closed page {page}"), Some(json!({ "page": page })))
            }
        };
        Ok(Some(result))
    })
}

fn format_page_line(page: &browseros_core::pages::PageInfo) -> String {
    if page.title.is_empty() {
        format!("[{}] {}", page.page_id.0, page.url)
    } else {
        format!("[{}] {} ({})", page.page_id.0, page.url, page.title)
    }
}
