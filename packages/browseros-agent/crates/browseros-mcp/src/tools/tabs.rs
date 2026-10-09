use crate::framework::{
    ToolCtx, ToolExecResult, ToolResult, error_result, page_json, parse_args, text_result,
};
use browseros_core::{PageId, pages::NewPageOptions};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "\
Manage browser tabs: list open pages (with their page ids), show the active page, \
open a new page in the background (snapshot attached unless snapshot=false), \
or close one. \
Use the returned page id with snapshot/act/navigate. \
action=\"list\" reports the tab group id of every grouped page. Record yours on \
your first list of a task, before you need it. Ownership is per connection: every \
remade connection starts a new session and loses it, so list before you open \
anything on a new connection and pass that id as groupId on action=\"new\". Your \
pages then keep going to that group instead of a second one being started for the \
same task, and the tabs already in it read as yours again. If you no longer have \
the id, find it in the listing: after a reconnect your tabs read as another \
agent's, and your group is titled with your own name as <yourName>/<task>. Only \
action=\"new\" reclaims; groupId is ignored on the other actions.";

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
    /// Retired: new pages always open in the background so an agent never
    /// switches the user's tab. Still accepted, and ignored, so clients that
    /// cached the old schema do not trip `deny_unknown_fields`.
    #[serde(default, rename = "background")]
    #[schemars(skip)]
    _background: Option<bool>,
    /// Page id for action="close".
    page: Option<u32>,
    /// Tab group id for action="new". Pass the group id a previous call reported
    /// to keep continuing work in the same group; omit it to use your own group.
    #[serde(default, rename = "groupId")]
    group_id: Option<String>,
    /// Set false on action="new" to open the page without its snapshot. Use it
    /// when you already know what you are going to do with the page, or will
    /// snapshot it yourself: a search results page can run to hundreds of
    /// elements and the snapshot is the largest part of the result.
    #[serde(default, rename = "snapshot")]
    snapshot: Option<bool>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<TabsArgs>(
        "tabs",
        DESCRIPTION,
        Some(super::open_world_annotations()),
        handler,
    )
}

/// Whether a tab group is currently open. A listing failure answers `true` so
/// a transient CDP problem never produces a misleading note.
async fn group_is_open(ctx: &ToolCtx, group_id: &str) -> bool {
    let Ok(result) = ctx
        .session
        .cdp("Browser.getTabGroups", json!({}), None)
        .await
    else {
        return true;
    };
    result
        .get("groups")
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .any(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id))
        })
        .unwrap_or(true)
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
                            // The group is how an agent names the work it is
                            // continuing, so it has to survive into the result.
                            "groupId": page.group_id,
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
                            // Never foreground: focus decisions belong to the user
                            // (cockpit Watch), not to the agent.
                            background: Some(true),
                            window_id: ctx.defaults.default_window_id.clone(),
                            tab_group_id: args
                                .group_id
                                .clone()
                                .or_else(|| ctx.defaults.default_tab_group_id.clone()),
                        },
                    )
                    .await?;
                // A mistyped or half-remembered group id used to be silently
                // ignored, which produced the very outcome the group argument
                // exists to avoid: the page continues in the session's own
                // group while the agent believes it rejoined an earlier one.
                // Said rather than refused, so a wrong id still opens a page.
                if let Some(requested) = args.group_id.as_deref()
                    && !group_is_open(ctx, requested).await
                {
                    // Grouping runs detached so it cannot delay this response,
                    // so the note says what was NOT done rather than asserting
                    // a placement that has not happened yet.
                    response.text(format!(
                        "note: tab group {requested} is not open, so page {} was not added to it and will go to your own group. List tab groups to find the id you meant.",
                        page.0
                    ));
                }
                response.text(format!("opened page {}", page.0));
                // Claw-server hooks key ownership/grouping off this "page" field.
                response.data(json!({ "page": page.0 }));
                if args.snapshot.unwrap_or(true) {
                    response.include_snapshot(page.0);
                }
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

#[cfg(test)]
mod tests {
    use super::{DESCRIPTION, TabsAction, TabsArgs};
    use serde_json::json;

    /// An agent that no longer holds the id has only the listing to find it in,
    /// and after a reconnect its own tabs read as another agent's. Both halves
    /// have to be stated or the reclaim is undiscoverable from this tool alone.
    #[test]
    fn the_description_says_how_to_find_a_group_id_you_no_longer_have() {
        assert!(DESCRIPTION.contains("list before you open"));
        assert!(DESCRIPTION.contains("<yourName>/<task>"));
        assert!(DESCRIPTION.contains("read as another"));
    }

    #[test]
    fn retired_background_field_is_accepted_and_ignored() -> anyhow::Result<()> {
        // Clients that cached the old schema still send it; it must not trip
        // `deny_unknown_fields`, and it must not influence the tab's focus.
        let args: TabsArgs =
            serde_json::from_value(json!({ "action": "new", "background": false }))?;
        assert!(matches!(args.action, TabsAction::New));
        assert_eq!(args._background, Some(false));
        Ok(())
    }

    #[test]
    fn a_group_id_is_accepted_on_new_and_defaults_to_none() -> anyhow::Result<()> {
        let named: TabsArgs =
            serde_json::from_value(json!({ "action": "new", "groupId": "ABC123" }))?;
        assert_eq!(named.group_id.as_deref(), Some("ABC123"));
        let plain: TabsArgs = serde_json::from_value(json!({ "action": "new" }))?;
        assert_eq!(plain.group_id, None);
        Ok(())
    }

    /// The snapshot is the largest part of a `new` result and was unavoidable.
    /// Opting out has to be possible, and the default has to stay on: it is why
    /// a caller can often answer from the open alone.
    #[test]
    fn the_snapshot_can_be_declined_and_defaults_to_on() -> anyhow::Result<()> {
        let quiet: TabsArgs =
            serde_json::from_value(json!({ "action": "new", "snapshot": false }))?;
        assert_eq!(quiet.snapshot, Some(false));
        assert!(!quiet.snapshot.unwrap_or(true));

        let default: TabsArgs = serde_json::from_value(json!({ "action": "new" }))?;
        assert_eq!(default.snapshot, None);
        assert!(
            default.snapshot.unwrap_or(true),
            "omitting it must keep the snapshot"
        );
        Ok(())
    }

    #[test]
    fn unknown_fields_are_still_rejected() {
        let result = serde_json::from_value::<TabsArgs>(json!({ "action": "new", "hidden": true }));
        assert!(result.is_err_and(|error| error.to_string().contains("hidden")));
    }
}
