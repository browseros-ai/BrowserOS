use crate::{
    api::mcp::{
        dispatch::{ToolCall, ToolEffect, ToolEffectContext, result_page_id},
        naming::{client_prefix_from_slug, desired_group_title},
        timeouts::TAB_GROUP_OPERATION,
    },
    ids::ConvoId,
    services::{
        browser::color_for_slug,
        sessions::{PageOwnership, Session},
    },
};
use browseros_core::{BrowserSession, PageId};
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
        let page_id = if context.call.flags.new_page {
            result_page_id(context.result)
        } else {
            None
        };
        // Computed before the group work is spawned, because that work creates this
        // session's own group and the listing would then report it as a candidate
        // the agent should consider reusing.
        let notice = own_group_candidates_notice(&context).await;
        // Detach browser-group synchronization so cosmetic/durable grouping cannot
        // delay the tool response.
        drop(spawn_tab_group_work(context.call.clone(), page_id));
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
    let browser = context.call.browser_session.as_ref()?;
    // The title is `{prefix}/{label}`, so the separator terminates the prefix and
    // this match cannot run into a longer client name the way a bare prefix would.
    let prefix = format!("{}/", client_prefix_from_slug(identity.agent.slug()));
    let candidates = open_groups(browser, context.call.output_files.clone())
        .await?
        .into_iter()
        .filter(|(_, title)| title.starts_with(&prefix))
        .map(|(group_id, title)| format!("{title} (id {group_id})"))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    Some(format!(
        "note: a new tab group is being started for this session. These open groups are already \
         named for you, so they are tasks of yours from an earlier connection: {}. If you are \
         continuing one of them, pass its id as groupId on tabs action=\"new\" and your pages go \
         there instead, with the tabs already in it reading as yours again.",
        candidates.join("; ")
    ))
}

/// Every open group as `(id, title)`, or `None` when the listing cannot be read.
///
/// Silent on failure on purpose: a note that names no groups, or invents the
/// absence of them, is worse than no note.
async fn open_groups(
    browser: &Arc<BrowserSession>,
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

fn spawn_tab_group_work(call: ToolCall, page_id: Option<u32>) -> JoinHandle<()> {
    tokio::spawn(run_tab_group_work(call, page_id))
}

/// Ensures the calling agent's tab group exists and, when `page_id` is set,
/// places that page in it. Shared by the tab-groups effect and the code-mode
/// script hook so a page a script opens joins the agent's group the same way a
/// `tabs new` page does.
pub(crate) async fn run_tab_group_work(call: ToolCall, page_id: Option<u32>) {
    let (Some(identity), Some(browser), Some(tab_groups)) = (
        call.identity.as_ref(),
        call.browser_session.as_ref(),
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
    let Some(page_id) = page_id else {
        return;
    };
    // A caller that names a group is continuing work in it, so that group
    // becomes this session's and later pages follow it. This has to happen
    // before the default-group reconciliation below, which would otherwise see
    // the page in a group that is not the session's and clear the reference it
    // was just given.
    //
    // The unclaimed pages already in that group become this session's too, so
    // the reconnected agent picks up the work rather than treating its own tabs
    // as someone else's. Claims are written rather than inferred at read time
    // because `tabs list`, the ownership notice and code-mode helper discovery
    // all read the claim; inferring it in one makes the three disagree.
    //
    // Nothing is authorized and nothing is refused. Ownership is a label, never
    // a permission, asserted in guards/mod.rs and enforced nowhere, so there is
    // no privilege here to gate. An earlier revision checked the group title
    // against the client's slug; that was removed because it gated nothing real
    // while breaking the honest case, since `tab_groups update` renames any
    // group for anyone and a user renaming a group would have locked its own
    // agent out of it.
    //
    // A page held by another session of the same client is taken over, because
    // that session is the caller's own earlier self: a reconnect mints a new
    // conversation while the old one stays in the ownership map holding the
    // tabs, which is the whole reason the agent sees its own work as foreign.
    // Only taking unclaimed pages made this do nothing at all in the case it
    // exists for, since the pages are claimed, just by the previous session.
    //
    // A page held by a session of a DIFFERENT client is left alone. Not a
    // refusal of this caller, just not relabelling someone else's work.
    //
    // A group that is no longer open is not adopted either. A remembered id goes
    // stale as soon as the group is closed, and adopting one would replace a
    // reference that works with one that cannot: the add then fails, the failure
    // path clears the reference and returns without creating a replacement, and
    // the page ends up in no group at all. One listing answers both questions,
    // so this costs nothing extra.
    let named_group = call
        .raw_args
        .get("groupId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(str::to_string);
    let named_requested = named_group.is_some();
    if let Some(requested) = named_group
        && let Some(pages) =
            group_pages_if_open(browser, &requested, call.output_files.clone()).await
    {
        ownership
            .set_tab_group_ref(identity.ownership_key.clone(), Some(requested))
            .await;
        let held_by_others = live_convos_of_other_clients(&call.state, identity).await;
        for page in pages {
            let claimable = match ownership.owner_of_page(&PageId(page)).await {
                None => true,
                Some(owner) => !held_by_others.contains(&owner),
            };
            if claimable {
                ownership
                    .claim_page(identity.ownership_key.clone(), PageId(page))
                    .await;
            }
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
            .pages
            .get_info(PageId(page_id))
            .await
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

/// Conversations of OTHER clients that are currently live. The only claims a
/// reclaim leaves alone.
///
/// Stated as the exclusion rather than as "my own conversations" on purpose. A
/// session that disconnects or idles out is removed from the live set while its
/// claims survive until the retained window is reaped, so listing the caller's
/// own conversations would miss exactly the case a reconnect is: the previous
/// session is gone from the snapshot but still holds the tabs. Inverting it also
/// covers an orphaned claim from any dead session, which no live caller is using
/// and which would otherwise sit unreclaimable until reaping.
async fn live_convos_of_other_clients(
    state: &crate::AppState,
    identity: &crate::api::mcp::dispatch::ToolIdentity,
) -> std::collections::BTreeSet<ConvoId> {
    let slug = identity.agent.slug();
    state
        .sessions
        .snapshot()
        .await
        .into_iter()
        .filter(|session| session.agent().slug() != slug)
        .map(|session| session.convo_id().clone())
        .collect()
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
    browser: &Arc<BrowserSession>,
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
    browser: &Arc<BrowserSession>,
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
                    .set_tab_group(identity.ownership_key.clone(), None, None)
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

/// Collapses the durable group when its session enters retention, confirming absence on failure.
pub async fn collapse_agent_tab_group(
    browser: Option<&Arc<BrowserSession>>,
    ownership: &Arc<PageOwnership>,
    key: &ConvoId,
) -> bool {
    let operation_lock = ownership.group_operation_lock(key).await;
    let _guard = operation_lock.lock().await;
    if ownership.tab_group_collapsed(key).await {
        return true;
    }
    let Some(group_id) = ownership.tab_group_ref(key).await else {
        return true;
    };
    let Some(browser) = browser else {
        return false;
    };
    let output_files = browseros_mcp::output_file::create_browser_output_file_access();
    let collapsed = dispatch_tab_groups(
        cached_tab_groups_tool(),
        browser,
        CancellationToken::new(),
        output_files.clone(),
        json!({ "action": "update", "groupId": group_id, "collapsed": true }),
    )
    .await;
    let collapse_error = match collapsed {
        Ok(_) => {
            ownership
                .set_tab_group_collapsed_if_current(key, &group_id, true)
                .await;
            return true;
        }
        Err(error) => error,
    };
    if group_exists_unlocked(browser, &group_id, output_files).await == Some(false) {
        ownership
            .clear_tab_group_ref_if_current(key, &group_id)
            .await;
        return true;
    }
    warn!(key = %key, error = %collapse_error, "agent tab group collapse failed");
    false
}

/// Closes a retained session group, confirming absence after a failed close.
pub async fn close_agent_tab_group(
    browser: Option<&Arc<BrowserSession>>,
    ownership: &Arc<PageOwnership>,
    key: &ConvoId,
) -> bool {
    let operation_lock = ownership.group_operation_lock(key).await;
    let _guard = operation_lock.lock().await;
    let Some(group_id) = ownership.tab_group_ref(key).await else {
        return true;
    };
    let Some(browser) = browser else {
        return false;
    };
    let output_files = browseros_mcp::output_file::create_browser_output_file_access();
    let closed = dispatch_tab_groups(
        cached_tab_groups_tool(),
        browser,
        CancellationToken::new(),
        output_files.clone(),
        json!({ "action": "close", "groupId": group_id }),
    )
    .await;
    let close_error = match closed {
        Ok(_) => {
            return ownership
                .clear_tab_group_ref_if_current(key, &group_id)
                .await;
        }
        Err(error) => error,
    };
    if group_exists_unlocked(browser, &group_id, output_files).await == Some(false) {
        return ownership
            .clear_tab_group_ref_if_current(key, &group_id)
            .await;
    }
    warn!(key = %key, error = %close_error, "agent tab group close failed");
    false
}

async fn group_exists_unlocked(
    browser: &Arc<BrowserSession>,
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
    browser: &Arc<BrowserSession>,
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
                .set_tab_group_collapsed_if_current(key, &group_id, false)
                .await;
        }
        Err(reason) => warn!(key = %key, error = %reason, "agent tab group expand failed"),
    }
}

/// Stores the desired title and best-effort applies it to the current group.
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
    browser: &Arc<BrowserSession>,
    ownership: &Arc<PageOwnership>,
    key: &ConvoId,
    cancel: CancellationToken,
    output_files: OutputFileAccess,
) {
    let Some((group_id, title)) = ownership.pending_group_title(key).await else {
        return;
    };
    match dispatch_tab_groups(
        tab_groups,
        browser,
        cancel,
        output_files,
        json!({ "action": "update", "groupId": group_id, "title": title }),
    )
    .await
    {
        Ok(_) => {
            ownership
                .mark_group_title_synced(key, &group_id, &title)
                .await;
        }
        Err(reason) => {
            warn!(key = %key, error = %reason, "session name tab group retitle failed");
        }
    }
}

async fn dispatch_tab_groups(
    tab_groups: &ToolDef,
    browser: &Arc<BrowserSession>,
    cancel: CancellationToken,
    output_files: OutputFileAccess,
    args: Value,
) -> Result<ToolResult, String> {
    let operation_cancel = cancel.child_token();
    let ctx = ToolCtx::new(BrowserToolOptions {
        session: browser.clone(),
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
    use browseros_core::{BrowserSessionHooks, CdpConnection, SessionId};
    use std::{
        collections::{BTreeSet, HashMap},
        sync::{
            Arc, Mutex as StdMutex,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };
    use tokio::sync::{Notify, broadcast};

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
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
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
        let browser = BrowserSession::new(recorder, BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
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
        let browser = BrowserSession::new(recorder, BrowserSessionHooks::default());
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
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

        assert!(collapse.await?);
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-earlier" }),
        )
        .await?;
        call.browser_session = Some(browser);
        let key = call
            .identity
            .as_ref()
            .unwrap_or_else(|| unreachable!())
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();

        run_tab_group_work(call.clone(), Some(2)).await;

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
            Some(&key),
            "the unclaimed page already in the group becomes this session's, so \
             tabs list, the ownership notice and helper discovery agree"
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-closed" }),
        )
        .await?;
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

        run_tab_group_work(call.clone(), Some(2)).await;

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

    /// The case the live run exposed. A reconnect leaves the previous session in
    /// the ownership map still holding the tabs, so the page is claimed, not
    /// unclaimed. Taking over a page held by an earlier session of the same
    /// client is what makes the reclaim actually reclaim.
    #[tokio::test]
    async fn a_page_an_earlier_session_of_this_client_holds_is_taken_over() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-earlier", [101]);
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-earlier" }),
        )
        .await?;
        call.browser_session = Some(browser);
        let identity = call.identity.as_ref().unwrap_or_else(|| unreachable!());
        let key = identity.ownership_key.clone();
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

        run_tab_group_work(call.clone(), Some(2)).await;

        assert_eq!(
            ownership.owner_of_page(&PageId(1)).await.as_ref(),
            Some(&key),
            "the earlier session of this client held it, so the reconnect takes it over"
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
        recorder.seed_group_titled("group-theirs", "cowork/research", [102]);
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        call.browser_session = Some(browser);
        // The first tabs new of a fresh connection: no group for this session yet.
        call.default_tab_group_id = None;

        let annotated = apply_tabs_new(&call)
            .await
            .unwrap_or_else(|| panic!("expected a note naming the earlier group"));
        let text = note_text(&annotated);

        assert!(text.contains("codex/invoice-run"), "{text}");
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-earlier" }),
        )
        .await?;
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-earlier" }),
        )
        .await?;
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

        run_tab_group_work(call.clone(), Some(2)).await;

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
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-shared" }),
        )
        .await?;
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

        run_tab_group_work(call.clone(), Some(2)).await;

        assert_eq!(
            ownership.owner_of_page(&PageId(1)).await.as_ref(),
            Some(&other),
            "the live session of the other client keeps its page"
        );
        Ok(())
    }

    /// A claim whose session is gone belongs to nobody. Leaving it in place
    /// would make the page unreclaimable by anyone until reaping runs, so the
    /// reclaim takes it.
    #[tokio::test]
    async fn a_page_held_by_a_claim_with_no_live_session_is_reclaimed() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-orphan", [101]);
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "new", "groupId": "group-orphan" }),
        )
        .await?;
        call.browser_session = Some(browser);
        let key = call
            .identity
            .as_ref()
            .unwrap_or_else(|| unreachable!())
            .ownership_key
            .clone();
        let ownership = call.state.sessions.ownership();
        ownership
            .claim_page(ConvoId::new("cowork-long-gone"), PageId(1))
            .await;

        run_tab_group_work(call.clone(), Some(2)).await;

        assert_eq!(
            ownership.owner_of_page(&PageId(1)).await.as_ref(),
            Some(&key),
            "nobody live holds it, so the reclaim takes it"
        );
        Ok(())
    }

    /// Omitting it must not disturb anything, since every call that is not a
    /// reclaim omits it.
    #[tokio::test]
    async fn omitting_the_group_id_leaves_the_existing_group_alone() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.seed_group("group-1", [101]);
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
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

        run_tab_group_work(call.clone(), Some(2)).await;

        assert_eq!(
            ownership.tab_group_ref(&key).await.as_deref(),
            Some("group-1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_first_pages_share_one_created_group() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let mut first_call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        first_call.browser_session = Some(browser);
        let second_call = first_call.clone();
        let (first, second) = tokio::join!(
            spawn_tab_group_work(first_call.clone(), Some(1)),
            spawn_tab_group_work(second_call, Some(2))
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
        let creation = spawn_tab_group_work(call.clone(), Some(1));
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

        spawn_tab_group_work(call, Some(1)).await?;

        assert_eq!(recorder.create_count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn transient_group_add_failure_keeps_the_winning_group_reference() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        let (call, _browser) = connected_call(recorder.clone()).await?;
        spawn_tab_group_work(call.clone(), Some(1)).await?;
        recorder.fail_group_add(true);
        spawn_tab_group_work(call.clone(), Some(2)).await?;
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

        spawn_tab_group_work(call.clone(), Some(1)).await?;

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
        spawn_tab_group_work(call.clone(), Some(1)).await?;
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
        spawn_tab_group_work(call.clone(), Some(1)).await?;
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
        let creation = spawn_tab_group_work(call.clone(), Some(1));
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
        spawn_tab_group_work(call.clone(), Some(1)).await?;
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
        let browser = BrowserSession::new(recorder, BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        // SQLite setup needs real time; only the dispatch timeout uses the paused clock.
        tokio::time::pause();
        let dispatch = tokio::spawn(async move {
            dispatch_tab_groups(
                call.tool_named("tab_groups")
                    .unwrap_or_else(|| unreachable!()),
                &browser,
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
