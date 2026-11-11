//! Orders the host's page-cache observations and correlates them with the page-event log.
//! One lock spans all reads in an observation. The current target must also be absent from
//! the log's close records before a cached tab can be reported live. Delivered claims and
//! remembered dead bindings remain decisive after ring eviction, until core detaches or rebinds.

use futures_util::future::BoxFuture;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use browseros_core::pages::PageInfo;
use browseros_core::{TabId, TargetId};
use serde::Serialize;
use tokio::sync::Mutex;
#[cfg(test)]
use tokio::sync::MutexGuard;

use crate::browser::{BrowserLink, TabRegistry};

/// The event evidence needed to reconcile a cache read with the independent target watcher.
pub(crate) trait EventLog: Send + Sync {
    /// Drain already-received lifecycle facts before capturing a foreground observation.
    fn synchronize(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    /// Newest event recorded before the batch begins reading core.
    fn head(&self) -> u64;
    /// Dedicated workers for a live page; absent for hosts without worker observation.
    fn workers(&self, _target: &TargetId) -> Option<Vec<WorkerFact>> {
        None
    }
    /// Retained close evidence for the exact current target from the cache read.
    fn closed(&self, target: &TargetId) -> Option<u64>;
    /// The newest retained navigation of the target's main frame, including one the page made
    /// itself, which core's cache does not follow. `None` when the log has none.
    fn navigated(&self, _target: &TargetId) -> Option<u64> {
        None
    }
}

/// A worker identity in the host protocol, independent of the browser adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct WorkerFact {
    pub(crate) id: String,
    pub(crate) url: String,
}

/// Freshness of a batch of page facts. Neither counter is a page identity.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Observation {
    pub(crate) seq: u64,
    pub(crate) events: u64,
    /// Whether the operation's payload describes the binding read by this observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) payload_current: Option<bool>,
    /// Close records applied by this observation, consumed in event order by the prelude.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) closes: Vec<ApprovedClose>,
}

/// One delivered close selected for the current binding, named by its existing event position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ApprovedClose {
    pub(crate) tab: TabId,
    pub(crate) seq: u64,
}

/// A delivered target-bound close, retained even after the event leaves the ring.
#[derive(Clone, Debug)]
pub(crate) struct Claim {
    pub(crate) target: TargetId,
    pub(crate) seq: u64,
    pub(crate) released: bool,
}

/// The operation's admitted payload source, private to the host.
pub(crate) struct ActedTarget {
    pub(crate) tab: TabId,
    pub(crate) target: TargetId,
}

/// Cache entries in input order, with the shared observation that ordered their reads.
pub(crate) struct ObservedPages {
    pub(crate) observed: Observation,
    pub(crate) infos: Vec<Option<PageInfo>>,
    pub(crate) workers: Vec<Option<Vec<WorkerFact>>>,
    /// Closed only by records withheld from this reply. Internal evidence still updates.
    pub(crate) withheld: Vec<TabId>,
}

/// Serializes fact reads across all operations and run initializations in this process.
pub(crate) struct Observer {
    serial: Mutex<u64>,
    log: Arc<dyn EventLog>,
}

impl Observer {
    pub(crate) fn new(log: Arc<dyn EventLog>) -> Self {
        Self {
            serial: Mutex::new(0),
            log,
        }
    }

    /// The event log's head, before a refresh reads core.
    pub(crate) fn head(&self) -> u64 {
        self.log.head()
    }

    /// The newest retained main-frame navigation of `target`.
    pub(crate) fn navigated(&self, target: &TargetId) -> Option<u64> {
        self.log.navigated(target)
    }

    /// Groups all delivered closes by tab. Multiple target-bound subscriptions can coexist
    /// during rebind, and every target remains evidence regardless of reporting order.
    pub(crate) async fn observe(
        &self,
        browser: &BrowserLink,
        registry: &TabRegistry,
        tabs: &[TabId],
        claims: &HashMap<TabId, Vec<Claim>>,
        acted: Option<&ActedTarget>,
    ) -> ObservedPages {
        self.read(registry, tabs, claims, acted, |tab| browser.page(tab))
            .await
    }

    // The read callback keeps the same critical section testable with suspended cache reads.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "the accepted observation contract serializes complete cache-read batches"
    )]
    async fn read<F, R>(
        &self,
        registry: &TabRegistry,
        tabs: &[TabId],
        claims: &HashMap<TabId, Vec<Claim>>,
        acted: Option<&ActedTarget>,
        mut page: F,
    ) -> ObservedPages
    where
        F: FnMut(TabId) -> R,
        R: Future<Output = Option<PageInfo>>,
    {
        let mut serial = self.serial.lock().await;
        self.log.synchronize().await;
        *serial += 1;
        let mut observed = Observation {
            seq: *serial,
            events: self.log.head(),
            payload_current: acted.map(|_| false),
            closes: Vec::new(),
        };
        let mut infos = Vec::with_capacity(tabs.len());
        let mut workers = Vec::with_capacity(tabs.len());
        let mut withheld = Vec::new();
        for &tab in tabs {
            let delivered = claims.get(&tab).map_or(&[][..], Vec::as_slice);
            loop {
                let seen = registry.seen();
                let Some(info) = page(tab).await else {
                    if let Some(claim) = delivered
                        .iter()
                        .filter(|claim| claim.released)
                        .max_by_key(|claim| claim.seq)
                    {
                        observed.closes.push(ApprovedClose {
                            tab,
                            seq: claim.seq,
                        });
                    } else if !delivered.is_empty() {
                        withheld.push(tab);
                    }
                    workers.push(Some(Vec::new()));
                    infos.push(None);
                    break;
                };
                let remembered = registry.gone_target(tab);
                if !registry.read_is_current(tab, seen) {
                    continue;
                }
                let approved = delivered
                    .iter()
                    .filter(|claim| claim.target == info.target_id && claim.released)
                    .max_by_key(|claim| claim.seq);
                let matched = delivered.iter().any(|claim| claim.target == info.target_id);
                let dead = matched
                    || remembered.is_some_and(|target| target == info.target_id)
                    || self.log.closed(&info.target_id).is_some();
                if dead && !registry.note_gone(tab, info.target_id.clone(), seen) {
                    // Retry after a detach invalidated the cache snapshot.
                    continue;
                }
                if !dead {
                    registry.forget_gone(tab);
                }
                if matched && approved.is_none() {
                    withheld.push(tab);
                }
                if acted.is_some_and(|source| source.tab == tab) {
                    observed.payload_current =
                        Some(!dead && acted.is_some_and(|source| source.target == info.target_id));
                }
                if let Some(claim) = approved {
                    observed.closes.push(ApprovedClose {
                        tab,
                        seq: claim.seq,
                    });
                }
                workers.push(if dead {
                    Some(Vec::new())
                } else {
                    self.log.workers(&info.target_id)
                });
                infos.push((!dead).then_some(info));
                break;
            }
        }
        ObservedPages {
            observed,
            infos,
            workers,
            withheld,
        }
    }

    /// Lets a test suspend the observation boundary while core updates its cache.
    #[cfg(test)]
    pub(crate) async fn hold(&self) -> MutexGuard<'_, u64> {
        self.serial.lock().await
    }

    #[cfg(test)]
    pub(crate) fn unrecorded() -> Self {
        struct Empty;
        impl EventLog for Empty {
            fn head(&self) -> u64 {
                0
            }
            fn closed(&self, _: &TargetId) -> Option<u64> {
                None
            }
        }
        Self::new(Arc::new(Empty))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use tokio::task;

    use crate::browser::TabRegistry;
    use browseros_core::pages::PageInfo;
    use browseros_core::{PageId, TabId, TargetId};

    use super::{ActedTarget, ApprovedClose, Claim, EventLog, Observation, Observer};

    #[derive(Default)]
    struct Log {
        head: AtomicU64,
        closed: HashMap<TargetId, u64>,
    }
    impl EventLog for Log {
        fn head(&self) -> u64 {
            self.head.load(Ordering::SeqCst)
        }
        fn closed(&self, target: &TargetId) -> Option<u64> {
            self.closed.get(target).copied()
        }
    }

    fn info(target: &str) -> PageInfo {
        PageInfo {
            page_id: PageId(1),
            target_id: TargetId(target.to_owned()),
            tab_id: TabId(51),
            url: "https://example.test/".into(),
            title: "Example".into(),
            is_active: false,
            is_loading: false,
            load_progress: 1.0,
            is_pinned: false,
            window_id: None,
            index: None,
            group_id: None,
            browser_profile_id: None,
            live_browser_context_id: None,
        }
    }

    #[tokio::test]
    async fn approves_only_the_current_bindings_latest_delivered_close() {
        let tab = TabId(51);
        let claims = HashMap::from([(
            tab,
            vec![
                Claim {
                    target: TargetId("T2".into()),
                    seq: 5,
                    released: true,
                },
                Claim {
                    target: TargetId("T1".into()),
                    seq: 6,
                    released: true,
                },
                Claim {
                    target: TargetId("T1".into()),
                    seq: 7,
                    released: true,
                },
            ],
        )]);
        for (target, approved) in [
            (Some("T2"), Some(5)),
            (Some("T1"), Some(7)),
            (None, Some(7)),
            (Some("T3"), None),
        ] {
            let observer = Observer::unrecorded();
            let registry = TabRegistry::new();
            let result = observer
                .read(&registry, &[tab], &claims, None, |_| async {
                    target.map(info)
                })
                .await;
            assert_eq!(result.infos[0].is_none(), approved.is_some());
            assert_eq!(
                result.observed.closes,
                approved
                    .into_iter()
                    .map(|seq| ApprovedClose { tab, seq })
                    .collect::<Vec<_>>()
            );
        }
        let registry = TabRegistry::new();
        assert!(registry.note_gone(tab, TargetId("T1".into()), registry.seen()));
        let result = Observer::unrecorded()
            .read(&registry, &[tab], &HashMap::new(), None, |_| async {
                Some(info("T1"))
            })
            .await;
        assert!(result.infos[0].is_none());
        assert!(
            result.observed.closes.is_empty(),
            "remembered evidence has no delivered record to approve"
        );
    }

    #[tokio::test]
    async fn payload_current_requires_the_same_live_binding_even_at_the_same_url() {
        let tab = TabId(51);
        let acted = ActedTarget {
            tab,
            target: TargetId("T1".into()),
        };
        for (target, dead, expected) in [
            (Some("T1"), false, true),
            (Some("T2"), false, false),
            (None, false, false),
            (Some("T1"), true, false),
        ] {
            let registry = TabRegistry::new();
            let claims = if dead {
                HashMap::from([(
                    tab,
                    vec![Claim {
                        target: acted.target.clone(),
                        seq: 7,
                        released: true,
                    }],
                )])
            } else {
                HashMap::new()
            };
            let result = Observer::unrecorded()
                .read(&registry, &[tab], &claims, Some(&acted), |_| async {
                    target.map(info)
                })
                .await;
            assert_eq!(result.observed.payload_current, Some(expected));
        }
        let result = Observer::unrecorded()
            .read(
                &TabRegistry::new(),
                &[tab],
                &HashMap::new(),
                None,
                |_| async { Some(info("T1")) },
            )
            .await;
        assert_eq!(result.observed.payload_current, None);
    }

    #[tokio::test]
    async fn withheld_claims_keep_dead_evidence_and_only_released_records_get_approval() {
        let tab = TabId(51);
        for released in [false, true] {
            let registry = TabRegistry::new();
            let claims = HashMap::from([(
                tab,
                vec![
                    Claim {
                        target: TargetId("T".into()),
                        seq: 7,
                        released: false,
                    },
                    Claim {
                        target: TargetId("T".into()),
                        seq: 5,
                        released,
                    },
                ],
            )]);
            let result = Observer::unrecorded()
                .read(&registry, &[tab], &claims, None, |_| async {
                    Some(info("T"))
                })
                .await;
            assert!(result.infos[0].is_none());
            assert_eq!(registry.gone_target(tab), Some(TargetId("T".into())));
            assert_eq!(result.withheld.contains(&tab), !released);
            assert_eq!(
                result.observed.closes.first().map(|close| close.seq),
                released.then_some(5)
            );
        }
    }

    #[test]
    fn optional_observation_fields_serialize_without_private_identity() -> Result<(), Box<dyn Error>>
    {
        let empty = Observation {
            seq: 1,
            events: 2,
            ..Observation::default()
        };
        assert_eq!(
            serde_json::to_value(&empty)?,
            serde_json::json!({"seq":1,"events":2})
        );
        let full = Observation {
            payload_current: Some(false),
            closes: vec![ApprovedClose {
                tab: TabId(51),
                seq: 7,
            }],
            ..empty
        };
        assert_eq!(
            serde_json::to_value(full.clone())?,
            serde_json::json!({"seq":1,"events":2,"payloadCurrent":false,"closes":[{"tab":51,"seq":7}]})
        );
        Ok(())
    }

    #[tokio::test]
    async fn serials_increase_and_event_head_precedes_the_cache_reads() {
        let log = Arc::new(Log::default());
        log.head.store(7, Ordering::SeqCst);
        let observer = Observer::new(Arc::clone(&log) as _);
        let registry = TabRegistry::new();
        let claims = HashMap::new();
        let first = observer
            .read(&registry, &[TabId(51)], &claims, None, |_| async {
                log.head.store(8, Ordering::SeqCst);
                Some(info("T"))
            })
            .await;
        let second = observer
            .read(&registry, &[], &claims, None, |_| async { None })
            .await;
        assert_eq!(first.observed.seq, 1);
        assert_eq!(first.observed.events, 7);
        assert_eq!(second.observed.seq, 2);
        assert_eq!(second.observed.events, 8);
    }

    #[tokio::test]
    async fn concurrent_observations_keep_all_cache_reads_together() -> Result<(), Box<dyn Error>> {
        let observer = Observer::unrecorded();
        let registry = TabRegistry::new();
        let claims = HashMap::new();
        let reads = Arc::new(Mutex::new(Vec::new()));
        let read = |tab: TabId| {
            let reads = Arc::clone(&reads);
            async move {
                reads
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(tab.0);
                task::yield_now().await;
                None
            }
        };
        let (first, second) = tokio::join!(
            observer.read(&registry, &[TabId(1), TabId(2)], &claims, None, read),
            observer.read(&registry, &[TabId(3), TabId(4)], &claims, None, read)
        );
        assert_eq!(first.observed.seq, 1);
        assert_eq!(second.observed.seq, 2);
        assert_eq!(*reads.lock().map_err(|e| e.to_string())?, [1, 2, 3, 4]);
        Ok(())
    }

    #[tokio::test]
    async fn a_recorded_close_overrides_a_lagging_cache_but_not_a_replacement_target() {
        let mut log = Log::default();
        log.closed.insert(TargetId("old".into()), 1);
        log.head.store(1, Ordering::SeqCst);
        let observer = Observer::new(Arc::new(log));
        let registry = TabRegistry::new();
        let claims = HashMap::new();
        let closed = observer
            .read(&registry, &[TabId(51)], &claims, None, |_| async {
                Some(info("old"))
            })
            .await;
        let replaced = observer
            .read(&registry, &[TabId(51)], &claims, None, |_| async {
                Some(info("new"))
            })
            .await;
        assert!(closed.infos[0].is_none());
        assert_eq!(
            replaced.infos[0]
                .as_ref()
                .map(|page| page.target_id.as_str()),
            Some("new")
        );
    }
    #[tokio::test]
    async fn delivered_claim_survives_eviction_and_is_forgotten_on_rebind() {
        let log = Arc::new(Log::default());
        log.head.store(1025, Ordering::SeqCst);
        let observer = Observer::new(log);
        let registry = TabRegistry::new();
        let claims = HashMap::from([(
            TabId(51),
            vec![Claim {
                target: TargetId("old".into()),
                seq: 1,
                released: true,
            }],
        )]);
        let first = observer
            .read(&registry, &[TabId(51)], &claims, None, |_| async {
                Some(info("old"))
            })
            .await;
        assert!(first.infos[0].is_none());
        assert_eq!(first.observed.events, 1025);
        assert_eq!(
            registry.gone_target(TabId(51)),
            Some(TargetId("old".into()))
        );
        let no_claims = HashMap::new();
        let later = observer
            .read(&registry, &[TabId(51)], &no_claims, None, |_| async {
                Some(info("old"))
            })
            .await;
        assert!(
            later.infos[0].is_none(),
            "the remembered binding survives an empty ring"
        );
        let rebound = observer
            .read(&registry, &[TabId(51)], &claims, None, |_| async {
                Some(info("new"))
            })
            .await;
        assert!(
            rebound.infos[0].is_some(),
            "an old claim cannot close the replacement target"
        );
        assert_eq!(registry.gone_target(TabId(51)), None);
    }

    #[tokio::test]
    async fn detach_during_a_cache_read_cannot_recreate_or_reopen_a_dead_binding() {
        for delivered in [true, false] {
            let observer = Observer::unrecorded();
            let registry = TabRegistry::new();
            assert!(registry.note_gone(TabId(51), TargetId("old".into()), registry.seen()));
            let claims = if delivered {
                HashMap::from([(
                    TabId(51),
                    vec![Claim {
                        target: TargetId("old".into()),
                        seq: 1,
                        released: true,
                    }],
                )])
            } else {
                HashMap::new()
            };
            let mut reads = 0;
            let observed = observer
                .read(&registry, &[TabId(51)], &claims, None, |_| {
                    reads += 1;
                    let result = if reads == 1 {
                        registry.removing(TabId(51))();
                        Some(info("old"))
                    } else {
                        None
                    };
                    async move { result }
                })
                .await;
            assert_eq!(reads, 2, "the detached cache snapshot is retried");
            assert!(observed.infos[0].is_none());
            assert_eq!(registry.gone_target(TabId(51)), None);
        }
    }
    #[tokio::test]
    async fn delayed_removal_cleanup_preserves_readmitted_dead_bindings() {
        for target in ["same", "replacement"] {
            let observer = Observer::unrecorded();
            let registry = TabRegistry::new();
            let tab = TabId(51);
            // Core invokes removing under its cache lock, before the tab can re-enter.
            let delayed_cleanup = registry.removing(tab);
            assert!(registry.note_gone(tab, TargetId(target.into()), registry.seen()));
            delayed_cleanup();
            let observed = observer
                .read(&registry, &[tab], &HashMap::new(), None, |_| async {
                    Some(info(target))
                })
                .await;
            assert!(observed.infos[0].is_none());
            assert_eq!(registry.gone_target(tab), Some(TargetId(target.into())));
            registry.removing(tab)();
            assert_eq!(registry.gone_target(tab), None);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn same_target_readmission_survives_delayed_post_removal_work()
    -> Result<(), Box<dyn Error>> {
        use browseros_core::pages::{AfterPageRemoved, PageManager, PageManagerHooks};
        use browseros_core::testing::TestConnection;
        use std::sync::mpsc;
        use std::time::Duration;
        use tokio::sync::Notify;
        let tab = TabId(51);
        let registry = Arc::new(TabRegistry::new());
        let ready = Arc::new(Notify::new());
        let (release, wait) = mpsc::channel();
        let wait = Arc::new(Mutex::new(wait));
        let retiring = Arc::clone(&registry);
        let reached = Arc::clone(&ready);
        let hook = Arc::new(move |info: &PageInfo| -> AfterPageRemoved {
            let cleanup = retiring.removing(info.tab_id);
            let reached = Arc::clone(&reached);
            let wait = Arc::clone(&wait);
            Box::new(move || {
                reached.notify_one();
                assert!(
                    wait.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .recv_timeout(Duration::from_secs(5))
                        .is_ok()
                );
                cleanup();
            })
        });
        let live = serde_json::json!({"tabId":51,"targetId":"same","url":"https://example.test/","title":"Example","isActive":true,"isLoading":false,"loadProgress":1.0,"isPinned":false,"isHidden":false});
        let mut excluded = live.clone();
        excluded["url"] = serde_json::json!("chrome-extension://excluded/page.html");
        let manager = Arc::new(PageManager::new(
            TestConnection::new([
                (
                    "Browser.getTabs",
                    serde_json::json!({"tabs":[live.clone()]}),
                ),
                ("Browser.getTabs", serde_json::json!({"tabs":[excluded]})),
                ("Browser.getTabs", serde_json::json!({"tabs":[live]})),
            ]),
            PageManagerHooks {
                on_page_removed: Some(hook),
                ..PageManagerHooks::default()
            },
        ));
        let first = manager.list().await?.remove(0);
        let pruning = Arc::clone(&manager);
        let pending = tokio::spawn(async move { pruning.list().await });
        ready.notified().await;
        let returned = manager.list().await?.remove(0);
        assert_ne!(first.page_id, returned.page_id);
        assert_eq!(first.target_id, returned.target_id);
        let observer = Observer::unrecorded();
        let claims = HashMap::from([(
            tab,
            vec![Claim {
                target: returned.target_id.clone(),
                seq: 7,
                released: true,
            }],
        )]);
        let closed = observer
            .read(&registry, &[tab], &claims, None, |tab| {
                manager.get_info_by_tab(tab)
            })
            .await;
        assert!(closed.infos[0].is_none());
        release.send(())?;
        assert!(pending.await??.is_empty());
        let later = observer
            .read(&registry, &[tab], &HashMap::new(), None, |tab| {
                manager.get_info_by_tab(tab)
            })
            .await;
        assert!(
            later.infos[0].is_none(),
            "empty event ring must not reopen a confirmed dead binding"
        );
        assert_eq!(registry.gone_target(tab), Some(returned.target_id));
        Ok(())
    }
}
