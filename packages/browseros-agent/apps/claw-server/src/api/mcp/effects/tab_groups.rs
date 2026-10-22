use crate::{
    api::mcp::{
        dispatch::{CreatedPageCapture, ToolCall, ToolEffect, ToolEffectContext, result_page_id},
        naming::{desired_group_title, is_session_group_candidate},
        timeouts::TAB_GROUP_OPERATION,
    },
    ids::ConvoId,
    services::{
        browser::color_for_slug,
        sessions::{PageOwnership, ScopedGroupKey, Session, TabGroup, TitleSync},
    },
};
use browseros_core::{BrowserSession, PageId, WindowId, target::BrowserView};
use browseros_mcp::{
    BrowserToolDefaults, BrowserToolOptions, OutputFileAccess, ToolCtx, ToolDef, ToolResult,
    execute_tool,
};
use futures_util::future::BoxFuture;
use rmcp::model::ContentBlock;
use serde_json::{Value, json};
use std::sync::{Arc, LazyLock};
use tokio::{task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// Creates or joins the durable tab group for a successful `tabs new` call.
pub fn apply(context: ToolEffectContext<'_>) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        if context.result.is_error {
            return Ok(None);
        }
        if context.call.identity.is_none() || context.call.browser_session.is_none() {
            return Ok(None);
        }
        let created = if context.call.flags.new_page {
            match result_page_id(context.result) {
                Some(page_id) => created_page(context.call, page_id).await,
                None => None,
            }
        } else {
            None
        };
        // Computed before the group work is spawned, because that work creates this
        // session's own group and the listing would then report it as a candidate
        // the agent should consider reusing.
        let notice = own_group_candidates_notice(&context).await;
        // Detach browser-group synchronization so cosmetic/durable grouping cannot
        // delay the tool response. Failed creation capture skips all group writes;
        // existing-page calls can still synchronize their already captured view.
        if !context.call.flags.new_page || created.is_some() {
            drop(spawn_tab_group_work(context.call.clone(), created));
        }
        Ok(notice.map(|notice| append_notice(context.result, notice)))
    })
}

/// Names the open groups already titled for this client, on the one call that is
/// about to start another one.
///
/// The instructions and the tool descriptions both tell an agent to reuse its
/// group, and a live client skipped them anyway and opened a duplicate for a task
/// it already had a group for. Those texts are read before the work starts and
/// ask the agent to carry an id across the interruption that just made it forget;
/// this arrives in the result of the call that makes the mistake, with the ids in
/// hand, so recovering needs no memory of the earlier session at all.
///
/// Only a note. Nothing is refused and nothing is reassigned: the agent asked for
/// a new page and gets one, in a new group, exactly as before.
async fn own_group_candidates_notice(context: &ToolEffectContext<'_>) -> Option<String> {
    if !context.call.flags.new_page {
        return None;
    }
    // A session that already has a group is mid-task, not reconnecting, and one
    // that named a group is already doing the thing this note would ask for.
    if context.call.default_tab_group_id.is_some() {
        return None;
    }
    if context
        .call
        .raw_args
        .get("groupId")
        .and_then(Value::as_str)
        .is_some_and(|group| !group.trim().is_empty())
    {
        return None;
    }
    let identity = context.call.identity.as_ref()?;
    let browser = context.call.view.as_ref()?;
    let candidates = open_groups(browser, context.call.output_files.clone())
        .await?
        .into_iter()
        .filter(|(_, title)| is_session_group_candidate(&identity.session, title))
        .map(|(group_id, title)| format!("{title} (id {group_id})"))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    Some(format!(
        "note: a new tab group is being started for this session. These open groups have a matching client prefix and may be \
         earlier tasks: {}. If you are \
         continuing one of them, pass its id as groupId on tabs action=\"new\" and your pages go \
         there instead. Existing tabs keep their actual ownership; a known group also recovers its task binding.",
        candidates.join("; ")
    ))
}

/// Every open group as `(id, title)`, or `None` when the listing cannot be read.
///
/// Silent on failure on purpose: a note that names no groups, or invents the
/// absence of them, is worse than no note.
async fn open_groups(
    browser: &BrowserView,
    output_files: OutputFileAccess,
) -> Option<Vec<(String, String)>> {
    let result = dispatch_tab_groups(
        cached_tab_groups_tool(),
        browser,
        CancellationToken::new(),
        output_files,
        json!({ "action": "list" }),
    )
    .await
    .ok()?;
    Some(
        result
            .structured_content
            .as_ref()?
            .get("groups")?
            .as_array()?
            .iter()
            .filter_map(|group| {
                Some((
                    group.get("groupId").and_then(Value::as_str)?.to_string(),
                    group
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ))
            })
            .collect(),
    )
}

/// Adds the note as an extra text block, leaving the original content untouched so
/// a caller parsing the first block is unaffected.
fn append_notice(result: &ToolResult, notice: String) -> ToolResult {
    let mut annotated = result.clone();
    annotated.content.push(ContentBlock::text(notice));
    annotated
}

/// Actual creation destination recorded before the asynchronous grouping handoff.
#[derive(Debug, Clone, Copy)]
pub struct CreatedPage {
    pub page_id: u32,
    pub window_id: Option<WindowId>,
}

pub(crate) async fn created_page(call: &ToolCall, page_id: u32) -> Option<CreatedPage> {
    // The shared registry can reconcile another incarnation while ownership or
    // response work awaits. Core validates this candidate against the creating
    // view's immutable receipt; the registry alone cannot prove creation identity.
    let info = call
        .browser_session
        .as_ref()?
        .pages
        .get_info(PageId(page_id))
        .await?;
    // Derive before any detached work: capturing at callback entry would adopt a
    // reconnect's epoch for an unbound call. Core carries this expectation through
    // awaited admission and the final transport publication for every group write.
    let view = call.view.as_ref()?.with_expected_created_page(&info).ok()?;
    let created = CreatedPage {
        page_id,
        window_id: info.window_id,
    };
    call.created_page_snapshots
        .lock()
        .await
        .insert(page_id, CreatedPageCapture { page: info, view });
    Some(created)
}

fn spawn_tab_group_work(call: ToolCall, created: Option<CreatedPage>) -> JoinHandle<()> {
    tokio::spawn(run_tab_group_work(call, created))
}

/// Ensures the calling agent's tab group exists and, when `page_id` is set,
/// places that page in it. Shared by the tab-groups effect and the code-mode
/// script hook so a page a script opens joins the agent's group the same way a
/// `tabs new` page does.
pub(crate) async fn run_tab_group_work(call: ToolCall, created: Option<CreatedPage>) {
    let (Some(identity), Some(browser), Some(tab_groups)) = (
        call.identity.as_ref(),
        call.view.as_ref(),
        call.tool_named("tab_groups"),
    ) else {
        return;
    };
    let session_cancel = identity.session.child_token();
    if session_cancel.is_cancelled() {
        return;
    }
    let ownership = call.state.sessions.ownership();
    let operation_lock = ownership
        .group_operation_lock(&identity.ownership_key)
        .await;
    let _guard = operation_lock.lock().await;
    if session_cancel.is_cancelled() {
        return;
    }
    // Session teardown gates queued work above; once browser mutation starts, let it
    // finish so any created group can be recorded.
    let operation_cancel = CancellationToken::new();
    let current_group = ownership.group(&identity.ownership_key).await;
    let has_creation_snapshot = match created {
        Some(page) if page.window_id.is_some() => call
            .created_page_snapshots
            .lock()
            .await
            .contains_key(&page.page_id),
        _ => false,
    };
    // Even an unbound call must not move its newly created tab to a group that
    // has since been dragged to another window. Missing creation metadata keeps
    // the legacy unbound path, while every known destination is revalidated.
    let must_keep_actual_scope = has_creation_snapshot
        || browser.binding().is_some()
        || current_group
            .as_ref()
            .is_some_and(|group| group.scope.is_some())
        || current_group
            .as_ref()
            .and_then(|group| group.window_id)
            .zip(created.and_then(|page| page.window_id))
            .is_some_and(|(group_window, page_window)| group_window != page_window);
    if must_keep_actual_scope {
        // A late run may belong to an earlier revision/window, including an unbound
        // capture. Recover its actual destination independently of the current pointer.
        if let Some(created) = created {
            ensure_captured_tab_group(&call, &ownership, &operation_cancel, created).await;
        }
        return;
    }
    sync_pending_group_title_unlocked(
        tab_groups,
        browser,
        &ownership,
        &identity.ownership_key,
        operation_cancel.child_token(),
        call.output_files.clone(),
    )
    .await;
    expand_agent_tab_group_unlocked(
        tab_groups,
        browser,
        &ownership,
        &identity.ownership_key,
        operation_cancel.child_token(),
        call.output_files.clone(),
    )
    .await;
    let Some(CreatedPage { page_id, .. }) = created else {
        return;
    };
    // Named groups are placement/correlation, not an ownership claim. Reusing
    // a user's or another task's group must leave its existing tabs and cleanup
    // responsibilities with their actual owner.
    let named_group = call
        .raw_args
        .get("groupId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(str::to_string);
    let named_requested = named_group.is_some();
    if let Some(requested) = named_group
        && group_pages_if_open(browser, &requested, call.output_files.clone())
            .await
            .is_some()
    {
        let borrowed = !call
            .state
            .targets
            .owns_group(&call.session_id, &requested)
            .await;
        ownership
            .update_group(&identity.ownership_key, |group| {
                group.group_ref = Some(requested);
                group.borrowed = borrowed;
                group.window_id = created.and_then(|page| page.window_id);
            })
            .await;
        ensure_agent_tab_group_unlocked(
            &call,
            tab_groups,
            browser,
            &ownership,
            &operation_cancel,
            page_id,
        )
        .await;
        return;
    }
    // A named group that is not open falls through as though none had been
    // named, straight to the session's own group. Letting it reach the
    // reconciliation below cleared the session's working reference, because the
    // page had landed outside it, and a second group with the same title was
    // then minted for one task.
    if named_requested {
        ensure_agent_tab_group_unlocked(
            &call,
            tab_groups,
            browser,
            &ownership,
            &operation_cancel,
            page_id,
        )
        .await;
        return;
    }
    if let Some(default_group_id) = &call.default_tab_group_id {
        let page_group_id = browser
            .page(PageId(page_id))
            .await
            .ok()
            .and_then(|page| page.group_id);
        if page_group_id.as_ref() == Some(default_group_id) {
            return;
        }
        ownership
            .set_tab_group_ref(identity.ownership_key.clone(), None)
            .await;
    }
    ensure_agent_tab_group_unlocked(
        &call,
        tab_groups,
        browser,
        &ownership,
        &operation_cancel,
        page_id,
    )
    .await;
}

async fn ensure_captured_tab_group(
    call: &ToolCall,
    ownership: &Arc<PageOwnership>,
    cancel: &CancellationToken,
    created: CreatedPage,
) {
    let Some(identity) = &call.identity else {
        return;
    };
    let Some(captured) = call
        .created_page_snapshots
        .lock()
        .await
        .get(&created.page_id)
        .cloned()
    else {
        return;
    };
    // Use one creation-time capability throughout this flow, including title and
    // expansion helpers. An entry-only comparison cannot protect later awaits, and
    // MCP must not re-resolve a replacement through the original unrestricted view.
    let browser = &captured.view;
    let Ok(scope) = browser.scope() else {
        return;
    };
    // Re-admit the actual page before delayed mutation. Closing/replacing the
    // context or dragging the tab must fail the captured view, never reroute it.
    let Ok(mut page) = browser.page(PageId(created.page_id)).await else {
        return;
    };
    let Some(window_id) = captured.page.window_id else {
        return;
    };
    if page.window_id != Some(window_id) {
        return;
    }
    let key = ScopedGroupKey {
        browser_profile_id: scope.map(|scope| scope.binding.intent.browser_profile_id.clone()),
        window_id,
    };
    let tab_groups = cached_tab_groups_tool();
    let mut saved = ownership.scoped_group(&identity.ownership_key, &key).await;
    if page.group_id.as_ref().is_some_and(|actual| {
        saved.as_ref().and_then(|group| group.group_ref.as_ref()) != Some(actual)
    }) {
        // Explicit tabs-new placement can choose another group in this same capture.
        // Keep the former group's lifecycle record, but do not move this page out of
        // its requested destination when the detached callback finally runs.
        saved = None;
    }
    if let Some(group) = &saved
        && let Some(group_id) = &group.group_ref
    {
        let Ok(reply) = browser.cdp("Browser.getTabGroups", json!({}), None).await else {
            return;
        };
        let actual_window = reply
            .get("groups")
            .and_then(Value::as_array)
            .and_then(|groups| {
                groups
                    .iter()
                    .find(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id))
            })
            .and_then(|group| group.get("windowId"))
            .and_then(Value::as_i64);
        if actual_window != Some(window_id.0) {
            // A saved group can be dragged since creation. Never add a tab from a
            // different actual window merely because the old scope key matched.
            // A scoped absence does not erase its global lifecycle ownership.
            ownership
                .detach_captured_group(&identity.ownership_key, &key, group_id)
                .await;
            saved = None;
        }
    }
    // Group membership is mutable presentation/placement, not creation identity.
    // A native actor can place this page while group discovery awaits, outside
    // our conversation mutex. Refresh through the SAME deep capability before
    // choosing a destination; never erase a newer explicit group or ungrouping.
    let Ok(current) = browser.page(PageId(created.page_id)).await else {
        return;
    };
    if current.window_id != Some(window_id) {
        return;
    }
    if current.group_id != page.group_id {
        if current.group_id.is_none() {
            return;
        }
        saved = None;
    }
    page = current;
    if let Some(group) = saved
        && let Some(group_id) = group.group_ref
    {
        sync_pending_group_title_unlocked(
            tab_groups,
            browser,
            ownership,
            &identity.ownership_key,
            cancel.child_token(),
            call.output_files.clone(),
        )
        .await;
        if group.collapsed && !group.borrowed {
            expand_group_unlocked(
                tab_groups,
                browser,
                ownership,
                &identity.ownership_key,
                &group_id,
                cancel.child_token(),
                call.output_files.clone(),
            )
            .await;
        }
        if let Err(error) = dispatch_tab_groups(
            tab_groups,
            browser,
            cancel.child_token(),
            call.output_files.clone(),
            json!({"action":"create", "groupId":group_id, "pages":[created.page_id]}),
        )
        .await
        {
            if group_exists_unlocked(browser, &group_id, call.output_files.clone()).await
                == Some(false)
            {
                ownership
                    .detach_captured_group(&identity.ownership_key, &key, &group_id)
                    .await;
            }
            warn!(%error, "captured tab group add failed");
        }
        return;
    }
    // A page may already be in an explicitly requested group. Reuse its actual
    // destination without claiming other tabs or granting lifecycle ownership.
    let title = desired_group_title(&identity.session).await;
    let color = color_for_slug(identity.agent.slug());
    let (group_id, borrowed) = if let Some(group_id) = page.group_id {
        let borrowed = !call
            .state
            .targets
            .owns_group(&call.session_id, &group_id)
            .await;
        (group_id, borrowed)
    } else {
        let result = dispatch_tab_groups(
            tab_groups,
            browser,
            cancel.child_token(),
            call.output_files.clone(),
            json!({"action":"create", "pages":[created.page_id], "title":title}),
        )
        .await;
        let group_id = match result {
            Ok(result) => result_group_id(&result),
            Err(error) => {
                warn!(%error, "captured tab group create failed");
                return;
            }
        };
        let Some(group_id) = group_id else { return };
        (group_id, false)
    };
    ownership
        .remember_scoped_group(
            &identity.ownership_key,
            key,
            TabGroup {
                group_ref: Some(group_id.clone()),
                window_id: Some(window_id),
                color: Some(color),
                title: TitleSync::Synced(title.clone()),
                borrowed,
                ..TabGroup::default()
            },
        )
        .await;
    if !borrowed {
        if let Err(error) = call
            .state
            .targets
            .remember_group(&call.session_id, &group_id)
            .await
        {
            warn!(%error, "tab group task correlation persistence failed");
        }
        if let Err(error) = dispatch_tab_groups(
            tab_groups,
            browser,
            cancel.child_token(),
            call.output_files.clone(),
            json!({"action":"update", "groupId":group_id, "color":color, "collapsed":false}),
        )
        .await
        {
            warn!(%error, "captured tab group color update failed");
        }
        // Naming can race the native create await. Record the created group's
        // actual title first, then apply a newer task title to all owned groups.
        let desired_title = desired_group_title(&identity.session).await;
        if desired_title != title {
            ownership
                .set_desired_group_title(identity.ownership_key.clone(), desired_title)
                .await;
            sync_pending_group_title_unlocked(
                tab_groups,
                browser,
                ownership,
                &identity.ownership_key,
                cancel.child_token(),
                call.output_files.clone(),
            )
            .await;
        }
    }
}

/// The pages in a named tab group, or `None` only when the listing says that
/// group is not there.
///
/// A listing that fails or cannot be parsed answers with an empty page list
/// rather than `None`, so the named group is still adopted and nothing is
/// claimed. Collapsing those two cases would let a timeout look exactly like a
/// closed group and silently route the page somewhere else, splitting one task
/// across two groups because of a transient error.
async fn group_pages_if_open(
    browser: &BrowserView,
    group_id: &str,
    output_files: OutputFileAccess,
) -> Option<Vec<u32>> {
    let Ok(result) = dispatch_tab_groups(
        cached_tab_groups_tool(),
        browser,
        CancellationToken::new(),
        output_files,
        json!({ "action": "list" }),
    )
    .await
    else {
        return Some(Vec::new());
    };
    let Some(groups) = result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("groups"))
        .and_then(Value::as_array)
    else {
        return Some(Vec::new());
    };
    let group = groups
        .iter()
        .find(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id))?
        .clone();
    Some(
        group
            .get("pageIds")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_u64)
            .filter_map(|page| u32::try_from(page).ok())
            .collect(),
    )
}

async fn ensure_agent_tab_group_unlocked(
    call: &ToolCall,
    tab_groups: &ToolDef,
    browser: &BrowserView,
    ownership: &Arc<PageOwnership>,
    operation_cancel: &CancellationToken,
    page_id: u32,
) {
    let Some(identity) = call.identity.as_ref() else {
        return;
    };
    if let Some(group_id) = ownership.tab_group_ref(&identity.ownership_key).await {
        let output_files = call.output_files.clone();
        if let Err(reason) = dispatch_tab_groups(
            tab_groups,
            browser,
            operation_cancel.child_token(),
            output_files.clone(),
            json!({ "action": "create", "groupId": group_id, "pages": [page_id] }),
        )
        .await
        {
            if group_exists_unlocked(browser, &group_id, output_files).await == Some(false) {
                ownership
                    .forget_group_by_ref(&identity.ownership_key, &group_id)
                    .await;
            }
            warn!(
                dispatch_id = %call.dispatch_id,
                error = %reason,
                "tab group add failed"
            );
        }
        return;
    }

    let color = ownership
        .tab_group_color(&identity.ownership_key)
        .await
        .unwrap_or_else(|| color_for_slug(identity.agent.slug()));
    let creation_title = desired_group_title(&identity.session).await;
    ownership
        .set_desired_group_title(identity.ownership_key.clone(), creation_title.clone())
        .await;
    let group_result = match dispatch_tab_groups(
        tab_groups,
        browser,
        operation_cancel.child_token(),
        call.output_files.clone(),
        json!({ "action": "create", "pages": [page_id], "title": creation_title }),
    )
    .await
    {
        Ok(result) => result,
        Err(reason) => {
            warn!(
                dispatch_id = %call.dispatch_id,
                error = %reason,
                "tab group create failed"
            );
            return;
        }
    };
    let Some(group_id) = result_group_id(&group_result) else {
        warn!(
            dispatch_id = %call.dispatch_id,
            "tab group create returned no group id"
        );
        return;
    };
    ownership
        .set_tab_group_with_title(
            identity.ownership_key.clone(),
            group_id.clone(),
            color,
            creation_title.clone(),
        )
        .await;
    let window_id = match call.browser_session.as_ref() {
        Some(session) => session
            .pages
            .get_info(PageId(page_id))
            .await
            .and_then(|page| page.window_id),
        None => None,
    };
    ownership
        .update_group(&identity.ownership_key, |group| group.window_id = window_id)
        .await;
    if let Err(error) = call
        .state
        .targets
        .remember_group(&call.session_id, &group_id)
        .await
    {
        warn!(%error, "tab group task correlation persistence failed");
    }
    if let Err(reason) = dispatch_tab_groups(
        tab_groups,
        browser,
        operation_cancel.child_token(),
        call.output_files.clone(),
        json!({ "action": "update", "groupId": group_id, "color": color }),
    )
    .await
    {
        warn!(
            dispatch_id = %call.dispatch_id,
            group_color = %color,
            error = %reason,
            "tab group color lock failed"
        );
    }
    let desired_title = desired_group_title(&identity.session).await;
    if desired_title != creation_title {
        ownership
            .set_desired_group_title(identity.ownership_key.clone(), desired_title)
            .await;
        sync_pending_group_title_unlocked(
            tab_groups,
            browser,
            ownership,
            &identity.ownership_key,
            operation_cancel.child_token(),
            call.output_files.clone(),
        )
        .await;
    }
}

/// Collapses every owned group when the task enters retention. Borrowed groups
/// remain under their original owner's lifecycle even if they were used by this task.
pub async fn collapse_agent_tab_group(
    browser: Option<&Arc<BrowserSession>>,
    ownership: &Arc<PageOwnership>,
    key: &ConvoId,
) -> bool {
    retained_groups(browser, ownership, key, false).await
}

/// Closes the task's own retained groups across every captured profile/window.
pub async fn close_agent_tab_group(
    browser: Option<&Arc<BrowserSession>>,
    ownership: &Arc<PageOwnership>,
    key: &ConvoId,
) -> bool {
    retained_groups(browser, ownership, key, true).await
}

async fn retained_groups(
    browser: Option<&Arc<BrowserSession>>,
    ownership: &Arc<PageOwnership>,
    key: &ConvoId,
    close: bool,
) -> bool {
    let operation_lock = ownership.group_operation_lock(key).await;
    let _guard = operation_lock.lock().await;
    let groups = ownership.groups(key).await;
    let mut complete = true;
    for group in groups {
        let Some(group_id) = group.group_ref else {
            continue;
        };
        if group.borrowed {
            if close {
                ownership.forget_group_by_ref(key, &group_id).await;
            }
            continue;
        }
        if !close && group.collapsed {
            continue;
        }
        let Some(browser) = browser else {
            complete = false;
            continue;
        };
        // Host cleanup follows recorded ownership over multiple former bindings.
        // It does not execute a new browser call on behalf of the current target.
        let view = BrowserView::unbound(browser.clone(), None);
        let output_files = browseros_mcp::output_file::create_browser_output_file_access();
        let args = if close {
            json!({"action":"close", "groupId":group_id})
        } else {
            json!({"action":"update", "groupId":group_id, "collapsed":true})
        };
        let result = dispatch_tab_groups(
            cached_tab_groups_tool(),
            &view,
            CancellationToken::new(),
            output_files.clone(),
            args,
        )
        .await;
        let absent = result.is_err()
            && group_exists_unlocked(&view, &group_id, output_files).await == Some(false);
        if absent || result.is_ok() && close {
            ownership.forget_group_by_ref(key, &group_id).await;
        } else if result.is_ok() {
            ownership
                .update_group_by_ref(key, &group_id, |group| group.collapsed = true)
                .await;
        } else {
            complete = false;
            warn!(%key, error = ?result.err(), "retained tab group cleanup failed");
        }
    }
    complete
        && ownership
            .groups(key)
            .await
            .iter()
            .all(|group| !close && (group.collapsed || group.borrowed))
}

async fn group_exists_unlocked(
    browser: &BrowserView,
    group_id: &str,
    output_files: OutputFileAccess,
) -> Option<bool> {
    let result = dispatch_tab_groups(
        cached_tab_groups_tool(),
        browser,
        CancellationToken::new(),
        output_files,
        json!({ "action": "list" }),
    )
    .await
    .ok()?;
    let groups = result
        .structured_content
        .as_ref()?
        .get("groups")?
        .as_array()?;
    Some(
        groups
            .iter()
            .any(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id)),
    )
}

async fn expand_agent_tab_group_unlocked(
    tab_groups: &ToolDef,
    browser: &BrowserView,
    ownership: &Arc<PageOwnership>,
    key: &ConvoId,
    cancel: CancellationToken,
    output_files: OutputFileAccess,
) {
    if !ownership.tab_group_collapsed(key).await {
        return;
    }
    let Some(group_id) = ownership.tab_group_ref(key).await else {
        return;
    };
    expand_group_unlocked(
        tab_groups,
        browser,
        ownership,
        key,
        &group_id,
        cancel,
        output_files,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn expand_group_unlocked(
    tab_groups: &ToolDef,
    browser: &BrowserView,
    ownership: &Arc<PageOwnership>,
    key: &ConvoId,
    group_id: &str,
    cancel: CancellationToken,
    output_files: OutputFileAccess,
) {
    match dispatch_tab_groups(
        tab_groups,
        browser,
        cancel,
        output_files,
        json!({ "action": "update", "groupId": group_id, "collapsed": false }),
    )
    .await
    {
        Ok(_) => {
            ownership
                .update_group_by_ref(key, group_id, |group| group.collapsed = false)
                .await;
        }
        Err(reason) => warn!(key = %key, error = %reason, "agent tab group expand failed"),
    }
}

/// Stores the desired title and best-effort applies it to every owned native group.
pub async fn apply_agent_tab_group_title(
    browser: Option<&Arc<BrowserSession>>,
    ownership: &Arc<PageOwnership>,
    key: &ConvoId,
    session: &Session,
    cancel: CancellationToken,
) {
    let operation_lock = ownership.group_operation_lock(key).await;
    let _guard = operation_lock.lock().await;
    let title = desired_group_title(session).await;
    ownership.set_desired_group_title(key.clone(), title).await;
    let Some(browser) = browser else {
        return;
    };
    let view = BrowserView::unbound(browser.clone(), None);
    let browser = &view;
    sync_pending_group_title_unlocked(
        cached_tab_groups_tool(),
        browser,
        ownership,
        key,
        cancel,
        browseros_mcp::output_file::create_browser_output_file_access(),
    )
    .await;
}

async fn sync_pending_group_title_unlocked(
    tab_groups: &ToolDef,
    browser: &BrowserView,
    ownership: &Arc<PageOwnership>,
    key: &ConvoId,
    cancel: CancellationToken,
    output_files: OutputFileAccess,
) {
    for group in ownership.groups(key).await {
        if group.borrowed {
            continue;
        }
        let (Some(group_id), TitleSync::Pending(title)) = (group.group_ref, group.title) else {
            continue;
        };
        match dispatch_tab_groups(
            tab_groups,
            browser,
            cancel.child_token(),
            output_files.clone(),
            json!({"action":"update", "groupId":group_id, "title":title}),
        )
        .await
        {
            Ok(_) => {
                ownership
                    .update_group_by_ref(key, &group_id, |group| {
                        if matches!(&group.title, TitleSync::Pending(pending) if pending == &title)
                        {
                            group.title = TitleSync::Synced(title.clone());
                        }
                    })
                    .await
            }
            Err(reason) => warn!(%key, error = %reason, "session name tab group retitle failed"),
        }
    }
}

async fn dispatch_tab_groups(
    tab_groups: &ToolDef,
    browser: &BrowserView,
    cancel: CancellationToken,
    output_files: OutputFileAccess,
    args: Value,
) -> Result<ToolResult, String> {
    let operation_cancel = cancel.child_token();
    let ctx = ToolCtx::new(BrowserToolOptions {
        browser: browser.clone(),
        target: None,
        defaults: BrowserToolDefaults::default(),
        cancel: operation_cancel.clone(),
        output_files,
        inner_call_hook: None,
        preloaded_helpers: Vec::new(),
    });
    let execution = timeout(TAB_GROUP_OPERATION, execute_tool(tab_groups, args, &ctx)).await;
    let result = match execution {
        Ok(result) => result,
        Err(_) => {
            operation_cancel.cancel();
            return Err(format!(
                "tab_groups operation timed out after {}ms",
                TAB_GROUP_OPERATION.as_millis()
            ));
        }
    };
    match result {
        Ok(result) if !result.is_error => Ok(result),
        Ok(result) => Err(first_text(&result)),
        Err(error) => Err(error.to_string()),
    }
}

fn cached_tab_groups_tool() -> &'static ToolDef {
    static TAB_GROUPS_TOOL: LazyLock<ToolDef> = LazyLock::new(|| {
        browseros_mcp::catalog()
            .into_iter()
            .find(|tool| tool.name == "tab_groups")
            .unwrap_or_else(|| panic!("tab_groups tool missing from catalog"))
    });
    &TAB_GROUPS_TOOL
}

fn result_group_id(result: &ToolResult) -> Option<String> {
    result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("group"))
        .and_then(|value| value.get("groupId"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn first_text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

const _: ToolEffect = apply;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{AuditLog, DATABASE_FILENAME, Database, SessionTabLedger},
        identity::{ClientIdentity, ConversationIdentity},
        ids::SessionId as AppSessionId,
        services::sessions::{RetainedGroupAction, Sessions},
    };
    use browseros_cdp::{CdpError, CdpEvent};
    use browseros_core::{CdpConnection, SessionId};
    use std::{
        collections::{BTreeSet, HashMap},
        sync::{
            Arc, Mutex as StdMutex,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };
    use tokio::sync::{Notify, broadcast};

    #[tokio::test]
    async fn dragged_group_is_detached_from_placement_but_retained_for_cleanup()
    -> anyhow::Result<()> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        let transport = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let mut call = tool_call("tabs", json!({"action":"new"})).await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(
            BrowserView::capture(
                browser.clone(),
                Some(ProfileConnection::binding("eng", 1, 1)),
                None,
            )
            .await,
        );
        assert!(
            call.view
                .as_ref()
                .is_some_and(|view| view.admission_error().is_none())
        );
        let first = created_page(&call, 1).await;
        run_tab_group_work(call.clone(), first).await;
        let key = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();
        let original = ownership.tab_group_ref(&key).await;
        // Model a user dragging the group after creation. A later created tab must
        // stay in its actual window, while cleanup must still remember the old group.
        transport.groups.lock().unwrap_or_else(|p| p.into_inner())[0]["windowId"] = json!(3);
        transport.tabs.lock().unwrap_or_else(|p| p.into_inner())[0]["windowId"] = json!(3);
        let root = BrowserView::unbound(browser.clone(), None);
        root.cdp(
            "Browser.createTab",
            json!({"url":"https://synthetic.invalid/late", "windowId":1}),
            None,
        )
        .await?;
        browser.pages.list().await?;
        let late = created_page(&call, 4).await;
        run_tab_group_work(call, late).await;
        let replacement = ownership.tab_group_ref(&key).await;
        assert!(replacement.is_some());
        assert_ne!(original, replacement);
        assert_eq!(ownership.groups(&key).await.len(), 2);
        let late = transport
            .tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|tab| tab["tabId"] == 104)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("late tab missing"))?;
        assert_eq!(late["windowId"], 1);
        assert_eq!(late["groupId"].as_str(), replacement.as_deref());
        assert!(close_agent_tab_group(Some(&browser), &ownership, &key).await);
        assert!(
            transport
                .groups
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn explicit_borrowed_group_in_same_capture_preserves_owned_group_cleanup()
    -> anyhow::Result<()> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        let transport = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let mut call = tool_call("tabs", json!({"action":"new"})).await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(
            BrowserView::capture(
                browser.clone(),
                Some(ProfileConnection::binding("eng", 1, 1)),
                None,
            )
            .await,
        );
        assert!(
            call.view
                .as_ref()
                .is_some_and(|view| view.admission_error().is_none())
        );
        let created = created_page(&call, 1).await;
        run_tab_group_work(call.clone(), created).await;
        let key = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();
        let owned = ownership.tab_group_ref(&key).await;
        let root = BrowserView::unbound(browser.clone(), None);
        root.cdp(
            "Browser.createTab",
            json!({"url":"https://synthetic.invalid/borrowed", "windowId":1}),
            None,
        )
        .await?;
        let borrowed = root
            .cdp(
                "Browser.createTabGroup",
                json!({"tabIds":[104], "title":"User's group"}),
                None,
            )
            .await?;
        let borrowed_id = borrowed["group"]["groupId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("borrowed group missing"))?
            .to_string();
        browser.pages.list().await?;
        let other_owner = ConvoId::new("other-task");
        ownership.claim_page(other_owner.clone(), PageId(4)).await;
        let created = created_page(&call, 4).await;
        run_tab_group_work(call, created).await;
        assert_eq!(
            ownership.tab_group_ref(&key).await,
            Some(borrowed_id.clone())
        );
        assert_eq!(ownership.groups(&key).await.len(), 2);
        assert_eq!(ownership.owner_of_page(&PageId(4)).await, Some(other_owner));
        let tab = transport
            .tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|tab| tab["tabId"] == 104)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("borrowed tab missing"))?;
        assert_eq!(tab["groupId"], borrowed_id);
        assert!(close_agent_tab_group(Some(&browser), &ownership, &key).await);
        let remaining = root.cdp("Browser.getTabGroups", json!({}), None).await?;
        let remaining = remaining["groups"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("groups missing"))?;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0]["groupId"], borrowed_id);
        assert!(
            remaining
                .iter()
                .all(|group| group["groupId"].as_str() != owned.as_deref())
        );
        Ok(())
    }

    #[tokio::test]
    async fn delayed_bound_grouping_refuses_replacement_during_group_lookup() -> anyhow::Result<()>
    {
        assert_change_during_group_lookup(true, false).await
    }

    #[tokio::test]
    async fn delayed_unbound_grouping_refuses_replacement_during_group_lookup() -> anyhow::Result<()>
    {
        assert_change_during_group_lookup(false, false).await
    }

    #[tokio::test]
    async fn delayed_bound_grouping_refuses_reconnect_during_group_lookup() -> anyhow::Result<()> {
        assert_change_during_group_lookup(true, true).await
    }

    #[tokio::test]
    async fn delayed_unbound_grouping_refuses_reconnect_during_group_lookup() -> anyhow::Result<()>
    {
        assert_change_during_group_lookup(false, true).await
    }

    #[tokio::test]
    async fn delayed_unbound_grouping_keeps_creation_epoch_before_callback() -> anyhow::Result<()> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        let transport = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let mut call = tool_call("tabs", json!({"action":"new"})).await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(BrowserView::unbound(browser, None));
        let created = created_page(&call, 1).await;
        assert!(created.is_some());
        let before = transport
            .calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len();

        // Reused native IDs and metadata can look identical after reconnect. A
        // callback-entry derivation would capture epoch 2 and mutate that new peer.
        transport.reconnect();
        run_tab_group_work(call, created).await;
        let calls = transport.calls.lock().unwrap_or_else(|p| p.into_inner());
        assert!(
            calls[before..]
                .iter()
                .all(|(method, _)| !method.contains("TabGroup"))
        );
        assert!(
            transport
                .groups
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_empty()
        );
        Ok(())
    }

    async fn assert_change_during_group_lookup(bound: bool, reconnect: bool) -> anyhow::Result<()> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        use browseros_core::pages::NewPageOptions;
        use std::sync::atomic::Ordering;

        let transport = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let root = BrowserView::unbound(browser.clone(), None);
        let native_group = root
            .cdp(
                "Browser.createTabGroup",
                json!({"tabIds":[101], "title":"original title"}),
                None,
            )
            .await?;
        let group_id = native_group["group"]["groupId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("group missing"))?
            .to_string();
        transport.groups.lock().unwrap_or_else(|p| p.into_inner())[0]["collapsed"] = json!(true);
        let page = root
            .new_page(
                "https://synthetic.test/created",
                NewPageOptions {
                    window_id: Some(WindowId(1)),
                    ..NewPageOptions::default()
                },
            )
            .await?;
        let mut call = tool_call("tabs", json!({"action":"new"})).await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(if bound {
            BrowserView::capture(browser, Some(ProfileConnection::binding("eng", 1, 1)), None).await
        } else {
            root
        });
        let created = created_page(&call, page.page_id.0).await;
        assert!(created.is_some());
        let key = ScopedGroupKey {
            browser_profile_id: bound
                .then(|| browseros_core::BrowserProfileId::from("eng".to_string())),
            window_id: WindowId(1),
        };
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        call.state
            .sessions
            .ownership()
            .remember_scoped_group(
                &identity.ownership_key,
                key,
                TabGroup {
                    group_ref: Some(group_id.clone()),
                    window_id: Some(WindowId(1)),
                    collapsed: true,
                    title: TitleSync::Pending("updated title".into()),
                    ..TabGroup::default()
                },
            )
            .await;
        let before = transport
            .calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len();
        transport.pause_group_lookup.store(true, Ordering::SeqCst);
        let entered = transport.group_lookup_entered.notified();
        let work = tokio::spawn(run_tab_group_work(call, created));
        timeout(std::time::Duration::from_secs(2), entered).await?;

        // The original incarnation passed callback entry. Replacement now happens
        // while native group discovery is awaited, before title/expand/add work.
        // Keep native tab and window IDs stable to expose PageId-only re-resolution.
        if reconnect {
            transport.reconnect();
        } else {
            let mut tabs = transport.tabs.lock().unwrap_or_else(|p| p.into_inner());
            let tab = tabs
                .iter_mut()
                .find(|tab| tab["tabId"] == page.tab_id.0)
                .ok_or_else(|| anyhow::anyhow!("created tab missing"))?;
            assert_eq!(tab["targetId"], page.target_id.as_str());
            tab["targetId"] = json!("replacement-created-target");
        }
        transport.group_lookup_release.notify_one();
        timeout(std::time::Duration::from_secs(2), work).await??;
        let calls = transport.calls.lock().unwrap_or_else(|p| p.into_inner());
        let mutations = calls[before..]
            .iter()
            .filter(|(method, _)| {
                matches!(
                    method.as_str(),
                    "Browser.createTabGroup"
                        | "Browser.addTabsToGroup"
                        | "Browser.updateTabGroup"
                        | "Browser.removeTabsFromGroup"
                        | "Browser.closeTabGroup"
                        | "Browser.moveTabGroup"
                )
            })
            .collect::<Vec<_>>();
        assert!(
            mutations.is_empty(),
            "replacement received group mutations: {mutations:?}"
        );
        let groups = transport.groups.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["groupId"], group_id);
        assert_eq!(groups[0]["title"], "original title");
        assert_eq!(groups[0]["collapsed"], true);
        assert_eq!(groups[0]["tabIds"], json!([101]));
        Ok(())
    }

    #[tokio::test]
    async fn bound_tabs_creation_receipt_survives_response_handoff() -> anyhow::Result<()> {
        assert_creation_survives_response_handoff(true).await
    }

    #[tokio::test]
    async fn unbound_tabs_creation_receipt_survives_response_handoff() -> anyhow::Result<()> {
        assert_creation_survives_response_handoff(false).await
    }

    async fn assert_creation_survives_response_handoff(bound: bool) -> anyhow::Result<()> {
        use crate::api::mcp::{
            dispatch::dispatch_tool_call,
            test_support::{ProfileConnection, tool_call},
        };
        let native = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(native.clone());
        browser.pages.list().await?;
        let mut call = tool_call(
            "tabs",
            json!({"action":"new", "url":"https://synthetic.test/response-handoff"}),
        )
        .await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(if bound {
            BrowserView::capture(
                browser.clone(),
                Some(ProfileConnection::binding("eng", 1, 1)),
                None,
            )
            .await
        } else {
            BrowserView::unbound(browser.clone(), Some(WindowId(1)))
        });
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let ownership = call.state.sessions.ownership();
        let operation = ownership
            .group_operation_lock(&identity.ownership_key)
            .await;
        let queued = operation.lock().await;
        native
            .pause_response_evaluation
            .store(true, Ordering::SeqCst);
        let entered = native.response_evaluation_entered.notified();
        let work = tokio::spawn(dispatch_tool_call(call.clone()));
        timeout(Duration::from_secs(3), entered).await?;
        let original = browser
            .pages
            .get_info(PageId(4))
            .await
            .ok_or_else(|| anyhow::anyhow!("new page missing at response pause"))?;
        assert!(call.created_page_snapshots.lock().await.is_empty());
        // Creation returned through a clone of the admitted view, but response
        // settling is still awaited. Reconcile the same native tab into a new
        // target before effects read the shared registry candidate.
        {
            let mut tabs = native.tabs.lock().unwrap_or_else(|p| p.into_inner());
            let tab = tabs
                .iter_mut()
                .find(|tab| tab["tabId"] == original.tab_id.0)
                .ok_or_else(|| anyhow::anyhow!("created tab missing"))?;
            tab["targetId"] = json!("replacement-before-response-capture");
        }
        browser.pages.list().await?;
        let replaced = browser
            .pages
            .get_info(original.page_id.clone())
            .await
            .ok_or_else(|| anyhow::anyhow!("reconciled page missing"))?;
        assert_eq!(replaced.tab_id, original.tab_id);
        assert_eq!(replaced.window_id, original.window_id);
        assert_ne!(replaced.target_id, original.target_id);
        native.response_evaluation_release.notify_one();
        let result = timeout(Duration::from_secs(3), work).await???;
        assert_eq!(result.is_error, Some(false));
        assert!(result.content.iter().any(|block| matches!(block,
            ContentBlock::Text(text) if text.text.contains("opened page 4"))));
        tokio::task::yield_now().await;
        drop(queued);
        let _finished = operation.lock().await;
        {
            let calls = native.calls.lock().unwrap_or_else(|p| p.into_inner());
            let mutations = calls
                .iter()
                .filter(|(method, _)| {
                    method.contains("TabGroup") && method != "Browser.getTabGroups"
                })
                .collect::<Vec<_>>();
            assert!(
                mutations.is_empty(),
                "replacement received tabs group writes: {mutations:?}"
            );
        }
        assert!(
            call.created_page_snapshots.lock().await.is_empty(),
            "replacement was adopted after response settling"
        );
        Ok(())
    }

    #[tokio::test]
    async fn delayed_grouping_refuses_replacement_of_created_page_incarnation() -> anyhow::Result<()>
    {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        let transport = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let mut call = tool_call("tabs", json!({"action":"new"})).await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(
            BrowserView::capture(browser, Some(ProfileConnection::binding("eng", 1, 1)), None)
                .await,
        );
        assert!(
            call.view
                .as_ref()
                .is_some_and(|view| view.admission_error().is_none())
        );
        let created = created_page(&call, 1).await;
        transport.tabs.lock().unwrap_or_else(|p| p.into_inner())[0]["targetId"] =
            json!("replacement-T101");
        run_tab_group_work(call, created).await;
        assert!(
            transport
                .calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .all(|(method, _)| method != "Browser.createTabGroup"
                    && method != "Browser.addTabsToGroup")
        );
        Ok(())
    }

    #[tokio::test]
    async fn delayed_unbound_grouping_recovers_its_original_window_after_binding()
    -> anyhow::Result<()> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        let transport = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let mut old = tool_call("tabs", json!({"action":"new"})).await?;
        old.browser_session = Some(browser.clone());
        old.view = Some(BrowserView::unbound(browser.clone(), None));
        let first_page = created_page(&old, 1).await;
        run_tab_group_work(old.clone(), first_page).await;
        let key = old
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        let ownership = old.state.sessions.ownership();
        let old_group = ownership.tab_group_ref(&key).await;
        assert!(old_group.is_some());
        // This tab was admitted before target selection, but its grouping callback
        // runs only after the newly bound call has replaced the presentation group.
        old.view
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("view missing"))?
            .cdp(
                "Browser.createTab",
                json!({"url":"https://synthetic.invalid/late", "windowId":1}),
                None,
            )
            .await?;
        browser.pages.list().await?;
        let late_page = created_page(&old, 4).await;
        let binding = old
            .state
            .targets
            .set(
                &old.session_id,
                ProfileConnection::binding("ops", 1, 2).intent,
            )
            .await?;
        let mut bound = old.clone();
        bound.view = Some(BrowserView::capture(browser.clone(), Some(binding.clone()), None).await);
        assert!(
            bound
                .view
                .as_ref()
                .is_some_and(|view| view.admission_error().is_none())
        );
        let bound_page = created_page(&bound, 2).await;
        run_tab_group_work(bound, bound_page).await;
        let bound_group = ownership
            .group_for_capture(
                &key,
                &binding.intent.browser_profile_id,
                Some(WindowId(2)),
                binding.revision,
            )
            .await
            .and_then(|group| group.group_ref);
        assert!(bound_group.is_some());
        assert_ne!(bound_group, old_group);
        run_tab_group_work(old, late_page).await;
        assert_eq!(ownership.groups(&key).await.len(), 2);
        let tabs = transport.tabs.lock().unwrap_or_else(|p| p.into_inner());
        let late = tabs
            .iter()
            .find(|tab| tab["tabId"] == 104)
            .ok_or_else(|| anyhow::anyhow!("late tab missing"))?;
        assert_eq!(late["windowId"], 1);
        assert_eq!(late["groupId"].as_str(), old_group.as_deref());
        Ok(())
    }

    #[tokio::test]
    async fn scope_rebind_reuses_groups_and_late_creation_keeps_original_scope()
    -> anyhow::Result<()> {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        use browseros_core::pages::NewPageOptions;
        let transport = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let mut base = tool_call("tabs", json!({"action":"new"})).await?;
        base.browser_session = Some(browser.clone());
        let key = base
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        let ownership = base.state.sessions.ownership();
        let mut groups = HashMap::new();
        let mut pages = Vec::new();
        let mut late = None;
        // Use real persisted binding revisions and native creation receipts. The
        // same destination must remain reusable after visiting other destinations.
        for (index, (profile, window)) in
            [("eng", 1), ("ops", 2), ("eng", 3), ("eng", 1), ("ops", 2)]
                .into_iter()
                .enumerate()
        {
            let binding = base
                .state
                .targets
                .set(
                    &base.session_id,
                    ProfileConnection::binding(profile, 0, window).intent,
                )
                .await?;
            let mut call = base.clone();
            call.view =
                Some(BrowserView::capture(browser.clone(), Some(binding.clone()), None).await);
            let view = call
                .view
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("view missing"))?;
            let page = view
                .new_page(
                    &format!("https://synthetic.invalid/scope-rebind-{index}"),
                    NewPageOptions::default(),
                )
                .await?;
            let created = created_page(&call, page.page_id.0).await;
            assert!(created.is_some());
            run_tab_group_work(call.clone(), created).await;
            let current = view.page(page.page_id.clone()).await?;
            assert_eq!(current.target_id, page.target_id);
            assert_eq!(current.tab_id, page.tab_id);
            assert_eq!(current.window_id, Some(WindowId(window)));
            let group = current
                .group_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("group missing"))?;
            let destination = (profile, window);
            if let Some(previous) = groups.get(&destination) {
                assert_eq!(
                    &group, previous,
                    "returning to {profile}/window{window} forgot its retained group"
                );
            } else {
                assert!(!groups.values().any(|previous| previous == &group));
                groups.insert(destination, group.clone());
            }
            assert_eq!(
                ownership
                    .group_for_capture(
                        &key,
                        &binding.intent.browser_profile_id,
                        Some(WindowId(window)),
                        binding.revision,
                    )
                    .await
                    .and_then(|group| group.group_ref)
                    .as_deref(),
                Some(group.as_str())
            );
            pages.push((page.page_id, page.tab_id, window, group));
            if index == 0 {
                // The creation receipt is retained now, but its detached callback
                // runs only after the task's current binding has moved to ops/w2.
                let page = view
                    .new_page(
                        "https://synthetic.invalid/late-old-revision",
                        NewPageOptions::default(),
                    )
                    .await?;
                let created = created_page(&call, page.page_id.0)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("late capture missing"))?;
                late = Some((call, created, page, binding));
            }
        }
        assert_eq!(groups.len(), 3);
        assert_eq!(ownership.groups(&key).await.len(), 3);
        let (old, created, page, binding) =
            late.ok_or_else(|| anyhow::anyhow!("old callback missing"))?;
        let current_binding = base
            .state
            .targets
            .get(&base.session_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("current binding missing"))?;
        assert_eq!(current_binding.intent.browser_profile_id.as_str(), "ops");
        assert!(current_binding.revision > binding.revision);
        assert_eq!(
            old.view.as_ref().and_then(BrowserView::binding),
            Some(&binding)
        );
        run_tab_group_work(old.clone(), Some(created)).await;
        let current = old
            .view
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("old view missing"))?
            .page(page.page_id.clone())
            .await?;
        let original_group = groups
            .get(&("eng", 1))
            .ok_or_else(|| anyhow::anyhow!("original group missing"))?;
        assert_eq!(current.target_id, page.target_id);
        assert_eq!(current.tab_id, page.tab_id);
        assert_eq!(current.window_id, Some(WindowId(1)));
        assert_eq!(current.group_id.as_deref(), Some(original_group.as_str()));
        assert_eq!(ownership.groups(&key).await.len(), 3);
        let root = BrowserView::unbound(browser, None);
        // The complete inventory still holds every page in its original window
        // and group after projected calls in other scopes and the late callback.
        for (page, tab, window, group) in pages {
            let current = root.page(page).await?;
            assert_eq!(current.tab_id, tab);
            assert_eq!(current.window_id, Some(WindowId(window)));
            assert_eq!(current.group_id.as_deref(), Some(group.as_str()));
        }
        assert_eq!(
            transport
                .groups
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .len(),
            3
        );
        Ok(())
    }

    #[tokio::test]
    async fn delayed_bound_grouping_preserves_each_actual_window_and_revision() -> anyhow::Result<()>
    {
        use crate::api::mcp::test_support::{ProfileConnection, tool_call};
        let transport = ProfileConnection::new();
        let browser = crate::api::mcp::test_support::current_session(transport.clone());
        browser.pages.list().await?;
        let mut old = tool_call("tabs", json!({"action":"new"})).await?;
        old.browser_session = Some(browser.clone());
        let eng = old
            .state
            .targets
            .set(
                &old.session_id,
                ProfileConnection::binding("eng", 1, 1).intent,
            )
            .await?;
        old.view = Some(BrowserView::capture(browser.clone(), Some(eng.clone()), None).await);
        assert!(
            old.view
                .as_ref()
                .is_some_and(|view| view.admission_error().is_none())
        );
        let old_page = created_page(&old, 1).await;
        let other_eng_window = created_page(&old, 3).await;
        let ops = old
            .state
            .targets
            .set(
                &old.session_id,
                ProfileConnection::binding("ops", 2, 2).intent,
            )
            .await?;
        let mut new = old.clone();
        new.view = Some(BrowserView::capture(browser.clone(), Some(ops.clone()), None).await);
        let new_page = created_page(&new, 2).await;
        run_tab_group_work(new.clone(), new_page).await;
        run_tab_group_work(old.clone(), old_page).await;
        run_tab_group_work(old.clone(), other_eng_window).await;
        let key = old
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        let ownership = old.state.sessions.ownership();
        let ops_group = ownership
            .group_for_capture(
                &key,
                &ops.intent.browser_profile_id,
                Some(WindowId(2)),
                ops.revision,
            )
            .await;
        let eng_group = ownership
            .group_for_capture(
                &key,
                &eng.intent.browser_profile_id,
                Some(WindowId(1)),
                eng.revision,
            )
            .await;
        let eng_other_group = ownership
            .group_for_capture(
                &key,
                &eng.intent.browser_profile_id,
                Some(WindowId(3)),
                eng.revision,
            )
            .await;
        assert!(ops_group.is_some() && eng_group.is_some() && eng_other_group.is_some());
        assert_ne!(
            ops_group
                .as_ref()
                .and_then(|group| group.group_ref.as_ref()),
            eng_group
                .as_ref()
                .and_then(|group| group.group_ref.as_ref())
        );
        assert_eq!(ownership.groups(&key).await.len(), 3);
        let tabs = transport
            .tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        assert_eq!(
            tabs.iter()
                .map(|tab| tab["windowId"].clone())
                .collect::<Vec<_>>(),
            vec![json!(1), json!(2), json!(3)]
        );
        // Rebinding changes the captured generation, but native group identity
        // remains conversation/profile/actual-window. The view retains revision.
        let rebound = old
            .state
            .targets
            .set(&old.session_id, eng.intent.clone())
            .await?;
        assert_eq!(
            ownership
                .group_for_capture(
                    &key,
                    &eng.intent.browser_profile_id,
                    Some(WindowId(1)),
                    rebound.revision
                )
                .await
                .and_then(|group| group.group_ref),
            eng_group.and_then(|group| group.group_ref),
        );
        // Lifecycle covers all groups, rather than losing the old run's group behind
        // whichever group most recently updated the conversation's presentation.
        assert!(close_agent_tab_group(Some(&browser), &ownership, &key).await);
        assert!(ownership.groups(&key).await.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn borrowed_group_cleanup_leaves_its_tabs_and_group_open() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("borrowed", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let ownership = Arc::new(PageOwnership::new());
        let key = ConvoId::new("borrower");
        ownership
            .update_group(&key, |group| {
                group.group_ref = Some("borrowed".into());
                group.borrowed = true;
                group.window_id = Some(WindowId(1));
            })
            .await;
        assert!(collapse_agent_tab_group(Some(&browser), &ownership, &key).await);
        assert!(close_agent_tab_group(Some(&browser), &ownership, &key).await);
        assert_eq!(recorder.tab_group_call_count(), 0);
        assert_eq!(recorder.group_members("borrowed"), BTreeSet::from([101]));
        Ok(())
    }

    struct GroupDispatchRecorder {
        sender: broadcast::Sender<CdpEvent>,
        calls: StdMutex<Vec<(String, Value)>>,
        members: StdMutex<HashMap<String, BTreeSet<i64>>>,
        titles: StdMutex<HashMap<String, String>>,
        block_create: AtomicBool,
        fail_group_add: AtomicBool,
        fail_title_updates: AtomicBool,
        fail_collapse: AtomicBool,
        fail_close: AtomicBool,
        fail_list: AtomicBool,
        malformed_list: AtomicBool,
        block_list: AtomicBool,
        create_release: Notify,
        list_entered: Notify,
        list_release: Notify,
    }

    impl GroupDispatchRecorder {
        fn new() -> Self {
            let (sender, _) = broadcast::channel(8);
            Self {
                sender,
                calls: StdMutex::new(Vec::new()),
                members: StdMutex::new(HashMap::new()),
                titles: StdMutex::new(HashMap::new()),
                block_create: AtomicBool::new(false),
                fail_group_add: AtomicBool::new(false),
                fail_title_updates: AtomicBool::new(false),
                fail_collapse: AtomicBool::new(false),
                fail_close: AtomicBool::new(false),
                fail_list: AtomicBool::new(false),
                malformed_list: AtomicBool::new(false),
                block_list: AtomicBool::new(false),
                create_release: Notify::new(),
                list_entered: Notify::new(),
                list_release: Notify::new(),
            }
        }

        fn record(&self, method: &str, params: &Value) {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((method.to_string(), params.clone()));
        }

        fn group_result(&self, group_id: &str, params: &Value) -> Value {
            let tab_ids = self
                .members
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(group_id)
                .cloned()
                .unwrap_or_default();
            json!({
                "group": {
                    "groupId": group_id,
                    "windowId": 1,
                    "title": params
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            self.titles
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .get(group_id)
                                .cloned()
                        })
                        .unwrap_or_else(|| "codex".to_string()),
                    "color": params.get("color").and_then(Value::as_str).unwrap_or("blue"),
                    "collapsed": params.get("collapsed").and_then(Value::as_bool).unwrap_or(false),
                    "tabIds": tab_ids
                }
            })
        }

        fn create_count(&self) -> usize {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .filter(|(method, _)| method == "Browser.createTabGroup")
                .count()
        }

        fn tab_group_call_count(&self) -> usize {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .filter(|(method, _)| method.starts_with("Browser.") && method.contains("TabGroup"))
                .count()
        }

        fn group_members(&self, group_id: &str) -> BTreeSet<i64> {
            self.members
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(group_id)
                .cloned()
                .unwrap_or_default()
        }

        fn create_title(&self) -> Option<String> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .find(|(method, _)| method == "Browser.createTabGroup")
                .and_then(|(_, params)| params.get("title"))
                .and_then(Value::as_str)
                .map(str::to_string)
        }

        fn title_updates(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .filter(|(method, _)| method == "Browser.updateTabGroup")
                .filter_map(|(_, params)| params.get("title"))
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        }

        fn block_group_creation(&self) {
            self.block_create.store(true, Ordering::SeqCst);
        }

        fn release_group_creation(&self) {
            self.create_release.notify_one();
        }

        fn fail_title_updates(&self, fail: bool) {
            self.fail_title_updates.store(fail, Ordering::SeqCst);
        }

        fn fail_group_add(&self, fail: bool) {
            self.fail_group_add.store(fail, Ordering::SeqCst);
        }

        fn seed_group(&self, group_id: &str, tab_ids: impl IntoIterator<Item = i64>) {
            self.members
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(group_id.to_string(), tab_ids.into_iter().collect());
        }

        fn seed_group_titled(
            &self,
            group_id: &str,
            title: &str,
            tab_ids: impl IntoIterator<Item = i64>,
        ) {
            self.seed_group(group_id, tab_ids);
            self.titles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(group_id.to_string(), title.to_string());
        }

        fn fail_close(&self, fail: bool) {
            self.fail_close.store(fail, Ordering::SeqCst);
        }

        fn fail_collapse(&self, fail: bool) {
            self.fail_collapse.store(fail, Ordering::SeqCst);
        }

        fn fail_list(&self, fail: bool) {
            self.fail_list.store(fail, Ordering::SeqCst);
        }

        fn malformed_list(&self, malformed: bool) {
            self.malformed_list.store(malformed, Ordering::SeqCst);
        }

        fn block_group_list(&self) {
            self.block_list.store(true, Ordering::SeqCst);
        }

        fn release_group_list(&self) {
            self.list_release.notify_one();
        }
    }

    impl CdpConnection for GroupDispatchRecorder {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a browseros_core::SessionId>,
        ) -> futures_util::future::BoxFuture<'a, Result<Value, browseros_cdp::CdpError>> {
            self.send_at_epoch(1, method, params, session)
        }

        fn send_at_epoch<'a>(
            &'a self,
            expected_epoch: u64,
            method: &'a str,
            params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                // This fixture's epoch is immutable; validation and any synchronous
                // recorded publication cannot be separated by a reconnect.
                if expected_epoch != 1 {
                    return Err(browseros_cdp::CdpError::ConnectionEpochChanged {
                        expected: expected_epoch,
                        actual: 1,
                    });
                }
                self.record(method, &params);
                match method {
                    "Browser.getTabs" => Ok(json!({
                        "tabs": [test_tab(101, "target-1"), test_tab(102, "target-2")]
                    })),
                    "Browser.getTabGroups" => {
                        if self.block_list.load(Ordering::SeqCst) {
                            self.list_entered.notify_one();
                            self.list_release.notified().await;
                        }
                        if self.fail_list.load(Ordering::SeqCst) {
                            return Err(CdpError::Protocol {
                                code: -1,
                                message: "group list failed".to_string(),
                            });
                        }
                        if self.malformed_list.load(Ordering::SeqCst) {
                            return Ok(json!({ "unexpected": [] }));
                        }
                        let group_ids = self
                            .members
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>();
                        let groups = group_ids
                            .iter()
                            .map(|group_id| {
                                self.group_result(group_id, &json!({}))
                                    .get("group")
                                    .cloned()
                                    .unwrap_or(Value::Null)
                            })
                            .collect::<Vec<_>>();
                        Ok(json!({ "groups": groups }))
                    }
                    "Browser.createTabGroup" => {
                        if self.block_create.load(Ordering::SeqCst) {
                            self.create_release.notified().await;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        let tab_ids = params
                            .get("tabIds")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_i64)
                            .collect::<BTreeSet<_>>();
                        self.members
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .insert("group-1".to_string(), tab_ids);
                        Ok(self.group_result("group-1", &params))
                    }
                    "Browser.addTabsToGroup" => {
                        if self.fail_group_add.load(Ordering::SeqCst) {
                            return Err(CdpError::Protocol {
                                code: -1,
                                message: "group add failed".to_string(),
                            });
                        }
                        let group_id = params
                            .get("groupId")
                            .and_then(Value::as_str)
                            .unwrap_or("group-1");
                        let mut members = self
                            .members
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        members.entry(group_id.to_string()).or_default().extend(
                            params
                                .get("tabIds")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_i64),
                        );
                        drop(members);
                        Ok(self.group_result(group_id, &params))
                    }
                    "Browser.updateTabGroup" => {
                        if params.get("collapsed") == Some(&Value::Bool(true))
                            && self.fail_collapse.load(Ordering::SeqCst)
                        {
                            return Err(CdpError::Protocol {
                                code: -1,
                                message: "Tab group not found".to_string(),
                            });
                        }
                        if params.get("title").is_some()
                            && self.fail_title_updates.load(Ordering::SeqCst)
                        {
                            return Err(CdpError::Protocol {
                                code: -1,
                                message: "title update failed".to_string(),
                            });
                        }
                        let group_id = params
                            .get("groupId")
                            .and_then(Value::as_str)
                            .unwrap_or("group-1");
                        Ok(self.group_result(group_id, &params))
                    }
                    "Browser.closeTabGroup" => {
                        if self.fail_close.load(Ordering::SeqCst) {
                            return Err(CdpError::Protocol {
                                code: -1,
                                message: "group close failed".to_string(),
                            });
                        }
                        if let Some(group_id) = params.get("groupId").and_then(Value::as_str) {
                            self.members
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .remove(group_id);
                        }
                        Ok(json!({}))
                    }
                    _ => Err(CdpError::Protocol {
                        code: -1,
                        message: format!("unexpected CDP call: {method}"),
                    }),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async move {
                Err(CdpError::Protocol {
                    code: -1,
                    message: format!("unexpected raw CDP call: {method}"),
                })
            })
        }

        fn send_raw_json_at_epoch<'a>(
            &'a self,
            expected_epoch: u64,
            method: &'a str,
            params_json: &'a str,
            session: Option<&'a browseros_core::SessionId>,
        ) -> futures_util::future::BoxFuture<'a, Result<String, browseros_cdp::CdpError>> {
            Box::pin(async move {
                let params = serde_json::from_str(params_json).map_err(|error| {
                    browseros_cdp::CdpError::Protocol {
                        code: -1,
                        message: error.to_string(),
                    }
                })?;
                Ok(self
                    .send_at_epoch(expected_epoch, method, params, session)
                    .await?
                    .to_string())
            })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.sender.subscribe()
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    fn test_tab(tab_id: i64, target_id: &str) -> Value {
        json!({
            "tabId": tab_id,
            "targetId": target_id,
            "url": format!("https://example.com/{target_id}"),
            "title": target_id,
            "isActive": true,
            "isLoading": false,
            "loadProgress": 1.0,
            "isPinned": false,
            "isHidden": false,
            "windowId": 1,
            "index": tab_id - 101
        })
    }

    async fn connected_call(
        recorder: Arc<GroupDispatchRecorder>,
    ) -> anyhow::Result<(ToolCall, Arc<BrowserSession>)> {
        let browser = crate::api::mcp::test_support::current_session(recorder);
        assert_eq!(browser.pages.list().await?.len(), 2);
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser.clone());
        Ok((call, browser))
    }

    #[test]
    fn first_text_returns_empty_when_result_has_no_text() {
        let result = ToolResult::image("aGVsbG8=", "image/jpeg", json!({}));
        assert!(first_text(&result).is_empty());
    }

    #[test]
    fn cached_tab_groups_tool_definition_is_reused() {
        assert!(std::ptr::eq(
            cached_tab_groups_tool(),
            cached_tab_groups_tool()
        ));
    }

    #[tokio::test]
    async fn retained_group_collapse_updates_state_and_disconnected_close_retries()
    -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-1", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder);
        assert_eq!(browser.pages.list().await?.len(), 2);
        let ownership = Arc::new(PageOwnership::new());
        let key = ConvoId::new("codex-agile-alpaca");
        ownership
            .set_tab_group_ref(key.clone(), Some("group-1".to_string()))
            .await;

        assert!(collapse_agent_tab_group(Some(&browser), &ownership, &key).await);
        assert!(ownership.tab_group_collapsed(&key).await);
        assert!(!close_agent_tab_group(None, &ownership, &key).await);
        assert_eq!(
            ownership.tab_group_ref(&key).await.as_deref(),
            Some("group-1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn retained_group_collapse_confirms_absence_and_stops_sweep_cdp_work()
    -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.fail_collapse(true);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let dir = tempfile::tempdir()?;
        let database = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        let sessions = Sessions::new(
            Arc::new(AuditLog::new(database.clone())),
            Arc::new(SessionTabLedger::new(database)),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let hook_browser = browser.clone();
        sessions.set_retained_group_hook(Arc::new(move |ownership, key, action| {
            let hook_browser = hook_browser.clone();
            Box::pin(async move {
                match action {
                    RetainedGroupAction::Collapse => {
                        collapse_agent_tab_group(Some(&hook_browser), &ownership, &key).await
                    }
                    RetainedGroupAction::Close => {
                        close_agent_tab_group(Some(&hook_browser), &ownership, &key).await
                    }
                }
            })
        }));
        let session = Session::new(
            AppSessionId::new("session-1"),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "agile-alpaca".to_string()),
            "Codex".to_string(),
            tokio::time::Instant::now(),
        );
        let key = session.convo_id().clone();
        sessions.insert_for_testing(session.clone()).await;
        let ownership = sessions.ownership();
        ownership.claim_page(key.clone(), PageId(1)).await;
        ownership
            .set_tab_group_ref(key.clone(), Some("group-1".to_string()))
            .await;
        ownership.remove_page(&PageId(1)).await;

        assert!(sessions.remove(session.id(), "closed", None).await?);
        assert_eq!(ownership.tab_group_ref(&key).await, None);
        assert_eq!(recorder.tab_group_call_count(), 2);

        assert_eq!(sessions.sweep_idle().await?, 0);
        assert_eq!(recorder.tab_group_call_count(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn retained_group_collapse_keeps_state_when_group_exists() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-1", [101]);
        recorder.fail_collapse(true);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let ownership = Arc::new(PageOwnership::new());
        let key = ConvoId::new("codex-agile-alpaca");
        ownership
            .set_tab_group_ref(key.clone(), Some("group-1".to_string()))
            .await;

        assert!(!collapse_agent_tab_group(Some(&browser), &ownership, &key).await);
        assert_eq!(
            ownership.tab_group_ref(&key).await.as_deref(),
            Some("group-1")
        );
        assert!(!ownership.tab_group_collapsed(&key).await);
        assert_eq!(recorder.tab_group_call_count(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn retained_group_collapse_keeps_state_when_existence_is_unknown() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-1", [101]);
        recorder.fail_collapse(true);
        recorder.fail_list(true);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let ownership = Arc::new(PageOwnership::new());
        let key = ConvoId::new("codex-agile-alpaca");
        ownership
            .set_tab_group_ref(key.clone(), Some("group-1".to_string()))
            .await;

        assert!(!collapse_agent_tab_group(Some(&browser), &ownership, &key).await);
        recorder.fail_list(false);
        recorder.malformed_list(true);
        assert!(!collapse_agent_tab_group(Some(&browser), &ownership, &key).await);
        assert_eq!(
            ownership.tab_group_ref(&key).await.as_deref(),
            Some("group-1")
        );
        assert_eq!(recorder.tab_group_call_count(), 4);
        Ok(())
    }

    #[tokio::test]
    async fn retained_group_collapse_does_not_clear_a_replacement_group() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.fail_collapse(true);
        recorder.block_group_list();
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let ownership = Arc::new(PageOwnership::new());
        let key = ConvoId::new("codex-agile-alpaca");
        ownership
            .set_tab_group_ref(key.clone(), Some("group-1".to_string()))
            .await;
        let list_entered = recorder.list_entered.notified();
        let collapse_browser = browser.clone();
        let collapse_ownership = ownership.clone();
        let collapse_key = key.clone();
        let collapse = tokio::spawn(async move {
            collapse_agent_tab_group(Some(&collapse_browser), &collapse_ownership, &collapse_key)
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), list_entered).await?;
        recorder.seed_group("group-2", [102]);
        ownership
            .set_tab_group_ref(key.clone(), Some("group-2".to_string()))
            .await;
        recorder.release_group_list();

        assert!(
            !collapse.await?,
            "a replacement group still needs its own collapse"
        );
        assert_eq!(
            ownership.tab_group_ref(&key).await.as_deref(),
            Some("group-2")
        );
        assert!(!ownership.tab_group_collapsed(&key).await);
        Ok(())
    }

    #[tokio::test]
    async fn retained_group_close_succeeds_and_confirms_already_absent_groups() -> anyhow::Result<()>
    {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-1", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let ownership = Arc::new(PageOwnership::new());
        let key = ConvoId::new("codex-agile-alpaca");
        ownership
            .set_tab_group_ref(key.clone(), Some("group-1".to_string()))
            .await;

        assert!(close_agent_tab_group(Some(&browser), &ownership, &key).await);
        ownership
            .set_tab_group_ref(key.clone(), Some("group-1".to_string()))
            .await;
        recorder.fail_close(true);
        assert!(close_agent_tab_group(Some(&browser), &ownership, &key).await);
        Ok(())
    }

    #[tokio::test]
    async fn retained_group_close_keeps_state_when_group_may_still_exist() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-1", [101]);
        recorder.fail_close(true);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let ownership = Arc::new(PageOwnership::new());
        let key = ConvoId::new("codex-agile-alpaca");
        ownership
            .set_tab_group_ref(key.clone(), Some("group-1".to_string()))
            .await;

        assert!(!close_agent_tab_group(Some(&browser), &ownership, &key).await);
        recorder.fail_list(true);
        assert!(!close_agent_tab_group(Some(&browser), &ownership, &key).await);
        assert_eq!(
            ownership.tab_group_ref(&key).await.as_deref(),
            Some("group-1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn effect_returns_before_group_creation_finishes() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.block_group_creation();
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        let result = ToolResult::text("opened", Some(json!({ "page": 1 })));

        let applied = tokio::time::timeout(
            Duration::from_millis(50),
            apply(ToolEffectContext {
                call: &call,
                result: &result,
                cancelled: false,
                duration_ms: 1,
            }),
        )
        .await;
        recorder.release_group_creation();
        assert!(
            applied.is_ok(),
            "tab-group effect blocked the tool response"
        );
        Ok(())
    }

    /// A reconnected agent names the group it was working in, and later pages
    /// follow it instead of a second group being created for one task.
    ///
    /// Adoption deliberately changes no page's owner. It grants nothing a
    /// caller did not already have, because `tab_groups action="create"` with
    /// an existing groupId already moved pages into any group.
    #[tokio::test]
    async fn naming_a_group_makes_it_this_sessions_group() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-earlier", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-earlier" }),
        )
        .await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        let key = call
            .identity
            .as_ref()
            .unwrap_or_else(|| unreachable!())
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();

        run_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 2,
                window_id: Some(WindowId(1)),
            }),
        )
        .await;

        assert_eq!(
            ownership.tab_group_ref(&key).await.as_deref(),
            Some("group-earlier"),
            "the named group survives the default-group reconciliation"
        );
        assert!(
            recorder.group_members("group-earlier").contains(&102),
            "the page joins it: {:?}",
            recorder.group_members("group-earlier")
        );
        assert_eq!(
            recorder.create_count(),
            0,
            "an existing group is joined, never recreated"
        );
        assert_eq!(
            ownership.owner_of_page(&PageId(1)).await.as_ref(),
            None,
            "borrowing a group does not claim its pre-existing user tab"
        );
        Ok(())
    }

    /// A remembered id goes stale the moment its group is closed. Adopting one
    /// would swap a working reference for one that cannot be added to, and the
    /// add-failure path clears the reference and returns without creating a
    /// replacement, so the page would end up in no group at all.
    #[tokio::test]
    async fn naming_a_closed_group_keeps_the_session_grouped() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-live", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-closed" }),
        )
        .await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        // The live server sets this from the session's group on every call. A
        // fixture that leaves it None cannot reach the reconciliation that this
        // guards, which is why an earlier version of this test passed with the
        // guard removed.
        call.default_tab_group_id = Some("group-live".to_string());
        let key = call
            .identity
            .as_ref()
            .unwrap_or_else(|| unreachable!())
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();
        ownership
            .set_tab_group_ref(key.clone(), Some("group-live".to_string()))
            .await;

        run_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 2,
                window_id: Some(WindowId(1)),
            }),
        )
        .await;

        assert_eq!(
            ownership.tab_group_ref(&key).await.as_deref(),
            Some("group-live"),
            "the working reference survives a stale id being named"
        );
        assert!(
            recorder.group_members("group-live").contains(&102),
            "and the page joins the session's own group rather than a second one: {:?}",
            recorder.group_members("group-live")
        );
        assert_eq!(
            recorder.create_count(),
            0,
            "no second group is minted for a task that already has one"
        );
        Ok(())
    }

    /// The client label alone does not prove task identity or transfer ownership.
    /// A second task can borrow a group while the first still owns its tabs.
    #[tokio::test]
    async fn borrowing_a_group_keeps_an_earlier_sessions_actual_ownership() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-earlier", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-earlier" }),
        )
        .await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        let identity = call.identity.as_ref().unwrap_or_else(|| unreachable!());
        // The same client reconnecting: a second session with the caller's slug.
        let earlier = crate::services::sessions::Session::new(
            crate::ids::SessionId::new("earlier"),
            identity.agent.clone(),
            crate::identity::ConversationIdentity::new(identity.agent.slug(), "prior".to_string()),
            identity.agent_label.clone(),
            tokio::time::Instant::now(),
        );
        call.state
            .sessions
            .insert_for_testing(earlier.clone())
            .await;
        let ownership = call.state.sessions.ownership();
        ownership
            .claim_page(earlier.convo_id().clone(), PageId(1))
            .await;

        run_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 2,
                window_id: Some(WindowId(1)),
            }),
        )
        .await;

        assert_eq!(
            ownership.owner_of_page(&PageId(1)).await.as_ref(),
            Some(earlier.convo_id()),
            "borrowing a group preserves the earlier task's actual tab ownership"
        );
        Ok(())
    }

    fn note_text(result: &ToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn apply_tabs_new(call: &ToolCall) -> Option<ToolResult> {
        let result = ToolResult::text("opened", Some(json!({ "page": 1 })));
        apply(ToolEffectContext {
            call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await
        .unwrap_or_else(|error| panic!("effect failed: {error}"))
    }

    /// The nudge that needs no memory. Both texts already tell an agent to reuse
    /// its group and a live client skipped them, so the one call that starts a
    /// second group names the candidates with their ids.
    #[tokio::test]
    async fn starting_a_group_names_the_groups_already_titled_for_this_client() -> anyhow::Result<()>
    {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group_titled("group-earlier", "codex/invoice-run", [101]);
        recorder.seed_group_titled("group-legacy", "codex-mcp-client/legacy-task", [103]);
        recorder.seed_group_titled("group-truncated", "codex-posthog/dashboard", [104]);
        recorder.seed_group_titled("group-theirs", "cowork/research", [102]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        // The first tabs new of a fresh connection: no group for this session yet.
        call.default_tab_group_id = None;

        let annotated = apply_tabs_new(&call)
            .await
            .unwrap_or_else(|| panic!("expected a note naming the earlier group"));
        let text = note_text(&annotated);

        assert!(text.contains("codex/invoice-run"), "{text}");
        assert!(text.contains("codex-mcp-client/legacy-task"), "{text}");
        assert!(text.contains("codex-posthog/dashboard"), "{text}");
        assert!(text.contains("may be earlier tasks"), "{text}");
        assert!(!text.contains("they are tasks of yours"), "{text}");
        assert!(
            text.contains("group-earlier"),
            "names the id to pass: {text}"
        );
        assert!(text.contains("groupId"), "names the argument: {text}");
        assert!(
            !text.contains("cowork/research"),
            "another client's group is not a candidate of mine: {text}"
        );
        assert!(
            text.contains("opened"),
            "the original result survives: {text}"
        );
        Ok(())
    }

    /// An agent that named a group is already doing what the note would ask for.
    #[tokio::test]
    async fn naming_a_group_suppresses_the_nudge() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group_titled("group-earlier", "codex/invoice-run", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-earlier" }),
        )
        .await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        call.default_tab_group_id = None;

        assert!(
            apply_tabs_new(&call).await.is_none(),
            "an agent already passing a groupId needs no nudge"
        );
        Ok(())
    }

    /// Mid-task, not reconnecting. The session has its group and every later tab
    /// would otherwise carry the note.
    #[tokio::test]
    async fn a_session_that_already_has_a_group_gets_no_nudge() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group_titled("group-earlier", "codex/invoice-run", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        call.default_tab_group_id = Some("group-mine".to_string());

        assert!(
            apply_tabs_new(&call).await.is_none(),
            "a session mid-task must not be told to reuse something"
        );
        Ok(())
    }

    /// A client's genuine first task. Nothing is its own, so there is nothing to
    /// suggest and the note would be noise.
    #[tokio::test]
    async fn no_group_of_this_clients_own_means_no_nudge() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group_titled("group-theirs", "cowork/research", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        call.default_tab_group_id = None;

        assert!(
            apply_tabs_new(&call).await.is_none(),
            "only this client's own groups are candidates"
        );
        Ok(())
    }

    /// A listing that fails must not look like a closed group. The named group
    /// is still adopted, so a timeout cannot silently split one task across two
    /// groups. Nothing is claimed, because without a listing there is no page
    /// list to claim from.
    #[tokio::test]
    async fn a_failed_group_listing_still_adopts_the_named_group() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-earlier", [101]);
        recorder.fail_list(true);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-earlier" }),
        )
        .await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        // Set the same way the live server sets it, so the reconciliation this
        // has to beat is actually reachable.
        call.default_tab_group_id = Some("group-fallback".to_string());
        let key = call
            .identity
            .as_ref()
            .unwrap_or_else(|| unreachable!())
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();

        run_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 2,
                window_id: Some(WindowId(1)),
            }),
        )
        .await;

        assert_eq!(
            ownership.tab_group_ref(&key).await.as_deref(),
            Some("group-earlier"),
            "a transient listing failure must not be read as the group being closed"
        );
        assert_eq!(
            recorder.create_count(),
            0,
            "and no second group is minted for the task"
        );
        Ok(())
    }

    /// Claiming skips a page a LIVE session of a different client holds. Not a
    /// refusal of this caller, just not relabelling someone else's work while
    /// they are still using it. The other session has to be live in the
    /// snapshot for this to mean anything, since a bare claim with no session
    /// behind it is the orphan case below.
    #[tokio::test]
    async fn a_page_another_client_holds_is_left_alone() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-shared", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-shared" }),
        )
        .await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        let other_client = Session::new(
            AppSessionId::new("other-client"),
            ClientIdentity::Ephemeral {
                slug: "cowork".to_string(),
                label: "Cowork".to_string(),
            },
            ConversationIdentity::new("cowork", "busy-badger".to_string()),
            "Cowork".to_string(),
            tokio::time::Instant::now(),
        );
        call.state
            .sessions
            .insert_for_testing(other_client.clone())
            .await;
        let ownership = call.state.sessions.ownership();
        let other = other_client.convo_id().clone();
        ownership.claim_page(other.clone(), PageId(1)).await;

        run_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 2,
                window_id: Some(WindowId(1)),
            }),
        )
        .await;

        assert_eq!(
            ownership.owner_of_page(&PageId(1)).await.as_ref(),
            Some(&other),
            "the live session of the other client keeps its page"
        );
        Ok(())
    }

    /// Session retirement does not prove that a live tab became the borrower's.
    /// Keep the recorded claim until global page reconciliation or owner cleanup.
    #[tokio::test]
    async fn borrowing_a_group_keeps_claims_even_when_the_owner_is_retired() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-orphan", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-orphan" }),
        )
        .await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        let ownership = call.state.sessions.ownership();
        ownership
            .claim_page(ConvoId::new("cowork-long-gone"), PageId(1))
            .await;

        run_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 2,
                window_id: Some(WindowId(1)),
            }),
        )
        .await;

        assert_eq!(
            ownership.owner_of_page(&PageId(1)).await.as_ref(),
            Some(&ConvoId::new("cowork-long-gone")),
            "retirement alone does not transfer actual tab ownership"
        );
        Ok(())
    }

    /// Omitting it must not disturb anything, since every call that is not a
    /// reclaim omits it.
    #[tokio::test]
    async fn omitting_the_group_id_leaves_the_existing_group_alone() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-1", [101]);
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        call.view = Some(BrowserView::unbound(browser.clone(), None));
        call.browser_session = Some(browser);
        let key = call
            .identity
            .as_ref()
            .unwrap_or_else(|| unreachable!())
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();
        ownership
            .set_tab_group_ref(key.clone(), Some("group-1".to_string()))
            .await;

        run_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 2,
                window_id: Some(WindowId(1)),
            }),
        )
        .await;

        assert_eq!(
            ownership.tab_group_ref(&key).await.as_deref(),
            Some("group-1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_first_pages_share_one_created_group() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        let browser = crate::api::mcp::test_support::current_session(recorder.clone());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let mut first_call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        first_call.view = Some(BrowserView::unbound(browser.clone(), None));
        first_call.browser_session = Some(browser);
        let second_call = first_call.clone();
        let (first, second) = tokio::join!(
            spawn_tab_group_work(
                first_call.clone(),
                Some(CreatedPage {
                    page_id: 1,
                    window_id: Some(WindowId(1))
                })
            ),
            spawn_tab_group_work(
                second_call,
                Some(CreatedPage {
                    page_id: 2,
                    window_id: Some(WindowId(1))
                })
            )
        );
        first?;
        second?;

        assert_eq!(recorder.create_count(), 1);
        assert_eq!(
            recorder.group_members("group-1"),
            BTreeSet::from([101, 102])
        );
        let key = first_call
            .identity
            .as_ref()
            .unwrap_or_else(|| unreachable!())
            .ownership_key
            .clone();
        assert_eq!(
            first_call
                .state
                .sessions
                .ownership()
                .tab_group_ref(&key)
                .await
                .as_deref(),
            Some("group-1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn session_cancellation_during_create_still_records_the_group() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.block_group_creation();
        let (call, _browser) = connected_call(recorder.clone()).await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let creation = spawn_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 1,
                window_id: Some(WindowId(1)),
            }),
        );
        for _ in 0..100 {
            if recorder.create_count() > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(recorder.create_count(), 1);
        identity.session.cancel();
        recorder.release_group_creation();
        creation.await?;

        assert_eq!(
            call.state
                .sessions
                .ownership()
                .tab_group_ref(&identity.ownership_key)
                .await
                .as_deref(),
            Some("group-1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn group_work_does_not_start_after_session_teardown() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        let (call, _browser) = connected_call(recorder.clone()).await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        identity.session.cancel();

        spawn_tab_group_work(
            call,
            Some(CreatedPage {
                page_id: 1,
                window_id: Some(WindowId(1)),
            }),
        )
        .await?;

        assert_eq!(recorder.create_count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn transient_group_add_failure_keeps_the_winning_group_reference() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        let (call, _browser) = connected_call(recorder.clone()).await?;
        spawn_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 1,
                window_id: Some(WindowId(1)),
            }),
        )
        .await?;
        recorder.fail_group_add(true);
        spawn_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 2,
                window_id: Some(WindowId(1)),
            }),
        )
        .await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;

        assert_eq!(recorder.create_count(), 1);
        assert_eq!(
            call.state
                .sessions
                .ownership()
                .tab_group_ref(&identity.ownership_key)
                .await
                .as_deref(),
            Some("group-1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn rename_before_first_tab_sets_the_creation_title() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        let (call, _browser) = connected_call(recorder.clone()).await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        identity
            .session
            .rename("invoice-processing".to_string())
            .await;

        spawn_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 1,
                window_id: Some(WindowId(1)),
            }),
        )
        .await?;

        assert_eq!(
            recorder.create_title().as_deref(),
            Some("codex/invoice-processing")
        );
        let state = call
            .state
            .sessions
            .ownership()
            .tab_group_state(&identity.ownership_key)
            .await
            .ok_or_else(|| anyhow::anyhow!("group state missing"))?;
        assert_eq!(
            state.desired_title.as_deref(),
            Some("codex/invoice-processing")
        );
        assert!(!state.title_sync_pending);
        Ok(())
    }

    #[tokio::test]
    async fn existing_group_rename_and_rapid_second_rename_keep_the_newest_title()
    -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        let (call, browser) = connected_call(recorder.clone()).await?;
        spawn_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 1,
                window_id: Some(WindowId(1)),
            }),
        )
        .await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let ownership = call.state.sessions.ownership();

        for label in ["invoice-processing", "quarterly-reporting"] {
            identity.session.rename(label.to_string()).await;
            apply_agent_tab_group_title(
                Some(&browser),
                &ownership,
                &identity.ownership_key,
                identity.session.as_ref(),
                identity.session.child_token(),
            )
            .await;
        }

        assert_eq!(
            recorder.title_updates().last().map(String::as_str),
            Some("codex/quarterly-reporting")
        );
        let state = ownership
            .tab_group_state(&identity.ownership_key)
            .await
            .ok_or_else(|| anyhow::anyhow!("group state missing"))?;
        assert_eq!(
            state.desired_title.as_deref(),
            Some("codex/quarterly-reporting")
        );
        assert!(!state.title_sync_pending);
        Ok(())
    }

    #[tokio::test]
    async fn delayed_rename_publication_recomputes_the_current_session_title() -> anyhow::Result<()>
    {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        let (call, browser) = connected_call(recorder.clone()).await?;
        spawn_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 1,
                window_id: Some(WindowId(1)),
            }),
        )
        .await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let ownership = call.state.sessions.ownership();
        identity.session.rename("first-rename".to_string()).await;
        identity.session.rename("newest-rename".to_string()).await;

        for _ in 0..2 {
            apply_agent_tab_group_title(
                Some(&browser),
                &ownership,
                &identity.ownership_key,
                identity.session.as_ref(),
                identity.session.child_token(),
            )
            .await;
        }

        assert_eq!(
            recorder.title_updates().last().map(String::as_str),
            Some("codex/newest-rename")
        );
        assert_eq!(
            ownership
                .tab_group_state(&identity.ownership_key)
                .await
                .and_then(|state| state.desired_title),
            Some("codex/newest-rename".to_string())
        );
        Ok(())
    }

    #[tokio::test]
    async fn rename_during_group_creation_ends_with_the_newest_title() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.block_group_creation();
        let (call, browser) = connected_call(recorder.clone()).await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let creation = spawn_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 1,
                window_id: Some(WindowId(1)),
            }),
        );
        for _ in 0..100 {
            if recorder.create_count() > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(recorder.create_count(), 1);
        identity
            .session
            .rename("invoice-processing".to_string())
            .await;
        recorder.release_group_creation();
        creation.await?;
        apply_agent_tab_group_title(
            Some(&browser),
            &call.state.sessions.ownership(),
            &identity.ownership_key,
            identity.session.as_ref(),
            identity.session.child_token(),
        )
        .await;

        assert_eq!(
            recorder.title_updates().last().map(String::as_str),
            Some("codex/invoice-processing")
        );
        Ok(())
    }

    #[tokio::test]
    async fn disconnected_and_failed_title_updates_retry_on_later_dispatch() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        let (call, browser) = connected_call(recorder.clone()).await?;
        spawn_tab_group_work(
            call.clone(),
            Some(CreatedPage {
                page_id: 1,
                window_id: Some(WindowId(1)),
            }),
        )
        .await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let ownership = call.state.sessions.ownership();

        identity
            .session
            .rename("disconnected-rename".to_string())
            .await;
        apply_agent_tab_group_title(
            None,
            &ownership,
            &identity.ownership_key,
            identity.session.as_ref(),
            identity.session.child_token(),
        )
        .await;
        assert!(
            ownership
                .tab_group_state(&identity.ownership_key)
                .await
                .is_some_and(|state| state.title_sync_pending)
        );
        run_tab_group_work(call.clone(), None).await;
        assert_eq!(
            recorder.title_updates().last().map(String::as_str),
            Some("codex/disconnected-rename")
        );

        recorder.fail_title_updates(true);
        identity.session.rename("retry-title".to_string()).await;
        apply_agent_tab_group_title(
            Some(&browser),
            &ownership,
            &identity.ownership_key,
            identity.session.as_ref(),
            identity.session.child_token(),
        )
        .await;
        assert!(
            ownership
                .tab_group_state(&identity.ownership_key)
                .await
                .is_some_and(|state| state.title_sync_pending)
        );
        recorder.fail_title_updates(false);
        run_tab_group_work(call.clone(), None).await;
        let state = ownership
            .tab_group_state(&identity.ownership_key)
            .await
            .ok_or_else(|| anyhow::anyhow!("group state missing"))?;
        assert_eq!(state.desired_title.as_deref(), Some("codex/retry-title"));
        assert!(!state.title_sync_pending);
        Ok(())
    }

    #[tokio::test]
    async fn each_group_dispatch_uses_the_shared_timeout() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.block_group_creation();
        let browser = crate::api::mcp::test_support::current_session(recorder);
        assert_eq!(browser.pages.list().await?.len(), 2);
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        // SQLite setup needs real time; only the dispatch timeout uses the paused clock.
        tokio::time::pause();
        let dispatch = tokio::spawn(async move {
            dispatch_tab_groups(
                call.tool_named("tab_groups")
                    .unwrap_or_else(|| unreachable!()),
                &BrowserView::unbound(browser, None),
                CancellationToken::new(),
                call.output_files.clone(),
                json!({ "action": "create", "pages": [1] }),
            )
            .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(TAB_GROUP_OPERATION).await;
        let dispatch_result = dispatch.await?;
        let Err(error) = dispatch_result else {
            panic!("group dispatch should time out");
        };
        assert!(error.contains("timed out after 10000ms"));
        Ok(())
    }
}
