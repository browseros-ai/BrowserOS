//! Document identity table mapping APC [`DocToken`]s to [`FrameSeq`]s and proven
//! [`IsolatedWorld`]s (design sections 6.2 and 6.3).
//!
//! `DocTable` lives inside `FrameWorlds`'s existing `std::sync::Mutex` so seq numbering,
//! `FrameAttachLog` replay, and document-to-world bindings stay atomic without a second lock or
//! any lock held across `.await`.
//!
//! ```text
//!  APC capture ──▶ DocToken ──▶ Reserved { seq, frame } ──▶ Bound { seq, world } ──▶ Retired { seq, at }
//!                                      │                                                    ▲   │
//!                                      └────────────── retire_doc / frame navigation ───────┘   │
//!                     ▲                                                                          │
//!                     └──── a capture requested after `at` reports it again: a fresh seq ◀──────┘
//! ```
//!
//! Invariants:
//! - **One active document per frame:** at most one `Reserved` or `Bound` [`DocToken`] is active
//!   for any [`FrameId`]. Recording a new document on the same frame retires the previous one.
//! - **A reserved seq survives promotion:** once a document receives a [`FrameSeq`], binding a
//!   proven [`IsolatedWorld`] later keeps that reserved seq even if the world booted with a higher
//!   candidate seq.
//! - **A retired seq stays retired:** a document that comes back (a page restored from the
//!   back/forward cache keeps its token) takes a fresh seq, so refs from its earlier life still
//!   answer `element_detached`. Only a capture requested after the retirement can bring it back.
//! - **A stale capture changes nothing:** captures of one tab can be in flight together, and their
//!   reports arrive in any order. Each frame remembers when its document was last reported; a
//!   capture requested before that neither displaces the frame's document nor activates its own,
//!   which it can only have seen before the frame moved on.
//! - **Every retirement is reported once:** a document becomes `Retired` only through
//!   [`DocTable::retire_token`] or [`DocTable::insert_retired`], and each queues the token for
//!   [`DocTable::take_retired`]. `TabTree` drains that queue
//!   into its ref table, so a document's refs go when its seq retires (design 2.3), including
//!   retirements made by `FrameWorlds` paths that never pass through `TabTree`.
//! - **No page URLs in diagnostics:** [`DocTable`] never stores raw URLs (only whether a document
//!   was explicitly `"about:blank"` for Playwright AR-053 reuse), and neither [`DocTable`] nor
//!   [`DocContext`] prints URLs in [`fmt::Debug`].

#![cfg_attr(not(test), expect(dead_code, reason = "wired by the switch-over"))]

use std::collections::HashMap;
use std::{fmt, mem};

use browseros_core::FrameId;
use neo_snapshot_apc::tree::DocToken;

use crate::world::{FrameSeq, IsolatedWorld, LinkedParent, Stamp};

/// Whether an APC document belongs to the tab's top-level main frame or a nested child frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocPlace {
    /// The tab's top-level document: participates in initial seq `0` and `about:blank` reuse.
    Main,
    /// Any nested `<iframe>` document: follows attach order and never reuses a previous seq.
    Child,
}

/// Document metadata supplied by an APC capture when assigning or looking up a document's seq.
#[derive(Clone, Copy)]
pub(crate) struct DocContext<'a> {
    /// Stable APC document token (`DocumentIdentifier.serialized_token`).
    pub(crate) token: &'a DocToken,
    /// CDP frame id when BrowserOS frame metadata reported it (`devtools_frame_token`).
    pub(crate) frame: Option<&'a FrameId>,
    /// Whether this document is the tab's main frame or a child iframe.
    pub(crate) place: DocPlace,
    /// Document URL when APC reported it, inspected only to recognize explicit `"about:blank"`
    /// transitions and seed the initial `main_url`; never printed in `Debug`.
    pub(crate) url: Option<&'a str>,
    /// When the capture that reported this document was requested: what orders its report
    /// against other captures' reports and against retirements.
    pub(crate) requested_at: Stamp,
}

impl fmt::Debug for DocContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocContext")
            .field("token", &self.token)
            .field("frame", &self.frame)
            .field("place", &self.place)
            .field("has_url", &self.url.is_some())
            .field("requested_at", &self.requested_at)
            .finish()
    }
}

/// Lifecycle state of one APC [`DocToken`] inside [`DocTable`].
#[derive(Clone, Debug)]
pub(crate) enum DocBinding {
    /// Assigned a [`FrameSeq`] during an APC capture before an isolated world was proven for it.
    Reserved {
        /// The document's permanent seq (kept when later promoted to `Bound`).
        seq: FrameSeq,
        /// The CDP frame id, when APC frame metadata supplied one.
        frame: Option<FrameId>,
    },
    /// Bound to an [`IsolatedWorld`] proven by section 6.2 to host this exact document.
    Bound {
        /// The document's permanent seq (may differ from `world.seq()` when promoted from
        /// `Reserved`).
        seq: FrameSeq,
        /// The proven isolated world hosting this document.
        world: IsolatedWorld,
    },
    /// The document navigated away, its world failed `Gone`, or a fresh capture proved it absent.
    Retired {
        /// The seq this document held before retirement.
        seq: FrameSeq,
        /// When it retired: a capture requested later that still reports the document proves it
        /// came back.
        at: Stamp,
    },
}

impl DocBinding {
    /// The document's current seq, in any state.
    pub(crate) const fn seq(&self) -> FrameSeq {
        match self {
            Self::Reserved { seq, .. } | Self::Bound { seq, .. } | Self::Retired { seq, .. } => {
                *seq
            }
        }
    }
}

/// What a [`FrameSeq`] names in [`DocTable`].
pub(crate) enum SeqBinding<'a> {
    /// `seq` is its document's current seq.
    Current(&'a DocBinding),
    /// `seq` belongs to an earlier life of a document that came back under a fresh seq.
    Past,
}

/// How a capture's report of a frame's document compares with what [`DocTable`] already knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameReport {
    /// No newer report of this frame exists; the capture's view of it is current.
    Current,
    /// A capture requested later, or a hydration proof, already reported this frame.
    Stale,
}

/// Resolution result for a [`DocToken`] or [`FrameSeq`] lookup through `FrameWorlds`.
#[derive(Debug)]
pub(crate) enum DocLookup {
    /// Document has a live bound world whose parent chain is also live.
    Bound {
        /// The document's permanent seq.
        seq: FrameSeq,
        /// Boxed so `DocLookup` stays compact across variants.
        world: Box<IsolatedWorld>,
        /// Live ancestor worlds from the main frame down to this frame's direct parent.
        parents: Vec<LinkedParent>,
    },
    /// Document has a reserved seq but no proven live world yet; hydration installs a world and
    /// recaptures (design section 6.2 case 3).
    Reserved {
        /// The document's reserved seq.
        seq: FrameSeq,
        /// The CDP frame id, when known.
        frame: Option<FrameId>,
    },
    /// Document is gone, its bound world or an ancestor world was invalidated, or the seq was
    /// retired by this or an earlier `Playwright` instance.
    Retired,
    /// Neither `DocTable` nor `FrameWorlds` has ever issued this token or seq.
    Unknown,
}

/// Classification of a document's URL for Playwright AR-053 main-frame seq reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageUrlKind {
    /// Explicitly reported as `"about:blank"`: the first real main document may reuse its seq.
    AboutBlank,
    /// Any other URL, or an absent/empty URL treated conservatively as real so a later document
    /// never aliases its seq.
    RealOrUnknown,
}

impl PageUrlKind {
    /// Classifies an optional capture URL, treating absent or empty URLs conservatively as real.
    pub(crate) fn from_url(url: Option<&str>) -> Self {
        if url == Some("about:blank") {
            Self::AboutBlank
        } else {
            Self::RealOrUnknown
        }
    }
}

/// Metadata returned when an active document transitions into [`DocBinding::Retired`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RetiredDoc {
    /// The seq the retired document held.
    pub(crate) seq: FrameSeq,
    /// Whether the retired document was explicitly `"about:blank"`.
    pub(crate) url_kind: PageUrlKind,
}

#[derive(Clone)]
struct DocEntry {
    binding: DocBinding,
    url_kind: PageUrlKind,
}

impl DocEntry {
    fn frame(&self) -> Option<&FrameId> {
        match &self.binding {
            DocBinding::Reserved { frame, .. } => frame.as_ref(),
            DocBinding::Bound { world, .. } => Some(world.frame()),
            DocBinding::Retired { .. } => None,
        }
    }
}

/// Per-tab mapping between APC [`DocToken`]s, [`FrameSeq`]s, and proven [`IsolatedWorld`]s.
///
/// Owned by `WorldsInner` inside `FrameWorlds` so every transition shares the page's single
/// mutex with `FrameAttachLog` replay and isolated-world invalidation.
pub(crate) struct DocTable {
    by_token: HashMap<DocToken, DocEntry>,
    /// Most recent [`DocToken`] owning each [`FrameSeq`]; updated in place only when a main-frame
    /// real page legally reuses an explicit `"about:blank"` document's seq.
    by_seq: HashMap<FrameSeq, DocToken>,
    /// Currently active (`Reserved` or `Bound`) document for each known [`FrameId`].
    active_by_frame: HashMap<FrameId, DocToken>,
    /// When each frame's document was last reported: the newest request time of a capture that
    /// reported it, or the moment hydration proved a world on it.
    frame_seen: HashMap<FrameId, Stamp>,
    /// Tokens retired since the last [`Self::take_retired`], in retirement order. Only documents
    /// a `TabTree` capture or hydration numbered can retire, so on a page `TabTree` never reads
    /// the queue stays empty.
    retired_since_take: Vec<DocToken>,
}

impl fmt::Debug for DocTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut reserved = 0usize;
        let mut bound = 0usize;
        let mut retired = 0usize;
        for entry in self.by_token.values() {
            match entry.binding {
                DocBinding::Reserved { .. } => reserved += 1,
                DocBinding::Bound { .. } => bound += 1,
                DocBinding::Retired { .. } => retired += 1,
            }
        }
        f.debug_struct("DocTable")
            .field("docs", &self.by_token.len())
            .field("reserved", &reserved)
            .field("bound", &bound)
            .field("retired", &retired)
            .field("seqs", &self.by_seq.len())
            .field("active_frames", &self.active_by_frame.len())
            .field("seen_frames", &self.frame_seen.len())
            .field("retired_since_take", &self.retired_since_take.len())
            .finish()
    }
}

impl DocTable {
    /// Creates an empty table with no reserved, bound, or retired documents.
    pub(crate) fn new() -> Self {
        Self {
            by_token: HashMap::new(),
            by_seq: HashMap::new(),
            active_by_frame: HashMap::new(),
            frame_seen: HashMap::new(),
            retired_since_take: Vec::new(),
        }
    }

    /// Whether `seq` belonged to a non-`about:blank` document that has left it: retired, or come
    /// back under a fresh seq.
    pub(crate) fn is_retired_real_seq(&self, seq: FrameSeq) -> bool {
        let Some(token) = self.by_seq.get(&seq) else {
            return false;
        };
        let Some(entry) = self.by_token.get(token) else {
            return false;
        };
        let left =
            entry.binding.seq() != seq || matches!(entry.binding, DocBinding::Retired { .. });
        left && entry.url_kind == PageUrlKind::RealOrUnknown
    }

    /// Compares a capture's report of `frame`, requested at `requested_at`, with the frame's last
    /// report, and records it when it is the newest.
    pub(crate) fn report_frame(&mut self, frame: &FrameId, requested_at: Stamp) -> FrameReport {
        match self.frame_seen.get(frame) {
            Some(&seen) if seen > requested_at => FrameReport::Stale,
            Some(_) | None => {
                self.frame_seen.insert(frame.clone(), requested_at);
                FrameReport::Current
            }
        }
    }

    /// Retires every active document bound to `world` (`same_install`), in token order so the
    /// retirement queue stays deterministic.
    pub(crate) fn retire_by_world(&mut self, world: &IsolatedWorld) {
        let mut matched: Vec<DocToken> = self
            .by_token
            .iter()
            .filter_map(|(token, entry)| match &entry.binding {
                DocBinding::Bound { world: bound, .. } if bound.same_install(world) => {
                    Some(token.clone())
                }
                DocBinding::Reserved { .. }
                | DocBinding::Bound { .. }
                | DocBinding::Retired { .. } => None,
            })
            .collect();
        matched.sort();
        for token in &matched {
            let _ = self.retire_token(token);
        }
    }

    /// Returns the current binding for `token`, if seen.
    pub(crate) fn binding(&self, token: &DocToken) -> Option<&DocBinding> {
        self.by_token.get(token).map(|entry| &entry.binding)
    }

    /// Returns what `seq` names: the current binding of the most recent document assigned it, or
    /// a life that document has left.
    pub(crate) fn binding_by_seq(&self, seq: FrameSeq) -> Option<SeqBinding<'_>> {
        let token = self.by_seq.get(&seq)?;
        let binding = self.binding(token)?;
        Some(if binding.seq() == seq {
            SeqBinding::Current(binding)
        } else {
            SeqBinding::Past
        })
    }

    /// Returns the token that was last assigned `seq`, whether it still holds it or not.
    pub(crate) fn token_for_seq(&self, seq: FrameSeq) -> Option<&DocToken> {
        self.by_seq.get(&seq)
    }

    /// Whether `token` was explicitly recorded with URL `"about:blank"`.
    pub(crate) fn is_about_blank(&self, token: &DocToken) -> bool {
        self.by_token
            .get(token)
            .is_some_and(|entry| entry.url_kind == PageUrlKind::AboutBlank)
    }

    /// Returns the currently active (`Reserved` or `Bound`) token for `frame`, if any.
    pub(crate) fn active_for_frame(&self, frame: &FrameId) -> Option<&DocToken> {
        self.active_by_frame.get(frame)
    }

    /// Whether `world` (`same_install`) is already bound to a different active document.
    pub(crate) fn is_world_bound_to_other_active(
        &self,
        world: &IsolatedWorld,
        token: &DocToken,
    ) -> bool {
        self.by_token.iter().any(|(other, entry)| {
            other != token
                && matches!(
                    &entry.binding,
                    DocBinding::Bound { world: bound, .. } if bound.same_install(world)
                )
        })
    }

    /// Inserts a newly seen document in the `Reserved` state.
    pub(crate) fn insert_reserved(
        &mut self,
        token: DocToken,
        seq: FrameSeq,
        frame: Option<FrameId>,
        url: Option<&str>,
    ) {
        if let Some(frame) = &frame {
            self.retire_other_active_on_frame(frame, &token);
            self.active_by_frame.insert(frame.clone(), token.clone());
        }
        self.by_seq.insert(seq, token.clone());
        self.by_token.insert(
            token,
            DocEntry {
                binding: DocBinding::Reserved { seq, frame },
                url_kind: PageUrlKind::from_url(url),
            },
        );
    }

    /// Inserts a document that only a stale capture reported: the frame had moved on by the time a
    /// newer capture reported it, so the document is already gone.
    pub(crate) fn insert_retired(&mut self, token: DocToken, seq: FrameSeq, url: Option<&str>) {
        self.by_seq.insert(seq, token.clone());
        self.retired_since_take.push(token.clone());
        self.by_token.insert(
            token,
            DocEntry {
                binding: DocBinding::Retired {
                    seq,
                    at: Stamp::next(),
                },
                url_kind: PageUrlKind::from_url(url),
            },
        );
    }

    /// Inserts a newly seen document directly in the `Bound` state.
    pub(crate) fn insert_bound(
        &mut self,
        token: DocToken,
        seq: FrameSeq,
        world: IsolatedWorld,
        url: Option<&str>,
    ) {
        let frame = world.frame().clone();
        self.retire_other_active_on_frame(&frame, &token);
        self.active_by_frame.insert(frame, token.clone());
        self.by_seq.insert(seq, token.clone());
        self.by_token.insert(
            token,
            DocEntry {
                binding: DocBinding::Bound { seq, world },
                url_kind: PageUrlKind::from_url(url),
            },
        );
    }

    /// Promotes an existing `Reserved` document or updates an existing `Bound` document to `world`,
    /// preserving its existing [`FrameSeq`]. Returns `None` if `token` is absent or `Retired`.
    pub(crate) fn bind_existing(
        &mut self,
        token: &DocToken,
        world: IsolatedWorld,
    ) -> Option<FrameSeq> {
        let (seq, previous_frame) = {
            let entry = self.by_token.get(token)?;
            let seq = match &entry.binding {
                DocBinding::Reserved { seq, .. } | DocBinding::Bound { seq, .. } => *seq,
                DocBinding::Retired { .. } => return None,
            };
            (seq, entry.frame().cloned())
        };
        let new_frame = world.frame().clone();
        if let Some(old_frame) = previous_frame
            && old_frame != new_frame
        {
            self.clear_active_frame(&old_frame, token);
        }
        self.retire_other_active_on_frame(&new_frame, token);
        self.active_by_frame.insert(new_frame, token.clone());
        if let Some(entry) = self.by_token.get_mut(token) {
            entry.binding = DocBinding::Bound { seq, world };
        }
        Some(seq)
    }

    /// Records a newly discovered [`FrameId`] on an existing `Reserved` document whose frame was
    /// previously `None`.
    pub(crate) fn set_reserved_frame(&mut self, token: &DocToken, frame: FrameId) {
        let needs_update = matches!(
            self.by_token.get(token).map(|entry| &entry.binding),
            Some(DocBinding::Reserved { frame: None, .. })
        );
        if !needs_update {
            return;
        }
        self.retire_other_active_on_frame(&frame, token);
        self.active_by_frame.insert(frame.clone(), token.clone());
        if let Some(entry) = self.by_token.get_mut(token)
            && let DocBinding::Reserved {
                frame: saved @ None,
                ..
            } = &mut entry.binding
        {
            *saved = Some(frame);
        }
    }

    /// Upgrades a document's URL classification if a non-`about:blank` URL is observed for it;
    /// never downgrades a real/unknown document back to `AboutBlank`.
    pub(crate) fn note_url(&mut self, token: &DocToken, url: Option<&str>) {
        if let Some(url) = url
            && !url.is_empty()
            && url != "about:blank"
            && let Some(entry) = self.by_token.get_mut(token)
        {
            entry.url_kind = PageUrlKind::RealOrUnknown;
        }
    }

    /// Transitions an active (`Reserved` or `Bound`) document to `Retired`. Returns `None` if
    /// `token` is unknown or already `Retired`.
    pub(crate) fn retire_token(&mut self, token: &DocToken) -> Option<RetiredDoc> {
        let (seq, url_kind, frame) = {
            let entry = self.by_token.get_mut(token)?;
            let seq = match &entry.binding {
                DocBinding::Reserved { seq, .. } | DocBinding::Bound { seq, .. } => *seq,
                DocBinding::Retired { .. } => return None,
            };
            let frame = entry.frame().cloned();
            let url_kind = entry.url_kind;
            entry.binding = DocBinding::Retired {
                seq,
                at: Stamp::next(),
            };
            (seq, url_kind, frame)
        };
        if let Some(frame) = frame {
            self.clear_active_frame(&frame, token);
        }
        self.retired_since_take.push(token.clone());
        Some(RetiredDoc { seq, url_kind })
    }

    /// Takes every token retired since the last call, in retirement order.
    #[must_use = "a dropped token keeps its document's refs live"]
    pub(crate) fn take_retired(&mut self) -> Vec<DocToken> {
        mem::take(&mut self.retired_since_take)
    }

    fn retire_other_active_on_frame(&mut self, frame: &FrameId, incoming: &DocToken) {
        if let Some(existing) = self.active_by_frame.get(frame)
            && existing != incoming
        {
            let existing = existing.clone();
            let _ = self.retire_token(&existing);
        }
    }

    fn clear_active_frame(&mut self, frame: &FrameId, token: &DocToken) {
        if self.active_by_frame.get(frame) == Some(token) {
            self.active_by_frame.remove(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::FrameId;
    use neo_snapshot_apc::tree::DocToken;

    use super::{DocContext, DocPlace, DocTable};
    use crate::world::{FrameSeq, Stamp};

    type TestResult = Result<(), Box<dyn Error>>;

    #[test]
    fn debug_formats_counts_and_ids_without_printing_page_urls() -> TestResult {
        let secret_url = "https://secret-account.example/reset?token=super-secret-value";
        let token = DocToken::new("doc-secret-check").ok_or("non-empty doc token")?;
        let frame = FrameId("frame-main".to_owned());
        let ctx = DocContext {
            token: &token,
            frame: Some(&frame),
            place: DocPlace::Main,
            url: Some(secret_url),
            requested_at: Stamp::next(),
        };
        let mut table = DocTable::new();
        table.insert_reserved(
            token.clone(),
            FrameSeq(0),
            Some(frame.clone()),
            Some(secret_url),
        );

        let table_debug = format!("{table:?}");
        let ctx_debug = format!("{ctx:?}");
        assert!(
            !table_debug.contains("secret-account") && !table_debug.contains("super-secret-value"),
            "DocTable Debug must never leak page URLs: {table_debug}"
        );
        assert!(
            !ctx_debug.contains("secret-account") && !ctx_debug.contains("super-secret-value"),
            "DocContext Debug must never leak page URLs: {ctx_debug}"
        );
        assert!(
            table_debug.contains("docs: 1") && table_debug.contains("reserved: 1"),
            "DocTable Debug should report structural counts: {table_debug}"
        );
        Ok(())
    }

    #[test]
    fn retirements_queue_each_token_once_until_taken() -> TestResult {
        let frame = FrameId("frame-main".to_owned());
        let first = DocToken::new("doc-first").ok_or("non-empty doc token")?;
        let second = DocToken::new("doc-second").ok_or("non-empty doc token")?;
        let mut table = DocTable::new();
        table.insert_reserved(first.clone(), FrameSeq(0), Some(frame.clone()), None);
        table.insert_reserved(second.clone(), FrameSeq(1), Some(frame), None);
        assert_eq!(
            table.take_retired(),
            vec![first.clone()],
            "a document displaced from its frame must be queued"
        );
        assert!(
            table.take_retired().is_empty(),
            "taking must empty the queue"
        );

        assert!(
            table.retire_token(&second).is_some(),
            "an active document retires"
        );
        assert!(
            table.retire_token(&second).is_none(),
            "retirement is terminal, so a second retire changes nothing"
        );
        assert!(
            table.retire_token(&first).is_none(),
            "a document displaced earlier is already retired"
        );
        assert_eq!(
            table.take_retired(),
            vec![second],
            "each retirement must be queued exactly once"
        );
        Ok(())
    }
}
