//! Orders the host's page-cache observations and correlates them with the page-event log.
//! One lock spans all reads in an observation. The current target must also be absent from
//! the log's close records before a cached tab can be reported live. Delivered claims and
//! remembered dead bindings remain decisive after ring eviction, until core detaches or rebinds.
//!
//! Each observation first brings the log up to a fence it took before it queued: the events
//! received until the op's work was done, never those a busy page sends while it waits.
//!
//! ```text
//!  op done ─▶ fence = events received now ─▶ lock (serial turn) ─▶ log reaches fence? ─▶ seq,
//!                                                 │                  (≤ 2 s)              head,
//!                                                 └─ a wait already gave up and the log  cache
//!                                                    is still short of that fence: skip  reads
//! ```
//!
//! One stall costs one bound. Without the skip, every reply queued behind a watcher that fell
//! behind waited its own 2 s in turn: a run's start, which observes once per open tab, cost
//! 2 s per tab. A batch whose events are not proven in the log carries no worker list, so a
//! stale one cannot retire workers the script holds; closes stay ordered by the log's seqs.

use futures_util::future::BoxFuture;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use browseros_core::pages::PageInfo;
use browseros_core::{TabId, TargetId};
use neo_playwright::EventFence;
use serde::Serialize;
use tokio::sync::Mutex;
#[cfg(test)]
use tokio::sync::MutexGuard;
use ts_rs::TS;

use crate::browser::{BrowserLink, TabRegistry};

/// The event evidence needed to reconcile a cache read with the independent target watcher.
pub(crate) trait EventLog: Send + Sync {
    /// Every event received until now: what an observation taken now owes the script.
    fn fence(&self) -> EventFence {
        EventFence::default()
    }
    /// Records every event before `fence` and brings the log's leases in line, waiting at
    /// most its bound. Whether it did: `false` means the log is still short of `fence`.
    fn synchronize(&self, _fence: EventFence) -> BoxFuture<'_, bool> {
        Box::pin(async { true })
    }
    /// Whether every event before `fence` is already recorded, without waiting.
    fn handled_through(&self, _fence: EventFence) -> bool {
        true
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
    /// The target's frame-tree version (`PageFact::frames_changed`). `None` when the log has
    /// seen no change of its frames.
    fn frames_changed(&self, _target: &TargetId) -> Option<u64> {
        None
    }
    /// The newest frame-tree version of any tab; 0 before any change.
    fn frames_head(&self) -> u64 {
        0
    }
}

/// A worker identity in the host protocol, independent of the browser adapter. The prelude's
/// `Worker` objects are keyed by `id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(rename = "WorkerInfo")]
pub(crate) struct WorkerFact {
    pub(crate) id: String,
    pub(crate) url: String,
}

/// Freshness of a batch of page facts. Neither counter is a page identity.
//
// The `#[ts(..)]` lines keep the generated type's optional fields in step with the
// `skip_serializing_if` lines: ts-rs reads those as optional only next to `serde(default)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Observation {
    /// The host's read serial: a later batch of facts replaces an earlier one, never the reverse.
    pub(crate) seq: u64,
    /// The page-event log's head before this read.
    pub(crate) events: u64,
    /// Whether the operation's payload describes the binding read by this observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) payload_current: Option<bool>,
    /// Close records applied by this observation, consumed in event order by the prelude.
    /// Absent when there are none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[ts(optional = nullable)]
    pub(crate) closes: Vec<ApprovedClose>,
}

/// One delivered close selected for the current binding, named by its existing event position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
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
    turn: Mutex<Turn>,
    log: Arc<dyn EventLog>,
}

/// What the observation turn keeps between observations.
#[derive(Debug, Default)]
pub(crate) struct Turn {
    /// The host's read serial: the newest observation's `seq`.
    serial: u64,
    /// The fence the newest wait gave up on, while the log is still short of it. Until the log
    /// passes it, observations do not wait again: theirs would end the same way.
    stalled_at: Option<EventFence>,
}

/// How an observation's wait for its fence ended, for the debug log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waited {
    /// The log recorded every event before the fence.
    Reached,
    /// The bound passed first; later observations skip until the log passes this fence.
    GaveUp,
    /// No wait: an earlier one gave up on a fence the log is still short of.
    Stalled,
}

impl Observer {
    pub(crate) fn new(log: Arc<dyn EventLog>) -> Self {
        Self {
            turn: Mutex::new(Turn::default()),
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

    /// `target`'s frame-tree version (`PageFact::frames_changed`).
    pub(crate) fn frames_changed(&self, target: &TargetId) -> Option<u64> {
        self.log.frames_changed(target)
    }

    /// The newest frame-tree version of any tab (`Envelope::Ok` `frames_head`).
    pub(crate) fn frames_head(&self) -> u64 {
        self.log.frames_head()
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
        // Taken before the turn: what this observation owes is fixed when its op's work is
        // done, not stretched by the events that arrive while it queues or waits.
        let fence = self.log.fence();
        let started = Instant::now();
        let mut turn = self.turn.lock().await;
        let queued = started.elapsed();
        let waited = self.catch_up(&mut turn, fence).await;
        let synchronized = waited == Waited::Reached || self.log.handled_through(fence);
        let caught_up = started.elapsed();
        turn.serial += 1;
        let mut observed = Observation {
            seq: turn.serial,
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
                // A list read before the log caught up could miss a worker the script holds,
                // and the prelude retires every worker a list leaves out: none is safer.
                workers.push(if dead {
                    Some(Vec::new())
                } else if synchronized {
                    self.log.workers(&info.target_id)
                } else {
                    None
                });
                infos.push((!dead).then_some(info));
                break;
            }
        }
        tracing::debug!(
            seq = observed.seq,
            tabs = tabs.len(),
            queued_ms = queued.as_millis(),
            wait_ms = caught_up.saturating_sub(queued).as_millis(),
            ?waited,
            read_ms = started.elapsed().saturating_sub(caught_up).as_millis(),
            "observation"
        );
        ObservedPages {
            observed,
            infos,
            workers,
            withheld,
        }
    }

    /// Brings the log up to `fence` in the observation's turn, unless an earlier wait gave up
    /// on a fence the log is still short of: then this one would wait out the same stall, and
    /// a run's replies would pay it one after another. A skipped wait publishes what a wait
    /// that gave up always published, facts the log has not caught up with: a close still
    /// reaches the script in log order (a live fact with an older head cannot reopen a page the
    /// log closed later), and the batch carries no worker list.
    async fn catch_up(&self, turn: &mut Turn, fence: EventFence) -> Waited {
        if let Some(stalled) = turn.stalled_at {
            if !self.log.handled_through(stalled) {
                return Waited::Stalled;
            }
            turn.stalled_at = None;
        }
        if self.log.synchronize(fence).await {
            Waited::Reached
        } else {
            turn.stalled_at = Some(fence);
            Waited::GaveUp
        }
    }

    /// Lets a test suspend the observation boundary while core updates its cache.
    #[cfg(test)]
    pub(crate) async fn hold(&self) -> MutexGuard<'_, Turn> {
        self.turn.lock().await
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
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use futures_util::future::{BoxFuture, join_all};
    use neo_playwright::EventFence;
    use tokio::{task, time};

    use crate::browser::TabRegistry;
    use browseros_core::pages::PageInfo;
    use browseros_core::{PageId, TabId, TargetId};

    use super::{ActedTarget, ApprovedClose, Claim, EventLog, Observation, Observer, WorkerFact};

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

    /// A watcher that fell behind: each synchronize waits out a 2 s bound and fails, until
    /// `caught` says it caught up. Counts the waits it was asked for.
    #[derive(Default)]
    struct Behind {
        waits: AtomicU64,
        caught: AtomicBool,
    }
    impl EventLog for Behind {
        fn fence(&self) -> EventFence {
            EventFence::at(10)
        }
        fn synchronize(&self, _fence: EventFence) -> BoxFuture<'_, bool> {
            Box::pin(async move {
                self.waits.fetch_add(1, Ordering::SeqCst);
                if self.caught.load(Ordering::SeqCst) {
                    return true;
                }
                time::sleep(Duration::from_secs(2)).await;
                false
            })
        }
        fn handled_through(&self, _fence: EventFence) -> bool {
            self.caught.load(Ordering::SeqCst)
        }
        fn head(&self) -> u64 {
            0
        }
        fn workers(&self, _target: &TargetId) -> Option<Vec<WorkerFact>> {
            Some(vec![WorkerFact {
                id: "w".to_owned(),
                url: "https://example.test/w.js".to_owned(),
            }])
        }
        fn closed(&self, _target: &TargetId) -> Option<u64> {
            None
        }
    }

    /// A run's start observes once per open tab, all queued on the one turn. Behind a watcher
    /// that cannot catch up, the first wait gives up after its bound and the rest do not wait
    /// it again (each used to, in turn: 2 s per tab). Facts the log was not proven to hold
    /// carry no worker list; once the watcher catches up, observations wait and carry it again.
    #[tokio::test(start_paused = true)]
    async fn one_stalled_watcher_costs_one_bound_for_every_queued_observation()
    -> Result<(), Box<dyn Error>> {
        let log = Arc::new(Behind::default());
        let observer = Observer::new(Arc::clone(&log) as _);
        let registry = TabRegistry::new();
        let claims = HashMap::new();
        let read = |_: TabId| async { Some(info("T")) };
        let started = time::Instant::now();
        let observed =
            join_all((0..8).map(|_| observer.read(&registry, &[TabId(51)], &claims, None, read)))
                .await;
        assert_eq!(
            started.elapsed(),
            Duration::from_secs(2),
            "one bound in all"
        );
        assert_eq!(log.waits.load(Ordering::SeqCst), 1, "the other seven skip");
        assert_eq!(
            observed
                .iter()
                .map(|pages| pages.observed.seq)
                .collect::<Vec<_>>(),
            (1..=8).collect::<Vec<_>>(),
            "still published one by one, in turn order"
        );
        assert!(
            observed
                .iter()
                .all(|pages| pages.workers == [None] && pages.infos[0].is_some()),
            "facts without the workers a lagging log may have missed"
        );

        log.caught.store(true, Ordering::SeqCst);
        let caught_up = observer
            .read(&registry, &[TabId(51)], &claims, None, read)
            .await;
        assert_eq!(
            log.waits.load(Ordering::SeqCst),
            2,
            "past the stall, it waits again"
        );
        assert!(caught_up.workers[0].is_some(), "and carries the workers");
        Ok(())
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
