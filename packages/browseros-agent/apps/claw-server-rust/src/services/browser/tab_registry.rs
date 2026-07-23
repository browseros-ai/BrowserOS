use crate::{clock::now_epoch_ms, db::SessionTabLedger, error::AppResult};
use browseros_cdp::{CdpEvent, browser};
use browseros_core::BrowserSession;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, RwLock};
use tokio::time::Instant;
use tracing::warn;

const NO_EPOCH: u64 = u64::MAX;
// Retain destroyed tab-to-target mappings for late recorder batches that still need attribution.
const GRACE_MS: u64 = 5 * 60 * 1_000;

type TargetClaimReleaser = Arc<dyn Fn(String) -> BoxFuture<'static, AppResult<()>> + Send + Sync>;
type TabOwnerInheritor =
    Arc<dyn Fn(i64, i64, String) -> BoxFuture<'static, AppResult<()>> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TabIdentity {
    tab_id: i64,
    target_id: String,
}

impl TabIdentity {
    fn new(tab_id: i64, target_id: impl Into<String>) -> Self {
        Self {
            tab_id,
            target_id: target_id.into(),
        }
    }
}

#[derive(Default)]
struct TargetMaps {
    target_by_tab: HashMap<i64, String>,
    tab_by_target: HashMap<String, i64>,
    /// Chrome never reuses tab ids within a browser session, so live entries
    /// cannot alias these destroyed entries; lookups still prefer live.
    recently_destroyed: HashMap<i64, RecentlyDestroyed>,
}

struct RecentlyDestroyed {
    target_id: String,
    destroyed_at: Instant,
}

/// Maintains the Chrome tab id to stable CDP target id identity boundary.
pub struct TabRegistry {
    maps: RwLock<TargetMaps>,
    current_epoch: AtomicU64,
    ready_epoch: AtomicU64,
    rebuild: Mutex<()>,
    release_target_claims: TargetClaimReleaser,
    inherit_tab_owner: TabOwnerInheritor,
}

impl TabRegistry {
    #[must_use]
    pub fn new(session_tabs: Arc<SessionTabLedger>) -> Arc<Self> {
        let release_session_tabs = session_tabs.clone();
        Self::new_with_callbacks(
            Arc::new(move |target_id| {
                release_session_tabs.enqueue_release_claims_for_target(target_id);
                Box::pin(async { Ok(()) })
            }),
            Arc::new(move |opener_tab_id, tab_id, target_id| {
                session_tabs.enqueue_inherit_tab_ownership(
                    opener_tab_id,
                    tab_id,
                    target_id,
                    now_epoch_ms() - 1_000,
                );
                Box::pin(async { Ok(()) })
            }),
        )
    }

    #[cfg(test)]
    fn new_with_releaser(release_target_claims: TargetClaimReleaser) -> Arc<Self> {
        Self::new_with_callbacks(
            release_target_claims,
            Arc::new(|_, _, _| Box::pin(async { Ok(()) })),
        )
    }

    fn new_with_callbacks(
        release_target_claims: TargetClaimReleaser,
        inherit_tab_owner: TabOwnerInheritor,
    ) -> Arc<Self> {
        Arc::new(Self {
            maps: RwLock::new(TargetMaps::default()),
            current_epoch: AtomicU64::new(NO_EPOCH),
            ready_epoch: AtomicU64::new(NO_EPOCH),
            rebuild: Mutex::new(()),
            release_target_claims,
            inherit_tab_owner,
        })
    }

    /// Subscribes to target lifecycle events and seeds the map from live tabs.
    pub async fn observe_session(
        self: &Arc<Self>,
        session: Arc<BrowserSession>,
        epoch: u64,
    ) -> anyhow::Result<()> {
        self.current_epoch.store(epoch, Ordering::SeqCst);
        let mut events = session.cdp_events();
        let seed_result = self.rebuild_from_session(&session, epoch, false).await;
        let map = self.clone();
        tokio::spawn(async move {
            loop {
                if map.current_epoch.load(Ordering::SeqCst) != epoch {
                    return;
                }
                match events.recv().await {
                    Ok(event) => map.handle_event(epoch, event).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(
                            epoch,
                            skipped, "tab target event listener lagged; rebuilding"
                        );
                        // Resubscribe before the snapshot so only events concurrent with the rebuild replay over it.
                        events = session.cdp_events();
                        if let Err(error) = map.rebuild_from_session(&session, epoch, true).await {
                            warn!(epoch, error = %error, "failed to rebuild tab target map");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        seed_result
    }

    #[must_use]
    pub fn is_ready(&self, epoch: u64) -> bool {
        self.ready_epoch.load(Ordering::SeqCst) == epoch
    }

    /// Resolves a tab id, rebuilding after reconnect and using Browser.getTabInfo on a miss.
    pub async fn resolve(
        &self,
        tab_id: i64,
        session: Option<Arc<BrowserSession>>,
        epoch: u64,
    ) -> Option<String> {
        let session = session?;
        if self.ready_epoch.load(Ordering::SeqCst) != epoch
            && let Err(error) = self.rebuild_from_session(&session, epoch, false).await
        {
            warn!(epoch, error = %error, "failed to rebuild tab target map before lookup");
        }
        self.resolve_with(tab_id, |tab_id| async move {
            let value = session
                .cdp("Browser.getTabInfo", json!({ "tabId": tab_id }), None)
                .await
                .ok()?;
            let result = serde_json::from_value::<browser::GetTabInfoResult>(value).ok()?;
            Some(TabIdentity::new(result.tab.tab_id, result.tab.target_id))
        })
        .await
    }

    pub async fn tab_for_target(&self, target_id: &str) -> Option<i64> {
        self.maps.read().await.tab_by_target.get(target_id).copied()
    }

    async fn target_for_tab_cached(&self, tab_id: i64) -> Option<String> {
        let mut maps = self.maps.write().await;
        if let Some(target_id) = maps.target_by_tab.get(&tab_id) {
            return Some(target_id.clone());
        }
        prune_recently_destroyed(&mut maps, Instant::now());
        maps.recently_destroyed
            .get(&tab_id)
            .map(|entry| entry.target_id.clone())
    }

    async fn resolve_with<F, Fut>(&self, tab_id: i64, fallback: F) -> Option<String>
    where
        F: FnOnce(i64) -> Fut,
        Fut: Future<Output = Option<TabIdentity>>,
    {
        if let Some(target_id) = self.target_for_tab_cached(tab_id).await {
            return Some(target_id);
        }
        let identity = fallback(tab_id).await?;
        let target_id = identity.target_id.clone();
        self.upsert(identity).await;
        Some(target_id)
    }

    async fn rebuild_from_session(
        &self,
        session: &BrowserSession,
        epoch: u64,
        force: bool,
    ) -> anyhow::Result<()> {
        let _guard = self.rebuild.lock().await;
        if !force && self.ready_epoch.load(Ordering::SeqCst) == epoch {
            return Ok(());
        }
        session
            .cdp(
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
                None,
            )
            .await?;
        let value = session
            .cdp("Browser.getTabs", json!({ "includeHidden": true }), None)
            .await?;
        let result = serde_json::from_value::<browser::GetTabsResult>(value)?;
        if self.current_epoch.load(Ordering::SeqCst) != epoch {
            return Ok(());
        }
        self.rebuild_from_tabs(
            epoch,
            result
                .tabs
                .into_iter()
                .map(|tab| TabIdentity::new(tab.tab_id, tab.target_id))
                .collect(),
        )
        .await;
        Ok(())
    }

    async fn rebuild_from_tabs(&self, epoch: u64, tabs: Vec<TabIdentity>) {
        if self.current_epoch.load(Ordering::SeqCst) != NO_EPOCH
            && self.current_epoch.load(Ordering::SeqCst) != epoch
        {
            return;
        }
        self.current_epoch.store(epoch, Ordering::SeqCst);
        let live_targets = tabs
            .iter()
            .map(|tab| tab.target_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let stale_targets = {
            let mut maps = self.maps.write().await;
            let now = Instant::now();
            prune_recently_destroyed(&mut maps, now);
            let stale = maps
                .tab_by_target
                .iter()
                .filter(|(target_id, _)| !live_targets.contains(target_id.as_str()))
                .map(|(target_id, tab_id)| (*tab_id, target_id.clone()))
                .collect::<Vec<_>>();
            for (tab_id, target_id) in &stale {
                maps.recently_destroyed.insert(
                    *tab_id,
                    RecentlyDestroyed {
                        target_id: target_id.clone(),
                        destroyed_at: now,
                    },
                );
            }
            maps.target_by_tab.clear();
            maps.tab_by_target.clear();
            stale
                .into_iter()
                .map(|(_, target_id)| target_id)
                .collect::<Vec<_>>()
        };
        for tab in tabs {
            self.upsert(tab).await;
        }
        self.ready_epoch.store(epoch, Ordering::SeqCst);
        for target_id in stale_targets {
            self.release_claims(target_id);
        }
    }

    async fn upsert(&self, identity: TabIdentity) {
        let mut maps = self.maps.write().await;
        if let Some(previous_target) = maps
            .target_by_tab
            .insert(identity.tab_id, identity.target_id.clone())
            && previous_target != identity.target_id
        {
            maps.tab_by_target.remove(&previous_target);
        }
        if let Some(previous_tab) = maps
            .tab_by_target
            .insert(identity.target_id.clone(), identity.tab_id)
            && previous_tab != identity.tab_id
        {
            maps.target_by_tab.remove(&previous_tab);
        }
    }

    async fn handle_event(&self, epoch: u64, event: CdpEvent) {
        if self.current_epoch.load(Ordering::SeqCst) != epoch {
            return;
        }
        match event.method.as_str() {
            "Target.targetCreated" | "Target.targetInfoChanged" => {
                let Some(info) = event.params.get("targetInfo") else {
                    return;
                };
                if info.get("type").and_then(Value::as_str) != Some("page") {
                    return;
                }
                let Some(tab_id) = info.get("tabId").and_then(Value::as_i64) else {
                    return;
                };
                let Some(target_id) = info.get("targetId").and_then(Value::as_str) else {
                    return;
                };
                let opener_tab_id = if event.method == "Target.targetCreated" {
                    let opener_id = info.get("openerId").and_then(Value::as_str);
                    match opener_id {
                        Some(opener_id) => self.tab_for_target(opener_id).await,
                        None => None,
                    }
                } else {
                    None
                };
                self.upsert(TabIdentity::new(tab_id, target_id)).await;
                if let Some(opener_tab_id) = opener_tab_id
                    && opener_tab_id != tab_id
                {
                    self.inherit_owner(opener_tab_id, tab_id, target_id.to_string());
                }
            }
            "Target.targetDestroyed" => {
                if let Some(target_id) = event.params.get("targetId").and_then(Value::as_str) {
                    self.remove(target_id).await;
                }
            }
            _ => {}
        }
    }

    async fn remove(&self, target_id: &str) {
        let mut maps = self.maps.write().await;
        if let Some(tab_id) = maps.tab_by_target.remove(target_id) {
            maps.target_by_tab.remove(&tab_id);
            let now = Instant::now();
            prune_recently_destroyed(&mut maps, now);
            maps.recently_destroyed.insert(
                tab_id,
                RecentlyDestroyed {
                    target_id: target_id.to_string(),
                    destroyed_at: now,
                },
            );
        }
        drop(maps);
        self.release_claims(target_id.to_string());
    }

    fn release_claims(&self, target_id: String) {
        let release = self.release_target_claims.clone();
        tokio::spawn(async move {
            if let Err(error) = release(target_id.clone()).await {
                warn!(target_id, error = %error, "failed to release claims for destroyed target");
            }
        });
    }

    fn inherit_owner(&self, opener_tab_id: i64, tab_id: i64, target_id: String) {
        let inherit = self.inherit_tab_owner.clone();
        tokio::spawn(async move {
            if let Err(error) = inherit(opener_tab_id, tab_id, target_id.clone()).await {
                warn!(
                    opener_tab_id,
                    tab_id,
                    target_id,
                    error = %error,
                    "failed to inherit popup tab ownership"
                );
            }
        });
    }
}

fn prune_recently_destroyed(maps: &mut TargetMaps, now: Instant) {
    maps.recently_destroyed.retain(|_, entry| {
        now.duration_since(entry.destroyed_at) < Duration::from_millis(GRACE_MS)
    });
}

#[cfg(test)]
mod tests {
    use super::{TabIdentity, TabRegistry};
    use browseros_cdp::{CdpError, CdpEvent, SessionId};
    use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::sync::broadcast;

    struct TabListConnection {
        events: broadcast::Sender<CdpEvent>,
        list_calls: AtomicUsize,
    }

    impl TabListConnection {
        fn new() -> Arc<Self> {
            let (events, _) = broadcast::channel(8);
            Arc::new(Self {
                events,
                list_calls: AtomicUsize::new(0),
            })
        }
    }

    impl CdpConnection for TabListConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "Browser.getTabs" => {
                        self.list_calls.fetch_add(1, Ordering::SeqCst);
                        Ok(json!({
                            "tabs": [{
                                "tabId": 11,
                                "targetId": "target-a",
                                "url": "https://example.com",
                                "title": "Example",
                                "isActive": true,
                                "isLoading": false,
                                "loadProgress": 1.0,
                                "isPinned": false,
                                "isHidden": false,
                                "windowId": 1,
                                "index": 0
                            }]
                        }))
                    }
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
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

    fn map_with_releases(releases: Arc<tokio::sync::Mutex<Vec<String>>>) -> Arc<TabRegistry> {
        TabRegistry::new_with_releaser(Arc::new(move |target_id| {
            let releases = releases.clone();
            Box::pin(async move {
                releases.lock().await.push(target_id);
                Ok(())
            }) as BoxFuture<'static, crate::error::AppResult<()>>
        }))
    }

    #[tokio::test]
    async fn rebuilds_and_maintains_both_lookup_directions() {
        let map = map_with_releases(Arc::default());
        map.rebuild_from_tabs(
            1,
            vec![
                TabIdentity::new(11, "target-a"),
                TabIdentity::new(22, "target-b"),
            ],
        )
        .await;

        assert_eq!(
            map.target_for_tab_cached(11).await.as_deref(),
            Some("target-a")
        );
        assert_eq!(map.tab_for_target("target-b").await, Some(22));
    }

    #[tokio::test]
    async fn resolves_a_miss_once_and_caches_it() {
        let map = map_with_releases(Arc::default());
        let calls = AtomicUsize::new(0);

        let first = map
            .resolve_with(33, |_| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Some(TabIdentity::new(33, "target-c"))
            })
            .await;
        let second = map
            .resolve_with(33, |_| async {
                calls.fetch_add(1, Ordering::SeqCst);
                None
            })
            .await;

        assert_eq!(first.as_deref(), Some("target-c"));
        assert_eq!(second.as_deref(), Some("target-c"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn target_events_upsert_only_page_targets_with_tab_ids() {
        let map = map_with_releases(Arc::default());
        map.rebuild_from_tabs(1, Vec::new()).await;
        map.handle_event(
            1,
            CdpEvent {
                method: "Target.targetCreated".to_string(),
                params: json!({"targetInfo": {"targetId": "target-d", "type": "page"}}),
                session_id: None,
            },
        )
        .await;
        assert_eq!(map.tab_for_target("target-d").await, None);

        map.handle_event(
            1,
            CdpEvent {
                method: "Target.targetInfoChanged".to_string(),
                params: json!({"targetInfo": {"targetId": "target-d", "type": "page", "tabId": 44}}),
                session_id: None,
            },
        )
        .await;

        assert_eq!(
            map.target_for_tab_cached(44).await.as_deref(),
            Some("target-d")
        );
    }

    #[tokio::test]
    async fn target_created_inherits_the_live_opener_tab_owner() {
        let inherited = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let captured = inherited.clone();
        let map = TabRegistry::new_with_callbacks(
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Arc::new(move |opener_tab_id, tab_id, target_id| {
                let captured = captured.clone();
                Box::pin(async move {
                    captured
                        .lock()
                        .await
                        .push((opener_tab_id, tab_id, target_id));
                    Ok(())
                })
            }),
        );
        map.rebuild_from_tabs(1, vec![TabIdentity::new(11, "target-opener")])
            .await;
        map.handle_event(
            1,
            CdpEvent {
                method: "Target.targetCreated".to_string(),
                params: json!({
                    "targetInfo": {
                        "targetId": "target-popup",
                        "type": "page",
                        "tabId": 22,
                        "openerId": "target-opener"
                    }
                }),
                session_id: None,
            },
        )
        .await;
        for _ in 0..100 {
            if !inherited.lock().await.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            *inherited.lock().await,
            vec![(11, 22, "target-popup".to_string())]
        );
    }

    #[tokio::test]
    async fn target_destroyed_removes_the_mapping_and_releases_claims() {
        let releases: Arc<tokio::sync::Mutex<Vec<String>>> = Arc::default();
        let map = map_with_releases(releases.clone());
        map.rebuild_from_tabs(1, vec![TabIdentity::new(55, "target-e")])
            .await;

        map.handle_event(
            1,
            CdpEvent {
                method: "Target.targetDestroyed".to_string(),
                params: json!({"targetId": "target-e"}),
                session_id: None,
            },
        )
        .await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !releases.lock().await.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|error| panic!("claim release timed out: {error}"));

        assert_eq!(map.tab_for_target("target-e").await, None);
        assert_eq!(*releases.lock().await, vec!["target-e"]);
    }

    #[tokio::test(start_paused = true)]
    async fn resolves_a_recently_destroyed_tab_until_the_grace_expires() {
        let map = map_with_releases(Arc::default());
        map.rebuild_from_tabs(1, vec![TabIdentity::new(55, "target-e")])
            .await;
        map.remove("target-e").await;
        let calls = AtomicUsize::new(0);

        let during_grace = map
            .resolve_with(55, |_| async {
                calls.fetch_add(1, Ordering::SeqCst);
                None
            })
            .await;
        tokio::time::advance(Duration::from_millis(super::GRACE_MS)).await;
        let after_grace = map
            .resolve_with(55, |_| async {
                calls.fetch_add(1, Ordering::SeqCst);
                None
            })
            .await;

        assert_eq!(during_grace.as_deref(), Some("target-e"));
        assert_eq!(after_grace, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn forced_rebuild_refreshes_an_already_ready_epoch() -> anyhow::Result<()> {
        let connection = TabListConnection::new();
        let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
        let map = map_with_releases(Arc::default());
        map.observe_session(session.clone(), 1).await?;
        assert!(map.is_ready(1));
        assert_eq!(connection.list_calls.load(Ordering::SeqCst), 1);

        map.rebuild_from_session(&session, 1, false).await?;
        assert_eq!(connection.list_calls.load(Ordering::SeqCst), 1);
        map.rebuild_from_session(&session, 1, true).await?;
        assert_eq!(connection.list_calls.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
