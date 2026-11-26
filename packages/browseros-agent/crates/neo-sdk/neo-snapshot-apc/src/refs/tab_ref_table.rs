use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::num::NonZeroU32;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::tree::{DocSeq, DocToken, DomNodeId, NodeKey, Role};

/// Default upper bound on retained node records per tab before evicting the oldest unseen entries.
pub const DEFAULT_MAX_RECORDS: usize = 200_000;

/// Origin channel of a [`Ref`]: printed in an ARIA snapshot (`e`-ref) or minted on demand (`n`-ref).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RefKind {
    /// Reference assigned during snapshot emission (`e<n>` or `f<seq>e<n>`).
    Snapshot,
    /// Reference minted on demand by `locator.ref()` (`n<n>` or `f<seq>n<n>`).
    Minted,
}

impl RefKind {
    const fn prefix(self) -> char {
        match self {
            Self::Snapshot => 'e',
            Self::Minted => 'n',
        }
    }

    pub(crate) const fn prefix_byte(self) -> u8 {
        match self {
            Self::Snapshot => b'e',
            Self::Minted => b'n',
        }
    }
}

/// Error returned when parsing a string that does not match canonical [`Ref`] syntax.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid ref syntax")]
pub struct RefParseError;

/// Canonical compact reference identifying a DOM element within a tab's [`TabRefTable`].
///
/// Formats as `e<n>` / `n<n>` when [`Self::seq`] is `0` (typically the main document) and as
/// `f<seq>e<n>` / `f<seq>n<n>` for non-zero document sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ref {
    seq: DocSeq,
    kind: RefKind,
    element: NonZeroU32,
}

impl Ref {
    /// Constructs a [`Ref`] from its document sequence, channel kind, and 1-based element counter.
    #[must_use]
    pub const fn new(seq: DocSeq, kind: RefKind, element: NonZeroU32) -> Self {
        Self { seq, kind, element }
    }

    /// Constructs a snapshot (`e`-ref) for `seq` and `element`.
    #[must_use]
    pub const fn snapshot(seq: DocSeq, element: NonZeroU32) -> Self {
        Self::new(seq, RefKind::Snapshot, element)
    }

    /// Constructs an on-demand minted (`n`-ref) for `seq` and `element`.
    #[must_use]
    pub const fn minted(seq: DocSeq, element: NonZeroU32) -> Self {
        Self::new(seq, RefKind::Minted, element)
    }

    /// Document sequence number that scopes this reference.
    #[must_use]
    pub const fn seq(self) -> DocSeq {
        self.seq
    }

    /// Whether this reference came from snapshot emission or on-demand minting.
    #[must_use]
    pub const fn kind(self) -> RefKind {
        self.kind
    }

    /// Monotonic 1-based per-document, per-kind element index.
    #[must_use]
    pub const fn element(self) -> NonZeroU32 {
        self.element
    }

    /// Parses an exact canonical reference (`[f<seq>]e<n>` or `[f<seq>]n<n>`).
    ///
    /// Rejects signs, whitespace, leading zeroes, zero counters (`e0`, `n0`), and redundant
    /// `f0` prefixes (`f0e5`, `f0n1`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let (seq, rest) = if let Some(after_f) = raw.strip_prefix('f') {
            let split_at = after_f.find(['e', 'n'])?;
            let (seq_digits, tail) = after_f.split_at_checked(split_at)?;
            let seq = parse_positive_u32(seq_digits)?;
            (DocSeq::new(seq.get()), tail)
        } else {
            (DocSeq::new(0), raw)
        };

        let (kind, num_digits) = if let Some(digits) = rest.strip_prefix('e') {
            (RefKind::Snapshot, digits)
        } else {
            let digits = rest.strip_prefix('n')?;
            (RefKind::Minted, digits)
        };

        let element = parse_positive_u32(num_digits)?;
        Some(Self { seq, kind, element })
    }
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = self.kind.prefix();
        if self.seq.raw() == 0 {
            write!(f, "{kind}{}", self.element)
        } else {
            write!(f, "f{}{kind}{}", self.seq.raw(), self.element)
        }
    }
}

impl FromStr for Ref {
    type Err = RefParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or(RefParseError)
    }
}

impl Serialize for Ref {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

struct RefVisitor;

impl Visitor<'_> for RefVisitor {
    type Value = Ref;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a canonical ref such as `e1`, `n1`, `f1e1`, or `f1n1`")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ref::parse(value).ok_or_else(|| E::custom(RefParseError))
    }
}

impl<'de> Deserialize<'de> for Ref {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(RefVisitor)
    }
}

/// Resolved DOM target and optional interactive click proxy for a live [`Ref`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefTarget {
    /// Composite document-and-node key of the target DOM element.
    pub key: NodeKey,
    /// Optional click-proxy target when an enclosing interactive element stands in for [`Self::key`].
    pub click_proxy: Option<NodeKey>,
}

/// Resolution outcome for looking up a [`Ref`] in a [`TabRefTable`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefLookup {
    /// The reference is live and resolves to a DOM node target.
    Key(RefTarget),
    /// The reference belonged to a document or node that has since been retired, forgotten, or
    /// evicted.
    Retired,
    /// The reference has never been issued by this table (or failed to parse).
    Unknown,
}

impl RefLookup {
    /// Returns the resolved [`RefTarget`] when this lookup is [`Self::Key`].
    #[must_use]
    pub const fn as_key(&self) -> Option<&RefTarget> {
        match self {
            Self::Key(target) => Some(target),
            Self::Retired | Self::Unknown => None,
        }
    }

    /// Whether this lookup resolved to [`Self::Retired`].
    #[must_use]
    pub const fn is_retired(&self) -> bool {
        matches!(self, Self::Retired)
    }

    /// Whether this lookup resolved to [`Self::Unknown`].
    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

enum CompactRole {
    Aria(Role),
    Iframe,
    Other(Box<str>),
}

impl CompactRole {
    fn from_role_str(role: &str) -> Self {
        if role == "iframe" {
            Self::Iframe
        } else if let Some(aria) = Role::parse(role) {
            Self::Aria(aria)
        } else {
            Self::Other(Box::from(role))
        }
    }

    fn matches_str(&self, role: &str) -> bool {
        match self {
            Self::Aria(aria) => aria.as_str() == role,
            Self::Iframe => role == "iframe",
            Self::Other(other) => other.as_ref() == role,
        }
    }
}

struct PrintedIdentity {
    role: CompactRole,
    name: String,
}

impl PrintedIdentity {
    fn new(role: &str, name: &str) -> Self {
        Self {
            role: CompactRole::from_role_str(role),
            name: name.to_owned(),
        }
    }

    fn matches(&self, role: &str, name: &str) -> bool {
        self.name == name && self.role.matches_str(role)
    }
}

struct NodeRecord {
    doc_seq: DocSeq,
    current_ref: Option<Ref>,
    minted_ref: Option<Ref>,
    aliases: Vec<Ref>,
    last_identity: Option<PrintedIdentity>,
    click_proxy: Option<NodeKey>,
    last_seen_doc_epoch: u64,
    last_seen_tick: u64,
}

struct DocState {
    token: DocToken,
    capture_epoch: u64,
}

#[derive(Clone, Copy)]
struct SeqCounters {
    next_e: u64,
    next_n: u64,
}

impl SeqCounters {
    const fn new() -> Self {
        Self {
            next_e: 1,
            next_n: 1,
        }
    }

    fn was_issued(self, kind: RefKind, element: NonZeroU32) -> bool {
        let next = match kind {
            RefKind::Snapshot => self.next_e,
            RefKind::Minted => self.next_n,
        };
        u64::from(element.get()) < next
    }

    fn alloc(&mut self, kind: RefKind) -> NonZeroU32 {
        let slot = match kind {
            RefKind::Snapshot => &mut self.next_e,
            RefKind::Minted => &mut self.next_n,
        };
        let current = *slot;
        *slot = current.saturating_add(1);
        u32::try_from(current)
            .ok()
            .and_then(NonZeroU32::new)
            .unwrap_or(NonZeroU32::MAX)
    }
}

/// Fast non-cryptographic integer hasher for internal `DomNodeId` and `Ref` lookup tables.
#[derive(Clone, Default)]
struct FastIntBuildHasher;

impl BuildHasher for FastIntBuildHasher {
    type Hasher = FastIntHasher;

    fn build_hasher(&self) -> Self::Hasher {
        FastIntHasher(0)
    }
}

struct FastIntHasher(u64);

impl FastIntHasher {
    #[inline]
    fn mix(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for FastIntHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.mix(u64::from(byte));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.mix(u64::from(i));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.mix(u64::from(i));
    }

    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.mix(u64::from(i.cast_unsigned()));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.mix(i);
    }

    #[inline]
    fn write_isize(&mut self, i: isize) {
        self.mix(u64::try_from(i.cast_unsigned()).unwrap_or(0));
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.mix(u64::try_from(i).unwrap_or(0));
    }
}

type FastIntMap<K, V> = HashMap<K, V, FastIntBuildHasher>;

/// Per-tab stable reference registry mapping DOM [`NodeKey`]s to canonical [`Ref`]s across
/// repeated and delta snapshots.
///
/// Reuses the same `e`-ref while a node's `(role, name)` stays unchanged, mints a fresh `e`-ref
/// while keeping earlier refs resolvable as aliases when `(role, name)` changes, unifies on-demand
/// `n`-refs with later snapshot `e`-refs, and bounds memory by evicting the oldest unseen records
/// past [`Self::max_records`]. Its manual `Debug` implementation prints counts only so cached
/// accessible names never leak into logs.
pub struct TabRefTable {
    by_doc: HashMap<DocToken, FastIntMap<DomNodeId, NodeRecord>>,
    by_ref: FastIntMap<Ref, NodeKey>,
    doc_to_seq: HashMap<DocToken, DocSeq>,
    seq_to_doc: HashMap<DocSeq, DocState>,
    seq_counters: HashMap<DocSeq, SeqCounters>,
    retired_seqs: HashSet<DocSeq>,
    next_auto_seq: u32,
    tick: u64,
    max_records: usize,
}

/// Document-scoped ref assignment handle into a [`TabRefTable`].
///
/// Resolves the document's record map, sequence counters, and capture epoch once per document so
/// snapshot emission hashes only the 4-byte [`DomNodeId`] per node instead of hashing and cloning
/// the owning [`DocToken`] on every element.
pub(crate) struct DocRefSession<'a> {
    doc: DocToken,
    doc_seq: DocSeq,
    epoch: u64,
    records: &'a mut FastIntMap<DomNodeId, NodeRecord>,
    by_ref: &'a mut FastIntMap<Ref, NodeKey>,
    counters: &'a mut SeqCounters,
    tick: &'a mut u64,
}

impl DocRefSession<'_> {
    pub(crate) fn assign(
        &mut self,
        dom_id: DomNodeId,
        role: &str,
        name: &str,
        click_proxy: Option<NodeKey>,
    ) -> Ref {
        *self.tick = self.tick.saturating_add(1);
        let tick = *self.tick;

        if let Some(record) = self.records.get_mut(&dom_id) {
            if record.doc_seq == self.doc_seq
                && let Some(current) = record.current_ref
                && record
                    .last_identity
                    .as_ref()
                    .is_some_and(|identity| identity.matches(role, name))
            {
                record.click_proxy = click_proxy;
                record.last_seen_doc_epoch = self.epoch;
                record.last_seen_tick = tick;
                return current;
            }

            let fresh = Ref::new(
                self.doc_seq,
                RefKind::Snapshot,
                self.counters.alloc(RefKind::Snapshot),
            );
            if let Some(previous) = record.current_ref.replace(fresh) {
                record.aliases.push(previous);
            }
            record.doc_seq = self.doc_seq;
            record.last_identity = Some(PrintedIdentity::new(role, name));
            record.click_proxy = click_proxy;
            record.last_seen_doc_epoch = self.epoch;
            record.last_seen_tick = tick;
            self.by_ref
                .insert(fresh, NodeKey::new(self.doc.clone(), dom_id));
            return fresh;
        }

        let fresh = Ref::new(
            self.doc_seq,
            RefKind::Snapshot,
            self.counters.alloc(RefKind::Snapshot),
        );
        self.records.insert(
            dom_id,
            NodeRecord {
                doc_seq: self.doc_seq,
                current_ref: Some(fresh),
                minted_ref: None,
                aliases: Vec::new(),
                last_identity: Some(PrintedIdentity::new(role, name)),
                click_proxy,
                last_seen_doc_epoch: self.epoch,
                last_seen_tick: tick,
            },
        );
        self.by_ref
            .insert(fresh, NodeKey::new(self.doc.clone(), dom_id));
        fresh
    }
}

impl TabRefTable {
    /// Creates an empty table with [`DEFAULT_MAX_RECORDS`] capacity before unseen eviction.
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_records(DEFAULT_MAX_RECORDS)
    }

    /// Creates an empty table enforcing a custom `max_records` cap for unseen-record eviction.
    #[must_use]
    pub fn with_max_records(max_records: usize) -> Self {
        Self {
            by_doc: HashMap::new(),
            by_ref: HashMap::with_hasher(FastIntBuildHasher),
            doc_to_seq: HashMap::new(),
            seq_to_doc: HashMap::new(),
            seq_counters: HashMap::new(),
            retired_seqs: HashSet::new(),
            next_auto_seq: 0,
            tick: 0,
            max_records,
        }
    }

    /// Maximum number of retained node records before unseen records are evicted on capture finish.
    #[must_use]
    pub const fn max_records(&self) -> usize {
        self.max_records
    }

    /// Number of live DOM node records currently tracked by this table.
    #[must_use]
    pub fn record_count(&self) -> usize {
        self.by_doc.values().map(HashMap::len).sum()
    }

    /// Number of live [`Ref`] entries (including aliases and `n`-refs) currently resolvable.
    #[must_use]
    pub fn ref_count(&self) -> usize {
        self.by_ref.len()
    }

    /// Returns or binds the [`DocSeq`] for `doc`.
    ///
    /// When `assigned` is `Some`, binds `doc` to that sequence, advances auto-allocation past it,
    /// and cleanly retires or displaces conflicting bindings. When `None`, returns `doc`'s
    /// existing sequence or allocates the next unused sequence starting at `0`.
    pub fn seq_for_doc(&mut self, doc: &DocToken, assigned: Option<DocSeq>) -> DocSeq {
        if let Some(seq) = assigned {
            return self.bind_assigned_seq(doc, seq);
        }
        if let Some(&existing) = self.doc_to_seq.get(doc) {
            return existing;
        }
        let seq = self.alloc_auto_seq();
        self.doc_to_seq.insert(doc.clone(), seq);
        self.seq_to_doc.insert(
            seq,
            DocState {
                token: doc.clone(),
                capture_epoch: 0,
            },
        );
        self.seq_counters
            .entry(seq)
            .or_insert_with(SeqCounters::new);
        seq
    }

    /// Begins a snapshot capture pass for `doc`, incrementing its capture epoch and returning its
    /// [`DocSeq`].
    pub fn begin_doc_capture(&mut self, doc: &DocToken, assigned_seq: Option<DocSeq>) -> DocSeq {
        let seq = self.seq_for_doc(doc, assigned_seq);
        if let Some(state) = self.seq_to_doc.get_mut(&seq) {
            state.capture_epoch = state.capture_epoch.saturating_add(1);
        }
        seq
    }

    /// Opens a document-scoped assignment session bound to `(doc, doc_seq)`.
    pub(crate) fn doc_session(&mut self, doc: &DocToken, doc_seq: DocSeq) -> DocRefSession<'_> {
        if self.doc_to_seq.get(doc) != Some(&doc_seq) {
            self.bind_assigned_seq(doc, doc_seq);
        }
        let epoch = self
            .seq_to_doc
            .get(&doc_seq)
            .map_or(0, |state| state.capture_epoch);
        let records = self.by_doc.entry(doc.clone()).or_default();
        let counters = self
            .seq_counters
            .entry(doc_seq)
            .or_insert_with(SeqCounters::new);
        DocRefSession {
            doc: doc.clone(),
            doc_seq,
            epoch,
            records,
            by_ref: &mut self.by_ref,
            counters,
            tick: &mut self.tick,
        }
    }

    /// Assigns or reuses a snapshot (`e`-ref) for `key` given its printed `(role, name)` and
    /// optional `click_proxy`.
    ///
    /// Reuses the node's current `e`-ref when `(role, name)` and `doc_seq` are unchanged;
    /// otherwise mints a fresh `e`-ref while keeping prior `e`-refs and any existing `n`-ref
    /// resolvable to `key`.
    pub fn assign_snapshot_ref(
        &mut self,
        doc_seq: DocSeq,
        key: &NodeKey,
        role: &str,
        name: &str,
        click_proxy: Option<NodeKey>,
    ) -> Ref {
        self.doc_session(&key.doc, doc_seq)
            .assign(key.node, role, name, click_proxy)
    }

    /// Finalizes a snapshot capture pass for `doc`; `present` says which keys the capture holds.
    ///
    /// A record whose node the capture holds counts as seen, ref or no ref: a node that lost its
    /// ref for a capture, say under `pointer-events: none`, keeps its printed identity, so it gets
    /// the same `e`-ref back with the same role and name, as upstream's `_ariaRef` does. A record
    /// whose node the capture lacks frees that identity but stays resolvable (design 3.6). Then the
    /// oldest unseen records across all documents are evicted while [`Self::record_count`] exceeds
    /// [`Self::max_records`].
    pub fn finish_doc_capture(&mut self, doc: &DocToken, present: impl Fn(&NodeKey) -> bool) {
        if let Some(&seq) = self.doc_to_seq.get(doc)
            && let Some(state) = self.seq_to_doc.get(&seq)
        {
            let epoch = state.capture_epoch;
            let tick = self.next_tick();
            if let Some(records) = self.by_doc.get_mut(doc) {
                for (&dom_id, record) in records {
                    if record.last_seen_doc_epoch >= epoch {
                        continue;
                    }
                    let key = NodeKey::new(doc.clone(), dom_id);
                    if present(&key) {
                        record.last_seen_doc_epoch = epoch;
                        record.last_seen_tick = tick;
                    } else {
                        record.last_identity = None;
                    }
                }
            }
        }

        let total = self.record_count();
        if total <= self.max_records {
            return;
        }
        let excess = total - self.max_records;

        let mut unseen: Vec<(u64, NodeKey)> = self
            .by_doc
            .iter()
            .flat_map(|(doc_token, records)| {
                let current_epoch = self
                    .doc_to_seq
                    .get(doc_token)
                    .and_then(|seq| self.seq_to_doc.get(seq))
                    .map(|state| state.capture_epoch);
                records.iter().filter_map(move |(&dom_id, record)| {
                    let is_unseen =
                        current_epoch.is_none_or(|epoch| record.last_seen_doc_epoch < epoch);
                    is_unseen.then(|| {
                        (
                            record.last_seen_tick,
                            NodeKey::new(doc_token.clone(), dom_id),
                        )
                    })
                })
            })
            .collect();

        unseen.sort_unstable();
        for (_, key) in unseen.into_iter().take(excess) {
            self.remove_record(&key);
        }
    }

    /// Returns a live [`Ref`] for `key`, preferring its snapshot `e`-ref, then its existing
    /// minted `n`-ref, or minting a fresh `n`-ref without forcing snapshot inclusion.
    pub fn ref_for(&mut self, key: &NodeKey, assigned_seq: Option<DocSeq>) -> Ref {
        let expected_seq = assigned_seq.or_else(|| self.doc_to_seq.get(&key.doc).copied());
        if let Some(record) = self.by_doc.get(&key.doc).and_then(|m| m.get(&key.node)) {
            if let Some(current) = record.current_ref
                && expected_seq.is_none_or(|seq| current.seq() == seq)
            {
                return current;
            }
            if let Some(minted) = record.minted_ref
                && expected_seq.is_none_or(|seq| minted.seq() == seq)
            {
                return minted;
            }
        }

        let doc_seq = self.seq_for_doc(&key.doc, assigned_seq);
        let epoch = self
            .seq_to_doc
            .get(&doc_seq)
            .map_or(0, |state| state.capture_epoch);
        let tick = self.next_tick();
        let fresh = Self::mint_ref_from(&mut self.seq_counters, doc_seq, RefKind::Minted);

        let records = self.by_doc.entry(key.doc.clone()).or_default();
        if let Some(record) = records.get_mut(&key.node) {
            if let Some(prev_current) = record.current_ref.take() {
                record.aliases.push(prev_current);
            }
            if let Some(prev_minted) = record.minted_ref.replace(fresh) {
                record.aliases.push(prev_minted);
            }
            record.doc_seq = doc_seq;
            record.last_identity = None;
            record.last_seen_tick = tick;
        } else {
            records.insert(
                key.node,
                NodeRecord {
                    doc_seq,
                    current_ref: None,
                    minted_ref: Some(fresh),
                    aliases: Vec::new(),
                    last_identity: None,
                    click_proxy: None,
                    last_seen_doc_epoch: epoch,
                    last_seen_tick: tick,
                },
            );
        }
        self.by_ref.insert(fresh, key.clone());
        fresh
    }

    /// Resolves a parsed [`Ref`] to a live [`RefTarget`], [`RefLookup::Retired`], or
    /// [`RefLookup::Unknown`].
    #[must_use]
    pub fn resolve(&self, r: &Ref) -> RefLookup {
        if let Some(key) = self.by_ref.get(r)
            && let Some(record) = self.by_doc.get(&key.doc).and_then(|m| m.get(&key.node))
        {
            return RefLookup::Key(RefTarget {
                key: key.clone(),
                click_proxy: record.click_proxy.clone(),
            });
        }
        if self.retired_seqs.contains(&r.seq()) {
            return RefLookup::Retired;
        }
        if self
            .seq_counters
            .get(&r.seq())
            .is_some_and(|counters| counters.was_issued(r.kind(), r.element()))
        {
            return RefLookup::Retired;
        }
        RefLookup::Unknown
    }

    /// Parses `raw` as a canonical [`Ref`] and resolves it, returning [`RefLookup::Unknown`] when
    /// parsing fails.
    #[must_use]
    pub fn resolve_str(&self, raw: &str) -> RefLookup {
        let Some(parsed) = Ref::parse(raw) else {
            return RefLookup::Unknown;
        };
        self.resolve(&parsed)
    }

    /// Retires `doc`, removing all of its records and marking its sequence retired.
    pub fn retire_doc(&mut self, doc: &DocToken) {
        self.remove_doc_records(doc);
        if let Some(seq) = self.doc_to_seq.remove(doc) {
            self.seq_to_doc.remove(&seq);
            self.retired_seqs.insert(seq);
        }
    }

    /// Retires `seq`, marking it retired and removing any owning document's records and live refs.
    pub fn retire_seq(&mut self, seq: DocSeq) {
        self.retired_seqs.insert(seq);
        if let Some(state) = self.seq_to_doc.remove(&seq) {
            self.doc_to_seq.remove(&state.token);
            self.remove_doc_records(&state.token);
        }
        self.by_ref.retain(|r, _| r.seq() != seq);
    }

    /// Drops all records and live refs for `doc` while retaining its sequence binding and
    /// monotonic `e`/`n` counters so prior refs resolve to [`RefLookup::Retired`] and never
    /// collide with future captures.
    pub fn forget_doc(&mut self, doc: &DocToken) {
        self.remove_doc_records(doc);
    }

    /// Drops all records and live refs across every document while retaining monotonic per-seq
    /// counters so every previously issued ref resolves to [`RefLookup::Retired`] and never
    /// collides with future captures.
    pub fn forget(&mut self) {
        self.by_doc.clear();
        self.by_ref.clear();
    }

    fn bind_assigned_seq(&mut self, doc: &DocToken, seq: DocSeq) -> DocSeq {
        if seq.raw() >= self.next_auto_seq {
            self.next_auto_seq = seq.raw().saturating_add(1);
        }

        if self.doc_to_seq.get(doc) == Some(&seq) {
            self.retired_seqs.remove(&seq);
            self.seq_counters
                .entry(seq)
                .or_insert_with(SeqCounters::new);
            return seq;
        }

        let mut preserved_epoch = 0;
        if let Some(old_seq) = self.doc_to_seq.remove(doc) {
            if let Some(old_state) = self.seq_to_doc.remove(&old_seq) {
                preserved_epoch = old_state.capture_epoch;
            }
            self.retired_seqs.insert(old_seq);
        }

        if let Some(displaced) = self.seq_to_doc.remove(&seq)
            && displaced.token != *doc
        {
            self.doc_to_seq.remove(&displaced.token);
            self.remove_doc_records(&displaced.token);
        }

        self.retired_seqs.remove(&seq);
        self.doc_to_seq.insert(doc.clone(), seq);
        self.seq_to_doc.insert(
            seq,
            DocState {
                token: doc.clone(),
                capture_epoch: preserved_epoch,
            },
        );
        self.seq_counters
            .entry(seq)
            .or_insert_with(SeqCounters::new);
        seq
    }

    fn alloc_auto_seq(&mut self) -> DocSeq {
        while self.is_seq_unavailable(DocSeq::new(self.next_auto_seq))
            && self.next_auto_seq < u32::MAX
        {
            self.next_auto_seq += 1;
        }
        let seq = DocSeq::new(self.next_auto_seq);
        self.next_auto_seq = self.next_auto_seq.saturating_add(1);
        seq
    }

    fn is_seq_unavailable(&self, seq: DocSeq) -> bool {
        self.seq_to_doc.contains_key(&seq)
            || self.retired_seqs.contains(&seq)
            || self.seq_counters.contains_key(&seq)
    }

    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.saturating_add(1);
        self.tick
    }

    fn mint_ref_from(
        seq_counters: &mut HashMap<DocSeq, SeqCounters>,
        doc_seq: DocSeq,
        kind: RefKind,
    ) -> Ref {
        let element = seq_counters
            .entry(doc_seq)
            .or_insert_with(SeqCounters::new)
            .alloc(kind);
        Ref::new(doc_seq, kind, element)
    }

    fn remove_doc_records(&mut self, doc: &DocToken) {
        if let Some(records) = self.by_doc.remove(doc) {
            for record in records.into_values() {
                self.remove_record_refs(record);
            }
        }
    }

    fn remove_record(&mut self, key: &NodeKey) {
        let (removed, empty) = if let Some(records) = self.by_doc.get_mut(&key.doc) {
            let removed = records.remove(&key.node);
            (removed, records.is_empty())
        } else {
            (None, false)
        };
        if empty {
            self.by_doc.remove(&key.doc);
        }
        if let Some(record) = removed {
            self.remove_record_refs(record);
        }
    }

    fn remove_record_refs(&mut self, record: NodeRecord) {
        if let Some(current) = record.current_ref {
            self.by_ref.remove(&current);
        }
        if let Some(minted) = record.minted_ref {
            self.by_ref.remove(&minted);
        }
        for alias in record.aliases {
            self.by_ref.remove(&alias);
        }
    }
}

impl Default for TabRefTable {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TabRefTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TabRefTable")
            .field("record_count", &self.record_count())
            .field("ref_count", &self.by_ref.len())
            .field("active_doc_count", &self.doc_to_seq.len())
            .field("retired_seq_count", &self.retired_seqs.len())
            .field("max_records", &self.max_records)
            .finish_non_exhaustive()
    }
}

fn parse_positive_u32(digits: &str) -> Option<NonZeroU32> {
    let mut bytes = digits.bytes();
    let first = bytes.next()?;
    if !matches!(first, b'1'..=b'9') {
        return None;
    }
    let mut value = u32::from(first - b'0');
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
    }
    NonZeroU32::new(value)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::num::NonZeroU32;

    use crate::tree::{DocSeq, DocToken, DomNodeId, NodeKey};

    use super::{
        DEFAULT_MAX_RECORDS, Ref, RefKind, RefLookup, RefParseError, RefTarget, TabRefTable,
    };

    fn nz(n: u32) -> Result<NonZeroU32, &'static str> {
        NonZeroU32::new(n).ok_or("expected positive u32")
    }

    fn doc(token: &str) -> Result<DocToken, &'static str> {
        DocToken::new(token).ok_or("expected non-empty DocToken")
    }

    fn key(token: &str, id: i32) -> Result<NodeKey, &'static str> {
        Ok(NodeKey::new(doc(token)?, DomNodeId::new(id)))
    }

    #[test]
    fn ref_parse_display_and_serde_enforce_canonical_grammar() -> Result<(), Box<dyn Error>> {
        let cases = [
            ("e1", Ref::snapshot(DocSeq::new(0), nz(1)?)),
            ("n7", Ref::minted(DocSeq::new(0), nz(7)?)),
            ("f1e5", Ref::snapshot(DocSeq::new(1), nz(5)?)),
            ("f12n34", Ref::minted(DocSeq::new(12), nz(34)?)),
            (
                "f4294967295e4294967295",
                Ref::snapshot(DocSeq::new(u32::MAX), nz(u32::MAX)?),
            ),
        ];

        for (raw, expected) in cases {
            assert_eq!(Ref::parse(raw), Some(expected));
            assert_eq!(raw.parse::<Ref>(), Ok(expected));
            assert_eq!(expected.to_string(), raw);
            let json = serde_json::to_string(&expected)?;
            assert_eq!(json, format!("\"{raw}\""));
            let round_tripped: Ref = serde_json::from_str(&json)?;
            assert_eq!(round_tripped, expected);
        }

        let invalid = [
            "",
            "e",
            "n",
            "f",
            "e0",
            "n0",
            "e01",
            "n007",
            "f0e5",
            "f0n1",
            "f01e5",
            "fe5",
            "f1e0",
            "f1n0",
            "f1e05",
            " e1",
            "e1 ",
            "e+1",
            "f+1e1",
            "f-1e1",
            "E1",
            "N1",
            "F1e1",
            "f1e2e3",
            "f1f2e3",
            "e4294967296",
            "f4294967296e1",
        ];

        for raw in invalid {
            assert_eq!(Ref::parse(raw), None, "expected `{raw}` to be rejected");
            assert_eq!(raw.parse::<Ref>(), Err(RefParseError));
        }

        let secret_payload = "\"SECRET_CANARY_REF_TEXT\"";
        let err = serde_json::from_str::<Ref>(secret_payload)
            .err()
            .ok_or("expected deserialize error")?;
        assert!(!err.to_string().contains("SECRET_CANARY_REF_TEXT"));
        assert!(err.to_string().contains("invalid ref syntax"));
        Ok(())
    }

    #[test]
    fn ticket_03_rule_1_and_2_reuse_ref_when_role_and_name_are_unchanged()
    -> Result<(), Box<dyn Error>> {
        let mut table = TabRefTable::new();
        assert_eq!(table.max_records(), DEFAULT_MAX_RECORDS);

        let main = doc("doc-main")?;
        let btn = key("doc-main", 10)?;
        let proxy1 = key("doc-main", 11)?;
        let proxy2 = key("doc-main", 12)?;

        let seq1 = table.begin_doc_capture(&main, None);
        assert_eq!(seq1, DocSeq::new(0));
        let r1 = table.assign_snapshot_ref(seq1, &btn, "button", "Submit", Some(proxy1));
        assert_eq!(r1.to_string(), "e1");
        table.finish_doc_capture(&main, |_| false);

        let seq2 = table.begin_doc_capture(&main, None);
        assert_eq!(seq2, DocSeq::new(0));
        let r2 = table.assign_snapshot_ref(seq2, &btn, "button", "Submit", Some(proxy2.clone()));
        assert_eq!(
            r2, r1,
            "unchanged (role, name) must keep the exact same e-ref"
        );
        table.finish_doc_capture(&main, |_| false);

        assert_eq!(
            table.resolve(&r1),
            RefLookup::Key(RefTarget {
                key: btn,
                click_proxy: Some(proxy2),
            }),
            "click_proxy must reflect the latest capture"
        );
        assert_eq!(table.record_count(), 1);
        assert_eq!(table.ref_count(), 1);
        Ok(())
    }

    #[test]
    fn a_node_the_capture_holds_keeps_its_identity_while_it_has_no_ref()
    -> Result<(), Box<dyn Error>> {
        let mut table = TabRefTable::new();
        let main = doc("doc-main")?;
        let pay = key("doc-main", 10)?;

        let seq = table.begin_doc_capture(&main, None);
        let first = table.assign_snapshot_ref(seq, &pay, "button", "Pay", None);
        table.finish_doc_capture(&main, |_| false);

        table.begin_doc_capture(&main, None);
        table.finish_doc_capture(&main, |candidate| *candidate == pay);
        let seq = table.begin_doc_capture(&main, None);
        let again = table.assign_snapshot_ref(seq, &pay, "button", "Pay", None);
        table.finish_doc_capture(&main, |_| false);
        assert_eq!(
            again, first,
            "present without a ref keeps the identity, so the ref returns"
        );

        table.begin_doc_capture(&main, None);
        table.finish_doc_capture(&main, |_| false);
        let seq = table.begin_doc_capture(&main, None);
        let after_absence = table.assign_snapshot_ref(seq, &pay, "button", "Pay", None);
        assert_ne!(
            after_absence, first,
            "absent from a capture frees the identity"
        );
        assert_eq!(
            table.resolve(&first),
            RefLookup::Key(RefTarget {
                key: pay,
                click_proxy: None,
            }),
            "the freed ref still resolves"
        );
        Ok(())
    }

    #[test]
    fn ticket_03_rule_3_mints_fresh_ref_and_keeps_old_aliases_on_identity_change()
    -> Result<(), Box<dyn Error>> {
        let mut table = TabRefTable::new();
        let main = doc("doc-main")?;
        let node = key("doc-main", 42)?;
        let proxy = key("doc-main", 43)?;

        let seq = table.begin_doc_capture(&main, None);
        let e1 = table.assign_snapshot_ref(seq, &node, "button", "Save", None);
        table.finish_doc_capture(&main, |_| false);

        let seq = table.begin_doc_capture(&main, None);
        let e2 = table.assign_snapshot_ref(seq, &node, "button", "Saved", Some(proxy.clone()));
        table.finish_doc_capture(&main, |_| false);

        let seq = table.begin_doc_capture(&main, None);
        let e3 = table.assign_snapshot_ref(seq, &node, "link", "Saved", Some(proxy.clone()));
        table.finish_doc_capture(&main, |_| false);

        assert_eq!(e1.to_string(), "e1");
        assert_eq!(e2.to_string(), "e2");
        assert_eq!(e3.to_string(), "e3");

        let expected = RefLookup::Key(RefTarget {
            key: node,
            click_proxy: Some(proxy),
        });
        assert_eq!(table.resolve(&e1), expected);
        assert_eq!(table.resolve(&e2), expected);
        assert_eq!(table.resolve(&e3), expected);
        assert_eq!(table.resolve_str("e1"), expected);
        assert_eq!(table.record_count(), 1);
        assert_eq!(table.ref_count(), 3);
        Ok(())
    }

    #[test]
    fn ticket_03_rule_4_unifies_pre_snapshot_minted_n_ref_with_later_e_ref()
    -> Result<(), Box<dyn Error>> {
        let mut table = TabRefTable::new();
        let main = doc("doc-main")?;
        let target = key("doc-main", 5)?;
        let hidden = key("doc-main", 6)?;
        let proxy = key("doc-main", 7)?;

        let n1 = table.ref_for(&target, None);
        assert_eq!(n1.to_string(), "n1");
        assert_eq!(n1.kind(), RefKind::Minted);
        assert_eq!(
            table.ref_for(&target, None),
            n1,
            "repeated ref_for reuses n-ref"
        );

        let n2 = table.ref_for(&hidden, None);
        assert_eq!(n2.to_string(), "n2");

        let seq = table.begin_doc_capture(&main, None);
        let e1 = table.assign_snapshot_ref(seq, &target, "button", "Go", Some(proxy.clone()));
        table.finish_doc_capture(&main, |_| false);

        assert_eq!(e1.to_string(), "e1");
        assert_eq!(
            table.ref_for(&target, None),
            e1,
            "ref_for must prefer live snapshot e-ref once assigned"
        );

        let expected_target = RefLookup::Key(RefTarget {
            key: target,
            click_proxy: Some(proxy),
        });
        assert_eq!(table.resolve(&n1), expected_target);
        assert_eq!(table.resolve(&e1), expected_target);
        assert_eq!(
            table.resolve(&n2),
            RefLookup::Key(RefTarget {
                key: hidden,
                click_proxy: None,
            }),
            "un-snapshotted n-ref remains resolvable without forcing snapshot inclusion"
        );
        Ok(())
    }

    #[test]
    fn unseen_records_drop_identity_keep_ref_and_evict_oldest_past_cap()
    -> Result<(), Box<dyn Error>> {
        let mut table = TabRefTable::with_max_records(2);
        let main = doc("doc-main")?;
        let k1 = key("doc-main", 1)?;
        let k2 = key("doc-main", 2)?;
        let k3 = key("doc-main", 3)?;

        // Capture 1 sees k1, k2, k3: none are evicted because all were seen in the active capture.
        let seq = table.begin_doc_capture(&main, None);
        let e1 = table.assign_snapshot_ref(seq, &k1, "button", "One", None);
        let e2 = table.assign_snapshot_ref(seq, &k2, "button", "Two", None);
        let e3 = table.assign_snapshot_ref(seq, &k3, "button", "Three", None);
        table.finish_doc_capture(&main, |_| false);
        assert_eq!(table.record_count(), 3);

        // Capture 2 sees only k3: k1 (oldest unseen) is evicted; k2 drops identity but stays live.
        let seq = table.begin_doc_capture(&main, None);
        let e3_again = table.assign_snapshot_ref(seq, &k3, "button", "Three", None);
        assert_eq!(e3_again, e3);
        table.finish_doc_capture(&main, |_| false);

        assert_eq!(table.record_count(), 2);
        assert!(table.resolve(&e1).is_retired());
        let resolved_e2 = table.resolve(&e2);
        assert_eq!(
            resolved_e2.as_key(),
            Some(&RefTarget {
                key: k2.clone(),
                click_proxy: None,
            })
        );
        let resolved_e3 = table.resolve(&e3);
        assert_eq!(
            resolved_e3.as_key(),
            Some(&RefTarget {
                key: k3,
                click_proxy: None,
            })
        );

        // When k2 reappears in capture 3 after its identity was dropped, it mints a fresh e-ref
        // while keeping e2 resolvable as an alias.
        let seq = table.begin_doc_capture(&main, None);
        let e2_new = table.assign_snapshot_ref(seq, &k2, "button", "Two", None);
        table.finish_doc_capture(&main, |_| false);
        assert_ne!(e2_new, e2);
        let expected_k2 = RefLookup::Key(RefTarget {
            key: k2,
            click_proxy: None,
        });
        assert_eq!(table.resolve(&e2), expected_k2);
        assert_eq!(table.resolve(&e2_new), expected_k2);
        Ok(())
    }

    #[test]
    fn retire_and_forget_transition_refs_without_colliding() -> Result<(), Box<dyn Error>> {
        let mut table = TabRefTable::new();
        let main = doc("doc-main")?;
        let iframe = doc("doc-iframe")?;
        let k_main = key("doc-main", 1)?;
        let k_iframe = key("doc-iframe", 1)?;

        let s_main = table.begin_doc_capture(&main, Some(DocSeq::new(0)));
        let e_main = table.assign_snapshot_ref(s_main, &k_main, "button", "Main", None);
        table.finish_doc_capture(&main, |_| false);

        let s_iframe = table.begin_doc_capture(&iframe, None);
        assert_eq!(s_iframe, DocSeq::new(1));
        let e_iframe = table.assign_snapshot_ref(s_iframe, &k_iframe, "iframe", "Sub", None);
        assert_eq!(e_iframe.to_string(), "f1e1");
        table.finish_doc_capture(&iframe, |_| false);

        // Unissued refs resolve to Unknown.
        assert!(table.resolve_str("e2").is_unknown());
        assert!(table.resolve_str("f9e1").is_unknown());
        assert!(table.resolve_str("bad-ref").is_unknown());

        // forget_doc keeps doc sequence and monotonic counters.
        table.forget_doc(&main);
        assert!(table.resolve(&e_main).is_retired());
        assert!(table.resolve(&e_iframe).as_key().is_some());

        let s_main_again = table.begin_doc_capture(&main, None);
        assert_eq!(s_main_again, DocSeq::new(0));
        let e_main_2 = table.assign_snapshot_ref(s_main_again, &k_main, "button", "Main", None);
        assert_eq!(e_main_2.to_string(), "e2");
        assert!(table.resolve(&e_main).is_retired());
        table.finish_doc_capture(&main, |_| false);

        // retire_doc retires the iframe's seq so even unissued refs on that seq resolve Retired.
        table.retire_doc(&iframe);
        assert!(table.resolve(&e_iframe).is_retired());
        assert!(table.resolve_str("f1e99").is_retired());

        // New auto-allocated doc skips retired seq 1.
        let iframe2 = doc("doc-iframe-2")?;
        let s_iframe2 = table.seq_for_doc(&iframe2, None);
        assert_eq!(s_iframe2, DocSeq::new(2));

        // forget() clears all records while preserving monotonic counters.
        table.forget();
        assert_eq!(table.record_count(), 0);
        assert_eq!(table.ref_count(), 0);
        assert!(table.resolve(&e_main_2).is_retired());
        assert!(table.resolve_str("e3").is_unknown());

        let s_main_3 = table.begin_doc_capture(&main, None);
        let e_main_3 = table.assign_snapshot_ref(s_main_3, &k_main, "button", "Main", None);
        assert_eq!(e_main_3.to_string(), "e3");

        // Explicit retire_seq removes owning doc records and marks the seq retired.
        table.retire_seq(DocSeq::new(0));
        assert!(table.resolve(&e_main_3).is_retired());
        assert_eq!(table.record_count(), 0);
        Ok(())
    }

    #[test]
    fn debug_output_redacts_cached_names_and_roles() -> Result<(), Box<dyn Error>> {
        let mut table = TabRefTable::new();
        let main = doc("doc-secret-token")?;
        let node = key("doc-secret-token", 99)?;

        let seq = table.begin_doc_capture(&main, None);
        table.assign_snapshot_ref(seq, &node, "custom-secret-role", "CANARY_SECRET_NAME", None);

        let debug = format!("{table:?}");
        assert!(!debug.contains("CANARY_SECRET_NAME"));
        assert!(!debug.contains("custom-secret-role"));
        assert!(!debug.contains("doc-secret-token"));
        assert!(debug.contains("record_count: 1"));
        assert!(debug.contains("ref_count: 1"));
        Ok(())
    }
}
