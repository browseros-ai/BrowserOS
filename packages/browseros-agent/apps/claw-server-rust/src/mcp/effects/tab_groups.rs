use crate::{
    domain::{AgentKey, AgentPageOwnership, color_for_slug},
    mcp::{
        dispatch::{ToolCall, ToolEffect, ToolEffectContext, result_page_id},
        naming::desired_group_title,
        timeouts::TAB_GROUP_OPERATION,
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
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, OnceLock, Weak},
};
use tokio::{sync::Mutex, task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;
use tracing::warn;

type GroupCreateLock = Mutex<()>;

static INFLIGHT_CREATES: OnceLock<Mutex<HashMap<AgentKey, Weak<GroupCreateLock>>>> =
    OnceLock::new();

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
        drop(spawn_tab_group_work(context.call.clone(), page_id));
        Ok(None)
    })
}

fn spawn_tab_group_work(call: ToolCall, page_id: Option<u32>) -> JoinHandle<()> {
    tokio::spawn(run_tab_group_work(call, page_id))
}

async fn run_tab_group_work(call: ToolCall, page_id: Option<u32>) {
    let (Some(identity), Some(browser), Some(tab_groups)) = (
        call.identity.as_ref(),
        call.browser_session.as_ref(),
        call.tool_named("tab_groups"),
    ) else {
        return;
    };
    let ownership = call.state.sessions.ownership();
    let expand = expand_agent_tab_group(
        tab_groups,
        browser,
        &ownership,
        &identity.ownership_key,
        identity.session.child_token(),
        call.output_files.clone(),
    );
    let Some(page_id) = page_id else {
        expand.await;
        return;
    };
    if let Some(default_group_id) = &call.default_tab_group_id {
        let page_group_id = browser
            .pages
            .get_info(PageId(page_id))
            .await
            .and_then(|page| page.group_id);
        if page_group_id.as_ref() == Some(default_group_id) {
            expand.await;
            return;
        }
        ownership
            .set_tab_group_ref(identity.ownership_key.clone(), None)
            .await;
    }
    tokio::join!(
        expand,
        ensure_agent_tab_group(&call, tab_groups, browser, &ownership, page_id)
    );
}

/// Serializes durable group creation and joins concurrent pages to the winning group.
async fn ensure_agent_tab_group(
    call: &ToolCall,
    tab_groups: &ToolDef,
    browser: &Arc<BrowserSession>,
    ownership: &Arc<AgentPageOwnership>,
    page_id: u32,
) {
    let Some(identity) = call.identity.as_ref() else {
        return;
    };
    let create_lock = group_create_lock(&identity.ownership_key).await;
    let _guard = create_lock.lock().await;
    if let Some(group_id) = ownership.tab_group_ref(&identity.ownership_key).await {
        if let Err(reason) = dispatch_tab_groups(
            tab_groups,
            browser,
            identity.session.child_token(),
            call.output_files.clone(),
            json!({ "action": "create", "groupId": group_id, "pages": [page_id] }),
        )
        .await
        {
            ownership
                .set_tab_group(identity.ownership_key.clone(), None, None)
                .await;
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
    let group_result = match dispatch_tab_groups(
        tab_groups,
        browser,
        identity.session.child_token(),
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
        .set_tab_group(
            identity.ownership_key.clone(),
            Some(group_id.clone()),
            Some(color),
        )
        .await;
    if let Err(reason) = dispatch_tab_groups(
        tab_groups,
        browser,
        identity.session.child_token(),
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
    if desired_title != creation_title
        && let Err(reason) = dispatch_tab_groups(
            tab_groups,
            browser,
            identity.session.child_token(),
            call.output_files.clone(),
            json!({ "action": "update", "groupId": group_id, "title": desired_title }),
        )
        .await
    {
        warn!(
            dispatch_id = %call.dispatch_id,
            error = %reason,
            "tab group late title apply failed"
        );
    }
}

async fn group_create_lock(key: &AgentKey) -> Arc<GroupCreateLock> {
    let locks = INFLIGHT_CREATES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks.lock().await;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key.clone(), Arc::downgrade(&lock));
    lock
}

/// Collapses the durable group after its final live session ends.
pub async fn collapse_agent_tab_group(
    browser: &Arc<BrowserSession>,
    ownership: &Arc<AgentPageOwnership>,
    key: &AgentKey,
) {
    if ownership.tab_group_collapsed(key).await {
        return;
    }
    let Some(group_id) = ownership.tab_group_ref(key).await else {
        return;
    };
    match dispatch_tab_groups(
        cached_tab_groups_tool(),
        browser,
        CancellationToken::new(),
        browseros_mcp::output_file::create_browser_output_file_access(),
        json!({ "action": "update", "groupId": group_id, "collapsed": true }),
    )
    .await
    {
        Ok(_) => {
            ownership.set_tab_group_collapsed(key.clone(), true).await;
        }
        Err(reason) => warn!(key = %key, error = %reason, "agent tab group collapse failed"),
    }
}

async fn expand_agent_tab_group(
    tab_groups: &ToolDef,
    browser: &Arc<BrowserSession>,
    ownership: &Arc<AgentPageOwnership>,
    key: &AgentKey,
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
            ownership.set_tab_group_collapsed(key.clone(), false).await;
        }
        Err(reason) => warn!(key = %key, error = %reason, "agent tab group expand failed"),
    }
}

/// Applies a completed session name to the durable group when it exists.
pub async fn retitle_agent_tab_group(
    tab_groups: &ToolDef,
    browser: &Arc<BrowserSession>,
    ownership: &Arc<AgentPageOwnership>,
    key: &AgentKey,
    title: &str,
    cancel: CancellationToken,
) {
    let Some(group_id) = ownership.tab_group_ref(key).await else {
        return;
    };
    if let Err(reason) = dispatch_tab_groups(
        tab_groups,
        browser,
        cancel,
        browseros_mcp::output_file::create_browser_output_file_access(),
        json!({ "action": "update", "groupId": group_id, "title": title }),
    )
    .await
    {
        warn!(key = %key, error = %reason, "session name tab group retitle failed");
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
        block_create: AtomicBool,
        create_release: Notify,
    }

    impl GroupDispatchRecorder {
        fn new() -> Self {
            let (sender, _) = broadcast::channel(8);
            Self {
                sender,
                calls: StdMutex::new(Vec::new()),
                members: StdMutex::new(HashMap::new()),
                block_create: AtomicBool::new(false),
                create_release: Notify::new(),
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
                    "title": params.get("title").and_then(Value::as_str).unwrap_or("codex"),
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

        fn group_members(&self, group_id: &str) -> BTreeSet<i64> {
            self.members
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(group_id)
                .cloned()
                .unwrap_or_default()
        }

        fn block_group_creation(&self) {
            self.block_create.store(true, Ordering::SeqCst);
        }

        fn release_group_creation(&self) {
            self.create_release.notify_one();
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
                        let group_id = params
                            .get("groupId")
                            .and_then(Value::as_str)
                            .unwrap_or("group-1");
                        Ok(self.group_result(group_id, &params))
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

    #[test]
    fn first_text_returns_empty_when_result_has_no_text() {
        let result = ToolResult::image("aGVsbG8=", "image/jpeg", json!({}));
        assert!(first_text(&result).is_empty());
    }

    #[test]
    fn teardown_fallback_tool_definition_is_cached() {
        assert!(std::ptr::eq(
            cached_tab_groups_tool(),
            cached_tab_groups_tool()
        ));
    }

    #[tokio::test]
    async fn effect_returns_before_group_creation_finishes() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.block_group_creation();
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let mut call =
            crate::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
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

    #[tokio::test]
    async fn concurrent_first_pages_share_one_created_group() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        let browser = BrowserSession::new(recorder.clone(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let mut first_call =
            crate::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
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

    #[tokio::test(start_paused = true)]
    async fn each_group_dispatch_uses_the_shared_timeout() -> anyhow::Result<()> {
        let recorder = Arc::new(GroupDispatchRecorder::new());
        recorder.block_group_creation();
        let browser = BrowserSession::new(recorder, BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 2);
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
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
