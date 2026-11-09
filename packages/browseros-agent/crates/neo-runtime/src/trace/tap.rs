//! What every emitter of one run shares: the run's id, the sinks that listen to it, its clock,
//! its secrets and why it ended. Each place a fact of the run starts holds an `Arc` of it: the
//! run's guard, its trace and every call entry, every delivery cell, the page registry and its
//! pages, the getters.
//!
//! ```text
//!  RunGuard ─┐                                  RunTap ─▶ run sinks
//!  Trace ────┤                                    ▲  holds nothing that holds it,
//!  CallEntry ┼─ Arc<RunTap> ─ emit(&RunEvent) ────┘  so no cycle keeps a run unsettled
//!  DeliveryCell, Tabs, PageEntry, Getters ─┘
//!                              last Arc dropped ─▶ Drop: RunSettled, then the sinks go
//! ```
//!
//! `RunSettled` is provably the run's last event: a holder that emits from its own `Drop` does
//! so before its fields, its `Arc` of the tap among them, are dropped, so when the last `Arc`
//! goes nothing of the run is left that could emit. That holder may be a writer task that held
//! an input ticket past the run (up to browseros-cdp's 60 s request timeout), so a run settles
//! when its last effect does, not when its outcome is returned.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::time::Instant;

use super::redact::RunSecrets;
use super::{RunEvent, RunId, RunSink};
use crate::run::RunEnd;

/// Shared, acyclic lifetime of one run. Every emitter retains it until its last possible
/// event, so the final owner emits `RunSettled` before releasing the run's sinks.
pub(crate) struct RunTap {
    run: RunId,
    /// The sinks that took the run (`TraceSink::run`). Empty when none did; then emitters build
    /// no events, though records are still redacted.
    sinks: Box<[Arc<dyn RunSink>]>,
    /// When the run started, for every event's `offset_us`.
    start: Instant,
    secrets: RunSecrets,
    /// Why the run ended, set once by `Dispatcher::finish` and shared with its `RunScope`.
    /// Unset when the caller dropped the run's future, which reads as cancelled.
    end_reason: Arc<OnceLock<RunEnd>>,
}

impl RunTap {
    pub(crate) fn new(run: RunId, sinks: Vec<Arc<dyn RunSink>>) -> Arc<Self> {
        Arc::new(Self {
            run,
            sinks: sinks.into_boxed_slice(),
            start: Instant::now(),
            secrets: RunSecrets::default(),
            end_reason: Arc::default(),
        })
    }

    /// A tap no sink listens to, for tests that build a run's parts alone.
    #[cfg(test)]
    pub(crate) fn detached() -> Arc<Self> {
        Self::new(RunId::mint(), Vec::new())
    }

    pub(crate) fn run(&self) -> RunId {
        self.run
    }

    /// Whether any sink listens. Emitters check it before they build an event, so a run nobody
    /// traces pays one branch per fact.
    pub(crate) fn listening(&self) -> bool {
        !self.sinks.is_empty()
    }

    /// Hands `event` to every sink, in order. Synchronous: sinks only queue (`RunSink::emit`), so
    /// this may run on the realm's thread, inside a lock-free `Drop`, or outside tokio. Callers
    /// emit only while holding no lock.
    pub(crate) fn emit(&self, event: &RunEvent) {
        for sink in &self.sinks {
            sink.emit(event);
        }
    }

    /// Builds and emits an event, when any sink listens.
    pub(crate) fn emit_with(&self, event: impl FnOnce() -> RunEvent) {
        if self.listening() {
            self.emit(&event());
        }
    }

    pub(crate) fn start(&self) -> Instant {
        self.start
    }

    /// Microseconds since the run started.
    pub(crate) fn offset_us(&self) -> u64 {
        micros(self.start.elapsed())
    }

    pub(crate) fn secrets(&self) -> &RunSecrets {
        &self.secrets
    }

    /// The cell `RunScope` records the run's end reason in.
    pub(crate) fn end_cell(&self) -> Arc<OnceLock<RunEnd>> {
        Arc::clone(&self.end_reason)
    }
}

impl Drop for RunTap {
    fn drop(&mut self) {
        // The last holder of the run's state let go: see the module docs for why nothing can
        // follow this. The sinks are dropped right after, with the tap's fields.
        self.emit_with(|| RunEvent::RunSettled { run: self.run });
    }
}

/// `duration` in whole microseconds, saturating.
pub(crate) fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    use super::*;

    /// Captures sink ownership as well as events without depending on a tokio runtime.
    #[derive(Default)]
    pub(crate) struct Recording {
        pub(crate) events: Mutex<Vec<RunEvent>>,
        pub(crate) dropped_after: Mutex<Option<usize>>,
    }

    struct Sink(Arc<Recording>);

    impl RunSink for Sink {
        fn emit(&self, event: &RunEvent) {
            self.0.events.lock().unwrap().push(event.clone());
        }
    }

    impl Drop for Sink {
        fn drop(&mut self) {
            *self.0.dropped_after.lock().unwrap() = Some(self.0.events.lock().unwrap().len());
        }
    }

    pub(crate) fn tap() -> (Arc<RunTap>, Arc<Recording>) {
        let recording = Arc::new(Recording::default());
        let sink: Arc<dyn RunSink> = Arc::new(Sink(Arc::clone(&recording)));
        (RunTap::new(RunId::mint(), vec![sink]), recording)
    }

    #[test]
    fn the_last_owner_settles_then_releases_sinks_on_any_thread() {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let (tap, recording) = tap();
            let mut holders = [Some(Arc::clone(&tap)), Some(Arc::clone(&tap)), Some(tap)];
            for index in &order[..2] {
                drop(holders[*index].take());
                assert!(recording.events.lock().unwrap().is_empty());
                assert_eq!(*recording.dropped_after.lock().unwrap(), None);
            }
            let last = holders[order[2]].take();
            thread::spawn(move || drop(last)).join().unwrap();
            assert!(matches!(
                recording.events.lock().unwrap().as_slice(),
                [RunEvent::RunSettled { .. }]
            ));
            assert_eq!(*recording.dropped_after.lock().unwrap(), Some(1));
        }
    }

    #[test]
    fn no_listener_never_builds_an_event() {
        let tap = RunTap::detached();
        let built = AtomicBool::new(false);
        tap.emit_with(|| {
            built.store(true, Ordering::Relaxed);
            RunEvent::RunSettled { run: tap.run() }
        });
        assert!(!built.load(Ordering::Relaxed));
    }
}
