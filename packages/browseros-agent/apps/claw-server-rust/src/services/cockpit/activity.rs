use browseros_core::{BrowserSession, PageId, TargetId, pages::PageInfo};
use std::{
    cmp::Reverse,
    collections::{HashMap, VecDeque},
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const ACTIVE_WINDOW: Duration = Duration::from_secs(30);
const RECENT_TOOLS_CAP: usize = 8;
const USE_SYSTEM_TIME: i64 = i64::MIN;

#[derive(Debug, Clone)]
pub struct ToolEvent {
    pub name: String,
    /// Unix-epoch milliseconds.
    pub at: i64,
}

#[derive(Debug, Clone)]
pub struct TabActivityRecord {
    pub target_id: String,
    /// Browser tab id joining recorder batches to CDP-side state.
    pub tab_id: i64,
    pub page_id: u32,
    pub url: String,
    pub title: String,
    /// MCP session currently claiming the tab; recording ingest rejects stale claims.
    pub session_id: String,
    pub agent_id: String,
    pub slug: String,
    /// Unix-epoch milliseconds.
    pub first_tool_at: i64,
    /// Unix-epoch milliseconds.
    pub last_tool_at: i64,
    pub last_tool_name: String,
    pub tool_count: usize,
    pub recent_tools: Vec<ToolEvent>,
    pub status: &'static str,
}

#[derive(Debug, Clone)]
struct RawRecord {
    /// Replaced on every mutation so reconciliation can detect both updates and remove/reinsert ABA races.
    version: Arc<()>,
    target_id: String,
    tab_id: i64,
    page_id: u32,
    session_id: String,
    agent_id: String,
    slug: String,
    first_tool_at: i64,
    last_tool_at: i64,
    last_tool_name: String,
    tool_count: usize,
    recent_tools: VecDeque<ToolEvent>,
}

impl RawRecord {
    fn new(target_id: String, input: RecordToolInput, now: i64) -> Self {
        let mut recent_tools = VecDeque::new();
        recent_tools.push_back(ToolEvent {
            name: input.tool_name.clone(),
            at: now,
        });
        Self {
            version: Arc::new(()),
            target_id,
            tab_id: input.tab_id,
            page_id: input.page_id,
            session_id: input.session_id,
            agent_id: input.agent_id,
            slug: input.slug,
            first_tool_at: now,
            last_tool_at: now,
            last_tool_name: input.tool_name,
            tool_count: 1,
            recent_tools,
        }
    }
}

enum LivePageState {
    Live {
        target_id: String,
        tab_id: i64,
        url: String,
        title: String,
    },
    Missing,
    Unavailable,
}

#[derive(Clone)]
pub struct TabActivityService {
    records: Arc<Mutex<HashMap<String, RawRecord>>>,
    now_override_ms: Arc<AtomicI64>,
}

impl Default for TabActivityService {
    fn default() -> Self {
        Self {
            records: Arc::new(Mutex::new(HashMap::new())),
            now_override_ms: Arc::new(AtomicI64::new(USE_SYSTEM_TIME)),
        }
    }
}

pub struct RecordToolInput {
    pub target_id: TargetId,
    pub tab_id: i64,
    pub page_id: u32,
    pub session_id: String,
    pub agent_id: String,
    pub slug: String,
    pub tool_name: String,
}

impl TabActivityService {
    pub async fn record_tool(&self, input: RecordToolInput) {
        let now = self.now_ms();
        let target_key = input.target_id.as_str().to_string();
        let mut records = self.records.lock().await;
        if let Some(existing) = records.get_mut(&target_key)
            && existing.session_id == input.session_id
        {
            existing.version = Arc::new(());
            existing.tab_id = input.tab_id;
            existing.page_id = input.page_id;
            existing.agent_id = input.agent_id;
            existing.slug = input.slug;
            existing.last_tool_at = now;
            existing.last_tool_name = input.tool_name.clone();
            existing.tool_count += 1;
            existing.recent_tools.push_back(ToolEvent {
                name: input.tool_name,
                at: now,
            });
            while existing.recent_tools.len() > RECENT_TOOLS_CAP {
                existing.recent_tools.pop_front();
            }
            return;
        }
        records.insert(target_key.clone(), RawRecord::new(target_key, input, now));
    }

    pub async fn remove_incarnation(&self, page_id: u32, target_id: &str) -> bool {
        let mut records = self.records.lock().await;
        if records
            .get(target_id)
            .is_some_and(|record| record.page_id == page_id)
        {
            records.remove(target_id);
            return true;
        }
        false
    }

    /// Reconciles stored trails against live page and target identity before exposing them.
    pub async fn snapshot(&self, session: Option<&BrowserSession>) -> Vec<TabActivityRecord> {
        let Some(session) = session else {
            return Vec::new();
        };
        if !session.is_connected() {
            return Vec::new();
        }
        self.snapshot_with(|page_id| async move {
            if !session.is_connected() {
                return LivePageState::Unavailable;
            }
            match session.pages.refresh(page_id).await {
                Ok(Some(info)) => LivePageState::Live {
                    target_id: info.target_id.into_inner(),
                    tab_id: info.tab_id.0,
                    url: info.url,
                    title: info.title,
                },
                Ok(None) => LivePageState::Missing,
                Err(_) => LivePageState::Unavailable,
            }
        })
        .await
    }

    /// Reconciles activity against one authoritative browser-page snapshot.
    pub async fn reconcile_pages(&self, pages: &[PageInfo]) -> Vec<TabActivityRecord> {
        let pages = pages
            .iter()
            .map(|page| (page.page_id.0, page.clone()))
            .collect::<HashMap<_, _>>();
        self.snapshot_with(|page_id| {
            let page = pages.get(&page_id.0).cloned();
            async move {
                match page {
                    Some(page) => LivePageState::Live {
                        target_id: page.target_id.into_inner(),
                        tab_id: page.tab_id.0,
                        url: page.url,
                        title: page.title,
                    },
                    None => LivePageState::Missing,
                }
            }
        })
        .await
    }

    async fn snapshot_with<F, Fut>(&self, resolve: F) -> Vec<TabActivityRecord>
    where
        F: Fn(PageId) -> Fut,
        Fut: Future<Output = LivePageState>,
    {
        let now = self.now_ms();
        let target_ids = self
            .records
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let mut rows = Vec::with_capacity(target_ids.len());
        for target_id in target_ids {
            loop {
                let candidate = {
                    let records = self.records.lock().await;
                    records.get(&target_id).cloned()
                };
                let Some(candidate) = candidate else {
                    break;
                };
                let live = resolve(PageId(candidate.page_id)).await;
                let mut records = self.records.lock().await;
                let Some(current) = records.get(&target_id) else {
                    break;
                };
                if !Arc::ptr_eq(&current.version, &candidate.version) {
                    drop(records);
                    continue;
                }
                match live {
                    LivePageState::Live {
                        target_id: live_target_id,
                        tab_id: live_tab_id,
                        url,
                        title,
                    } if live_target_id == candidate.target_id
                        && live_tab_id == candidate.tab_id =>
                    {
                        rows.push(TabActivityRecord {
                            target_id: candidate.target_id,
                            tab_id: candidate.tab_id,
                            page_id: candidate.page_id,
                            url,
                            title,
                            session_id: candidate.session_id,
                            agent_id: candidate.agent_id,
                            slug: candidate.slug,
                            first_tool_at: candidate.first_tool_at,
                            last_tool_at: candidate.last_tool_at,
                            last_tool_name: candidate.last_tool_name,
                            tool_count: candidate.tool_count,
                            recent_tools: candidate.recent_tools.into_iter().collect(),
                            status: if now.saturating_sub(candidate.last_tool_at)
                                < i64::try_from(ACTIVE_WINDOW.as_millis()).unwrap_or(30_000)
                            {
                                "active"
                            } else {
                                "idle"
                            },
                        });
                    }
                    LivePageState::Live { .. } | LivePageState::Missing => {
                        records.remove(&target_id);
                    }
                    // An unavailable browser refresh is not confirmed page death; retain the record
                    // so its activity can recover after reconnect.
                    LivePageState::Unavailable => {}
                }
                break;
            }
        }
        rows.sort_by_key(|row| Reverse(row.last_tool_at));
        rows
    }

    fn now_ms(&self) -> i64 {
        let override_ms = self.now_override_ms.load(Ordering::Relaxed);
        if override_ms == USE_SYSTEM_TIME {
            now_ms()
        } else {
            override_ms
        }
    }

    #[doc(hidden)]
    pub fn set_now_for_testing(&self, now_ms: i64) {
        self.now_override_ms.store(now_ms, Ordering::Relaxed);
    }
}

fn now_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{LivePageState, RecordToolInput, TabActivityService};
    use browseros_cdp::{CdpError, CdpEvent};
    use browseros_core::{
        BrowserSession, BrowserSessionHooks, CdpConnection, PageId, SessionId as ProtocolSessionId,
        TargetId,
    };
    use futures_util::future::BoxFuture;
    use serde_json::Value;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::sync::{Notify, broadcast};

    struct DisconnectedConnection {
        events: broadcast::Sender<CdpEvent>,
    }

    impl DisconnectedConnection {
        fn new() -> Arc<Self> {
            let (events, _) = broadcast::channel(1);
            Arc::new(Self { events })
        }
    }

    impl CdpConnection for DisconnectedConnection {
        fn send<'a>(
            &'a self,
            _method: &'a str,
            _params: Value,
            _session: Option<&'a ProtocolSessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async { Err(CdpError::NotConnected) })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a ProtocolSessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Err(CdpError::NotConnected) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.events.subscribe()
        }

        fn is_connected(&self) -> bool {
            false
        }

        fn connection_epoch(&self) -> u64 {
            0
        }
    }

    struct RebindingConnection {
        events: broadcast::Sender<CdpEvent>,
        list_calls: AtomicUsize,
        refresh_started: Notify,
        release_refresh: Notify,
    }

    impl RebindingConnection {
        fn new() -> Arc<Self> {
            let (events, _) = broadcast::channel(1);
            Arc::new(Self {
                events,
                list_calls: AtomicUsize::new(0),
                refresh_started: Notify::new(),
                release_refresh: Notify::new(),
            })
        }

        fn tab(target_id: &str) -> Value {
            serde_json::json!({
                "tabId": 101,
                "targetId": target_id,
                "url": format!("https://example.com/{target_id}"),
                "title": target_id,
                "isActive": true,
                "isLoading": false,
                "loadProgress": 1.0,
                "isPinned": false,
                "isHidden": false,
                "windowId": 1,
                "index": 0
            })
        }
    }

    impl CdpConnection for RebindingConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            _params: Value,
            _session: Option<&'a ProtocolSessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "Browser.getTabs" => {
                        let target_id = if self.list_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            "target-old"
                        } else {
                            "target-new"
                        };
                        Ok(serde_json::json!({ "tabs": [Self::tab(target_id)] }))
                    }
                    "Browser.getTabInfo" => {
                        self.refresh_started.notify_one();
                        self.release_refresh.notified().await;
                        Ok(serde_json::json!({ "tab": Self::tab("target-old") }))
                    }
                    _ => Ok(serde_json::json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a ProtocolSessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.events.subscribe()
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    async fn record(
        service: &TabActivityService,
        target_id: &str,
        page_id: u32,
        session_id: &str,
        tool_name: &str,
    ) {
        service
            .record_tool(RecordToolInput {
                target_id: TargetId::from(target_id.to_string()),
                tab_id: i64::from(page_id) + 100,
                page_id,
                session_id: session_id.to_string(),
                agent_id: session_id.to_string(),
                slug: "codex".to_string(),
                tool_name: tool_name.to_string(),
            })
            .await;
    }

    fn live(target_id: &str, tab_id: i64, url: &str, title: &str) -> LivePageState {
        LivePageState::Live {
            target_id: target_id.to_string(),
            tab_id,
            url: url.to_string(),
            title: title.to_string(),
        }
    }

    #[tokio::test]
    async fn latest_session_and_tab_replace_the_target_association() {
        let service = TabActivityService::default();
        service.set_now_for_testing(100);
        for (session_id, tab_id, tool_name) in [
            ("session-1", 101, "navigate"),
            ("session-2", 202, "snapshot"),
        ] {
            service
                .record_tool(RecordToolInput {
                    target_id: TargetId::from("target-1".to_string()),
                    tab_id,
                    page_id: 7,
                    session_id: session_id.to_string(),
                    agent_id: session_id.to_string(),
                    slug: "codex".to_string(),
                    tool_name: tool_name.to_string(),
                })
                .await;
            service.set_now_for_testing(200);
        }

        let records = service
            .snapshot_with(|_| async {
                live("target-1", 202, "https://example.com/current", "Current")
            })
            .await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session_id, "session-2");
        assert_eq!(records[0].tab_id, 202);
        assert_eq!(records[0].first_tool_at, 200);
        assert_eq!(records[0].last_tool_at, 200);
        assert_eq!(records[0].last_tool_name, "snapshot");
        assert_eq!(records[0].tool_count, 1);
        assert_eq!(records[0].recent_tools.len(), 1);
        assert_eq!(records[0].recent_tools[0].name, "snapshot");
        assert_eq!(records[0].url, "https://example.com/current");
        assert_eq!(records[0].title, "Current");
    }

    #[tokio::test]
    async fn same_session_target_activity_accumulates() {
        let service = TabActivityService::default();
        service.set_now_for_testing(100);
        record(&service, "target-1", 1, "session-1", "navigate").await;
        service.set_now_for_testing(200);
        record(&service, "target-1", 1, "session-1", "snapshot").await;

        let records = service
            .snapshot_with(|_| async {
                live("target-1", 101, "https://example.com/current", "Current")
            })
            .await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].first_tool_at, 100);
        assert_eq!(records[0].last_tool_at, 200);
        assert_eq!(records[0].tool_count, 2);
        assert_eq!(
            records[0]
                .recent_tools
                .iter()
                .map(|event| event.name.as_str())
                .collect::<Vec<_>>(),
            vec!["navigate", "snapshot"]
        );
    }

    #[tokio::test]
    async fn missing_page_is_evicted() {
        let service = TabActivityService::default();
        record(&service, "target-1", 7, "session-1", "navigate").await;

        assert!(
            service
                .snapshot_with(|_| async { LivePageState::Missing })
                .await
                .is_empty()
        );
        assert!(
            service
                .snapshot_with(|_| async {
                    live("target-1", 107, "https://example.com", "Example")
                })
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn reused_page_id_with_another_target_is_evicted() {
        let service = TabActivityService::default();
        record(&service, "target-old", 1, "session-1", "snapshot").await;

        let records = service
            .snapshot_with(|_| async {
                live("target-new", 101, "https://example.com/new", "New tab")
            })
            .await;
        assert!(records.is_empty());
        assert!(
            service
                .snapshot_with(|_| async {
                    live("target-old", 101, "https://example.com/old", "Old tab")
                })
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn session_absence_and_refresh_failure_hide_but_preserve_activity() {
        let service = TabActivityService::default();
        record(&service, "target-1", 1, "session-1", "snapshot").await;

        assert!(service.snapshot(None).await.is_empty());
        assert!(
            service
                .snapshot_with(|_| async { LivePageState::Unavailable })
                .await
                .is_empty()
        );

        let records = service
            .snapshot_with(|_| async {
                live(
                    "target-1",
                    101,
                    "https://example.com/reconnected",
                    "Reconnected",
                )
            })
            .await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].url, "https://example.com/reconnected");
    }

    #[tokio::test]
    async fn disconnected_session_returns_immediately_without_pruning() {
        let service = TabActivityService::default();
        record(&service, "target-1", 1, "session-1", "snapshot").await;
        let session = BrowserSession::new(
            DisconnectedConnection::new(),
            BrowserSessionHooks::default(),
        );

        let Ok(records) = tokio::time::timeout(
            Duration::from_millis(100),
            service.snapshot(Some(session.as_ref())),
        )
        .await
        else {
            panic!("disconnected snapshot waited for PageManager reconnect");
        };
        assert!(records.is_empty());

        let records = service
            .snapshot_with(|_| async {
                live(
                    "target-1",
                    101,
                    "https://example.com/reconnected",
                    "Reconnected",
                )
            })
            .await;
        assert_eq!(records.len(), 1);
    }

    #[tokio::test]
    async fn concurrent_page_rebind_wins_over_an_older_refresh() -> anyhow::Result<()> {
        let service = TabActivityService::default();
        record(&service, "target-old", 1, "session-1", "snapshot").await;
        let connection = RebindingConnection::new();
        let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
        let initial = session.pages.list().await?;
        assert_eq!(initial[0].target_id.as_str(), "target-old");

        let snapshot = {
            let service = service.clone();
            let session = session.clone();
            tokio::spawn(async move { service.snapshot(Some(session.as_ref())).await })
        };
        connection.refresh_started.notified().await;
        let rebound = session.pages.list().await?;
        assert_eq!(rebound[0].target_id.as_str(), "target-new");
        connection.release_refresh.notify_one();

        let rows = snapshot.await?;
        assert!(rows.is_empty());
        let current = session
            .pages
            .get_info(PageId(1))
            .await
            .ok_or_else(|| anyhow::anyhow!("rebound page missing"))?;
        assert_eq!(current.target_id.as_str(), "target-new");
        Ok(())
    }

    #[tokio::test]
    async fn update_during_stale_validation_is_retried_instead_of_pruned() {
        let service = TabActivityService::default();
        record(&service, "target-1", 1, "session-1", "navigate").await;
        let validation_started = Arc::new(Notify::new());
        let release_validation = Arc::new(Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));

        let snapshot = tokio::spawn({
            let service = service.clone();
            let validation_started = validation_started.clone();
            let release_validation = release_validation.clone();
            let calls = calls.clone();
            async move {
                service
                    .snapshot_with(move |_| {
                        let validation_started = validation_started.clone();
                        let release_validation = release_validation.clone();
                        let calls = calls.clone();
                        async move {
                            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                                validation_started.notify_one();
                                release_validation.notified().await;
                                LivePageState::Missing
                            } else {
                                live("target-1", 101, "https://example.com/updated", "Updated")
                            }
                        }
                    })
                    .await
            }
        });

        validation_started.notified().await;
        tokio::time::sleep(Duration::from_millis(1)).await;
        record(&service, "target-1", 1, "session-2", "snapshot").await;
        release_validation.notify_one();

        let records = snapshot
            .await
            .unwrap_or_else(|error| panic!("snapshot failed: {error}"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session_id, "session-2");
        assert_eq!(records[0].last_tool_name, "snapshot");
        assert_eq!(records[0].tool_count, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
