//! One page's sealed worlds: which frame documents have one, their seqs, and how each child
//! frame was reached from its parent.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{FrameId, ProtocolSession, SessionId};
use tokio::sync::OnceCell;

use crate::probe::ProbeFailure;
use crate::world::{Epoch, FrameSeq, Installed, SealedWorld, SessionKind, SessionSetup};

/// Where a world is installed: a frame on a raw session, and how that session was set up.
#[derive(Clone, Debug)]
pub(crate) struct Endpoint {
    pub(crate) session: ProtocolSession,
    pub(crate) frame: FrameId,
    pub(crate) kind: SessionKind,
}

/// What every install on one page shares besides its endpoint.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Installer<'a> {
    /// The process's per-session setup (`Runtime.enable` on OOPIFs).
    pub(crate) sessions: &'a SessionSetup,
    /// The `Playwright` instance's epoch.
    pub(crate) epoch: &'a Epoch,
}

/// Which frame of the page an install is for.
#[derive(Clone, Debug)]
pub(crate) enum Place {
    /// The page's main frame: its first installed document takes the page's first seq.
    Main,
    /// A child frame, and how this hop reached it from its parent.
    Child(ParentLink),
    /// A child frame found in its session's frame tree rather than through a hop (the
    /// evaluate guard's sweep of the same-process frames). There is no iframe key to record,
    /// so no parent link is; a later hop into the frame records one.
    Descendant,
}

/// How a child frame was reached from its parent, recorded on every hop into it (the
/// resolver's and the snapshot's). A ref jump (`f7e3`) lands in document 7 without walking its
/// parents; an action there walks these links up to hit-check each parent and to place its
/// point in page coordinates. A hop records one even when no page snapshot visited the frame,
/// because `frameLocator('#f').locator('body').ariaSnapshot({ mode: 'ai' })` mints refs there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParentLink {
    /// The parent document.
    pub(crate) parent: FrameSeq,
    /// The `<iframe>` element in the parent's ref map: its aria ref after a snapshot hop, or
    /// a Neo-internal key (`#4`) the probe registered on a resolver hop. Neo's aria-ref
    /// engine resolves either for as long as that element lives, and never to another one.
    pub(crate) iframe_key: String,
}

/// One parent of a frame, as a ref jump finds it: the parent's world and its iframe's key.
#[derive(Clone, Debug)]
pub(crate) struct LinkedParent {
    pub(crate) world: SealedWorld,
    pub(crate) iframe_key: String,
}

/// What a frame seq names right now.
#[derive(Debug)]
pub(crate) enum SeqLookup {
    /// Its world, and its parents from the main frame down (empty for a frame no hop reached).
    Live {
        /// Boxed: a world is large, and the other variants carry nothing.
        world: Box<SealedWorld>,
        parents: Vec<LinkedParent>,
    },
    /// This page handed the seq out and its document is gone, or a parent's is: a ref into it
    /// fails at once (`element_detached`).
    Retired,
    /// This page never handed the seq out (`invalid_input`).
    Unknown,
}

/// Where a world lives: a frame inside a CDP session. The same frame reached through another
/// session is another key, hence another install (which finds the same world by name).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct WorldKey {
    session: SessionId,
    frame: FrameId,
}

/// One page's frame worlds. Owned by `PageState` for the page's lifetime. The lock is a std
/// mutex held only to read or swap entries, never across an await: installs run inside the
/// cells, outside it.
///
/// Invariants, and what keeps each:
/// - **One install per key at a time.** A key's `OnceCell` makes concurrent first uses share
///   one install; a failed or dropped install leaves it empty for the next caller.
/// - **A seq names one document forever.** Seqs come from one counter and are never handed
///   out twice; the world's reply wins over the candidate, and a world this instance booted
///   keeps its seq. A new document always gets a new seq, so an old `f7e3` can only resolve
///   in document 7 or fail. `document.open()` (setContent) keeps the Document, so it keeps its
///   seq, as upstream's frame does when no new document commits.
/// - **An earlier process's seqs stay retired.** A re-key reports the seq the world held
///   under another `Playwright`; the counter skips it for good, so a ref that process printed
///   into that document can never name a document of this one.
/// - **Liveness is learned from failures.** A cell leaves the map only once a call proved its
///   world gone ([`FrameWorlds::invalidate`]), and its seq retires with it. A caller still
///   holding the old cell finds the stale world, fails again and gets the new cell.
#[derive(Debug)]
pub(crate) struct FrameWorlds {
    inner: Mutex<WorldsInner>,
}

#[derive(Debug)]
struct WorldsInner {
    cells: HashMap<WorldKey, Arc<OnceCell<SealedWorld>>>,
    /// Live seqs, each with the key whose cell holds its world.
    live: HashMap<FrameSeq, WorldKey>,
    /// How each child document was last reached.
    parents: HashMap<FrameSeq, ParentLink>,
    /// The page's first seq, kept for the main frame's first installed document: 0, so its
    /// refs print bare, or a random base for a page that was open before this `Playwright`.
    first: u32,
    /// The next seq a later document gets. Everything in `first..next` was handed out, or
    /// skipped as foreign.
    next: u32,
    first_used: bool,
    /// Seqs an earlier `Playwright` gave documents of this tab, learned from the worlds this
    /// one re-keyed. None is handed out here, and a ref into one is stale (`Retired`).
    foreign: HashSet<FrameSeq>,
}

impl FrameWorlds {
    /// No worlds yet; the main frame's first document will take `first`.
    pub(crate) fn new(first: FrameSeq) -> Self {
        Self {
            inner: Mutex::new(WorldsInner {
                cells: HashMap::new(),
                live: HashMap::new(),
                parents: HashMap::new(),
                first: first.0,
                next: first.0.saturating_add(1),
                first_used: false,
                foreign: HashSet::new(),
            }),
        }
    }

    /// The world of `endpoint`'s frame, installing it on first use: the session's setup, then
    /// the seal, the engine and boot (`SealedWorld::install`). A child's parent link is
    /// recorded on every call, so a later ref jump into the frame finds its parents.
    pub(crate) async fn world(
        &self,
        installer: Installer<'_>,
        endpoint: &Endpoint,
        place: Place,
    ) -> Result<SealedWorld, ProbeFailure> {
        let Some(session) = endpoint.session.session_id() else {
            return Err(ProbeFailure::Broken(
                "a sealed world needs a target session".to_owned(),
            ));
        };
        let key = WorldKey {
            session: session.clone(),
            frame: endpoint.frame.clone(),
        };
        self.world_with(key, place, |candidate| async move {
            installer
                .sessions
                .ensure(&endpoint.session, endpoint.kind)
                .await?;
            // Subscribed here, before the create, so the new world's event cannot be missed.
            let watch = installer.sessions.watch_contexts();
            SealedWorld::install(
                endpoint.session.clone(),
                endpoint.frame.clone(),
                candidate,
                installer.epoch,
                watch,
            )
            .await
        })
        .await
    }

    /// The bookkeeping around one key's install; `install` runs only in the first caller.
    async fn world_with<F, Fut>(
        &self,
        key: WorldKey,
        place: Place,
        install: F,
    ) -> Result<SealedWorld, ProbeFailure>
    where
        F: FnOnce(FrameSeq) -> Fut,
        Fut: Future<Output = Result<Installed, ProbeFailure>>,
    {
        let main = matches!(place, Place::Main);
        let cell = Arc::clone(self.lock().cells.entry(key.clone()).or_default());
        let world = cell
            .get_or_try_init(move || async move {
                let candidate = self.lock().candidate(main);
                let Installed { world, retired } = install(candidate).await?;
                self.lock().adopt(&key, world.seq(), retired);
                Ok(world)
            })
            .await?
            .clone();
        if let Place::Child(link) = place {
            self.lock().parents.insert(world.seq(), link);
        }
        Ok(world)
    }

    /// A call on `world` failed `Gone`: its document is gone. Drops its cell, only if the cell
    /// still holds this very install (a newer one by another op is kept), and retires its seq,
    /// only if the seq still belongs to this key.
    pub(crate) fn invalidate(&self, world: &SealedWorld) {
        let Some(session) = world.session_id() else {
            return;
        };
        let key = WorldKey {
            session: session.clone(),
            frame: world.frame().clone(),
        };
        let mut inner = self.lock();
        let holds_it = inner
            .cells
            .get(&key)
            .and_then(|cell| cell.get())
            .is_some_and(|held| held.same_install(world));
        if holds_it {
            inner.cells.remove(&key);
        }
        if inner.live.get(&world.seq()) == Some(&key) {
            inner.live.remove(&world.seq());
        }
    }

    /// What `seq` names, for a ref jump: its world and its parents, or why there is none.
    pub(crate) fn by_seq(&self, seq: FrameSeq) -> SeqLookup {
        let inner = self.lock();
        let Some(world) = inner.live_world(seq) else {
            return inner.dead(seq);
        };
        let mut parents = Vec::new();
        let mut child = seq;
        // Frames form a tree, so the walk ends at the main frame; the bound only keeps a
        // corrupted table from spinning while the lock is held.
        for _ in 0..=inner.parents.len() {
            let Some(link) = inner.parents.get(&child) else {
                break;
            };
            let Some(parent) = inner.live_world(link.parent) else {
                // A child document cannot outlive its parent's.
                return SeqLookup::Retired;
            };
            parents.push(LinkedParent {
                world: parent,
                iframe_key: link.iframe_key.clone(),
            });
            child = link.parent;
        }
        parents.reverse();
        SeqLookup::Live {
            world: Box::new(world),
            parents,
        }
    }

    fn lock(&self) -> MutexGuard<'_, WorldsInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WorldsInner {
    /// The seq a new document would take: the page's first for the main frame's first
    /// install, else the next one, never a foreign one. Taken once, even if the install then
    /// fails.
    fn candidate(&mut self, main: bool) -> FrameSeq {
        if main && !self.first_used {
            self.first_used = true;
            if !self.foreign.contains(&FrameSeq(self.first)) {
                return FrameSeq(self.first);
            }
        }
        while self.foreign.contains(&FrameSeq(self.next)) {
            self.next = self.next.saturating_add(1);
        }
        let seq = self.next;
        self.next = self.next.saturating_add(1);
        FrameSeq(seq)
    }

    /// `key`'s install reported `seq`: the world's answer wins. A seq this page already knew
    /// under another key is the same document reached through another session, whose old
    /// handle is dropped. `retired` is the seq an earlier `Playwright` gave the world, which the
    /// boot replaced.
    fn adopt(&mut self, key: &WorldKey, seq: FrameSeq, retired: Option<FrameSeq>) {
        self.foreign.extend(retired);
        if let Some(previous) = self.live.insert(seq, key.clone())
            && previous != *key
        {
            self.cells.remove(&previous);
        }
        self.next = self.next.max(seq.0.saturating_add(1));
    }

    fn live_world(&self, seq: FrameSeq) -> Option<SealedWorld> {
        let key = self.live.get(&seq)?;
        self.cells.get(key)?.get().cloned()
    }

    fn dead(&self, seq: FrameSeq) -> SeqLookup {
        if (self.first..self.next).contains(&seq.0) || self.foreign.contains(&seq) {
            SeqLookup::Retired
        } else {
            SeqLookup::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::atomic::{AtomicU32, Ordering};

    use browseros_core::{FrameId, ProtocolSession, SessionId};
    use tokio::task;

    use super::{FrameWorlds, ParentLink, Place, SeqLookup, WorldKey};
    use crate::probe::{ProbeFailure, Ran};
    use crate::test_env;
    use crate::world::{FrameSeq, Installed, SealedWorld};

    type TestResult = Result<(), Box<dyn Error>>;

    fn session(id: &str) -> ProtocolSession {
        ProtocolSession::for_session(
            test_env::offline_session().connection(),
            SessionId::from(id),
        )
    }

    fn key(session: &str, frame: &str) -> WorldKey {
        WorldKey {
            session: SessionId::from(session),
            frame: FrameId(frame.to_owned()),
        }
    }

    fn child_of(parent: u32, iframe_key: &str) -> Place {
        Place::Child(ParentLink {
            parent: FrameSeq(parent),
            iframe_key: iframe_key.to_owned(),
        })
    }

    /// Installs as a fresh world would: it takes the candidate, and counts the installs.
    async fn install(
        worlds: &FrameWorlds,
        at: (&str, &str),
        place: Place,
        installs: &AtomicU32,
    ) -> Result<SealedWorld, ProbeFailure> {
        rekey(worlds, at, place, installs, None).await
    }

    /// Installs into a world another `Playwright` numbered `retired`: the boot re-keys it to
    /// the candidate and reports the old seq.
    async fn rekey(
        worlds: &FrameWorlds,
        at: (&str, &str),
        place: Place,
        installs: &AtomicU32,
        retired: Option<u32>,
    ) -> Result<SealedWorld, ProbeFailure> {
        let (session_id, frame) = at;
        worlds
            .world_with(key(session_id, frame), place, |candidate| async move {
                let count = installs.fetch_add(1, Ordering::SeqCst);
                task::yield_now().await;
                let world = SealedWorld::for_tests(
                    session(session_id),
                    frame,
                    candidate.0,
                    &format!("{session_id}.{frame}.{count}"),
                );
                Ok(Installed {
                    world,
                    retired: retired.map(FrameSeq),
                })
            })
            .await
    }

    fn installed(world: SealedWorld) -> Installed {
        Installed {
            world,
            retired: None,
        }
    }

    #[tokio::test]
    async fn tests_that_the_main_frame_takes_the_first_seq_and_children_the_next() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(0));
        let installs = AtomicU32::new(0);
        let child = install(&worlds, ("S", "child"), child_of(0, "#1"), &installs).await?;
        let main = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        assert_eq!(
            (main.seq(), child.seq()),
            (FrameSeq(0), FrameSeq(1)),
            "0 waits for the main frame even when a child installs first"
        );
        let SeqLookup::Live { world, parents } = worlds.by_seq(FrameSeq(1)) else {
            return Err("seq 1 is live".into());
        };
        assert!(world.same_install(&child));
        assert_eq!(
            parents.len(),
            1,
            "one parent between the child and the page"
        );
        assert!(parents[0].world.same_install(&main));
        assert_eq!(parents[0].iframe_key, "#1");
        assert!(matches!(
            worlds.by_seq(FrameSeq(0)),
            SeqLookup::Live { parents, .. } if parents.is_empty()
        ));
        assert!(matches!(worlds.by_seq(FrameSeq(2)), SeqLookup::Unknown));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_page_open_before_the_instance_numbers_from_its_base() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(347));
        let installs = AtomicU32::new(0);
        let main = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        let child = install(&worlds, ("S", "child"), child_of(347, "#1"), &installs).await?;
        assert_eq!((main.seq(), child.seq()), (FrameSeq(347), FrameSeq(348)));
        assert!(
            matches!(worlds.by_seq(FrameSeq(0)), SeqLookup::Unknown),
            "a bare e5 from before the restart names no document here"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_concurrent_first_uses_share_one_install() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(0));
        let installs = AtomicU32::new(0);
        let (first, second) = tokio::join!(
            install(&worlds, ("S", "main"), Place::Main, &installs),
            install(&worlds, ("S", "main"), Place::Main, &installs)
        );
        let (first, second) = (first?, second?);
        assert_eq!(installs.load(Ordering::SeqCst), 1, "one install, shared");
        assert!(first.same_install(&second));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_gone_world_retires_its_seq_for_good() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(0));
        let installs = AtomicU32::new(0);
        let first = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        worlds.invalidate(&first);
        assert!(matches!(worlds.by_seq(FrameSeq(0)), SeqLookup::Retired));
        let second = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        assert_eq!(
            second.seq(),
            FrameSeq(1),
            "a new document never reuses a seq"
        );
        worlds.invalidate(&first);
        assert!(
            matches!(worlds.by_seq(FrameSeq(1)), SeqLookup::Live { .. }),
            "a late failure on the old install leaves the new one alone"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_failed_install_leaves_the_cell_for_the_next_caller() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(0));
        let failed = worlds
            .world_with(key("S", "main"), Place::Main, |_| async {
                Err(ProbeFailure::Gone(Ran::No))
            })
            .await;
        assert!(matches!(failed, Err(ProbeFailure::Gone(_))));
        let installs = AtomicU32::new(0);
        let world = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        assert_eq!(
            world.seq(),
            FrameSeq(1),
            "the failed install's candidate stays spent"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_worlds_own_seq_wins_and_moves_with_its_session() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(0));
        let installs = AtomicU32::new(0);
        let main = install(&worlds, ("S1", "main"), Place::Main, &installs).await?;
        // The same document through another session: the world keeps the seq it has.
        let reached = worlds
            .world_with(key("S2", "main"), Place::Main, |_| async {
                Ok(installed(SealedWorld::for_tests(
                    session("S2"),
                    "main",
                    0,
                    "S2.main",
                )))
            })
            .await?;
        assert_eq!(reached.seq(), FrameSeq(0));
        worlds.invalidate(&main);
        assert!(
            matches!(worlds.by_seq(FrameSeq(0)), SeqLookup::Live { world, .. } if world.same_install(&reached)),
            "the old session's failure does not retire the document"
        );
        let later = worlds
            .world_with(key("S3", "late"), child_of(0, "#2"), |_| async {
                Ok(installed(SealedWorld::for_tests(
                    session("S3"),
                    "late",
                    9,
                    "S3.late",
                )))
            })
            .await?;
        assert_eq!(
            later.seq(),
            FrameSeq(9),
            "the reply's seq wins over the candidate"
        );
        let next = install(&worlds, ("S3", "next"), child_of(0, "#3"), &installs).await?;
        assert_eq!(next.seq(), FrameSeq(10), "and the counter moves past it");
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_frame_whose_parent_is_gone_is_retired() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(0));
        let installs = AtomicU32::new(0);
        let main = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        install(&worlds, ("S", "child"), child_of(0, "#1"), &installs).await?;
        worlds.invalidate(&main);
        assert!(
            matches!(worlds.by_seq(FrameSeq(1)), SeqLookup::Retired),
            "a child document cannot outlive its parent's"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_seq_an_earlier_process_gave_is_never_handed_out_again() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(500_000));
        let installs = AtomicU32::new(0);
        let main = rekey(&worlds, ("S", "main"), Place::Main, &installs, Some(0)).await?;
        assert_eq!(main.seq(), FrameSeq(500_000));
        assert!(
            matches!(worlds.by_seq(FrameSeq(0)), SeqLookup::Retired),
            "a bare e5 the earlier process printed into this document is stale"
        );
        // An earlier process whose numbering overlapped this one's: it gave a child document
        // the seq this page would hand out next.
        let child = rekey(
            &worlds,
            ("S", "child"),
            child_of(500_000, "#1"),
            &installs,
            Some(500_002),
        )
        .await?;
        assert_eq!(child.seq(), FrameSeq(500_001));
        let next = install(&worlds, ("S", "next"), child_of(500_000, "#2"), &installs).await?;
        assert_eq!(
            next.seq(),
            FrameSeq(500_003),
            "the counter skips the earlier process's seq"
        );
        assert!(matches!(
            worlds.by_seq(FrameSeq(500_002)),
            SeqLookup::Retired
        ));
        worlds.invalidate(&child);
        let navigated =
            install(&worlds, ("S", "child"), child_of(500_000, "#1"), &installs).await?;
        assert_eq!(navigated.seq(), FrameSeq(500_004));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_main_frame_skips_a_first_seq_an_earlier_process_gave() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(500_000));
        let installs = AtomicU32::new(0);
        let child = rekey(
            &worlds,
            ("S", "child"),
            child_of(500_000, "#1"),
            &installs,
            Some(500_000),
        )
        .await?;
        let main = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        assert_eq!(
            (child.seq(), main.seq()),
            (FrameSeq(500_001), FrameSeq(500_002)),
            "the page's first seq was the earlier process's, so the main frame takes the next"
        );
        assert!(matches!(
            worlds.by_seq(FrameSeq(500_000)),
            SeqLookup::Retired
        ));
        Ok(())
    }

    #[tokio::test]
    async fn returning_tab_keeps_state_and_never_restarts_frame_sequences() -> TestResult {
        use crate::op_scope::AdmittedPage;
        use crate::page::PageRegistry;
        use browseros_core::pages::PageSession;
        use browseros_core::{PageId, TabId, TargetId};
        use std::sync::Arc;
        let browser = test_env::offline_session();
        let registry = PageRegistry::new();
        let admitted = |id| AdmittedPage {
            tab: TabId(51),
            core_page: PageId(id),
            session: PageSession {
                target_id: TargetId("tab-51".to_owned()),
                session_id: SessionId::from("session-51"),
                session: browser.protocol_session(SessionId::from("session-51")),
                url: "https://example.test/".to_owned(),
            },
        };
        let before = registry.page(&browser, &admitted(1)).await;
        let first = install(
            before.frames(),
            ("S", "main"),
            Place::Main,
            &AtomicU32::new(0),
        )
        .await?;
        let after = registry.page(&browser, &admitted(2)).await;
        assert!(
            Arc::ptr_eq(&before, &after),
            "a core cache replacement is the same browser tab"
        );
        let next = install(
            after.frames(),
            ("S2", "next"),
            Place::Main,
            &AtomicU32::new(0),
        )
        .await?;
        assert!(
            next.seq().0 > first.seq().0,
            "old refs cannot alias a returning tab's new document"
        );
        Ok(())
    }
}
