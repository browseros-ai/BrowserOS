use crate::framework::{
    ToolCtx, ToolError, ToolExecResult, ToolResult, error_result, parse_args, text_result,
};
use browseros_cdp::browser::TabGroupInfo;
use browseros_core::{PageId, TabId};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DESCRIPTION: &str = "\
Manage tab groups: list groups, group pages, update a group (title/color/collapsed), \
ungroup pages, or close a group. Page ids come from the tabs tool.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum TabGroupsAction {
    #[default]
    List,
    Create,
    Update,
    Ungroup,
    Close,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum TabGroupColor {
    Grey,
    Blue,
    Red,
    Yellow,
    Green,
    Pink,
    Purple,
    Cyan,
    Orange,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct TabGroupsArgs {
    #[serde(default)]
    action: TabGroupsAction,
    /// Page ids for action="create" or "ungroup".
    pages: Option<Vec<u32>>,
    /// Group id. Required for "update"/"close". Optional on "create" to add pages to an existing group.
    #[serde(rename = "groupId")]
    group_id: Option<String>,
    /// Group title for "create"/"update".
    title: Option<String>,
    /// Group color for "update".
    color: Option<TabGroupColor>,
    /// Collapse/expand the group for "update".
    collapsed: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TabGroupWithPages {
    group_id: String,
    window_id: i64,
    title: String,
    color: String,
    collapsed: bool,
    page_ids: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct GroupsResult {
    groups: Vec<TabGroupInfo>,
}

#[derive(Debug, Deserialize)]
struct GroupResult {
    group: TabGroupInfo,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<TabGroupsArgs>(
        "tab_groups",
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
        let args: TabGroupsArgs = parse_args(raw)?;
        let result = match args.action {
            TabGroupsAction::List => {
                let groups: GroupsResult = serde_json::from_value(
                    ctx.session
                        .cdp("Browser.getTabGroups", json!({}), None)
                        .await?,
                )?;
                let mut resolved = Vec::new();
                for group in groups.groups {
                    resolved.push(with_pages(ctx, group).await?);
                }
                let text = if resolved.is_empty() {
                    "(no tab groups)".to_string()
                } else {
                    resolved
                        .iter()
                        .map(format_group)
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                text_result(
                    text,
                    Some(json!({ "groups": resolved, "count": resolved.len() })),
                )
            }
            TabGroupsAction::Create => {
                let Some(page_ids) = args.pages.as_deref().filter(|pages| !pages.is_empty()) else {
                    return Ok(Some(error_result("tab_groups create: pages is required.")));
                };
                if args.group_id.is_some() && args.title.is_some() {
                    return Ok(Some(error_result(
                        "tab_groups create: title cannot be set when adding pages to an existing groupId; use action=\"update\" to rename.",
                    )));
                }
                let tab_ids = to_tab_ids(ctx, page_ids).await?;
                let value = if let Some(group_id) = args.group_id {
                    ctx.session
                        .cdp(
                            "Browser.addTabsToGroup",
                            json!({ "groupId": group_id, "tabIds": tab_ids }),
                            None,
                        )
                        .await?
                } else {
                    let mut params = json!({ "tabIds": tab_ids });
                    if let (Value::Object(object), Some(title)) = (&mut params, args.title) {
                        object.insert("title".to_string(), Value::String(title));
                    }
                    ctx.session
                        .cdp("Browser.createTabGroup", params, None)
                        .await?
                };
                let group: GroupResult = serde_json::from_value(value)?;
                let resolved = with_pages(ctx, group.group).await?;
                text_result(
                    format!("grouped into {}", format_group(&resolved)),
                    Some(json!({ "group": resolved })),
                )
            }
            TabGroupsAction::Update => {
                let Some(group_id) = args.group_id else {
                    return Ok(Some(error_result(
                        "tab_groups update: groupId is required.",
                    )));
                };
                if args.title.is_none() && args.color.is_none() && args.collapsed.is_none() {
                    return Ok(Some(error_result(
                        "tab_groups update: provide at least one of title, color, or collapsed.",
                    )));
                }
                let mut params = json!({ "groupId": group_id });
                if let Value::Object(object) = &mut params {
                    if let Some(title) = args.title {
                        object.insert("title".to_string(), Value::String(title));
                    }
                    if let Some(color) = args.color {
                        object.insert(
                            "color".to_string(),
                            Value::String(
                                serde_json::to_value(color)?
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string(),
                            ),
                        );
                    }
                    if let Some(collapsed) = args.collapsed {
                        object.insert("collapsed".to_string(), Value::Bool(collapsed));
                    }
                }
                let group: GroupResult = serde_json::from_value(
                    ctx.session
                        .cdp("Browser.updateTabGroup", params, None)
                        .await?,
                )?;
                let resolved = with_pages(ctx, group.group).await?;
                text_result(
                    format!("updated {}", format_group(&resolved)),
                    Some(json!({ "group": resolved })),
                )
            }
            TabGroupsAction::Ungroup => {
                let Some(page_ids) = args.pages.as_deref().filter(|pages| !pages.is_empty()) else {
                    return Ok(Some(error_result("tab_groups ungroup: pages is required.")));
                };
                let tab_ids = to_tab_ids(ctx, page_ids).await?;
                ctx.session
                    .cdp(
                        "Browser.removeTabsFromGroup",
                        json!({ "tabIds": tab_ids }),
                        None,
                    )
                    .await?;
                text_result(
                    format!("ungrouped {} page(s)", page_ids.len()),
                    Some(json!({ "pageIds": page_ids, "count": page_ids.len() })),
                )
            }
            TabGroupsAction::Close => {
                let Some(group_id) = args.group_id else {
                    return Ok(Some(error_result("tab_groups close: groupId is required.")));
                };
                ctx.session
                    .cdp(
                        "Browser.closeTabGroup",
                        json!({ "groupId": group_id }),
                        None,
                    )
                    .await?;
                text_result(
                    format!("closed tab group {group_id} and all its tabs"),
                    Some(json!({ "groupId": group_id })),
                )
            }
        };
        Ok(Some(result))
    })
}

async fn to_tab_ids(ctx: &ToolCtx, page_ids: &[u32]) -> ToolExecResult<Vec<i64>> {
    ctx.session.pages.list().await?;
    let mut out = Vec::with_capacity(page_ids.len());
    for page_id in page_ids {
        let info = ctx
            .session
            .pages
            .get_info(PageId(*page_id))
            .await
            .ok_or_else(|| {
                ToolError::message(format!(
                    "Unknown page {page_id}. Use the tabs tool to list pages."
                ))
            })?;
        out.push(info.tab_id.0);
    }
    Ok(out)
}

async fn with_pages(ctx: &ToolCtx, group: TabGroupInfo) -> ToolExecResult<TabGroupWithPages> {
    let tab_ids = group.tab_ids.iter().copied().map(TabId).collect::<Vec<_>>();
    let resolved = ctx.session.pages.resolve_tab_ids(&tab_ids).await?;
    let page_ids = group
        .tab_ids
        .iter()
        .filter_map(|tab_id| resolved.get(&TabId(*tab_id)).map(|page_id| page_id.0))
        .collect::<Vec<_>>();
    Ok(TabGroupWithPages {
        group_id: group.group_id,
        window_id: group.window_id,
        title: group.title,
        color: group.color,
        collapsed: group.collapsed,
        page_ids,
    })
}

fn format_group(group: &TabGroupWithPages) -> String {
    let collapsed = if group.collapsed { " [COLLAPSED]" } else { "" };
    let pages = if group.page_ids.is_empty() {
        "(none)".to_string()
    } else {
        group
            .page_ids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "[{}] \"{}\" ({}){collapsed} pages: {pages}",
        group.group_id,
        if group.title.is_empty() {
            "(unnamed)"
        } else {
            &group.title
        },
        group.color
    )
}
