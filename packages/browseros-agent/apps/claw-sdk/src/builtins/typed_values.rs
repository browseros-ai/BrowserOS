//! What was typed into sensitive fields, or read from them, held until each field is done.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::{future, mem};

use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
use browseros_core::{PageId, TabId};
use browseros_trace::{SecretSet, SecretSetConfig, SecretValue};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Notify, broadcast};
use tokio::time::{self, Instant};
use zeroize::Zeroizing;

use crate::config::Defaults;
use crate::dispatch::{Held, HeldValues, HeldWatch};

/// Values typed into sensitive fields, or found there by a read, that the secret set has not
/// learned yet: the latest value of each field. The set learns a field's value once no entry
/// came for [`Defaults::TYPED_QUIET`], or when its tab closes, and never learns its prefixes.
/// This is sdk-traces decision 32, the rule the takeover's control journal follows for the
/// owner's typing, applied to what browseros-playwright's journal and reads report.
///
/// ```text
///  PrivacyHooks::field_entered ─ note: a value that extends a held one supersedes it
///  PrivacyHooks::values_swept ── sweep: a value no held one covers is held beside the rest,
///                                       and one a held value extends is kept as superseded
///    quiet for TYPED_QUIET ─────────────── every held value ─┬─▶ SecretSet::register
///    its tab closes (TabRegistry) ──────── that tab's ───────┘
///    superseded ──── kept while a run that began before it goes, then forgotten, never
///                    registered
///  RunCtx::start ─ watch (dispatch's HeldValues): the run's watch keeps what is superseded
///    while it goes, until the run's context drops
///  each record a run sends, and its reply ─ its watch: what is held and what was superseded
///    since it began, as a set of its own (dispatch's Held) ─▶ masked with it too
/// ```
///
/// The seal reports every keystroke as the field's whole value. Registering each entry made every
/// prefix of four characters or more a secret for a day, so a password that starts `https` would
/// mask `http` in every later read. A read reports each field's whole value too, and one between
/// two keystrokes finds a prefix, so what it finds waits the same way; the read has masked it out
/// of its own result already. While a value is held, the seal still masks the field itself in
/// every read. A run's reply is masked once, as the run ends, and may show any value held while
/// the run went on, so it is masked with them too (its [`HeldWatch`]): a read of an echo
/// mid-typing returns a prefix that the next keystroke supersedes before the run ends. Each trace
/// record the run sends is masked with them as well, since the trace writer's own set learns a
/// value only once it is registered (decision 34). Registering them then instead would make a
/// secret of whatever another field held at that moment, a prefix the owner is still typing
/// included.
///
/// A superseded value is kept while a run that began before it was superseded goes, and no
/// longer. At most [`Defaults::MAX_SUPERSEDED`] are kept. Past that, the oldest is forgotten
/// anyway, and each run that may have read it loses it ([`Held::Lost`]): what such a run produces
/// is withheld from then on, since nothing can mask it out any more.
pub(crate) struct TypedValues {
    secrets: Arc<SecretSet>,
    pending: Mutex<Pending>,
    /// Wakes the timer when an entry moves its deadline, and when this is dropped.
    wake: Arc<Notify>,
}

/// The held values, a read's in front and the keystrokes in the order typed, and when they are
/// registered unless another entry comes first; the values a keystroke superseded, oldest first;
/// and the runs going, which those are kept for.
#[derive(Default)]
struct Pending {
    values: Vec<Typed>,
    quiet_until: Option<Instant>,
    superseded: VecDeque<Superseded>,
    runs: Vec<Arc<Going>>,
}

/// One field's latest value and where it was typed or read. A plain data bag.
struct Typed {
    page: PageId,
    /// The page's tab when an op had touched it, so that the tab's close registers the value.
    tab: Option<TabId>,
    /// Zeroed when dropped.
    value: Zeroizing<String>,
}

/// A value that stopped being held when a keystroke superseded it: a prefix the field was typed
/// past, or what a backspace took back. Or one a read found that a held value extends, which may
/// be such a prefix or another field's value ([`Pending::sweep`]). A run that was going while it
/// was held may have read it, so it masks that run's records and reply for as long as such a run
/// goes. A plain data bag.
struct Superseded {
    /// Zeroed when dropped.
    value: Zeroizing<String>,
    /// When it stopped being held.
    until: Instant,
}

/// A run going, as what is kept for it sees it. [`Pending`] and the run's [`RunWatch`] share it.
struct Going {
    /// When the run began. Every value superseded since masks the run's records and reply.
    start: Instant,
    /// Set for good once a value superseded since `start` was forgotten past
    /// [`Defaults::MAX_SUPERSEDED`]. Written and read under [`Pending`]'s lock only; atomic so
    /// that the watch can share it.
    lost: AtomicBool,
}

/// One run's watch ([`HeldValues::watch`]). The run's context owns it; dropping it ends what is
/// kept for the run.
struct RunWatch {
    values: Arc<TypedValues>,
    going: Arc<Going>,
}

impl TypedValues {
    /// Starts the timer, which registers what is held once it is quiet, and a tab's values as
    /// soon as `closed` reports the tab gone.
    pub(crate) fn start(secrets: Arc<SecretSet>, closed: broadcast::Receiver<TabId>) -> Arc<Self> {
        let values = Arc::new(Self {
            secrets,
            pending: Mutex::default(),
            wake: Arc::new(Notify::new()),
        });
        tokio::spawn(register_when_due(
            Arc::downgrade(&values),
            Arc::clone(&values.wake),
            closed,
        ));
        values
    }

    /// Holds `entry`, the whole value of a field on `page` that just got input.
    pub(crate) fn note(&self, page: PageId, tab: Option<TabId>, entry: JournalEntry) {
        let typed = Typed {
            page,
            tab,
            value: entry.value,
        };
        let overflow = self.lock().note(typed, Instant::now());
        self.register(overflow);
        self.wake.notify_one();
    }

    /// Holds what a read found in the sensitive fields of `page`: each field's whole value.
    pub(crate) fn swept(&self, page: &PageId, tab: Option<TabId>, values: &FieldValues) {
        let swept = values
            .registrable()
            .map(|value| Typed {
                page: page.clone(),
                tab,
                value: Zeroizing::new(value.to_owned()),
            })
            .collect();
        let overflow = self.lock().sweep(swept, Instant::now());
        self.register(overflow);
        self.wake.notify_one();
    }

    /// Registers every held value now: when the values are dropped, and when a missed close could
    /// have been any held value's tab.
    fn register_all(&self) {
        let held = {
            let mut pending = self.lock();
            pending.quiet_until = None;
            mem::take(&mut pending.values)
        };
        self.register(held);
    }

    /// Registers the values typed on `tab`, which closed: its fields can take no more input.
    fn register_tab(&self, tab: TabId) {
        let closed = {
            let mut pending = self.lock();
            let (closed, open): (Vec<Typed>, Vec<Typed>) = mem::take(&mut pending.values)
                .into_iter()
                .partition(|typed| typed.tab == Some(tab));
            pending.values = open;
            closed
        };
        self.register(closed);
    }

    /// Registers what is held once no entry came for the quiet period.
    fn register_if_quiet(&self, now: Instant) {
        let held = {
            let mut pending = self.lock();
            match pending.quiet_until {
                Some(until) if until <= now => {
                    pending.quiet_until = None;
                    mem::take(&mut pending.values)
                }
                Some(_) | None => return,
            }
        };
        self.register(held);
    }

    /// When what is held goes quiet.
    fn due(&self) -> Option<Instant> {
        self.lock().quiet_until
    }

    /// Registers each value with the set's full TTL, moving its buffer into the secret, which
    /// zeroes it when dropped. A value the set already holds keeps the later expiry.
    fn register(&self, values: impl IntoIterator<Item = Typed>) {
        for Typed { mut value, .. } in values {
            // Too short to mask without masking ordinary text: the set keeps nothing, which is
            // its rule, not a failure here.
            let _registration = self
                .secrets
                .register(SecretValue::new(mem::take(&mut *value)), None);
        }
    }

    /// Every value held at any moment since `going` began, those a keystroke superseded since
    /// included, as a secret set of its own that registers nothing in the process's set.
    ///
    /// The reply is masked with the process's set first, which leaves a held value that extends a
    /// registered one as `[private]` plus the rest. So each value is here as typed and as that mask
    /// leaves it, and the reply hides it whole either way. The trace writer masks a record with
    /// both sets at once, where the value as typed is enough.
    fn held_since(&self, going: &Going) -> Held {
        // Copied out zeroed-on-drop, so the lock is not held while the set's matcher builds.
        let (values, lost) = {
            let pending = self.lock();
            let superseded = pending
                .superseded
                .iter()
                .filter(|old| old.until >= going.start)
                .map(|old| &old.value);
            let values: Vec<Zeroizing<String>> = pending
                .values
                .iter()
                .map(|typed| &typed.value)
                .chain(superseded)
                .cloned()
                .collect();
            (values, going.lost.load(Ordering::Relaxed))
        };
        let set = (!values.is_empty()).then(|| {
            let held = SecretSet::new(SecretSetConfig::default());
            for mut value in values {
                let mut masked = value.clone();
                if self.secrets.mask_str(&mut masked) {
                    let _registration =
                        held.register(SecretValue::new(mem::take(&mut *masked)), None);
                }
                let _registration = held.register(SecretValue::new(mem::take(&mut *value)), None);
            }
            Arc::new(held)
        });
        if lost {
            Held::Lost(set)
        } else {
            Held::Kept(set)
        }
    }

    /// Outside an await, and never held while registering.
    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl HeldValues for TypedValues {
    fn watch(self: Arc<Self>) -> Box<dyn HeldWatch> {
        let going = {
            let mut pending = self.lock();
            // Taken under the lock, so no value superseded after the run began can miss it.
            pending.run_began(Instant::now())
        };
        Box::new(RunWatch {
            values: self,
            going,
        })
    }
}

impl HeldWatch for RunWatch {
    fn held(&self) -> Held {
        self.values.held_since(&self.going)
    }
}

/// The run's context is gone, so nothing can send another of its records.
impl Drop for RunWatch {
    fn drop(&mut self) {
        self.values.lock().run_ended(&self.going);
    }
}

/// What is still held counts too, as the journal's own `Drop` has it; the wake ends the timer.
impl Drop for TypedValues {
    fn drop(&mut self) {
        self.register_all();
        self.wake.notify_one();
    }
}

impl Pending {
    /// Takes one entry and returns the value in front when more than [`Defaults::MAX_TYPED`] are
    /// held, to register at once: a read's, or else the one typed longest ago.
    ///
    /// The seal reports a value, not the field it came from, so within a page fields are told
    /// apart by how one value follows another, as the journal does. A value that extends or
    /// repeats one held for the same page is that field typed further, and the held one goes,
    /// superseded: it is a prefix. A value one character shorter than the value typed last, on
    /// the same page, is a backspace there, which supersedes that value. Anything else is held
    /// beside the rest. A shorter value is not taken for the same field otherwise: a security code
    /// that starts like its card number would erase a value nothing registered yet.
    fn note(&mut self, typed: Typed, now: Instant) -> Option<Typed> {
        let value = typed.value.as_str();
        let backspace = self.values.last().is_some_and(|last| {
            last.page == typed.page
                && last
                    .value
                    .strip_prefix(value)
                    .is_some_and(|removed| removed.chars().count() == 1)
        });
        if backspace && let Some(taken_back) = self.values.pop() {
            self.supersede(taken_back.value, now);
        }
        let typed_past: Vec<Typed> = self
            .values
            .extract_if(.., |held| {
                held.page == typed.page && value.starts_with(held.value.as_str())
            })
            .collect();
        for prefix in typed_past {
            if prefix.value.as_str() != value {
                self.supersede(prefix.value, now);
            }
        }
        self.values.push(typed);
        self.quiet_until = Some(now + Defaults::TYPED_QUIET);
        (self.values.len() > Defaults::MAX_TYPED).then(|| self.values.remove(0))
    }

    /// Takes the values one read found on a page and returns those in front past
    /// [`Defaults::MAX_TYPED`], to register at once.
    ///
    /// A read races the journal both ways, so it replaces nothing. Keys typed after the read
    /// leave its value a prefix of the one held, and a backspace after it leaves its value longer
    /// than the field's own. So a value that one held for the same page equals or extends is
    /// left out of what is held: it is held already, or the field was typed past it. Anything
    /// else is held beside the rest, in front, so that the value typed last stays last for the
    /// next backspace, and a later entry that extends it replaces it, as it does any held prefix.
    /// A read that finds nothing new does not restart the quiet period, so a page read over and
    /// over still goes quiet.
    ///
    /// A value a held one extends may still be another field's, and nothing else would mask it:
    /// `separatePW` in one field beside `separatePWX` in the next reads like a field typed past
    /// `separatePW`. So it is kept as a superseded value is, from now: it masks the runs going,
    /// and, like any prefix, it is never registered.
    fn sweep(&mut self, swept: Vec<Typed>, now: Instant) -> Vec<Typed> {
        let (covered, fresh): (Vec<Typed>, Vec<Typed>) = swept.into_iter().partition(|typed| {
            self.values
                .iter()
                .any(|held| held.page == typed.page && held.value.starts_with(typed.value.as_str()))
        });
        for left_out in covered {
            let held_already = self.values.iter().any(|held| {
                held.page == left_out.page && held.value.as_str() == left_out.value.as_str()
            });
            if !held_already {
                self.supersede(left_out.value, now);
            }
        }
        if fresh.is_empty() {
            return Vec::new();
        }
        self.values = fresh
            .into_iter()
            .chain(mem::take(&mut self.values))
            .collect();
        self.quiet_until = Some(now + Defaults::TYPED_QUIET);
        let excess = self.values.len().saturating_sub(Defaults::MAX_TYPED);
        self.values.drain(..excess).collect()
    }

    /// Keeps `value`, which stopped being held at `now` or which a read found under a held one,
    /// for the runs going. One too short to mask anything is dropped, and an older copy of it
    /// gives way to this one, which masks every run the older one did.
    fn supersede(&mut self, value: Zeroizing<String>, now: Instant) {
        if value.chars().count() < SecretSetConfig::default().min_length {
            return;
        }
        self.superseded.retain(|old| *old.value != *value);
        self.superseded.push_back(Superseded { value, until: now });
        if self.superseded.len() > Defaults::MAX_SUPERSEDED
            && let Some(gone) = self.superseded.pop_front()
        {
            // Every run that began at or before `gone` stopped being held may have read it, and
            // nothing masks it out of what those runs send from now on, so they are lost: their
            // output is withheld instead. Never registered, since it may be a prefix, and never
            // dropped unseen, since a lost run's output would then stay raw. Keeping it past the
            // cap instead only moves the question to another bound, and the matcher a record
            // builds over these values costs O(N²) in them (`SecretSet::register` rebuilds it
            // per value), which is what the cap bounds.
            for run in &self.runs {
                if run.start <= gone.until {
                    run.lost.store(true, Ordering::Relaxed);
                }
            }
        }
        self.forget_unneeded();
    }

    /// A run began at `now`: from now on, what is superseded is kept for it.
    fn run_began(&mut self, now: Instant) -> Arc<Going> {
        let going = Arc::new(Going {
            start: now,
            lost: AtomicBool::new(false),
        });
        self.runs.push(Arc::clone(&going));
        going
    }

    /// `going` ended: nothing is kept for it any more.
    fn run_ended(&mut self, going: &Arc<Going>) {
        self.runs.retain(|run| !Arc::ptr_eq(run, going));
        self.forget_unneeded();
    }

    /// Forgets the superseded values no run that is going and not lost can need: those
    /// superseded before the oldest such run began, or every one when there is none. A lost run
    /// needs none, since what it sends from then on is withheld.
    fn forget_unneeded(&mut self) {
        let oldest = self
            .runs
            .iter()
            .filter(|run| !run.lost.load(Ordering::Relaxed))
            .map(|run| run.start)
            .min();
        match oldest {
            Some(oldest) => {
                while self
                    .superseded
                    .front()
                    .is_some_and(|old| old.until < oldest)
                {
                    self.superseded.pop_front();
                }
            }
            None => self.superseded.clear(),
        }
    }
}

/// The timer: registers the held values once they are quiet, and a closed tab's values at once,
/// until the values are dropped. It holds no strong reference across an await, so dropping the
/// values ends it.
async fn register_when_due(
    values: Weak<TypedValues>,
    wake: Arc<Notify>,
    mut closed: broadcast::Receiver<TabId>,
) {
    let mut watching_closes = true;
    loop {
        let Some(due) = values.upgrade().map(|values| values.due()) else {
            return;
        };
        tokio::select! {
            () = sleep_until_due(due) => {
                if let Some(values) = values.upgrade() {
                    values.register_if_quiet(Instant::now());
                }
            }
            () = wake.notified() => {}
            closed = closed.recv(), if watching_closes => {
                let Some(values) = values.upgrade() else {
                    return;
                };
                match closed {
                    Ok(tab) => values.register_tab(tab),
                    // A close was missed, so any held value may be that tab's.
                    Err(RecvError::Lagged(_)) => values.register_all(),
                    Err(RecvError::Closed) => watching_closes = false,
                }
            }
        }
    }
}

/// Ends when `due` comes, and never while nothing is due.
async fn sleep_until_due(due: Option<Instant>) {
    match due {
        Some(due) => time::sleep_until(due).await,
        None => future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use browseros_core::sensitive_fields::{FieldValues, JournalEntry, SensitiveKind};
    use browseros_core::{PageId, TabId};
    use browseros_trace::{SecretSet, SecretSetConfig, SecretValue};
    use tokio::sync::broadcast;
    use tokio::time;

    use super::TypedValues;
    use crate::config::Defaults;
    use crate::dispatch::{Held, HeldValues, HeldWatch};

    fn keystroke(value: &str) -> JournalEntry {
        JournalEntry {
            kind: SensitiveKind::Password,
            value: value.to_owned().into(),
        }
    }

    fn masked(secrets: &SecretSet, text: &str) -> String {
        let mut text = text.to_owned();
        secrets.mask_str(&mut text);
        text
    }

    /// `text` as a run's reply is masked: with the process's set, then with what its watch holds.
    fn reply_masked(secrets: &SecretSet, held: &Held, text: &str) -> String {
        let mut text = masked(secrets, text);
        if let Some(set) = held.set() {
            set.mask_str(&mut text);
        }
        text
    }

    /// Values over a fresh secret set, and the sender a tab registry would close tabs on.
    fn typed_values() -> (Arc<SecretSet>, Arc<TypedValues>, broadcast::Sender<TabId>) {
        let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
        let (closes, closed) = broadcast::channel(16);
        let values = TypedValues::start(Arc::clone(&secrets), closed);
        (secrets, values, closes)
    }

    /// A run beginning now, as the door begins one.
    fn run(values: &Arc<TypedValues>) -> Box<dyn HeldWatch> {
        Arc::clone(values).watch()
    }

    /// What a read of `page` reports: each sensitive field's whole value.
    fn read(values: &TypedValues, page: &PageId, tab: TabId, fields: &[&str]) {
        let found = FieldValues::new(fields.iter().map(|field| (*field).to_owned().into()));
        values.swept(page, Some(tab), &found);
    }

    fn type_into(values: &TypedValues, page: &PageId, tab: TabId, typed: &str) {
        for end in 1..=typed.len() {
            let prefix = typed.get(..end).unwrap_or(typed);
            values.note(page.clone(), Some(tab), keystroke(prefix));
        }
    }

    /// A page that keeps raising input events on its own fields: each `…ab` supersedes its `…a`,
    /// so each pair supersedes one value.
    fn supersede_pairs(values: &TypedValues, page: &PageId, pairs: Range<usize>) {
        for pair in pairs {
            for value in [format!("v{pair:04}a"), format!("v{pair:04}ab")] {
                values.note(page.clone(), Some(TabId(1)), keystroke(&value));
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_field_typed_a_key_at_a_time_is_learned_once_quiet_and_its_prefixes_never() {
        let (secrets, values, _closes) = typed_values();
        let page = PageId(1);
        type_into(&values, &page, TabId(1), "hunter2");
        assert_eq!(
            masked(&secrets, "hunter2"),
            "hunter2",
            "the field may still be typed"
        );
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "hunt, hunte, hunter: hunter2"),
            "hunt, hunte, hunter: [private]"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn each_keystroke_starts_the_quiet_period_again() {
        let (secrets, values, _closes) = typed_values();
        let page = PageId(1);
        type_into(&values, &page, TabId(1), "hunter");
        time::sleep(Defaults::TYPED_QUIET.saturating_sub(Duration::from_millis(100))).await;
        values.note(page.clone(), Some(TabId(1)), keystroke("hunter2"));
        time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            masked(&secrets, "hunter"),
            "hunter",
            "a key came within the quiet period, so `hunter` was a prefix"
        );
        time::sleep(Defaults::TYPED_QUIET).await;
        assert_eq!(masked(&secrets, "hunter2"), "[private]");
    }

    #[tokio::test(start_paused = true)]
    async fn a_backspace_replaces_the_value_and_another_field_is_kept_beside_it() {
        let (secrets, values, _closes) = typed_values();
        let page = PageId(1);
        type_into(&values, &page, TabId(1), "hunter22");
        values.note(page.clone(), Some(TabId(1)), keystroke("hunter2"));
        // The security code starts like the card number, and is kept beside it.
        type_into(&values, &page, TabId(1), "4242424242424242");
        values.note(page.clone(), Some(TabId(1)), keystroke("4242"));
        // Digits that extend the code, on another page, are another field's.
        type_into(&values, &PageId(2), TabId(2), "42424");
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        for (text, expected) in [
            ("hunter2", "[private]"),
            ("hunter22", "[private]2"),
            ("4242424242424242", "[private]"),
            ("4242", "[private]"),
        ] {
            assert_eq!(masked(&secrets, text), expected, "{text}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_closed_tab_registers_its_values_at_once() {
        let (secrets, values, closes) = typed_values();
        type_into(&values, &PageId(1), TabId(1), "hunter2");
        type_into(&values, &PageId(2), TabId(2), "swordfish");
        closes.send(TabId(1)).ok();
        time::sleep(Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "hunter2 swordfish"),
            "[private] swordfish",
            "only the closed tab's field is done"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn what_is_held_masks_a_reply_whole_and_registers_nothing() {
        let (secrets, values, _closes) = typed_values();
        let run = run(&values);
        // An old password the set knows, and a new one typed as the old one extended.
        let _registration = secrets.register(SecretValue::new("hunter2".to_owned()), None);
        type_into(&values, &PageId(1), TabId(1), "hunter2!!");
        assert_eq!(
            reply_masked(&secrets, &run.held(), "new: hunter2!!, old: hunter2"),
            "new: [private], old: [private]"
        );
        assert_eq!(
            masked(&secrets, "hunter2!! hunter2!"),
            "[private]!! [private]!",
            "the held value is still held, and none of it was registered"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn what_was_superseded_since_a_run_began_masks_its_reply_and_never_what_was_before() {
        let (secrets, values, _closes) = typed_values();
        let page = PageId(1);
        type_into(&values, &page, TabId(1), "hunt");
        let early = run(&values);
        // Typed past, then a key too many taken back: every value in between was held a moment.
        for value in ["hunte", "hunter", "hunter2", "hunter22", "hunter2"] {
            values.note(page.clone(), Some(TabId(1)), keystroke(value));
        }
        assert_eq!(
            reply_masked(
                &secrets,
                &early.held(),
                "hunt, hunte, hunter, hunter2 or hunter22"
            ),
            "[private], [private], [private], [private] or [private]",
            "`hunt` was held as the run began, and each later value since"
        );
        time::sleep(Duration::from_millis(10)).await;
        assert_eq!(
            reply_masked(&secrets, &run(&values).held(), "hunte or hunter2"),
            "hunte or [private]",
            "a run that began after `hunte` was superseded never saw it held"
        );
        time::sleep(Defaults::TYPED_QUIET).await;
        assert_eq!(
            masked(&secrets, "hunte, hunter22 or hunter2"),
            "hunte, [private]2 or [private]",
            "only the field's final value was registered"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_superseded_value_masks_a_run_that_saw_it_for_as_long_as_the_run_goes() {
        let (secrets, values, _closes) = typed_values();
        let long = run(&values);
        type_into(&values, &PageId(1), TabId(1), "hunter2");
        // Longer than any run may last: what keeps the prefixes is the run going, not a clock.
        time::sleep(Defaults::RUN_TIMEOUT_MAX * 2).await;
        assert_eq!(
            reply_masked(&secrets, &long.held(), "hunte or hunter2"),
            "[private] or [private]",
            "`hunte` was held while the run went on, and the run still goes"
        );
        assert_eq!(
            masked(&secrets, "hunte or hunter2"),
            "hunte or [private]",
            "only the field's final value was registered"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_read_that_raced_the_typing_erases_nothing_and_registers_no_prefix() {
        let (secrets, values, _closes) = typed_values();
        let page = PageId(1);
        type_into(&values, &page, TabId(1), "hunter2");
        // A read of the password from before its last key, and a security code that starts like
        // its card number: two fields, read at once.
        read(
            &values,
            &page,
            TabId(1),
            &["hunter", "4242", "4242424242424242"],
        );
        // A read of another password from before a backspace: longer than the field now is.
        type_into(&values, &PageId(2), TabId(2), "swordfish");
        read(&values, &PageId(2), TabId(2), &["swordfish!"]);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        for text in ["hunter2", "4242", "4242424242424242", "swordfish"] {
            assert_eq!(masked(&secrets, text), "[private]", "{text}");
        }
        assert_eq!(
            masked(&secrets, "hunter"),
            "hunter",
            "the field was typed past what the read saw"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn past_the_cap_a_run_that_may_have_read_the_value_forgotten_is_lost_and_none_registered()
    {
        let (secrets, values, _closes) = typed_values();
        let page = PageId(1);
        let early = run(&values);
        supersede_pairs(&values, &page, 0..Defaults::MAX_SUPERSEDED);
        assert!(
            matches!(early.held(), Held::Kept(_)),
            "every value superseded so far is kept"
        );
        time::sleep(Duration::from_millis(1)).await;
        let late = run(&values);
        supersede_pairs(
            &values,
            &page,
            Defaults::MAX_SUPERSEDED..Defaults::MAX_SUPERSEDED + 1,
        );
        assert!(
            early.held().withheld().is_some(),
            "`v0000a` went past the cap, and the early run began before it was superseded"
        );
        let held = late.held();
        assert!(
            held.withheld().is_none(),
            "the late run began after `v0000a` was superseded, so it lost nothing"
        );
        assert_eq!(
            reply_masked(&secrets, &held, "v0256a"),
            "[private]",
            "a run that lost nothing is still masked with what was superseded since it began"
        );
        assert_eq!(
            masked(&secrets, "v0000a"),
            "v0000a",
            "a value forgotten past the cap is never registered: it may be a prefix"
        );
        drop((early, late));
        let next = run(&values);
        supersede_pairs(
            &values,
            &page,
            Defaults::MAX_SUPERSEDED + 1..Defaults::MAX_SUPERSEDED + 2,
        );
        assert!(
            matches!(next.held(), Held::Kept(_)),
            "a run that begins after the ones that lost a value loses nothing"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_page_read_again_and_again_still_goes_quiet() {
        let (secrets, values, _closes) = typed_values();
        let page = PageId(1);
        type_into(&values, &page, TabId(1), "hunter2");
        for _ in 0..3 {
            time::sleep(Defaults::TYPED_QUIET / 2).await;
            read(&values, &page, TabId(1), &["hunter2"]);
        }
        assert_eq!(
            masked(&secrets, "hunter2"),
            "[private]",
            "a read of a value already held is no keystroke, so the field went quiet"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_page_read_more_often_than_the_cap_costs_its_run_nothing() {
        let (secrets, values, _closes) = typed_values();
        let page = PageId(1);
        let polling = run(&values);
        type_into(&values, &page, TabId(1), "separatePWX");
        // A second field whose value the first's extends, read over and over.
        for _ in 0..=Defaults::MAX_SUPERSEDED {
            read(&values, &page, TabId(1), &["separatePWX", "separatePW"]);
        }
        let held = polling.held();
        assert!(
            held.withheld().is_none(),
            "each read keeps the same value once, so the run lost nothing"
        );
        assert_eq!(
            reply_masked(&secrets, &held, "separatePW"),
            "[private]",
            "the second field's value masks the run"
        );
    }

    /// The cost of the set each record of a run builds ([`TypedValues::held_since`]) at the cap,
    /// and of the matcher behind it: `SecretSet::register` rebuilds it per value, so N values
    /// cost O(N²), while one more value past N costs what a single build of all N would. Run it
    /// in debug and release:
    /// `cargo test -p claw-sdk --lib [--release] held_set_build_cost -- --ignored --nocapture`.
    #[tokio::test(start_paused = true)]
    #[ignore = "a measurement, not a check: it prints build times and takes minutes in debug"]
    #[expect(clippy::print_stdout, reason = "the measurement is what it prints")]
    async fn held_set_build_cost() {
        let (_secrets, values, _closes) = typed_values();
        let full = run(&values);
        supersede_pairs(&values, &PageId(1), 0..Defaults::MAX_SUPERSEDED);
        for page in (2_u32..).take(Defaults::MAX_TYPED) {
            values.note(
                PageId(page),
                Some(TabId(1)),
                keystroke(&format!("Pw-{page:05}-x7!Qz")),
            );
        }
        for _ in 0..5 {
            let began = Instant::now();
            let held = full.held();
            assert!(held.withheld().is_none(), "nothing went past the cap");
            println!(
                "held set at the cap ({} superseded + {} held): {:?}",
                Defaults::MAX_SUPERSEDED,
                Defaults::MAX_TYPED,
                began.elapsed()
            );
        }
        for count in [64_usize, 256, 320, 1024, 4096] {
            let set = SecretSet::new(SecretSetConfig::default());
            let began = Instant::now();
            for value in 0..count - 1 {
                let _registration =
                    set.register(SecretValue::new(format!("Pw-{value:05}-x7!Qz")), None);
            }
            let last = Instant::now();
            let _registration = set.register(SecretValue::new("Pw-last-x7!Qz".to_owned()), None);
            assert_eq!(set.count(), count, "every value was kept");
            println!(
                "{count:5} values: {:?} in all, {:?} for the last (one build over all {count})",
                began.elapsed(),
                last.elapsed()
            );
        }
    }
}
