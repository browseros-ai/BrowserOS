//! Facade-lifetime renderer crash facts, independent of runs and the bounded event journal.
//! Requests and recovery proofs carry revisions: late replies cannot poison a replacement
//! document or recreate a closed target. Every transition is synchronous; no lock crosses an await.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_core::{FrameId, TargetId};

use crate::events::EventFence;

/// The single admission authority shared by foreground operations and the event watcher.
#[derive(Default)]
pub(crate) struct TargetCrashes {
    records: Mutex<Records>,
}

#[derive(Default)]
struct Records {
    revision: u64,
    observer: u64,
    targets: HashMap<TargetId, TargetRecord>,
}

struct TargetRecord {
    revision: u64,
    crashed: Option<EventFence>,
    main_loader: Option<String>,
}

/// The native target and document state against which one request began.
#[derive(Clone, Debug)]
pub(crate) struct TargetTicket {
    target: TargetId,
    revision: u64,
    epoch: u64,
}

/// A candidate is not recovery evidence. Only its matching fresh tree reply can clear refusal.
pub(crate) struct RecoveryTicket {
    target: TargetTicket,
    observer: u64,
    pub(crate) frame: FrameId,
    pub(crate) loader: String,
}

/// The records that existed before a fresh target listing was dispatched.
pub(crate) struct TargetSnapshot {
    observer: u64,
    epoch: u64,
    revisions: HashMap<TargetId, u64>,
}

impl Records {
    fn advance(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1);
        self.revision
    }
}

impl TargetCrashes {
    pub(crate) fn ticket(&self, target: &TargetId, epoch: u64) -> TargetTicket {
        let mut records = self.lock();
        let revision = if let Some(record) = records.targets.get(target) {
            record.revision
        } else {
            let revision = records.advance();
            records.targets.insert(
                target.clone(),
                TargetRecord {
                    revision,
                    crashed: None,
                    main_loader: None,
                },
            );
            revision
        };
        TargetTicket {
            target: target.clone(),
            revision,
            epoch,
        }
    }

    pub(crate) fn is_crashed(&self, target: &TargetId) -> bool {
        self.lock()
            .targets
            .get(target)
            .is_some_and(|record| record.crashed.is_some())
    }

    pub(crate) fn refuses(&self, ticket: &TargetTicket) -> bool {
        self.is_crashed(&ticket.target)
    }

    /// Untagged root events may add conservative evidence, never prove recovery. Record at
    /// ingress, before a frame-tree hold or public opening filter can delay the event.
    pub(crate) fn crashed_event(&self, target: TargetId, position: EventFence) {
        let mut records = self.lock();
        let revision = records.advance();
        let record = records.targets.entry(target).or_insert(TargetRecord {
            revision,
            crashed: None,
            main_loader: None,
        });
        record.revision = revision;
        record.crashed = Some(record.crashed.map_or(position, |old| old.max(position)));
    }

    /// The caller still gets a typed error for a stale reply, but that reply has no authority
    /// over a newer document, observer connection, or target lifetime.
    pub(crate) fn crashed_reply(&self, ticket: &TargetTicket, epoch: u64, fence: EventFence) {
        let mut records = self.lock();
        if ticket.epoch != epoch
            || records
                .targets
                .get(&ticket.target)
                .is_none_or(|record| record.revision != ticket.revision)
        {
            return;
        }
        let revision = records.advance();
        if let Some(record) = records.targets.get_mut(&ticket.target) {
            record.revision = revision;
            record.crashed = Some(record.crashed.map_or(fence, |old| old.max(fence)));
        }
    }

    /// Only a later main-document commit can request proof. Keep its original ingress fence:
    /// a commit held behind a tree read must not clear a crash observed after it arrived.
    pub(crate) fn main_document(
        &self,
        target: &TargetId,
        frame: FrameId,
        loader: String,
        ingress: EventFence,
        epoch: u64,
    ) -> Option<RecoveryTicket> {
        let mut records = self.lock();
        let record = records.targets.get(target)?;
        if record.main_loader.as_ref() == Some(&loader) {
            return None;
        }
        let crashed = record.crashed;
        if crashed.is_some_and(|fence| ingress <= fence) {
            return None;
        }
        // Superseding commits invalidate both earlier request replies and earlier proofs.
        let revision = records.advance();
        let observer = records.observer;
        let record = records.targets.get_mut(target)?;
        record.revision = revision;
        if crashed.is_none() {
            record.main_loader = Some(loader);
            return None;
        }
        Some(RecoveryTicket {
            target: TargetTicket {
                target: target.clone(),
                revision,
                epoch,
            },
            observer,
            frame,
            loader,
        })
    }

    /// Called only by the watcher after matching the fresh tree and applying every event
    /// through its reply fence. Healthy records remain so old request tickets stay invalid.
    pub(crate) fn recovered(&self, ticket: RecoveryTicket, epoch: u64) {
        let mut records = self.lock();
        if ticket.target.epoch != epoch || ticket.observer != records.observer {
            return;
        }
        if records
            .targets
            .get(&ticket.target.target)
            .is_none_or(|record| record.revision != ticket.target.revision)
        {
            return;
        }
        let revision = records.advance();
        if let Some(record) = records.targets.get_mut(&ticket.target.target) {
            record.revision = revision;
            record.crashed = None;
            record.main_loader = Some(ticket.loader);
        }
    }

    pub(crate) fn invalidate_recoveries(&self) {
        let mut records = self.lock();
        records.observer = records.observer.saturating_add(1);
    }

    pub(crate) fn confirmed_closed(&self, target: &TargetId) {
        self.lock().targets.remove(target);
    }

    pub(crate) fn snapshot(&self, epoch: u64) -> TargetSnapshot {
        let records = self.lock();
        TargetSnapshot {
            observer: records.observer,
            epoch,
            revisions: records
                .targets
                .iter()
                .map(|(target, record)| (target.clone(), record.revision))
                .collect(),
        }
    }

    /// Absence only retires records that existed unchanged before the epoch-pinned listing.
    /// A late listing cannot erase a crash learned while its request was in flight.
    pub(crate) fn reconcile(
        &self,
        snapshot: &TargetSnapshot,
        live: &HashSet<TargetId>,
        epoch: u64,
    ) {
        let mut records = self.lock();
        if snapshot.epoch == epoch && snapshot.observer == records.observer {
            records.targets.retain(|target, record| {
                live.contains(target) || snapshot.revisions.get(target) != Some(&record.revision)
            });
        }
    }

    fn lock(&self) -> MutexGuard<'_, Records> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::error::Error;

    use browseros_core::{FrameId, TargetId};

    use super::TargetCrashes;
    use crate::events::EventFence;

    #[test]
    fn late_request_replies_cannot_poison_recovery_or_a_closed_target() -> Result<(), Box<dyn Error>>
    {
        let crashes = TargetCrashes::default();
        let target = TargetId("page".to_owned());
        let before = crashes.ticket(&target, 1);
        crashes.crashed_reply(&before, 1, EventFence(3));
        let proof = crashes
            .main_document(
                &target,
                FrameId(target.0.clone()),
                "new".to_owned(),
                EventFence(4),
                1,
            )
            .ok_or("the replacement document can request proof")?;
        crashes.recovered(proof, 1);
        crashes.crashed_reply(&before, 1, EventFence(5));
        assert!(
            !crashes.is_crashed(&target),
            "a reply belongs to its original document"
        );

        let before_close = crashes.ticket(&target, 1);
        crashes.confirmed_closed(&target);
        let replacement = crashes.ticket(&target, 1);
        crashes.crashed_reply(&before_close, 1, EventFence(6));
        assert!(
            !crashes.refuses(&replacement),
            "retired tickets cannot recreate an old lifetime"
        );
        Ok(())
    }

    #[test]
    fn a_late_listing_cannot_erase_new_crash_evidence() {
        let crashes = TargetCrashes::default();
        let target = TargetId("page".to_owned());
        let _known = crashes.ticket(&target, 1);
        let listing = crashes.snapshot(1);
        crashes.crashed_event(target.clone(), EventFence(2));
        crashes.reconcile(&listing, &HashSet::new(), 1);
        assert!(
            crashes.is_crashed(&target),
            "the listing predates this crash"
        );
        let fresh = crashes.snapshot(1);
        crashes.reconcile(&fresh, &HashSet::new(), 2);
        assert!(
            crashes.is_crashed(&target),
            "an old connection cannot prove closure"
        );
        crashes.reconcile(&crashes.snapshot(2), &HashSet::new(), 2);
        assert!(
            !crashes.is_crashed(&target),
            "a fresh confirmed absence retires the record"
        );
    }

    #[test]
    fn crashes_and_observer_loss_invalidate_pending_recovery() -> Result<(), Box<dyn Error>> {
        let crashes = TargetCrashes::default();
        let target = TargetId("page".to_owned());
        let other = TargetId("healthy".to_owned());
        let healthy = crashes.ticket(&other, 1);
        crashes.crashed_event(target.clone(), EventFence(2));
        assert!(
            crashes
                .main_document(
                    &target,
                    FrameId(target.0.clone()),
                    "old".to_owned(),
                    EventFence(1),
                    1
                )
                .is_none(),
            "a held older commit cannot repair a newer eager crash"
        );
        let candidate = || {
            crashes
                .main_document(
                    &target,
                    FrameId(target.0.clone()),
                    "new".to_owned(),
                    EventFence(4),
                    1,
                )
                .ok_or("a later commit can request proof")
        };
        let lost = candidate()?;
        crashes.invalidate_recoveries();
        crashes.recovered(lost, 1);
        assert!(
            crashes.is_crashed(&target),
            "lag invalidates proof without clearing refusal"
        );
        let reconnect = candidate()?;
        crashes.recovered(reconnect, 2);
        assert!(
            crashes.is_crashed(&target),
            "another connection cannot complete the proof"
        );
        let older = candidate()?;
        crashes.crashed_event(target.clone(), EventFence(5));
        crashes.recovered(older, 1);
        assert!(
            crashes.is_crashed(&target),
            "a newer crash wins over an older success"
        );
        assert!(
            !crashes.refuses(&healthy),
            "another target's renderer remains usable"
        );
        Ok(())
    }
}
