//! One page's isolated worlds: which frame documents have one, their seqs, and how each child
//! frame was reached from its parent.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{BrowserSession, FrameId, ProtocolSession, SessionId, TargetId};
use neo_snapshot_apc::tree::{DocSeq, DocToken};
use tokio::sync::OnceCell;

use crate::probe::ProbeFailure;
use crate::world::doc_table::{FrameReport, PageUrlKind, RetiredDoc, SeqBinding};
use crate::world::frame_attach_log::{FrameAttachLog, FrameEvent, Taken};
use crate::world::{
    ContextWatch, DocBinding, DocContext, DocLookup, DocPlace, DocTable, Epoch, FrameSeq,
    Installed, IsolatedWorld, Stamp,
};

/// Where a world is installed: a frame on the raw session core's frame registry gave it. That is
/// the page's own session for the main frame and its same-process children, else an OOPIF's (or
/// the one an OOPIF's same-process child shares).
#[derive(Clone, Debug)]
pub(crate) struct Endpoint {
    pub(crate) session: ProtocolSession,
    pub(crate) frame: FrameId,
}

/// What every install on one page shares besides its endpoint. No `Debug`: the browser session
/// is not worth printing.
#[derive(Clone, Copy)]
pub(crate) struct Installer<'a> {
    /// The connection whose `Runtime` events an install watches (`ContextWatch`).
    pub(crate) browser: &'a BrowserSession,
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
    /// A child frame found in its session's frame tree rather than through a hop (a `frame.*`
    /// form's frame, or an element's). There is no iframe key to record, so no parent link is;
    /// a later hop into the frame records one.
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
    pub(crate) world: IsolatedWorld,
    pub(crate) iframe_key: String,
}

/// What a frame seq names right now.
#[derive(Debug)]
pub(crate) enum SeqLookup {
    /// Its world, and its parents from the main frame down (empty for a frame no hop reached).
    Live {
        /// Boxed: a world is large, and the other variants carry nothing.
        world: Box<IsolatedWorld>,
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
/// - **Seqs follow Playwright's order.** Upstream numbers a frame when it attaches, and
///   renumbers the main frame when it commits a new document after a real one (server/frames.ts
///   at v1.64.1), so `about:blank` to a page keeps `e5` and a page to another prints `f1e5`.
///   Here a seq is picked when a world first installs, so each pick first replays the tab's
///   [`FrameAttachLog`]: every frame attached since is numbered in attach order, and a child's
///   first document takes the seq its attach reserved (`f1` for the first iframe the page
///   attached, however the reads that install them race). Without a log (unit tests), every
///   later main document takes the next seq at install instead.
/// - **A real page proved gone counts as left, logged or not.** The watcher fills the log on a
///   task of its own, so an op can prove the main document gone and re-install before the
///   commit away from it is logged (a busy browser, or `location.href = …` and a ref at once).
///   Upstream renumbers on every commit that leaves a real page, so once a call proves a real
///   page's world gone, the next main document takes a new seq without waiting for the log.
///   Keeping the old seq would let `e2` from the old page name whatever a new snapshot calls
///   `e2`.
#[derive(Debug)]
pub(crate) struct FrameWorlds {
    inner: Mutex<WorldsInner>,
    /// Where the target watcher logs this tab's frames. Without one (unit tests) seqs follow
    /// install order alone.
    attach_log: Option<AttachLogBinding>,
}

/// One tab's place in the shared [`FrameAttachLog`].
#[derive(Debug)]
struct AttachLogBinding {
    log: Arc<FrameAttachLog>,
    tab: TargetId,
}

/// A main-frame document Neo installed a world in.
#[derive(Debug)]
struct MainDocument {
    /// The seq its world reported.
    seq: FrameSeq,
    /// Its URL when the world booted.
    url: String,
}

#[derive(Debug)]
struct WorldsInner {
    cells: HashMap<WorldKey, Arc<OnceCell<IsolatedWorld>>>,
    /// Live seqs, each with the key whose cell holds its world.
    live: HashMap<FrameSeq, WorldKey>,
    /// How each child document was last reached.
    parents: HashMap<FrameSeq, ParentLink>,
    /// The frame each seq's world was installed in, kept after the seq retires (ruling R63): the
    /// check call asks whether a dead ref's frame is still in the tab. A seq an earlier
    /// `Playwright` handed out has none, as nothing here knows where it was.
    frames: HashMap<FrameSeq, FrameId>,
    /// The page's first seq, kept for the main frame's first installed document: 0, so its
    /// refs print bare, or a random base for a page that was open before this `Playwright`.
    first: u32,
    /// The next seq a later document gets. Everything in `first..next` was handed out, or
    /// skipped as foreign.
    next: u32,
    /// Whether the main frame took `first` (or found it foreign).
    first_used: bool,
    /// Seqs an earlier `Playwright` gave documents of this tab, learned from the worlds this
    /// one re-keyed. None is handed out here, and a ref into one is stale (`Retired`).
    foreign: HashSet<FrameSeq>,
    /// Child frames numbered when they attached, until their first world installs.
    reserved: HashMap<FrameId, FrameSeq>,
    /// Child frames that took a seq, at attach or at install, so a late or repeated attach
    /// event never numbers one twice. A removed frame leaves it.
    numbered: HashSet<FrameId>,
    /// The main frame's seq, on a tab whose log reports its commits: every main document takes
    /// it, and a commit after a real URL moves it on. Unset until the first pick or move.
    main_seq: Option<FrameSeq>,
    /// The URL the main frame last committed, as the log reported it; none before the first.
    main_url: Option<String>,
    /// The main frame's last installed document. Once its seq is no longer live, a call proved
    /// the document gone.
    main_doc: Option<MainDocument>,
    /// Whether this tab has a log, so the main frame's seq follows its commits.
    follows_commits: bool,
    /// The log's gap count at the last replay.
    seen_gaps: u64,
    /// APC document identity table sharing this mutex so seq reservation, world binding, and
    /// retirement stay atomic with `FrameAttachLog` replay and `invalidate`.
    docs: DocTable,
}

impl FrameWorlds {
    /// No worlds yet; the main frame's first document will take `first`.
    pub(crate) fn new(first: FrameSeq) -> Self {
        Self {
            inner: Mutex::new(WorldsInner {
                cells: HashMap::new(),
                live: HashMap::new(),
                parents: HashMap::new(),
                frames: HashMap::new(),
                first: first.0,
                next: first.0.saturating_add(1),
                first_used: false,
                foreign: HashSet::new(),
                reserved: HashMap::new(),
                numbered: HashSet::new(),
                main_seq: None,
                main_url: None,
                main_doc: None,
                follows_commits: false,
                seen_gaps: 0,
                docs: DocTable::new(),
            }),
            attach_log: None,
        }
    }

    /// The same, numbering the frames of `tab` in the order `log` saw them attach.
    pub(crate) fn with_attach_log(mut self, log: Arc<FrameAttachLog>, tab: TargetId) -> Self {
        self.attach_log = Some(AttachLogBinding { log, tab });
        self.lock().follows_commits = true;
        self
    }

    /// The world of `endpoint`'s frame, installing it on first use: the engine and boot
    /// (`IsolatedWorld::install`). A child's parent link is
    /// recorded on every call, so a later ref jump into the frame finds its parents.
    pub(crate) async fn world(
        &self,
        installer: Installer<'_>,
        endpoint: &Endpoint,
        place: Place,
    ) -> Result<IsolatedWorld, ProbeFailure> {
        let Some(session) = endpoint.session.session_id() else {
            return Err(ProbeFailure::Broken(
                "an isolated world needs a target session".to_owned(),
            ));
        };
        let key = WorldKey {
            session: session.clone(),
            frame: endpoint.frame.clone(),
        };
        self.world_with(key, place, |candidate| async move {
            // Subscribed here, before the create, so the new world's event cannot be missed. No
            // `Runtime.enable` for it: the watch reads only what a `Runtime` lease someone else
            // holds on the session already reports (`IsolatedWorld::install`).
            let watch = ContextWatch::new(installer.browser.connection().events());
            IsolatedWorld::install(
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
    ) -> Result<IsolatedWorld, ProbeFailure>
    where
        F: FnOnce(FrameSeq) -> Fut,
        Fut: Future<Output = Result<Installed, ProbeFailure>>,
    {
        let main = matches!(place, Place::Main);
        let cell = Arc::clone(self.lock().cells.entry(key.clone()).or_default());
        let world = cell
            .get_or_try_init(move || async move {
                let candidate = self.candidate(&key.frame, main);
                let Installed {
                    world,
                    retired,
                    url,
                } = install(candidate).await?;
                let mut inner = self.lock();
                inner.adopt(&key, world.seq(), retired);
                if main {
                    inner.main_installed(world.seq(), url);
                }
                drop(inner);
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
    /// still holds this very install (a newer one by another op is kept), retires its seq only if
    /// the seq still belongs to this key, and retires any APC [`DocToken`] bound to this install
    /// (reported through [`Self::take_retired_docs`]).
    pub(crate) fn invalidate(&self, world: &IsolatedWorld) {
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
        inner.docs.retire_by_world(world);
    }

    /// Takes the seq of every document life `DocTable` retired since the last call, whichever path
    /// retired it: an invalidated world, a capture that displaced a frame's document, or an
    /// explicit [`Self::retire_doc`]. `TabTree` drops the refs printed under them.
    #[must_use = "a dropped seq keeps that document life's refs live"]
    pub(crate) fn take_retired_seqs(&self) -> Vec<FrameSeq> {
        self.lock().docs.take_retired()
    }

    /// Returns all currently live cached worlds so a capture can check `installed_at <
    /// requested_at` and prove them with `Describe` (design section 6.3).
    pub(crate) fn cached_worlds(&self) -> Vec<IsolatedWorld> {
        let inner = self.lock();
        inner
            .cells
            .values()
            .filter_map(|cell| cell.get().cloned())
            .collect()
    }

    /// Returns a live cached world for `frame`, preferring the most recent install if two
    /// sessions transiently hold cells for the same frame.
    #[cfg(test)]
    pub(crate) fn cached_world_for_frame(&self, frame: &FrameId) -> Option<IsolatedWorld> {
        let inner = self.lock();
        inner
            .cells
            .iter()
            .filter(|(key, _)| key.frame == *frame)
            .filter_map(|(_, cell)| cell.get())
            .max_by_key(|world| world.installed_at())
            .cloned()
    }

    /// Atomically replays [`FrameAttachLog`] and resolves, promotes, or reserves the [`DocSeq`]
    /// for an APC document (design section 6.3).
    ///
    /// Captures of one tab can be in flight together, so reports arrive in any order. A report
    /// older than its frame's last one changes nothing: the capture saw a document the frame has
    /// since left, and acting on it would retire the frame's live document for good.
    pub(crate) fn seq_for_doc(
        &self,
        ctx: DocContext<'_>,
        proven: &HashMap<FrameId, IsolatedWorld>,
    ) -> DocSeq {
        let mut inner = self.lock();
        self.replay_attach_log(&mut inner);
        let report = ctx.frame.map_or(FrameReport::Current, |frame| {
            inner.docs.report_frame(frame, ctx.requested_at)
        });

        match inner.docs.binding(ctx.token) {
            Some(binding) if report == FrameReport::Stale => {
                return DocSeq::new(binding.seq().0);
            }
            Some(DocBinding::Bound { seq, world }) => {
                let seq = *seq;
                let lookup_frame = ctx.frame.unwrap_or_else(|| world.frame()).clone();
                let existing = world.clone();
                inner.docs.note_url(ctx.token, ctx.url);
                if let Some(candidate) = proven.get(&lookup_frame)
                    && !candidate.same_install(&existing)
                    && candidate.installed_at() > existing.installed_at()
                    && !inner
                        .docs
                        .is_world_bound_to_other_active(candidate, ctx.token)
                {
                    let _ =
                        inner
                            .docs
                            .bind_existing(ctx.token, candidate.clone(), ctx.requested_at);
                    inner.frames.insert(seq, candidate.frame().clone());
                }
                return DocSeq::new(seq.0);
            }
            Some(DocBinding::Reserved { seq, frame: saved }) => {
                let seq = *seq;
                let effective_frame = ctx.frame.or(saved.as_ref()).cloned();
                let frame_was_missing = saved.is_none();
                inner.docs.note_url(ctx.token, ctx.url);
                if let Some(frame) = &effective_frame
                    && let Some(world) = proven.get(frame)
                    && !inner.docs.is_world_bound_to_other_active(world, ctx.token)
                {
                    let _ = inner
                        .docs
                        .bind_existing(ctx.token, world.clone(), ctx.requested_at);
                    inner.frames.insert(seq, world.frame().clone());
                } else if frame_was_missing && let Some(frame) = ctx.frame {
                    inner
                        .docs
                        .set_reserved_frame(ctx.token, frame.clone(), ctx.requested_at);
                    inner.frames.insert(seq, frame.clone());
                }
                if ctx.place == DocPlace::Main {
                    inner.record_initial_main_url(ctx.url);
                }
                return DocSeq::new(seq.0);
            }
            // Retired before this capture was requested, yet the capture reports it: the document
            // came back, as a page restored from the back/forward cache does with its token. It
            // takes a fresh seq below, like a new document; its old seq stays retired.
            Some(DocBinding::Retired { at, .. }) if *at < ctx.requested_at => {}
            Some(DocBinding::Retired { seq, .. }) => {
                return DocSeq::new(seq.0);
            }
            None => {}
        }

        if report == FrameReport::Stale {
            // Only a stale capture has seen this document, so it was gone before the frame's
            // newer report: it starts retired and displaces nothing.
            let seq = inner.next_seq();
            inner
                .docs
                .insert_retired(ctx.token.clone(), seq, ctx.url, ctx.requested_at);
            return DocSeq::new(seq.0);
        }

        DocSeq::new(inner.admit_doc(ctx, proven).0)
    }

    /// Binds a proven [`IsolatedWorld`] to `token`, preserving any previously reserved
    /// [`FrameSeq`]. Returns `None` if `token` is already `Retired`.
    pub(crate) fn bind_proven_world(
        &self,
        token: &DocToken,
        world: &IsolatedWorld,
    ) -> Option<FrameSeq> {
        let mut inner = self.lock();
        // The hydration that called this proved `world` alive and holding `token`'s node, so
        // `token` was the frame's document from the world's install until that proof (design 6.2).
        // Its evidence dates from the install, not from when its reply was processed: a capture
        // requested after the install that reported another document saw the frame after `token`
        // left it, and must win.
        let since = world.installed_at();
        if inner.docs.reported_other_since(world.frame(), token, since) {
            return None;
        }
        match inner.docs.binding(token) {
            Some(DocBinding::Retired { .. }) => None,
            Some(DocBinding::Reserved { .. } | DocBinding::Bound { .. }) => {
                let seq = inner.docs.bind_existing(token, world.clone(), since)?;
                inner.frames.insert(seq, world.frame().clone());
                let _ = inner.docs.report_frame(world.frame(), since);
                Some(seq)
            }
            None => {
                let place = if inner.main_seq == Some(world.seq())
                    || inner
                        .main_doc
                        .as_ref()
                        .is_some_and(|doc| doc.seq == world.seq())
                {
                    DocPlace::Main
                } else {
                    DocPlace::Child
                };
                let seq = inner.resolve_doc_seq(world.seq(), place, token, since);
                inner
                    .docs
                    .insert_bound(token.clone(), seq, world.clone(), None, since);
                inner.frames.insert(seq, world.frame().clone());
                inner.next = inner.next.max(seq.0.saturating_add(1));
                let _ = inner.docs.report_frame(world.frame(), since);
                Some(seq)
            }
        }
    }

    /// Resolves `token` to its live bound world and parent chain, reservation, retirement, or
    /// unknown status.
    pub(crate) fn lookup_doc(&self, token: &DocToken) -> DocLookup {
        let inner = self.lock();
        let Some(binding) = inner.docs.binding(token) else {
            return DocLookup::Unknown;
        };
        inner.resolve_binding(binding)
    }

    /// Resolves `seq` through [`DocTable`], falling back to `WorldsInner::dead` for seqs never
    /// recorded in `DocTable` (such as foreign seqs from an earlier `Playwright` process).
    pub(crate) fn lookup_doc_seq(&self, seq: FrameSeq) -> DocLookup {
        let inner = self.lock();
        match inner.docs.binding_by_seq(seq) {
            Some(SeqBinding::Current(binding)) => return inner.resolve_binding(binding),
            Some(SeqBinding::Past) => return DocLookup::Retired,
            None => {}
        }
        match inner.dead(seq) {
            SeqLookup::Retired => DocLookup::Retired,
            SeqLookup::Unknown | SeqLookup::Live { .. } => DocLookup::Unknown,
        }
    }

    /// Transitions an active (`Reserved` or `Bound`) document to `Retired` when a fresh capture
    /// proves it is no longer present (design section 6.2 case 4).
    pub(crate) fn retire_doc(&self, token: &DocToken) -> Option<FrameSeq> {
        self.lock()
            .docs
            .retire_token(token, Stamp::next())
            .map(|retired| retired.seq)
    }

    /// The frame `seq`'s world was installed in, live or retired; `None` for a seq this page never
    /// installed a world for, as one an earlier `Playwright` handed out (ruling R63).
    pub(crate) fn frame_of(&self, seq: FrameSeq) -> Option<FrameId> {
        self.lock().frames.get(&seq).cloned()
    }

    /// What `seq` names, for a ref jump: its world and its parents, or why there is none.
    pub(crate) fn by_seq(&self, seq: FrameSeq) -> SeqLookup {
        let inner = self.lock();
        let Some(world) = inner.live_world(seq) else {
            return inner.dead(seq);
        };
        let Some(parents) = inner.parent_chain(seq) else {
            return SeqLookup::Retired;
        };
        SeqLookup::Live {
            world: Box::new(world),
            parents,
        }
    }

    /// The seq `frame`'s next install takes, after the tab's log is replayed. The log is taken
    /// and replayed under this page's lock, so of two installs that race, the one that takes it
    /// numbers every frame in it before the other picks. The lock order is the page's, then the
    /// log's; the watcher only ever holds the log's.
    fn candidate(&self, frame: &FrameId, main: bool) -> FrameSeq {
        let mut inner = self.lock();
        self.replay_attach_log(&mut inner);
        let place = if main {
            DocPlace::Main
        } else {
            DocPlace::Child
        };
        inner.candidate_for(Some(frame), place)
    }

    fn replay_attach_log(&self, inner: &mut WorldsInner) {
        if let Some(AttachLogBinding { log, tab }) = &self.attach_log {
            inner.replay(log.take(tab));
        }
    }

    fn lock(&self) -> MutexGuard<'_, WorldsInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WorldsInner {
    /// The seq a new document would take. The main frame: its seq as its commits left it,
    /// starting at the page's first, or the next one once the real page under it is proved
    /// gone; without a log, the first for its first install and the next for every later one.
    /// A child frame: the seq its attach reserved for its first document, else the next one.
    /// Never a foreign one; taken even if the install then fails.
    fn candidate_for(&mut self, frame: Option<&FrameId>, place: DocPlace) -> FrameSeq {
        match place {
            DocPlace::Main => {
                if self.follows_commits {
                    let seq = match self.main_seq {
                        Some(seq) if self.left_real_page(seq) => self.next_seq(),
                        Some(seq) => seq,
                        None => self.first_main_seq(),
                    };
                    self.main_seq = Some(seq);
                    return seq;
                }
                if !self.first_used {
                    return self.first_main_seq();
                }
                self.next_seq()
            }
            DocPlace::Child => match frame {
                Some(frame) => {
                    self.numbered.insert(frame.clone());
                    self.reserved
                        .remove(frame)
                        .unwrap_or_else(|| self.next_seq())
                }
                None => self.next_seq(),
            },
        }
    }

    /// Admits a document `DocTable` holds no active entry for: a new token, or one that came back
    /// after it retired. It displaces the frame's previous document, then binds to the frame's
    /// proven world or reserves a seq by the attach-order `candidate` rule.
    fn admit_doc(
        &mut self,
        ctx: DocContext<'_>,
        proven: &HashMap<FrameId, IsolatedWorld>,
    ) -> FrameSeq {
        // A new DocToken on a known frame displaces any previous active document on that frame.
        // Record whether the replaced doc already owned the proven world so we never bind the new
        // document to the previous document's install.
        let mut world_belongs_to_replaced = false;
        if let Some(frame) = ctx.frame
            && let Some(old_token) = self.docs.active_for_frame(frame).cloned()
        {
            if let Some(world) = proven.get(frame)
                && matches!(
                    self.docs.binding(&old_token),
                    Some(DocBinding::Bound { world: bound, .. }) if bound.same_install(world)
                )
            {
                world_belongs_to_replaced = true;
            }
            if let Some(RetiredDoc {
                seq: old_seq,
                url_kind: PageUrlKind::RealOrUnknown,
            }) = self.docs.retire_token(&old_token, ctx.requested_at)
                && ctx.place == DocPlace::Main
                && self.main_seq == Some(old_seq)
            {
                self.main_seq = Some(self.next_seq());
            }
        }

        if !world_belongs_to_replaced
            && let Some(frame) = ctx.frame
            && let Some(world) = proven.get(frame)
            && !self.docs.is_world_bound_to_other_active(world, ctx.token)
        {
            let seq = self.resolve_doc_seq(world.seq(), ctx.place, ctx.token, ctx.requested_at);
            self.docs.insert_bound(
                ctx.token.clone(),
                seq,
                world.clone(),
                ctx.url,
                ctx.requested_at,
            );
            self.frames.insert(seq, world.frame().clone());
            self.next = self.next.max(seq.0.saturating_add(1));
            if ctx.place == DocPlace::Main {
                if self.follows_commits && self.main_seq.is_none() {
                    self.main_seq = Some(seq);
                }
                self.record_initial_main_url(ctx.url);
            }
            return seq;
        }

        let candidate = self.candidate_for(ctx.frame, ctx.place);
        let seq = self.resolve_doc_seq(candidate, ctx.place, ctx.token, ctx.requested_at);
        self.docs.insert_reserved(
            ctx.token.clone(),
            seq,
            ctx.frame.cloned(),
            ctx.url,
            ctx.requested_at,
        );
        if let Some(frame) = ctx.frame {
            self.frames.insert(seq, frame.clone());
        }
        self.next = self.next.max(seq.0.saturating_add(1));
        if ctx.place == DocPlace::Main {
            self.record_initial_main_url(ctx.url);
        }
        seq
    }

    /// Enforces the `DocTable` non-aliasing invariant on a candidate seq: reuse of a seq a document
    /// already held is permitted only for a main-frame document replacing an explicit
    /// `"about:blank"` document (Playwright AR-053), which retires as of `at`. A document that comes
    /// back never takes its own old seq, which stays retired.
    fn resolve_doc_seq(
        &mut self,
        candidate: FrameSeq,
        place: DocPlace,
        new_token: &DocToken,
        at: Stamp,
    ) -> FrameSeq {
        if let Some(previous) = self.docs.token_for_seq(candidate) {
            if previous != new_token
                && place == DocPlace::Main
                && self.docs.is_about_blank(previous)
            {
                let previous = previous.clone();
                let _ = self.docs.retire_token(&previous, at);
                return candidate;
            }
            let fresh = self.next_seq();
            if place == DocPlace::Main {
                self.main_seq = Some(fresh);
            }
            return fresh;
        }
        candidate
    }

    /// Seeds `main_url` from an APC capture when no commit or world boot has recorded one yet,
    /// so a later `MainCommitted` knows whether the main frame left a real page or `about:blank`.
    fn record_initial_main_url(&mut self, url: Option<&str>) {
        if self.main_url.is_none()
            && let Some(url) = url
            && !url.is_empty()
        {
            self.main_url = Some(url.to_owned());
        }
    }

    /// Numbers what the tab's frames did since the last pick, in the order they did it, as
    /// upstream's frame manager numbers them as the events arrive. A gap in the log since the
    /// last replay may have hidden a commit away from a real page, so the main frame moves on to
    /// a new seq as if it had seen one, and forgets which URL it is on.
    fn replay(&mut self, taken: Taken) {
        let Taken { events, gaps } = taken;
        if gaps != self.seen_gaps {
            self.seen_gaps = gaps;
            self.lost_track();
        }
        for event in events {
            match event {
                FrameEvent::Attached(frame) => {
                    if self.numbered.insert(frame.clone()) {
                        let seq = self.next_seq();
                        self.reserved.insert(frame, seq);
                    }
                }
                FrameEvent::Removed(frame) => {
                    self.numbered.remove(&frame);
                    self.reserved.remove(&frame);
                }
                // Upstream: `if (frame === this._mainFrame && previousUrl && previousUrl !==
                // 'about:blank') frame.seq = this._allocateFrameSeq();`
                FrameEvent::MainCommitted { url } => {
                    let left = self.main_url.replace(url);
                    if left.is_some_and(|left| is_real_page(&left)) {
                        self.main_seq = Some(self.next_seq());
                    }
                }
                FrameEvent::Skipped(spent) => {
                    for _ in 0..spent {
                        self.next_seq();
                    }
                }
                FrameEvent::Lost => self.lost_track(),
            }
        }
    }

    /// Some of the tab's events are lost, a commit away from a real page perhaps: the main
    /// frame's next document takes a new seq, and the URL it is on is learned again.
    fn lost_track(&mut self) {
        self.main_url = None;
        if self.main_seq.is_some() {
            self.main_seq = Some(self.next_seq());
        }
    }

    /// A main document installed with its world at `url`. With no commit seen since the log
    /// started (a tab loaded before the watcher, or after a gap), that is the page the next
    /// commit leaves, so a real page there renumbers as upstream's frame, which knows its URL
    /// from the frame tree. The document, by seq and URL, is also what a later proof of its
    /// world's death leaves ([`Self::left_real_page`]).
    fn main_installed(&mut self, seq: FrameSeq, url: String) {
        if self.main_url.is_none() {
            self.main_url = Some(url.clone());
        }
        self.main_doc = Some(MainDocument { seq, url });
    }

    /// Whether `seq` was the main frame's real page and a call proved its world gone since
    /// (`invalidate` took it out of `live`): the main frame left a real page, logged or not.
    /// A world gone without a new document (a lost session) re-installs into the same world,
    /// whose reply keeps the old seq, so moving on costs at most an unused number. When the
    /// late commit is replayed it moves the main frame on once more, which also only skips a
    /// number. A blank page left this way keeps its seq, as upstream does.
    fn left_real_page(&self, seq: FrameSeq) -> bool {
        !self.live.contains_key(&seq)
            && (self
                .main_doc
                .as_ref()
                .is_some_and(|doc| doc.seq == seq && is_real_page(&doc.url))
                || self.docs.is_retired_real_seq(seq))
    }

    /// The page's first seq for the main frame, unless an earlier `Playwright` gave it out.
    fn first_main_seq(&mut self) -> FrameSeq {
        self.first_used = true;
        if self.foreign.contains(&FrameSeq(self.first)) {
            self.next_seq()
        } else {
            FrameSeq(self.first)
        }
    }

    /// The next seq of the counter, never a foreign one.
    fn next_seq(&mut self) -> FrameSeq {
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
        self.frames.insert(seq, key.frame.clone());
        if let Some(previous) = self.live.insert(seq, key.clone())
            && previous != *key
        {
            self.cells.remove(&previous);
        }
        self.next = self.next.max(seq.0.saturating_add(1));
    }

    fn live_world(&self, seq: FrameSeq) -> Option<IsolatedWorld> {
        let key = self.live.get(&seq)?;
        self.cells.get(key)?.get().cloned()
    }

    fn parent_chain(&self, start: FrameSeq) -> Option<Vec<LinkedParent>> {
        let mut parents = Vec::new();
        let mut child = start;
        // Frames form a tree, so the walk ends at the main frame; the bound only keeps a
        // corrupted table from spinning while the lock is held.
        for _ in 0..=self.parents.len() {
            let Some(link) = self.parents.get(&child) else {
                break;
            };
            // A child document cannot outlive its parent's.
            let parent = self.live_world(link.parent)?;
            parents.push(LinkedParent {
                world: parent,
                iframe_key: link.iframe_key.clone(),
            });
            child = link.parent;
        }
        parents.reverse();
        Some(parents)
    }

    fn is_live_install(&self, world: &IsolatedWorld) -> bool {
        let Some(session) = world.session_id() else {
            return false;
        };
        let key = WorldKey {
            session: session.clone(),
            frame: world.frame().clone(),
        };
        self.cells
            .get(&key)
            .and_then(|cell| cell.get())
            .is_some_and(|held| held.same_install(world))
    }

    fn resolve_binding(&self, binding: &DocBinding) -> DocLookup {
        match binding {
            DocBinding::Bound { seq, world } => {
                if !self.is_live_install(world) {
                    return DocLookup::Retired;
                }
                let Some(parents) = self.parent_chain(world.seq()) else {
                    return DocLookup::Retired;
                };
                DocLookup::Bound {
                    seq: *seq,
                    world: Box::new(world.clone()),
                    parents,
                }
            }
            DocBinding::Reserved { seq, frame } => DocLookup::Reserved {
                seq: *seq,
                frame: frame.clone(),
            },
            DocBinding::Retired { .. } => DocLookup::Retired,
        }
    }

    fn dead(&self, seq: FrameSeq) -> SeqLookup {
        if (self.first..self.next).contains(&seq.0) || self.foreign.contains(&seq) {
            SeqLookup::Retired
        } else {
            SeqLookup::Unknown
        }
    }
}

/// Upstream's test for a page the main frame renumbers on leaving: `previousUrl &&
/// previousUrl !== 'about:blank'` (server/frames.ts).
fn is_real_page(url: &str) -> bool {
    !url.is_empty() && url != "about:blank"
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use browseros_core::{FrameId, ProtocolSession, SessionId, TargetId};
    use neo_snapshot_apc::tree::{DocSeq, DocToken};
    use tokio::task;

    use super::{FrameWorlds, ParentLink, Place, SeqLookup, WorldKey};
    use crate::probe::{ProbeFailure, Ran};
    use crate::test_env;
    use crate::world::{
        DocContext, DocLookup, DocPlace, FrameAttachLog, FrameEvent, FrameSeq, Installed,
        IsolatedWorld, Stamp,
    };

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
    ) -> Result<IsolatedWorld, ProbeFailure> {
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
    ) -> Result<IsolatedWorld, ProbeFailure> {
        boot(worlds, at, place, installs, retired, "about:blank").await
    }

    /// Installs into a document at `url`, as its boot reports it.
    async fn boot(
        worlds: &FrameWorlds,
        at: (&str, &str),
        place: Place,
        installs: &AtomicU32,
        retired: Option<u32>,
        url: &str,
    ) -> Result<IsolatedWorld, ProbeFailure> {
        let (session_id, frame) = at;
        worlds
            .world_with(key(session_id, frame), place, |candidate| async move {
                let count = installs.fetch_add(1, Ordering::SeqCst);
                task::yield_now().await;
                let world = IsolatedWorld::for_tests(
                    session(session_id),
                    frame,
                    candidate.0,
                    &format!("{session_id}.{frame}.{count}"),
                );
                Ok(Installed {
                    world,
                    retired: retired.map(FrameSeq),
                    url: url.to_owned(),
                })
            })
            .await
    }

    fn installed(world: IsolatedWorld) -> Installed {
        Installed {
            world,
            retired: None,
            url: "about:blank".to_owned(),
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

    /// A page numbering by `log`'s record of the tab `tab`, from 0.
    fn logged(log: &Arc<FrameAttachLog>) -> FrameWorlds {
        FrameWorlds::new(FrameSeq(0)).with_attach_log(Arc::clone(log), TargetId("tab".to_owned()))
    }

    fn record(log: &FrameAttachLog, events: impl IntoIterator<Item = FrameEvent>) {
        for event in events {
            log.record(&TargetId("tab".to_owned()), event);
        }
    }

    fn attached(frame: &str) -> FrameEvent {
        FrameEvent::Attached(FrameId(frame.to_owned()))
    }

    /// AR-052's page: two iframes, then two inside the first. Upstream numbers them as they
    /// attach; the snapshot's concurrent reads install them in any order.
    #[tokio::test]
    async fn tests_that_children_take_the_seqs_their_attach_reserved_in_any_install_order()
    -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        record(&log, ["two", "one", "uno", "dos"].map(attached));
        let main = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        let mut seqs = Vec::new();
        for frame in ["uno", "two", "dos", "one"] {
            let world = install(&worlds, ("S", frame), child_of(0, "#1"), &installs).await?;
            seqs.push((frame, world.seq().0));
        }
        assert_eq!(main.seq(), FrameSeq(0));
        assert_eq!(seqs, [("uno", 3), ("two", 1), ("dos", 4), ("one", 2)]);
        Ok(())
    }

    fn committed(url: &str) -> FrameEvent {
        FrameEvent::MainCommitted {
            url: url.to_owned(),
        }
    }

    /// Upstream renumbers the main frame when it leaves a real page, before the new document
    /// attaches its frames, even if the iframe's world installs first.
    #[tokio::test]
    async fn tests_that_a_main_commit_numbers_the_main_frame_before_the_frames_it_attaches()
    -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        record(&log, [committed("https://a.test/")]);
        let first = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        record(&log, [committed("https://b.test/"), attached("child")]);
        worlds.invalidate(&first);
        let child = install(&worlds, ("S", "child"), child_of(1, "#1"), &installs).await?;
        let main = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        assert_eq!(
            (first.seq(), main.seq(), child.seq()),
            (FrameSeq(0), FrameSeq(1), FrameSeq(2))
        );
        Ok(())
    }

    /// AR-053 and server/frames.ts: the first document after `about:blank` keeps the main
    /// frame's seq, so its refs print bare; leaving a real page, `about:blank` included as the
    /// page left, renumbers.
    #[tokio::test]
    async fn tests_that_the_main_frame_keeps_its_seq_until_it_leaves_a_real_page() -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        let mut seqs = Vec::new();
        for commits in [
            vec![],
            vec![committed("about:blank"), committed("https://a.test/")],
            vec![committed("https://a.test/")],
            vec![committed("about:blank"), committed("https://b.test/")],
        ] {
            record(&log, commits);
            let main = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
            seqs.push(main.seq().0);
            worlds.invalidate(&main);
        }
        assert_eq!(seqs, [0, 0, 1, 2]);
        Ok(())
    }

    /// The log only orders seqs: an install that ran ahead of its attach event keeps the seq
    /// it took, the late event numbers nothing, and a frame the log never names takes the next.
    #[tokio::test]
    async fn tests_that_an_attach_seen_after_its_install_numbers_nothing() -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        let early = install(&worlds, ("S", "early"), child_of(0, "#1"), &installs).await?;
        record(
            &log,
            [
                attached("early"),
                attached("gone"),
                FrameEvent::Removed(FrameId("gone".to_owned())),
                attached("late"),
            ],
        );
        let late = install(&worlds, ("S", "late"), child_of(0, "#2"), &installs).await?;
        let unlogged = install(&worlds, ("S", "unlogged"), child_of(0, "#3"), &installs).await?;
        assert_eq!(
            (early.seq(), late.seq(), unlogged.seq()),
            (FrameSeq(1), FrameSeq(3), FrameSeq(4)),
            "the removed frame's seq stays handed out"
        );
        Ok(())
    }

    /// A tab loaded before its commits were logged: the page Neo booted in is the one the
    /// first logged commit leaves, so leaving a real page renumbers, and leaving a blank one
    /// does not.
    #[tokio::test]
    async fn tests_that_the_first_logged_commit_leaves_the_page_the_world_booted_in() -> TestResult
    {
        let installs = AtomicU32::new(0);
        let mut seqs = Vec::new();
        for booted_in in ["https://old.test/", "about:blank"] {
            let log = Arc::new(FrameAttachLog::default());
            let worlds = logged(&log);
            let main = boot(
                &worlds,
                ("S", "main"),
                Place::Main,
                &installs,
                None,
                booted_in,
            )
            .await?;
            record(&log, [committed("https://new.test/")]);
            worlds.invalidate(&main);
            let next = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
            seqs.push((main.seq().0, next.seq().0));
        }
        assert_eq!(seqs, [(0, 1), (0, 0)]);
        Ok(())
    }

    /// Boots the main frame's world in a document at `url`.
    async fn main_at(
        worlds: &FrameWorlds,
        installs: &AtomicU32,
        url: &str,
    ) -> Result<IsolatedWorld, ProbeFailure> {
        boot(worlds, ("S", "main"), Place::Main, installs, None, url).await
    }

    /// The watcher has not logged the commit yet when an op proves the old page gone and
    /// re-installs (a busy browser, or `location.href = …` and a ref at once). Leaving a real
    /// page still takes a new seq, so the old page's `e2` stays retired; leaving a blank one
    /// keeps it, as upstream does. The late commit then hands out only unused seqs.
    #[tokio::test]
    async fn tests_that_a_real_page_proved_gone_moves_the_main_frame_on_before_the_log_does()
    -> TestResult {
        let installs = AtomicU32::new(0);
        let mut seqs = Vec::new();
        for left in ["https://old.test/", "about:blank"] {
            let log = Arc::new(FrameAttachLog::default());
            let worlds = logged(&log);
            record(&log, [committed(left)]);
            let old = main_at(&worlds, &installs, left).await?;
            worlds.invalidate(&old);
            let new = main_at(&worlds, &installs, "https://new.test/").await?;
            seqs.push((old.seq().0, new.seq().0));

            record(&log, [committed("https://new.test/"), attached("child")]);
            let into_new = child_of(new.seq().0, "#1");
            let child = install(&worlds, ("S", "child"), into_new, &installs).await?;
            assert!(child.seq() > new.seq(), "{child:?}");
        }
        assert_eq!(seqs, [(0, 1), (0, 0)]);
        Ok(())
    }

    /// A watcher that fell behind may have missed a commit away from a real page, so the next
    /// main document takes a new seq rather than risk sharing the old one's.
    #[tokio::test]
    async fn tests_that_a_gap_in_the_log_moves_the_main_frame_on() -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        let before = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        log.lost_events();
        worlds.invalidate(&before);
        let after = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        assert_eq!((before.seq(), after.seq()), (FrameSeq(0), FrameSeq(1)));
        Ok(())
    }

    /// Events a full queue had to drop may have held a main commit: the next main document
    /// takes a new seq.
    #[tokio::test]
    async fn tests_that_events_a_full_queue_dropped_move_the_main_frame_on() -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        let before = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        record(&log, [FrameEvent::Lost]);
        worlds.invalidate(&before);
        let after = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        assert_eq!((before.seq(), after.seq()), (FrameSeq(0), FrameSeq(1)));
        Ok(())
    }

    /// A compacted queue spends the removed frames' seqs, so a later frame keeps upstream's
    /// number.
    #[tokio::test]
    async fn tests_that_skipped_frames_still_spend_their_seqs() -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        record(&log, [FrameEvent::Skipped(5), attached("child")]);
        install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        let child = install(&worlds, ("S", "child"), child_of(0, "#1"), &installs).await?;
        assert_eq!(child.seq(), FrameSeq(6));
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

    /// Ruling R63: a seq keeps the frame its world was installed in after it retires, and a seq
    /// this page never installed a world for (an earlier `Playwright`'s) names no frame.
    #[tokio::test]
    async fn tests_that_a_retired_seq_keeps_its_frame_and_a_foreign_one_has_none() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(0));
        let installs = AtomicU32::new(0);
        let first = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        worlds.invalidate(&first);
        assert!(matches!(worlds.by_seq(FrameSeq(0)), SeqLookup::Retired));
        assert_eq!(
            worlds.frame_of(FrameSeq(0)),
            Some(FrameId("main".to_owned()))
        );
        let rekeyed = worlds
            .world_with(key("S", "card"), child_of(0, "#1"), |_| async {
                Ok(Installed {
                    retired: Some(FrameSeq(7)),
                    ..installed(IsolatedWorld::for_tests(session("S"), "card", 2, "S.card"))
                })
            })
            .await?;
        assert_eq!(rekeyed.seq(), FrameSeq(2));
        assert!(matches!(worlds.by_seq(FrameSeq(7)), SeqLookup::Retired));
        assert_eq!(
            worlds.frame_of(FrameSeq(7)),
            None,
            "an earlier process's seq"
        );
        assert_eq!(
            worlds.frame_of(FrameSeq(2)),
            Some(FrameId("card".to_owned()))
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
                Ok(installed(IsolatedWorld::for_tests(
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
                Ok(installed(IsolatedWorld::for_tests(
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
        use browseros_core::{PageId, TabId};
        let browser = test_env::offline_session();
        let log = Arc::new(FrameAttachLog::default());
        let registry = PageRegistry::new(Arc::clone(&log));
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
        // The tab navigated away from a real page meanwhile, as the watcher would log it.
        for url in ["https://example.test/", "https://example.test/next"] {
            log.record(
                &TargetId("tab-51".to_owned()),
                FrameEvent::MainCommitted {
                    url: url.to_owned(),
                },
            );
        }
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

    fn doc_token(raw: &str) -> Result<DocToken, Box<dyn Error>> {
        DocToken::new(raw).ok_or_else(|| format!("empty doc token: {raw:?}").into())
    }

    fn frame_id(raw: &str) -> FrameId {
        FrameId(raw.to_owned())
    }

    fn seq_for(
        worlds: &FrameWorlds,
        token: &DocToken,
        frame: Option<&FrameId>,
        place: DocPlace,
        url: Option<&str>,
        proven: &HashMap<FrameId, IsolatedWorld>,
    ) -> DocSeq {
        worlds.seq_for_doc(
            DocContext {
                token,
                frame,
                place,
                url,
                requested_at: Stamp::next(),
            },
            proven,
        )
    }

    /// A capture requested at `requested_at` reporting `token` as the main frame's document.
    fn report_main(
        worlds: &FrameWorlds,
        token: &DocToken,
        url: &str,
        requested_at: Stamp,
    ) -> DocSeq {
        worlds.seq_for_doc(
            DocContext {
                token,
                frame: Some(&frame_id("main")),
                place: DocPlace::Main,
                url: Some(url),
                requested_at,
            },
            &HashMap::new(),
        )
    }

    /// Two captures of one tab in flight: the newer one's report of the main frame lands first.
    /// The older capture's first sighting of the previous page must not retire the live one.
    #[tokio::test]
    async fn tests_that_a_stale_capture_never_retires_the_frames_newer_document() -> TestResult {
        let worlds = logged(&Arc::new(FrameAttachLog::default()));
        let old_page = doc_token("doc-old-page")?;
        let live_page = doc_token("doc-live-page")?;
        let older = Stamp::next();
        let newer = Stamp::next();

        let live_seq = report_main(&worlds, &live_page, "https://live.test/", newer);
        let _ = report_main(&worlds, &old_page, "https://old.test/", older);
        assert!(
            matches!(worlds.lookup_doc(&live_page), DocLookup::Reserved { seq, .. } if seq.0 == live_seq.raw()),
            "a report older than the frame's last one must not displace its document"
        );
        assert!(
            matches!(worlds.lookup_doc(&old_page), DocLookup::Retired),
            "a page only a stale capture saw was gone before the newer report"
        );
        assert!(
            !worlds
                .take_retired_seqs()
                .contains(&FrameSeq(live_seq.raw())),
            "the live page's refs must not be retired"
        );
        assert_eq!(
            report_main(&worlds, &live_page, "https://live.test/", Stamp::next()),
            live_seq,
            "the next capture must find the live page where the newer report left it"
        );
        Ok(())
    }

    /// A page restored from the back/forward cache reports the token it had before it left.
    #[tokio::test]
    async fn tests_that_a_page_restored_from_the_back_forward_cache_takes_a_fresh_seq() -> TestResult
    {
        let worlds = logged(&Arc::new(FrameAttachLog::default()));
        let first = doc_token("doc-first-visit")?;
        let second = doc_token("doc-second-page")?;

        let first_seq = report_main(&worlds, &first, "https://a.test/", Stamp::next());
        let second_seq = report_main(&worlds, &second, "https://b.test/", Stamp::next());
        assert!(matches!(worlds.lookup_doc(&first), DocLookup::Retired));
        assert_eq!(
            worlds.take_retired_seqs(),
            vec![FrameSeq(first_seq.raw())],
            "the queue names the first visit by its seq, so a late drain spares a later life"
        );

        let back_seq = report_main(&worlds, &first, "https://a.test/", Stamp::next());
        assert_ne!(
            back_seq, first_seq,
            "the restored page must take a fresh seq, never its old one"
        );
        assert!(
            matches!(worlds.lookup_doc(&first), DocLookup::Reserved { seq, .. } if seq.0 == back_seq.raw()),
            "the restored page must be active again, so its new refs can hydrate"
        );
        assert!(
            matches!(
                worlds.lookup_doc_seq(FrameSeq(first_seq.raw())),
                DocLookup::Retired
            ),
            "refs from the first visit must still answer element_detached"
        );
        assert!(
            matches!(worlds.lookup_doc(&second), DocLookup::Retired),
            "going back must retire the page it left"
        );
        assert_eq!(
            worlds.take_retired_seqs(),
            vec![FrameSeq(second_seq.raw())],
            "only the page left behind retires; the restored page does not retire twice"
        );
        Ok(())
    }

    /// Only a capture requested after a document retired can bring it back: an older one saw it
    /// while it was still alive.
    #[tokio::test]
    async fn tests_that_only_a_capture_requested_after_a_retirement_brings_the_document_back()
    -> TestResult {
        let worlds = logged(&Arc::new(FrameAttachLog::default()));
        let page = doc_token("doc-page")?;
        let first_seq = report_main(&worlds, &page, "https://a.test/", Stamp::next());
        let requested_before = Stamp::next();
        let _ = worlds.retire_doc(&page);

        assert_eq!(
            report_main(&worlds, &page, "https://a.test/", requested_before),
            first_seq,
            "a capture requested before the retirement saw the page alive: it gets the retired seq"
        );
        assert!(
            matches!(worlds.lookup_doc(&page), DocLookup::Retired),
            "a report older than the retirement must not bring the page back"
        );
        let back_seq = report_main(&worlds, &page, "https://a.test/", Stamp::next());
        assert!(
            back_seq != first_seq
                && matches!(worlds.lookup_doc(&page), DocLookup::Reserved { seq, .. } if seq.0 == back_seq.raw()),
            "a capture requested after the retirement brings the page back under a fresh seq"
        );
        Ok(())
    }

    /// A world installed after a capture was requested holds the frame's newer document, so once
    /// hydration proves it, that capture's late report of an older page must not undo the proof.
    #[tokio::test]
    async fn tests_that_a_stale_capture_never_displaces_a_document_hydration_proved() -> TestResult
    {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        let old_page = doc_token("doc-old-page")?;
        let proved = doc_token("doc-proved-page")?;

        let capture_requested = Stamp::next();
        let world = main_at(&worlds, &installs, "https://b.test/").await?;
        let proved_seq = worlds
            .bind_proven_world(&proved, &world)
            .ok_or("an unknown token binds to a proven world")?;
        let _ = report_main(&worlds, &old_page, "https://a.test/", capture_requested);
        assert!(
            matches!(worlds.lookup_doc(&proved), DocLookup::Bound { seq, .. } if seq == proved_seq),
            "a report requested before the proof must not displace the proven document"
        );
        assert!(
            matches!(worlds.lookup_doc(&old_page), DocLookup::Retired),
            "the older page the stale capture saw must start retired"
        );
        Ok(())
    }

    /// Leaving a page and coming straight back puts two captures in flight: one requested on the
    /// next page, then one on the restored page. Processing the first retires the restored page as
    /// of that capture's request, so the second, requested later, still brings it back.
    #[tokio::test]
    async fn tests_that_a_page_restored_while_the_previous_capture_was_in_flight_comes_back()
    -> TestResult {
        let worlds = logged(&Arc::new(FrameAttachLog::default()));
        let page_a = doc_token("doc-page-a")?;
        let page_b = doc_token("doc-page-b")?;
        let first_seq = report_main(&worlds, &page_a, "https://a.test/", Stamp::next());

        let requested_on_b = Stamp::next();
        let requested_back_on_a = Stamp::next();
        let _ = report_main(&worlds, &page_b, "https://b.test/", requested_on_b);
        let back_seq = report_main(&worlds, &page_a, "https://a.test/", requested_back_on_a);
        assert!(
            back_seq != first_seq
                && matches!(worlds.lookup_doc(&page_a), DocLookup::Reserved { seq, .. } if seq.0 == back_seq.raw()),
            "the restored page, captured after the next page's capture was requested, must come back"
        );
        assert!(
            matches!(worlds.lookup_doc(&page_b), DocLookup::Retired),
            "the page left behind must retire"
        );
        Ok(())
    }

    /// A hydration's pin succeeds, the page navigates, a capture is requested on the next page, and
    /// only then is the pin's reply processed. Its proof dates from its world's install, so the
    /// next page's capture is the newer report and must win.
    #[tokio::test]
    async fn tests_that_a_late_hydration_reply_never_makes_the_next_pages_capture_stale()
    -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        let world = main_at(&worlds, &installs, "https://a.test/").await?;
        let page_a = doc_token("doc-page-a")?;
        let page_b = doc_token("doc-page-b")?;

        let requested_on_b = Stamp::next();
        let _ = worlds.bind_proven_world(&page_a, &world);
        let b_seq = report_main(&worlds, &page_b, "https://b.test/", requested_on_b);
        assert!(
            matches!(worlds.lookup_doc(&page_b), DocLookup::Reserved { seq, .. } if seq.0 == b_seq.raw()),
            "a capture requested after the proven world's install saw the frame later and must win"
        );
        assert!(
            matches!(worlds.lookup_doc(&page_a), DocLookup::Retired),
            "the page the late reply proved has been left"
        );
        assert_eq!(
            worlds.bind_proven_world(&page_a, &world),
            None,
            "a proof older than the frame's newer report must not bind again"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_doc_reservation_follows_attach_order_and_stays_stable_across_captures()
    -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        record(&log, ["two", "one", "uno", "dos"].map(attached));

        let tok_main = doc_token("doc-main")?;
        let tok_two = doc_token("doc-two")?;
        let tok_one = doc_token("doc-one")?;
        let tok_uno = doc_token("doc-uno")?;
        let tok_dos = doc_token("doc-dos")?;
        let tok_anon = doc_token("doc-anon")?;

        let proven = HashMap::new();
        let f_main = frame_id("main");
        let f_two = frame_id("two");
        let f_one = frame_id("one");
        let f_uno = frame_id("uno");
        let f_dos = frame_id("dos");
        let f_anon = frame_id("anon");

        // Capture encounters documents out of attach order.
        let s_uno = seq_for(
            &worlds,
            &tok_uno,
            Some(&f_uno),
            DocPlace::Child,
            None,
            &proven,
        );
        let s_two = seq_for(
            &worlds,
            &tok_two,
            Some(&f_two),
            DocPlace::Child,
            None,
            &proven,
        );
        let s_main = seq_for(
            &worlds,
            &tok_main,
            Some(&f_main),
            DocPlace::Main,
            Some("https://a.test/"),
            &proven,
        );
        let s_dos = seq_for(
            &worlds,
            &tok_dos,
            Some(&f_dos),
            DocPlace::Child,
            None,
            &proven,
        );
        let s_one = seq_for(
            &worlds,
            &tok_one,
            Some(&f_one),
            DocPlace::Child,
            None,
            &proven,
        );
        let s_anon = seq_for(&worlds, &tok_anon, None, DocPlace::Child, None, &proven);

        assert_eq!(
            [s_main, s_two, s_one, s_uno, s_dos, s_anon],
            [0, 1, 2, 3, 4, 5].map(DocSeq::new),
            "child reservations follow FrameAttachLog attach order regardless of capture order"
        );
        assert_eq!(worlds.frame_of(FrameSeq(3)), Some(f_uno.clone()));
        assert!(matches!(
            worlds.lookup_doc(&tok_anon),
            DocLookup::Reserved {
                seq: FrameSeq(5),
                frame: None
            }
        ));

        // Repeated captures keep identical seqs and fill in a previously unknown frame id.
        for (token, frame, place, expected) in [
            (&tok_one, &f_one, DocPlace::Child, DocSeq::new(2)),
            (&tok_main, &f_main, DocPlace::Main, DocSeq::new(0)),
            (&tok_dos, &f_dos, DocPlace::Child, DocSeq::new(4)),
            (&tok_two, &f_two, DocPlace::Child, DocSeq::new(1)),
            (&tok_uno, &f_uno, DocPlace::Child, DocSeq::new(3)),
            (&tok_anon, &f_anon, DocPlace::Child, DocSeq::new(5)),
        ] {
            let repeated = seq_for(&worlds, token, Some(frame), place, None, &proven);
            assert_eq!(
                repeated, expected,
                "repeated captures of an existing DocToken keep its seq"
            );
        }
        assert!(matches!(
            worlds.lookup_doc_seq(FrameSeq(5)),
            DocLookup::Reserved {
                seq: FrameSeq(5),
                frame: Some(ref f),
            } if f == &f_anon
        ));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_reserved_doc_keeps_its_seq_when_later_bound_to_a_higher_seq_world()
    -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        let main = boot(
            &worlds,
            ("S", "main"),
            Place::Main,
            &installs,
            None,
            "https://a.test/",
        )
        .await?;
        record(&log, [attached("a"), attached("b")]);

        let (tok_a, tok_b) = (doc_token("doc-a")?, doc_token("doc-b")?);
        let (f_a, f_b) = (frame_id("a"), frame_id("b"));
        let empty = HashMap::new();
        assert_eq!(
            (
                seq_for(&worlds, &tok_a, Some(&f_a), DocPlace::Child, None, &empty),
                seq_for(&worlds, &tok_b, Some(&f_b), DocPlace::Child, None, &empty),
            ),
            (DocSeq::new(1), DocSeq::new(2)),
            "children take their attach-reserved seqs"
        );

        // Installing worlds after reservation consumes higher candidate seqs (3 and 4).
        let world_a = install(&worlds, ("S", "a"), child_of(0, "#a"), &installs).await?;
        let world_b = install(&worlds, ("S", "b"), child_of(0, "#b"), &installs).await?;
        assert_eq!(
            (world_a.seq(), world_b.seq()),
            (FrameSeq(3), FrameSeq(4)),
            "late installs boot with higher candidate seqs than the reserved doc seqs"
        );

        // Path 1: promote via bind_proven_world.
        assert_eq!(
            worlds.bind_proven_world(&tok_a, &world_a),
            Some(FrameSeq(1)),
            "binding a reserved doc keeps its reserved seq"
        );
        // Path 2: promote via seq_for_doc with proven world.
        let mut proven = HashMap::new();
        proven.insert(f_b.clone(), world_b.clone());
        assert_eq!(
            seq_for(&worlds, &tok_b, Some(&f_b), DocPlace::Child, None, &proven),
            DocSeq::new(2),
            "seq_for_doc promotion keeps reserved seq 2"
        );

        for (reserved_seq, expected_world, expected_key) in
            [(FrameSeq(1), &world_a, "#a"), (FrameSeq(2), &world_b, "#b")]
        {
            let DocLookup::Bound {
                seq,
                world,
                parents,
            } = worlds.lookup_doc_seq(reserved_seq)
            else {
                return Err(format!("reserved {reserved_seq:?} should resolve as Bound").into());
            };
            assert_eq!(seq, reserved_seq);
            assert!(world.same_install(expected_world));
            assert_eq!(parents.len(), 1);
            assert!(parents[0].world.same_install(&main));
            assert_eq!(parents[0].iframe_key, expected_key);
        }

        assert_eq!(worlds.cached_worlds().len(), 3);
        assert!(
            worlds
                .cached_world_for_frame(&f_a)
                .is_some_and(|cached| cached.same_install(&world_a))
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_proven_pre_capture_world_binds_and_invalidation_retires_doc_token()
    -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(0));
        let installs = AtomicU32::new(0);
        let main = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        let child = install(&worlds, ("S", "child"), child_of(0, "#1"), &installs).await?;

        let tok_main = doc_token("doc-main")?;
        let tok_child = doc_token("doc-child")?;
        let f_main = frame_id("main");
        let f_child = frame_id("child");

        let mut proven = HashMap::new();
        proven.insert(f_main.clone(), main.clone());
        proven.insert(f_child.clone(), child.clone());

        assert_eq!(
            worlds.seq_for_doc(
                DocContext {
                    token: &tok_main,
                    frame: Some(&f_main),
                    place: DocPlace::Main,
                    url: Some("https://a.test/"),
                    requested_at: Stamp::next(),
                },
                &proven,
            ),
            DocSeq::new(0)
        );
        assert_eq!(
            worlds.seq_for_doc(
                DocContext {
                    token: &tok_child,
                    frame: Some(&f_child),
                    place: DocPlace::Child,
                    url: Some("https://a.test/frame"),
                    requested_at: Stamp::next(),
                },
                &proven,
            ),
            DocSeq::new(1)
        );
        assert!(matches!(
            worlds.lookup_doc_seq(FrameSeq(1)),
            DocLookup::Bound {
                seq: FrameSeq(1),
                ..
            }
        ));

        // A newer install for the same document (e.g. reached via another session) updates the
        // bound world while keeping seq 0.
        let main_s2 = worlds
            .world_with(key("S2", "main"), Place::Main, |_| async {
                Ok(installed(IsolatedWorld::for_tests(
                    session("S2"),
                    "main",
                    0,
                    "S2.main",
                )))
            })
            .await?;
        let mut newer_proven = HashMap::new();
        newer_proven.insert(f_main.clone(), main_s2.clone());
        assert_eq!(
            worlds.seq_for_doc(
                DocContext {
                    token: &tok_main,
                    frame: Some(&f_main),
                    place: DocPlace::Main,
                    url: Some("https://a.test/"),
                    requested_at: Stamp::next(),
                },
                &newer_proven,
            ),
            DocSeq::new(0)
        );
        assert!(matches!(
            worlds.lookup_doc(&tok_main),
            DocLookup::Bound {
                seq: FrameSeq(0),
                ref world,
                ..
            } if world.same_install(&main_s2)
        ));

        // Invalidating the child world retires its DocToken and reports it exactly once.
        let _ = worlds.take_retired_seqs();
        worlds.invalidate(&child);
        assert_eq!(worlds.take_retired_seqs(), vec![FrameSeq(1)]);
        assert!(worlds.take_retired_seqs().is_empty());
        assert!(matches!(worlds.lookup_doc(&tok_child), DocLookup::Retired));
        assert!(matches!(
            worlds.lookup_doc_seq(FrameSeq(1)),
            DocLookup::Retired
        ));
        assert!(worlds.cached_world_for_frame(&f_child).is_none());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_snapshot_after_real_main_invalidation_takes_fresh_seq() -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        let f_main = frame_id("main");
        let old_world = main_at(&worlds, &installs, "https://old.test/").await?;
        let tok_old = doc_token("doc-old")?;
        let tok_new = doc_token("doc-new")?;

        let mut proven = HashMap::new();
        proven.insert(f_main.clone(), old_world.clone());
        assert_eq!(
            seq_for(
                &worlds,
                &tok_old,
                Some(&f_main),
                DocPlace::Main,
                Some("https://old.test/"),
                &proven,
            ),
            DocSeq::new(0)
        );

        worlds.invalidate(&old_world);
        let new_seq = seq_for(
            &worlds,
            &tok_new,
            Some(&f_main),
            DocPlace::Main,
            Some("https://new.test/"),
            &HashMap::new(),
        );
        assert_eq!(
            new_seq,
            DocSeq::new(1),
            "a new real page after an invalidated real main doc never aliases seq 0"
        );
        assert!(matches!(worlds.lookup_doc(&tok_old), DocLookup::Retired));
        assert!(matches!(
            worlds.lookup_doc_seq(FrameSeq(0)),
            DocLookup::Retired
        ));
        assert!(matches!(
            worlds.lookup_doc(&tok_new),
            DocLookup::Reserved {
                seq: FrameSeq(1),
                ..
            }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_about_blank_to_first_real_page_preserves_seq_zero_and_retires_blank()
    -> TestResult {
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let installs = AtomicU32::new(0);
        let f_main = frame_id("main");
        let blank_world = main_at(&worlds, &installs, "about:blank").await?;
        let tok_blank = doc_token("doc-blank")?;
        let tok_real = doc_token("doc-real")?;
        let tok_second = doc_token("doc-second")?;

        let mut proven = HashMap::new();
        proven.insert(f_main.clone(), blank_world.clone());
        assert_eq!(
            seq_for(
                &worlds,
                &tok_blank,
                Some(&f_main),
                DocPlace::Main,
                Some("about:blank"),
                &proven,
            ),
            DocSeq::new(0)
        );

        worlds.invalidate(&blank_world);
        let empty = HashMap::new();
        assert_eq!(
            seq_for(
                &worlds,
                &tok_real,
                Some(&f_main),
                DocPlace::Main,
                Some("https://real.test/"),
                &empty,
            ),
            DocSeq::new(0),
            "first real page after about:blank preserves seq 0 (AR-053)"
        );
        assert!(
            matches!(worlds.lookup_doc(&tok_blank), DocLookup::Retired),
            "the replaced about:blank DocToken must retire"
        );
        assert!(matches!(
            worlds.lookup_doc_seq(FrameSeq(0)),
            DocLookup::Reserved {
                seq: FrameSeq(0),
                ..
            }
        ));

        // Leaving that real page takes a fresh seq.
        assert_eq!(
            seq_for(
                &worlds,
                &tok_second,
                Some(&f_main),
                DocPlace::Main,
                Some("https://second.test/"),
                &empty,
            ),
            DocSeq::new(1)
        );
        assert!(matches!(worlds.lookup_doc(&tok_real), DocLookup::Retired));
        assert!(matches!(
            worlds.lookup_doc_seq(FrameSeq(0)),
            DocLookup::Retired
        ));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_uninstalled_main_navigation_retires_previous_doc_without_aliasing()
    -> TestResult {
        let f_main = frame_id("main");
        let empty = HashMap::new();

        // Both an explicit real URL and an unknown URL (None) are treated as real pages.
        for first_url in [Some("https://first.test/"), None] {
            let log = Arc::new(FrameAttachLog::default());
            let worlds = logged(&log);
            let tok_first = doc_token("doc-first")?;
            let tok_second = doc_token("doc-second")?;

            let s1 = worlds.seq_for_doc(
                DocContext {
                    token: &tok_first,
                    frame: Some(&f_main),
                    place: DocPlace::Main,
                    url: first_url,
                    requested_at: Stamp::next(),
                },
                &empty,
            );
            let s2 = worlds.seq_for_doc(
                DocContext {
                    token: &tok_second,
                    frame: Some(&f_main),
                    place: DocPlace::Main,
                    url: Some("https://second.test/"),
                    requested_at: Stamp::next(),
                },
                &empty,
            );
            assert_eq!(
                (s1, s2),
                (DocSeq::new(0), DocSeq::new(1)),
                "second real main DocToken must allocate a fresh seq even before MainCommitted (first_url={first_url:?})"
            );
            assert!(matches!(worlds.lookup_doc(&tok_first), DocLookup::Retired));
            assert!(matches!(
                worlds.lookup_doc_seq(FrameSeq(0)),
                DocLookup::Retired
            ));
            assert!(matches!(
                worlds.lookup_doc(&tok_second),
                DocLookup::Reserved {
                    seq: FrameSeq(1),
                    ..
                }
            ));
        }

        // An explicit about:blank without any installed world retires and hands seq 0 to the first
        // real page.
        let log = Arc::new(FrameAttachLog::default());
        let worlds = logged(&log);
        let tok_blank = doc_token("doc-blank-uninstalled")?;
        let tok_real = doc_token("doc-real-uninstalled")?;
        assert_eq!(
            worlds.seq_for_doc(
                DocContext {
                    token: &tok_blank,
                    frame: Some(&f_main),
                    place: DocPlace::Main,
                    url: Some("about:blank"),
                    requested_at: Stamp::next(),
                },
                &empty,
            ),
            DocSeq::new(0)
        );
        assert_eq!(
            worlds.seq_for_doc(
                DocContext {
                    token: &tok_real,
                    frame: Some(&f_main),
                    place: DocPlace::Main,
                    url: Some("https://real.test/"),
                    requested_at: Stamp::next(),
                },
                &empty,
            ),
            DocSeq::new(0)
        );
        assert!(matches!(worlds.lookup_doc(&tok_blank), DocLookup::Retired));
        assert!(matches!(
            worlds.lookup_doc_seq(FrameSeq(0)),
            DocLookup::Reserved {
                seq: FrameSeq(0),
                ..
            }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_child_doc_resolves_retired_when_parent_world_is_invalidated() -> TestResult
    {
        let worlds = FrameWorlds::new(FrameSeq(0));
        let installs = AtomicU32::new(0);
        let main = install(&worlds, ("S", "main"), Place::Main, &installs).await?;
        let child = install(&worlds, ("S", "child"), child_of(0, "#1"), &installs).await?;

        let tok_main = doc_token("doc-main")?;
        let tok_child = doc_token("doc-child")?;
        let f_main = frame_id("main");
        let f_child = frame_id("child");

        let mut proven = HashMap::new();
        proven.insert(f_main.clone(), main.clone());
        proven.insert(f_child.clone(), child.clone());

        worlds.seq_for_doc(
            DocContext {
                token: &tok_main,
                frame: Some(&f_main),
                place: DocPlace::Main,
                url: Some("https://a.test/"),
                requested_at: Stamp::next(),
            },
            &proven,
        );
        worlds.seq_for_doc(
            DocContext {
                token: &tok_child,
                frame: Some(&f_child),
                place: DocPlace::Child,
                url: None,
                requested_at: Stamp::next(),
            },
            &proven,
        );
        assert!(matches!(
            worlds.lookup_doc(&tok_child),
            DocLookup::Bound { ref parents, .. } if parents.len() == 1
        ));

        worlds.invalidate(&main);
        assert!(
            matches!(worlds.lookup_doc(&tok_child), DocLookup::Retired),
            "a child document cannot outlive its parent's world"
        );
        assert!(
            matches!(worlds.lookup_doc_seq(FrameSeq(1)), DocLookup::Retired),
            "seq lookup for a child whose parent died also reports Retired"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_retire_doc_retires_reserved_doc_and_refuses_later_binding() -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(0));
        let installs = AtomicU32::new(0);
        let tok_child = doc_token("doc-child-reserved")?;
        let f_child = frame_id("child");

        let seq = worlds.seq_for_doc(
            DocContext {
                token: &tok_child,
                frame: Some(&f_child),
                place: DocPlace::Child,
                url: None,
                requested_at: Stamp::next(),
            },
            &HashMap::new(),
        );
        assert_eq!(seq, DocSeq::new(1));

        assert_eq!(worlds.retire_doc(&tok_child), Some(FrameSeq(1)));
        assert_eq!(
            worlds.retire_doc(&tok_child),
            None,
            "retiring an already-retired document is a no-op"
        );
        assert!(matches!(worlds.lookup_doc(&tok_child), DocLookup::Retired));
        assert!(matches!(
            worlds.lookup_doc_seq(FrameSeq(1)),
            DocLookup::Retired
        ));

        let child_world = install(&worlds, ("S", "child"), child_of(0, "#1"), &installs).await?;
        assert_eq!(
            worlds.bind_proven_world(&tok_child, &child_world),
            None,
            "a retired document refuses any later world binding"
        );
        assert!(matches!(worlds.lookup_doc(&tok_child), DocLookup::Retired));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_foreign_seqs_resolve_retired_and_never_issued_seqs_resolve_unknown()
    -> TestResult {
        let worlds = FrameWorlds::new(FrameSeq(500_000));
        let installs = AtomicU32::new(0);
        rekey(&worlds, ("S", "main"), Place::Main, &installs, Some(0)).await?;
        rekey(
            &worlds,
            ("S", "child"),
            child_of(500_000, "#1"),
            &installs,
            Some(500_002),
        )
        .await?;

        assert!(
            matches!(worlds.lookup_doc_seq(FrameSeq(0)), DocLookup::Retired),
            "foreign seq 0 from an earlier Playwright restart resolves Retired"
        );
        assert!(
            matches!(worlds.lookup_doc_seq(FrameSeq(500_002)), DocLookup::Retired),
            "foreign seq 500_002 resolves Retired"
        );
        assert!(
            matches!(worlds.lookup_doc_seq(FrameSeq(42)), DocLookup::Unknown),
            "a seq below the base that was never foreign resolves Unknown"
        );
        assert!(
            matches!(worlds.lookup_doc_seq(FrameSeq(500_999)), DocLookup::Unknown),
            "a never-issued seq above next resolves Unknown"
        );
        let unseen = doc_token("doc-unseen")?;
        assert!(matches!(worlds.lookup_doc(&unseen), DocLookup::Unknown));
        Ok(())
    }
}
