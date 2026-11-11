//! What was typed into sensitive fields, or read from them, held until each field is done.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;
use std::{future, mem};

use browseros_core::TabId;
use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
use neo_playwright::TypedBy;
use neo_trace::{Registration, RunId, SecretSet, SecretSetConfig, SecretSource, SecretValue};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Notify, broadcast};
use tokio::time::{self, Instant};
use zeroize::Zeroizing;

use crate::config::Defaults;
use crate::dispatch::{Held, HeldValues, HeldWatch};

/// Values typed into sensitive fields, or found there by a read, that the secret set has not
/// learned yet: the latest value of each field. The set learns a field's value once no entry
/// came for the quiet period (`EnvConfig::typed_quiet`, [`Defaults::TYPED_QUIET`] unless set
/// longer), or when its tab closes, and never learns its prefixes.
/// This is sdk-traces decision 32, the rule the takeover's control journal follows for the
/// owner's typing, applied to what neo-playwright's journal and reads report.
///
/// ```text
///  PrivacyHooks::field_entered ─ note: a value that extends a held one supersedes it
///  PrivacyHooks::values_swept ── sweep: a value no held or learned one covers is held beside
///                                       the rest, and one that such a value extends is kept as
///                                       superseded
///    quiet for the quiet period ────────── every held value ─┬─▶ promote ─▶ register_from,
///    its tab closes (TabRegistry) ──────── that tab's ───────┤   learning until it returns;
///    more than MAX_TYPED held ──────────── the one in front ─┘   refused, every run going lost
///    learned ─────── kept while a run that went on as it left goes, so a sweep that reports it
///                    again holds nothing new
///    refused ─────── kept the same, but a sweep that reports it, or a prefix of it, again keeps
///                    that for masking only; and superseded as the set refuses it
///    superseded ──── kept while a run that began before it goes, lost or not, then forgotten,
///                    never registered
///  RunCtx::start ─ watch (dispatch's HeldValues): the run's watch keeps what is superseded
///    while it goes, until the run's context drops
///  each record a run sends, and its reply ─ its watch: what is held, what the set is still
///    learning and what was superseded since the run began, as a set of its own
///    (dispatch's Held) ─▶ masked with it too
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
///
/// The set learns each value under who put it in its field ([`Origin`]), which decides what a
/// full set drops for it (sdk-traces decision 53). A page decides what its fields hold, so a read
/// can report any number of values, and the set makes room for each by pushing out a page value,
/// never one the agent typed. With no page value left to push out, the set refuses the value, and
/// it masks the value nowhere from then on. Every run going held it, so each is lost, as past the
/// cap on superseded values. Lost withholds a run's output, but not the code and host data its end
/// record stores, so the value is superseded as it is refused, and masks those until the run ends
/// ([`Pending::refused`]).
pub(crate) struct TypedValues {
    secrets: Arc<SecretSet>,
    /// How long what is held waits for another entry before the set learns it.
    quiet: Duration,
    pending: Mutex<Pending>,
    /// Wakes the timer when an entry moves its deadline, and when this is dropped.
    wake: Arc<Notify>,
    /// Holds the next promotion between taking its values from what is held and handing them
    /// to the set, so tests can ask a run's held set in that gap.
    #[cfg(test)]
    promotion_pause: Mutex<Option<PromotionPause>>,
}

/// The pause [`TypedValues::pause_next_promotion`] sets: `reached` fires when a promotion is
/// held, `resume` lets it go on.
#[cfg(test)]
struct PromotionPause {
    reached: mpsc::Sender<()>,
    resume: mpsc::Receiver<()>,
}

/// The test's ends of a [`PromotionPause`]: `reached` hears that the promotion is held, and a
/// send on `resume` lets it go on, as dropping `resume` does.
#[cfg(test)]
struct PausedPromotion {
    reached: mpsc::Receiver<()>,
    resume: mpsc::Sender<()>,
}

/// The held values, a read's in front and the keystrokes in the order typed, and when they are
/// registered unless another entry comes first; what left them for the process's set; the values
/// a keystroke superseded, oldest first; and the runs going, which those are kept for.
#[derive(Default)]
struct Pending {
    values: Vec<Typed>,
    quiet_until: Option<Instant>,
    promotions: Promotions,
    superseded: VecDeque<Superseded>,
    runs: Vec<Arc<Going>>,
}

/// What left the held values for the process's set ([`TypedValues::promote`]). The set learns a
/// value after the lock is released, since `SecretSet::register_from` rebuilds its matcher for
/// each one, and the value stays here, learning, until that returns: every run's held set takes
/// it in meanwhile ([`TypedValues::held_since`]). The door masks a reply with the run's held set
/// and the process's set, so a value already out of what is held and not yet in the set would
/// reach the script raw, and the trace writer would keep it in the record.
///
/// What left while a run was going is kept after, as long as such a run goes ([`Pending::sweep`]).
/// neo-playwright reports a main-world script's sweeps as their union, before each send
/// and after it, so a prefix the field was typed past comes back beside the field's value once
/// the set learned it, and would otherwise be held as new: it would restart the quiet period
/// and become a secret for a day. At most [`Defaults::MAX_LEARNED`] are kept. Past that, the
/// oldest is forgotten, and nothing recognizes its prefix any more, so until every run that went
/// on as it left ends, a read's value that nothing covers is kept for masking only.
///
/// A value the set refused stays kept too, marked, since the union brings it back as well
/// ([`Pending::standing`]).
#[derive(Default)]
struct Promotions {
    /// The values the set is learning now, each under the ticket its promotion takes it back by.
    learning: Vec<Handoff>,
    /// The next value's ticket.
    next: Ticket,
    /// What left while a run was going, oldest first.
    kept: VecDeque<Learned>,
    /// When the newest value forgotten past the cap left. Cleared once no run that was going
    /// then goes.
    forgot: Option<Instant>,
}

/// Names one value on its way to the set, so that its promotion takes that value alone out of
/// what is learning, whatever other promotions go on beside it.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Ticket(u64);

/// One value on its way to the set, under its ticket. A plain data bag.
struct Handoff {
    ticket: Ticket,
    /// Who put it in its field: the set learns it as theirs.
    origin: Origin,
    /// Zeroed when dropped.
    value: Zeroizing<String>,
}

/// A value that left for the set while a run was going, kept for that run's sweeps. A plain data
/// bag.
struct Learned {
    /// The value and the page it was typed or read on.
    typed: Typed,
    /// When it left what is held.
    taken: Instant,
    /// The set refused it, so the process's set masks it nowhere ([`Pending::refused`]). Until
    /// then the set learned it, is learning it, or keeps nothing that short.
    refused: bool,
}

/// How a value a read found on a page stands to the values held for that page and those kept
/// for it ([`Promotions`]).
enum Standing {
    /// One of them: nothing new.
    Same,
    /// A prefix of one: the field was typed past it since, or it is another field's value.
    Under,
    /// Neither.
    New,
}

/// One field's latest value and where it was typed or read. A plain data bag.
struct Typed {
    /// Native identity remains usable even when the close arrived before this journal entry.
    tab: TabId,
    /// Who put the value in its field.
    origin: Origin,
    /// Zeroed when dropped.
    value: Zeroizing<String>,
}

/// Who put a held value in its field, as far as Neo can tell: the source the process's set
/// learns it under (sdk-traces decision 53). Declared in the set's rank order, lowest first, so
/// `max` keeps the one that outranks the other. Only these two reach the held values: the host
/// and the owner register their values themselves, through `POST /v1/secrets` and the
/// takeover's control journal.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    /// A page: a read found the value in a field, or someone other than the agent typed it. The
    /// crate cannot tell the page's own scripts, autofill and the owner in a takeover apart, and a
    /// page can raise input events on its own fields at will, so all of it is the page's, or a
    /// page could fill the set with values no page value pushes out. The owner's typing still
    /// registers as the owner's, through the control journal: the set keeps the higher source of
    /// a value registered twice.
    Page,
    /// The agent: one of its input ops was typing on the page ([`TypedBy::Agent`]).
    Agent,
}

impl Origin {
    /// The source the process's set registers the value under.
    fn source(self) -> SecretSource {
        match self {
            Self::Page => SecretSource::Page,
            Self::Agent => SecretSource::Agent,
        }
    }
}

impl From<TypedBy> for Origin {
    fn from(by: TypedBy) -> Self {
        match by {
            TypedBy::Agent => Self::Agent,
            TypedBy::NotAgent => Self::Page,
        }
    }
}

/// A value that stopped being held when a keystroke superseded it: a prefix the field was typed
/// past, or what a backspace took back. Or one a read found that a held value extends, which may
/// be such a prefix or another field's value ([`Pending::sweep`]). Or one the process's set refused
/// ([`Pending::refused`]). A run that was going while it was held may have read it, so it masks
/// that run's records and reply for as long as such a run goes, lost or not. A plain data bag.
struct Superseded {
    /// Zeroed when dropped.
    value: Zeroizing<String>,
    /// When it stopped being held.
    until: Instant,
}

/// A run going, as what is kept for it sees it. [`Pending`] and the run's [`RunWatch`] share it.
struct Going {
    /// Which run it is, for the log line that says the run was lost.
    id: RunId,
    /// When the run began. Every value superseded since masks the run's records and reply.
    start: Instant,
    /// Set for good once the run may have read a value out of the process's set's reach: one
    /// superseded since `start` was forgotten past [`Defaults::MAX_SUPERSEDED`], or the process's
    /// set refused one the run held ([`Pending::refused`]). A lost run still needs what is
    /// superseded since `start` ([`Pending::forget_unneeded`]). Written and read under
    /// [`Pending`]'s lock only; atomic so that the watch can share it.
    lost: AtomicBool,
}

/// One run's watch ([`HeldValues::watch`]). The run's context owns it; dropping it ends what is
/// kept for the run.
struct RunWatch {
    values: Arc<TypedValues>,
    going: Arc<Going>,
}

impl TypedValues {
    /// Starts the timer, which registers what is held once no entry came for `quiet`, and a
    /// tab's values as soon as `closed` reports the tab gone.
    pub(crate) fn start(
        secrets: Arc<SecretSet>,
        quiet: Duration,
        closed: broadcast::Receiver<TabId>,
    ) -> Arc<Self> {
        let values = Arc::new(Self {
            secrets,
            quiet,
            pending: Mutex::default(),
            wake: Arc::new(Notify::new()),
            #[cfg(test)]
            promotion_pause: Mutex::default(),
        });
        tokio::spawn(register_when_due(
            Arc::downgrade(&values),
            Arc::clone(&values.wake),
            closed,
        ));
        values
    }

    /// Holds `entry`, the whole value of a field on `page` that just got input, which `by` typed.
    pub(crate) fn note(&self, tab: TabId, entry: JournalEntry, by: TypedBy) {
        let typed = Typed {
            tab,
            origin: Origin::from(by),
            value: entry.value,
        };
        self.promote(|pending, now| pending.note(typed, now, self.quiet).into_iter().collect());
        self.wake.notify_one();
    }

    /// Holds what a read found in the sensitive fields of `page`: each field's whole value.
    pub(crate) fn swept(&self, tab: TabId, values: &FieldValues) {
        let swept = values
            .registrable()
            .map(|value| Typed {
                tab,
                origin: Origin::Page,
                value: Zeroizing::new(value.to_owned()),
            })
            .collect();
        self.promote(|pending, now| pending.sweep(swept, now, self.quiet));
        self.wake.notify_one();
    }

    /// Registers every held value now: when the values are dropped, and when a missed close could
    /// have been any held value's tab.
    fn register_all(&self) {
        self.promote(|pending, _| {
            pending.quiet_until = None;
            mem::take(&mut pending.values)
        });
    }

    /// Registers the values typed on `tab`, which closed: its fields can take no more input.
    fn register_tab(&self, tab: TabId) {
        self.promote(|pending, _| {
            pending
                .values
                .extract_if(.., |typed| typed.tab == tab)
                .collect()
        });
    }

    /// Registers what is held once no entry came for the quiet period.
    fn register_if_quiet(&self, now: Instant) {
        self.promote(|pending, _| match pending.quiet_until {
            Some(until) if until <= now => {
                pending.quiet_until = None;
                mem::take(&mut pending.values)
            }
            Some(_) | None => Vec::new(),
        });
    }

    /// When what is held goes quiet.
    fn due(&self) -> Option<Instant> {
        self.lock().quiet_until
    }

    /// Hands the process's set what `take` takes out of the held values, and returns once the set
    /// learned all of it. `take` runs under the lock, with the instant it runs at: what it
    /// supersedes is timed against the runs' starts, which are taken under the lock too. Every
    /// way a value leaves what is held goes through here: its field quiet, its tab closed, past
    /// the cap, or everything at once.
    ///
    /// Each value stays learning until the set answers ([`Promotions`]). It is registered with
    /// the set's full TTL, its buffer moved into the secret, which zeroes it when dropped; a value
    /// the set already holds keeps the later expiry. A value the set refuses loses every run
    /// going ([`Pending::refused`]), and a promotion that had any refused logs one warning.
    fn promote(&self, take: impl FnOnce(&mut Pending, Instant) -> Vec<Typed>) {
        let handed = {
            let mut pending = self.lock();
            let now = Instant::now();
            let taken = take(&mut pending, now);
            pending.hand(taken, now)
        };
        if handed.is_empty() {
            return;
        }
        #[cfg(test)]
        self.pause_if_asked();
        let mut refused = 0_usize;
        let mut lost = Vec::new();
        for Handoff {
            ticket,
            origin,
            mut value,
        } in handed
        {
            let registration = self.secrets.register_from(
                SecretValue::new(mem::take(&mut *value)),
                None,
                origin.source(),
            );
            // One line per value handed to the set, never the value: how often a page's fields
            // reach the set, which a main-world wait sweeping every 100 ms would show, and what
            // over-masking or a full set traces back to.
            tracing::debug!(
                source = ?origin.source(),
                ?registration,
                "a field value went to the secret set"
            );
            // The value leaves learning only once the set answered, under the lock held sets are
            // built under. Each one built until then masks it, and each one built after finds
            // the process's set masking it or, refused, its run lost.
            let mut pending = self.lock();
            match registration {
                // Too short to mask without masking ordinary text: the set keeps nothing, which
                // is its rule, not a failure here.
                Registration::Registered | Registration::Refreshed | Registration::TooShort => {
                    pending.promotions.answered(ticket);
                }
                Registration::Full => {
                    refused += 1;
                    lost.extend(pending.refused(ticket, Instant::now()));
                }
            }
        }
        if refused > 0 {
            lost.sort_unstable();
            lost.dedup();
            let runs: Vec<String> = lost.iter().map(RunId::to_string).collect();
            tracing::warn!(
                refused,
                runs = %runs.join(","),
                "the secret set was full of host, owner and agent values and refused values from \
                 password, card or code fields; nothing masks them, so every run going lost its \
                 output, which is withheld"
            );
        }
    }

    /// Every value held at any moment since `going` began, those a keystroke superseded since and
    /// those the process's set is still learning included, as a secret set of its own that keeps
    /// every one of them and registers nothing in the process's set.
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
                .chain(pending.promotions.learning())
                .chain(superseded)
                .cloned()
                .collect();
            (values, going.lost.load(Ordering::Relaxed))
        };
        let set = (!values.is_empty()).then(|| {
            let mut spellings = Vec::new();
            for mut value in values {
                let mut masked = value.clone();
                if self.secrets.mask_str(&mut masked) {
                    spellings.push(SecretValue::new(mem::take(&mut *masked)));
                }
                spellings.push(SecretValue::new(mem::take(&mut *value)));
            }
            // Room for every spelling: past `max_entries` a set drops its oldest values and masks
            // with the rest, and a value dropped would reach the reply raw before the process's
            // set learns it. A read reports every sensitive field on its page, so one promotion
            // can hand the process's set more values than a set keeps by default. This set needs
            // no cap of its own: past what is held and superseded, which their own caps bound, it
            // holds what promotions are handing the process's set, and each of those values costs
            // a rebuild of that set's matcher, where this costs one build over them all.
            let keep_all = SecretSetConfig {
                max_entries: spellings.len(),
                ..SecretSetConfig::default()
            };
            // One matcher build over all of them: each record and reply of the run builds this
            // set, and `SecretSet::register_from` would rebuild the matcher for every value.
            Arc::new(SecretSet::with_values(keep_all, spellings))
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

    /// Holds the next promotion that has values to hand the set, once it took them from what is
    /// held and before the set learns them.
    #[cfg(test)]
    fn pause_next_promotion(&self) -> PausedPromotion {
        let (reached, heard) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        *self
            .promotion_pause
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(PromotionPause {
            reached,
            resume: resumed,
        });
        PausedPromotion {
            reached: heard,
            resume,
        }
    }

    #[cfg(test)]
    fn pause_if_asked(&self) {
        let pause = self
            .promotion_pause
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(pause) = pause {
            let _held = pause.reached.send(());
            let _resumed = pause.resume.recv();
        }
    }
}

impl HeldValues for TypedValues {
    fn watch(self: Arc<Self>, run: RunId) -> Box<dyn HeldWatch> {
        let going = {
            let mut pending = self.lock();
            // Taken under the lock, so no value superseded after the run began can miss it.
            pending.run_began(run, Instant::now())
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
    ///
    /// The value keeps the highest origin of the held values it replaces in its field, as the set
    /// keeps the higher source of a value registered twice. So what the agent typed stays the
    /// agent's when the page then edits the field, echoes the value back, or the journal reports
    /// a key after the agent's typing window closed: none of that leaves it for a page flood.
    fn note(&mut self, mut typed: Typed, now: Instant, quiet: Duration) -> Option<Typed> {
        let value = typed.value.as_str();
        let backspace = self.values.last().is_some_and(|last| {
            last.tab == typed.tab
                && last
                    .value
                    .strip_prefix(value)
                    .is_some_and(|removed| removed.chars().count() == 1)
        });
        if backspace && let Some(taken_back) = self.values.pop() {
            typed.origin = typed.origin.max(taken_back.origin);
            self.supersede(taken_back.value, now);
        }
        let typed_past: Vec<Typed> = self
            .values
            .extract_if(.., |held| {
                held.tab == typed.tab && value.starts_with(held.value.as_str())
            })
            .collect();
        for prefix in typed_past {
            typed.origin = typed.origin.max(prefix.origin);
            if prefix.value.as_str() != value {
                self.supersede(prefix.value, now);
            }
        }
        self.values.push(typed);
        self.quiet_until = Some(now + quiet);
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
    ///
    /// What the set learned while a run was going counts as held here ([`Promotions`]): that
    /// run's sweeps come back as their union, the field's learned value and the prefixes it was
    /// typed past included. A value that extends a learned one is new: the field was typed
    /// further. Once a learned value was forgotten past the cap, a value nothing covers may be
    /// its prefix, so while a run that may report one goes, it is kept as superseded too. So is
    /// a value the set refused, and any prefix of one ([`Pending::standing`]).
    fn sweep(&mut self, swept: Vec<Typed>, now: Instant, quiet: Duration) -> Vec<Typed> {
        let mut fresh = Vec::new();
        for typed in swept {
            match self.standing(&typed) {
                Standing::Same => {}
                Standing::Under => self.supersede(typed.value, now),
                Standing::New if self.promotions.forgot_one() => self.supersede(typed.value, now),
                Standing::New => fresh.push(typed),
            }
        }
        if fresh.is_empty() {
            return Vec::new();
        }
        self.values = fresh
            .into_iter()
            .chain(mem::take(&mut self.values))
            .collect();
        self.quiet_until = Some(now + quiet);
        let excess = self.values.len().saturating_sub(Defaults::MAX_TYPED);
        self.values.drain(..excess).collect()
    }

    /// How `typed`, a value a read found, stands to the values held for its page and to those
    /// kept for it.
    ///
    /// A value the set refused is masked nowhere, so a read that finds it again must not take it
    /// for known, as it does a learned one. Held again, though, it would go to the set at the
    /// next quiet period, be refused again, and lose every run going, each time a union brings it
    /// back. So it stands under the refused value, as a prefix does, and so does any prefix of
    /// it: kept for masking only, it masks every run going, one that began after the refusal
    /// included, and is never registered.
    fn standing(&self, typed: &Typed) -> Standing {
        let mut standing = Standing::New;
        for known in self.values.iter().chain(self.promotions.learned()) {
            if known.tab != typed.tab {
                continue;
            }
            if known.value.as_str() == typed.value.as_str() {
                return Standing::Same;
            }
            if known.value.starts_with(typed.value.as_str()) {
                standing = Standing::Under;
            }
        }
        let refused = self
            .promotions
            .refused()
            .any(|known| known.tab == typed.tab && known.value.starts_with(typed.value.as_str()));
        if refused { Standing::Under } else { standing }
    }

    /// Hands `taken`, which left what is held at `now`, to the set ([`Promotions::hand`]). While
    /// a run is going it is kept too: the run's sweeps may report it again.
    fn hand(&mut self, taken: Vec<Typed>, now: Instant) -> Vec<Handoff> {
        let handed = self.promotions.hand(&taken);
        if !self.runs.is_empty() {
            for typed in taken {
                self.promotions.keep(typed, now);
            }
        }
        handed
    }

    /// Keeps `value`, which stopped being held at `now`, which a read found under a held one, or
    /// which the set refused, for the runs going. One too short to mask anything is dropped, and
    /// an older copy of it gives way to this one, which masks every run the older one did.
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
            // cap instead only moves the question to another bound, and each record builds a
            // matcher over these values, whose cost the cap bounds.
            for run in &self.runs {
                if run.start <= gone.until {
                    run.lost.store(true, Ordering::Relaxed);
                }
            }
        }
        self.forget_unneeded();
    }

    /// The process's set refused the value under `ticket`, at `now`, and masks it nowhere from
    /// now on. Every run going held it, as each held set took it in while it was learning, and
    /// may have read it. So each is lost, and what it sends from now on is withheld (decision
    /// 34a). Returns the runs' ids.
    ///
    /// Lost withholds a run's reply and its ops' outputs, but its end record still stores its code
    /// and host data, masked with the run's held set, and either may hold the value. So the value
    /// is superseded at `now` too. Every run going began by then, since runs begin under this
    /// lock, so it masks each of them until it ends, and it is never registered.
    fn refused(&mut self, ticket: Ticket, now: Instant) -> Vec<RunId> {
        let mut lost = Vec::with_capacity(self.runs.len());
        for run in &self.runs {
            run.lost.store(true, Ordering::Relaxed);
            lost.push(run.id);
        }
        if let Some(value) = self.promotions.refuse(ticket) {
            self.supersede(value, now);
        }
        lost
    }

    /// Run `id` began at `now`: from now on, what is superseded is kept for it.
    fn run_began(&mut self, id: RunId, now: Instant) -> Arc<Going> {
        let going = Arc::new(Going {
            id,
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

    /// Forgets the superseded values no run going can need: those superseded before the oldest
    /// run going began, or every one when none goes. And forgets what was kept of what the set
    /// learned once no run that was going as it left goes. A lost run counts like any other: its
    /// ops still report what they sweep, and its end record stores its code and host data masked
    /// with its held set, which must keep every value superseded since the run began.
    fn forget_unneeded(&mut self) {
        let oldest_run = self.runs.iter().map(|run| run.start).min();
        self.promotions.forget(oldest_run);
        match oldest_run {
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

impl Promotions {
    /// Takes `taken`, which just left what is held, on its way to the set, and returns each value
    /// under its ticket for the promotion to register.
    fn hand(&mut self, taken: &[Typed]) -> Vec<Handoff> {
        let mut handed = Vec::with_capacity(taken.len());
        for typed in taken {
            let ticket = self.next;
            self.next = Ticket(ticket.0.wrapping_add(1));
            // Copies, each zeroed when dropped: this one masks the runs until the set has the
            // value, and the promotion moves the other into the set.
            self.learning.push(Handoff {
                ticket,
                origin: typed.origin,
                value: typed.value.clone(),
            });
            handed.push(Handoff {
                ticket,
                origin: typed.origin,
                value: typed.value.clone(),
            });
        }
        handed
    }

    /// The set answered for the value under `ticket`, which is on its way no more: the process's
    /// set masks it from now on, or keeps nothing that short.
    fn answered(&mut self, ticket: Ticket) {
        self.learning.retain(|handoff| handoff.ticket != ticket);
    }

    /// The set refused the value under `ticket`, which is on its way no more, and hands it back.
    /// Every copy kept of it is masked nowhere, whatever page it was typed or read on, since the
    /// set holds none.
    fn refuse(&mut self, ticket: Ticket) -> Option<Zeroizing<String>> {
        let at = self
            .learning
            .iter()
            .position(|handoff| handoff.ticket == ticket)?;
        let refused = self.learning.remove(at);
        for learned in &mut self.kept {
            if learned.typed.value == refused.value {
                learned.refused = true;
            }
        }
        Some(refused.value)
    }

    /// Every value the set is learning now.
    fn learning(&self) -> impl Iterator<Item = &Zeroizing<String>> {
        self.learning.iter().map(|handoff| &handoff.value)
    }

    /// Keeps `typed`, which left what is held at `taken` while a run was going, for that run's
    /// sweeps, and forgets the oldest kept value past [`Defaults::MAX_LEARNED`].
    fn keep(&mut self, typed: Typed, taken: Instant) {
        self.kept.push_back(Learned {
            typed,
            taken,
            refused: false,
        });
        if self.kept.len() > Defaults::MAX_LEARNED
            && let Some(gone) = self.kept.pop_front()
        {
            // A run that was going as `gone` left may still report its prefix, which nothing
            // recognizes from now on. Until those runs end, a value no held or kept one covers
            // is kept for masking only (`Pending::sweep`): registering it could make that prefix
            // a secret for a day, and every later read of ordinary text would show it masked.
            self.forgot = self.forgot.max(Some(gone.taken));
        }
    }

    /// The values kept for the runs' sweeps that the set did not refuse.
    fn learned(&self) -> impl Iterator<Item = &Typed> {
        self.kept
            .iter()
            .filter(|learned| !learned.refused)
            .map(|learned| &learned.typed)
    }

    /// The values kept for the runs' sweeps that the set refused.
    fn refused(&self) -> impl Iterator<Item = &Typed> {
        self.kept
            .iter()
            .filter(|learned| learned.refused)
            .map(|learned| &learned.typed)
    }

    /// Whether a run going may report the prefix of a value forgotten past the cap.
    fn forgot_one(&self) -> bool {
        self.forgot.is_some()
    }

    /// Forgets what no run going can report again: what left before `oldest_run` began, the
    /// oldest run going, or everything when no run goes.
    fn forget(&mut self, oldest_run: Option<Instant>) {
        let Some(oldest) = oldest_run else {
            self.kept.clear();
            self.forgot = None;
            return;
        };
        while self
            .kept
            .front()
            .is_some_and(|learned| learned.taken < oldest)
        {
            self.kept.pop_front();
        }
        self.forgot = self.forgot.filter(|forgot| *forgot >= oldest);
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
    use std::error::Error;
    use std::ops::Range;
    use std::sync::{Arc, Mutex, Once, PoisonError};
    use std::time::{Duration, Instant};
    use std::{io, thread};

    use browseros_core::TabId;
    use browseros_core::sensitive_fields::{FieldValues, JournalEntry, SensitiveKind};
    use neo_playwright::TypedBy;
    use neo_trace::{
        ErrorOwner, Registration, RunId, SecretSet, SecretSetConfig, SecretSource, SecretValue,
    };
    use tokio::runtime::Handle;
    use tokio::sync::broadcast;
    use tokio::time;
    use tracing::subscriber;

    use super::TypedValues;
    use crate::config::Defaults;
    use crate::dispatch::{ErrorCode, Held, HeldValues, HeldWatch};

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
        typed_values_keeping(SecretSetConfig::default().max_entries)
    }

    /// As [`typed_values`], over a set that keeps `max_entries` values, so a test can fill it.
    fn typed_values_keeping(
        max_entries: usize,
    ) -> (Arc<SecretSet>, Arc<TypedValues>, broadcast::Sender<TabId>) {
        let secrets = Arc::new(SecretSet::new(SecretSetConfig {
            max_entries,
            ..SecretSetConfig::default()
        }));
        let (closes, closed) = broadcast::channel(16);
        let values = TypedValues::start(Arc::clone(&secrets), Defaults::TYPED_QUIET, closed);
        (secrets, values, closes)
    }

    /// What a read of a page with `count` sensitive fields reports: each field's whole value.
    fn read_fields(values: &TypedValues, page: TabId, tab: TabId, count: usize) {
        let fields: Vec<String> = (0..count)
            .map(|field| format!("page-field-{field:02}"))
            .collect();
        let found: Vec<&str> = fields.iter().map(String::as_str).collect();
        read(values, page, tab, &found);
    }

    /// A run beginning now, as the door begins one.
    fn run(values: &Arc<TypedValues>) -> Box<dyn HeldWatch> {
        Arc::clone(values).watch(RunId::new())
    }

    /// The owner and code of what a run's reply, records and realm get in place of its output,
    /// once it lost a value nothing masks; `None` while it lost none.
    fn withheld(run: &dyn HeldWatch) -> Option<(ErrorOwner, ErrorCode)> {
        run.held()
            .withheld()
            .map(|error| (error.owner(), error.code()))
    }

    /// A set over `values` that no page or agent value may push anything out of: it holds one
    /// host value and one the agent typed, and keeps two. Returns once the set learned both.
    async fn full_of_host_and_agent_values() -> (Arc<SecretSet>, Arc<TypedValues>) {
        let (secrets, values, _closes) = typed_values_keeping(2);
        let host = secrets.register_from(
            SecretValue::new("host-secret-7Qx".to_owned()),
            None,
            SecretSource::Host,
        );
        assert_eq!(host, Registration::Registered, "the set had room");
        type_into(
            &values,
            TabId(9),
            TabId(9),
            "agent-earlier-7Qx",
            TypedBy::Agent,
        );
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        (secrets, values)
    }

    /// Every line `tracing` writes in this test binary from the first call on, through one
    /// global subscriber. Not one scoped to a test's thread: while it is the only subscriber
    /// registered, tracing-core asks the thread that first reaches a callsite whether anything
    /// listens there, and caches that thread's answer for every thread. Another test reaching the
    /// same `warn!` first would leave a scoped subscriber deaf to it. A test finds its own lines
    /// by its run ids.
    fn test_log() -> &'static Mutex<Vec<u8>> {
        static LOG: Mutex<Vec<u8>> = Mutex::new(Vec::new());
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let collector = tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(|| Written(&LOG))
                .finish();
            // Fails only if another global subscriber is set, and no lib test sets one: a test
            // that reads this log would then find nothing.
            let _installed = subscriber::set_global_default(collector);
        });
        &LOG
    }

    /// One write of the subscriber [`test_log`] installs.
    struct Written(&'static Mutex<Vec<u8>>);

    impl io::Write for Written {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// What a read of `page` reports: each sensitive field's whole value.
    fn read(values: &TypedValues, _page: TabId, tab: TabId, fields: &[&str]) {
        let found = FieldValues::new(fields.iter().map(|field| (*field).to_owned().into()));
        values.swept(tab, &found);
    }

    /// What the journal reports as `by` enters `value`, a field's whole value, in a field of
    /// `page` in `tab`.
    fn enter(values: &TypedValues, _page: TabId, tab: TabId, value: &str, by: TypedBy) {
        values.note(tab, keystroke(value), by);
    }

    /// What the journal reports as `by` types `typed` into a field of `page`, a key at a time:
    /// the field's whole value after each key.
    fn type_into(values: &TypedValues, page: TabId, tab: TabId, typed: &str, by: TypedBy) {
        for end in 1..=typed.len() {
            let prefix = typed.get(..end).unwrap_or(typed);
            enter(values, page, tab, prefix, by);
        }
    }

    /// A page that keeps raising input events on its own fields: each `…ab` supersedes its `…a`,
    /// so each pair supersedes one value.
    fn supersede_pairs(values: &TypedValues, page: TabId, pairs: Range<usize>) {
        for pair in pairs {
            for value in [format!("v{pair:04}a"), format!("v{pair:04}ab")] {
                enter(values, page, TabId(1), &value, TypedBy::NotAgent);
            }
        }
    }

    /// The ways a held value leaves for the process's set.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Promotion {
        /// Its field went quiet.
        Quiet,
        /// Its tab closed.
        TabClosed,
        /// Every held value at once: the values are dropped, or a tab close was missed.
        Everything,
        /// A keystroke held one value past the cap, so the one typed longest ago went at once.
        KeystrokePastTheCap,
        /// A read found a value with the cap full, so the read's own went at once.
        ReadPastTheCap,
    }

    /// Holds `hunter2` on page 1, in tab 1, so that `promotion` is what sends it to the set
    /// next. The values that fill the cap are other pages', so none supersedes another.
    fn hold_for(values: &TypedValues, promotion: Promotion) {
        let filler = match promotion {
            Promotion::Quiet | Promotion::TabClosed | Promotion::Everything => 0,
            Promotion::KeystrokePastTheCap => Defaults::MAX_TYPED - 1,
            Promotion::ReadPastTheCap => Defaults::MAX_TYPED,
        };
        if promotion != Promotion::ReadPastTheCap {
            enter(values, TabId(1), TabId(1), "hunter2", TypedBy::NotAgent);
        }
        for page in (2_u32..).take(filler) {
            enter(
                values,
                TabId(i64::from(page)),
                TabId(2),
                &format!("filler-{page:03}"),
                TypedBy::NotAgent,
            );
        }
    }

    /// Sends what [`hold_for`] held for `promotion` to the set, the way it names.
    fn promote_by(values: &TypedValues, promotion: Promotion) {
        match promotion {
            Promotion::Quiet => {
                values.register_if_quiet(time::Instant::now() + Defaults::TYPED_QUIET);
            }
            Promotion::TabClosed => values.register_tab(TabId(1)),
            Promotion::Everything => values.register_all(),
            Promotion::KeystrokePastTheCap => {
                enter(values, TabId(999), TabId(2), "one-more", TypedBy::NotAgent);
            }
            Promotion::ReadPastTheCap => read(values, TabId(1), TabId(1), &["hunter2"]),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_field_typed_a_key_at_a_time_is_learned_once_quiet_and_its_prefixes_never() {
        let (secrets, values, _closes) = typed_values();
        let page = TabId(1);
        type_into(&values, page, TabId(1), "hunter2", TypedBy::NotAgent);
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
        let page = TabId(1);
        type_into(&values, page, TabId(1), "hunter", TypedBy::NotAgent);
        time::sleep(Defaults::TYPED_QUIET.saturating_sub(Duration::from_millis(100))).await;
        enter(&values, page, TabId(1), "hunter2", TypedBy::NotAgent);
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
        let page = TabId(1);
        type_into(&values, page, TabId(1), "hunter22", TypedBy::NotAgent);
        enter(&values, page, TabId(1), "hunter2", TypedBy::NotAgent);
        // The security code starts like the card number, and is kept beside it.
        type_into(
            &values,
            page,
            TabId(1),
            "4242424242424242",
            TypedBy::NotAgent,
        );
        enter(&values, page, TabId(1), "4242", TypedBy::NotAgent);
        // Digits that extend the code, on another page, are another field's.
        type_into(&values, TabId(2), TabId(2), "42424", TypedBy::NotAgent);
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
        type_into(&values, TabId(1), TabId(1), "hunter2", TypedBy::NotAgent);
        type_into(&values, TabId(2), TabId(2), "swordfish", TypedBy::NotAgent);
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
        let _registration = secrets.register_from(
            SecretValue::new("hunter2".to_owned()),
            None,
            SecretSource::Host,
        );
        type_into(&values, TabId(1), TabId(1), "hunter2!!", TypedBy::NotAgent);
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
        let page = TabId(1);
        type_into(&values, page, TabId(1), "hunt", TypedBy::NotAgent);
        let early = run(&values);
        // Typed past, then a key too many taken back: every value in between was held a moment.
        for value in ["hunte", "hunter", "hunter2", "hunter22", "hunter2"] {
            enter(&values, page, TabId(1), value, TypedBy::NotAgent);
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
        type_into(&values, TabId(1), TabId(1), "hunter2", TypedBy::NotAgent);
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
        let page = TabId(1);
        type_into(&values, page, TabId(1), "hunter2", TypedBy::NotAgent);
        // A read of the password from before its last key, and a security code that starts like
        // its card number: two fields, read at once.
        read(
            &values,
            page,
            TabId(1),
            &["hunter", "4242", "4242424242424242"],
        );
        // A read of another password from before a backspace: longer than the field now is.
        type_into(&values, TabId(2), TabId(2), "swordfish", TypedBy::NotAgent);
        read(&values, TabId(2), TabId(2), &["swordfish!"]);
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
        let page = TabId(1);
        let early = run(&values);
        supersede_pairs(&values, page, 0..Defaults::MAX_SUPERSEDED);
        assert!(
            matches!(early.held(), Held::Kept(_)),
            "every value superseded so far is kept"
        );
        time::sleep(Duration::from_millis(1)).await;
        let late = run(&values);
        supersede_pairs(
            &values,
            page,
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
            page,
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
        let page = TabId(1);
        type_into(&values, page, TabId(1), "hunter2", TypedBy::NotAgent);
        for _ in 0..3 {
            time::sleep(Defaults::TYPED_QUIET / 2).await;
            read(&values, page, TabId(1), &["hunter2"]);
        }
        assert_eq!(
            masked(&secrets, "hunter2"),
            "[private]",
            "a read of a value already held is no keystroke, so the field went quiet"
        );
    }

    /// M1 handoff line 2: neo-playwright reports what an op's sweeps found after every
    /// sweep, before each send and after it, as the union of what the op swept so far. So a value
    /// the field was typed past since an earlier sweep comes back in each later report, beside the
    /// field's latest value, and both keep coming back once the set learned that value. None of
    /// it is a keystroke: the field goes quiet a second after its last key, another field a
    /// second after its own, and the prefix the union repeats is never registered.
    #[tokio::test(start_paused = true)]
    async fn a_union_reported_before_each_send_restarts_no_quiet_period() {
        let (secrets, values, _closes) = typed_values();
        let page = TabId(1);
        // The run of the op that reports: its sweeps keep coming for as long as it goes.
        let _reporting = run(&values);
        // The op's first sweep finds the field mid-typing; then the page types its last key.
        read(&values, page, TabId(1), &["hunte"]);
        time::sleep(Duration::from_millis(100)).await;
        enter(&values, page, TabId(1), "hunter2", TypedBy::NotAgent);
        let last_key = time::Instant::now();
        for _ in 0..3 {
            time::sleep(Duration::from_millis(200)).await;
            read(&values, page, TabId(1), &["hunte", "hunter2"]);
        }
        assert_eq!(
            masked(&secrets, "hunter2"),
            "hunter2",
            "the field's last key was 600 ms ago, so it may still be typed"
        );
        time::sleep_until(last_key + Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "hunte or hunter2"),
            "hunte or [private]",
            "the reports restarted nothing, and the prefix they repeated never became a secret"
        );
        // The op goes on reporting the same union while another field is typed.
        enter(&values, TabId(2), TabId(2), "swordfish", TypedBy::NotAgent);
        let other_key = time::Instant::now();
        for _ in 0..3 {
            time::sleep(Duration::from_millis(200)).await;
            read(&values, page, TabId(1), &["hunte", "hunter2"]);
        }
        time::sleep_until(other_key + Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "swordfish"),
            "[private]",
            "a report of what the set learned is no keystroke either, so the other field went quiet"
        );
        time::sleep(Defaults::TYPED_QUIET).await;
        assert_eq!(
            masked(&secrets, "hunte or hunter2"),
            "hunte or [private]",
            "the prefix the union repeats after its field's value registered never became a secret"
        );
        assert_eq!(
            secrets.count(),
            2,
            "the set learned the two fields' values and nothing else"
        );
    }

    /// A read of the field after its value registered that finds the value typed further finds
    /// a new value, which is held and learned as any other.
    #[tokio::test(start_paused = true)]
    async fn a_read_of_a_learned_value_typed_further_registers_the_new_value() {
        let (secrets, values, _closes) = typed_values();
        let page = TabId(1);
        let _reporting = run(&values);
        enter(&values, page, TabId(1), "hunter2", TypedBy::NotAgent);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        read(&values, page, TabId(1), &["hunter2", "hunter2!x"]);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "hunter2 or hunter2!x"),
            "[private] or [private]",
            "the read found the field typed past its learned value, and that is its final value"
        );
    }

    /// Past the cap on what is kept of what the set learned, nothing recognizes a forgotten
    /// value's prefix in a later report. So while a run that was going as it left goes, a read's
    /// value that nothing covers masks that run and is never registered, and once those runs
    /// end, a read's new value registers again.
    #[tokio::test(start_paused = true)]
    async fn past_the_cap_on_what_was_learned_a_report_registers_no_prefix() {
        let (secrets, values, _closes) = typed_values();
        let page = TabId(1);
        let early = run(&values);
        read(&values, page, TabId(1), &["hunte"]);
        enter(&values, page, TabId(1), "hunter2", TypedBy::NotAgent);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        // As many other fields as the cap keeps, each learned once quiet: `hunter2` goes first.
        for other in (2_u32..).take(Defaults::MAX_LEARNED) {
            enter(
                &values,
                TabId(i64::from(other)),
                TabId(2),
                &format!("learned-{other:04}"),
                TypedBy::NotAgent,
            );
            time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        }
        read(&values, page, TabId(1), &["hunte", "hunter2"]);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "hunte"),
            "hunte",
            "nothing kept knows `hunte` for a prefix any more, so it was never registered"
        );
        assert_eq!(
            reply_masked(&secrets, &early.held(), "hunte"),
            "[private]",
            "it masks the run that may have read it"
        );
        drop(early);
        let _late = run(&values);
        read(&values, TabId(2000), TabId(2), &["swordfish"]);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "swordfish"),
            "[private]",
            "no run that was going as `hunter2` left goes, so a read's new value registers again"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_page_read_more_often_than_the_cap_costs_its_run_nothing() {
        let (secrets, values, _closes) = typed_values();
        let page = TabId(1);
        let polling = run(&values);
        type_into(&values, page, TabId(1), "separatePWX", TypedBy::NotAgent);
        // A second field whose value the first's extends, read over and over.
        for _ in 0..=Defaults::MAX_SUPERSEDED {
            read(&values, page, TabId(1), &["separatePWX", "separatePW"]);
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

    /// A page decides what its fields hold, so a read can report any number of values, and past
    /// its cap the set pushes out its oldest page value for each new one (sdk-traces decision
    /// 53). What the agent typed is the agent's, which no page value pushes out.
    #[tokio::test(start_paused = true)]
    async fn a_value_the_agent_typed_registers_as_the_agents() {
        let (secrets, values, _closes) = typed_values_keeping(4);
        type_into(&values, TabId(1), TabId(1), "agent-pw-7Qx", TypedBy::Agent);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        read_fields(&values, TabId(2), TabId(2), 8);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "page-field-00 page-field-07"),
            "page-field-00 [private]",
            "the page's eight values went through a set that keeps four, the oldest pushed out"
        );
        assert_eq!(
            masked(&secrets, "agent-pw-7Qx"),
            "[private]",
            "no page value pushes out a value the agent typed"
        );
    }

    /// A page may enter its own value in a field the agent typed in, and the journal reports a
    /// key that comes after the agent's typing window closed as `NotAgent`. Either way the field's
    /// value stays the agent's, since it keeps the highest origin of the values it replaced.
    #[tokio::test(start_paused = true)]
    async fn a_field_the_agent_typed_in_stays_the_agents_whoever_enters_it_after() {
        let (secrets, values, _closes) = typed_values_keeping(4);
        // The page echoes the agent's value back.
        type_into(&values, TabId(1), TabId(1), "agent-pw-7Qx", TypedBy::Agent);
        enter(
            &values,
            TabId(1),
            TabId(1),
            "agent-pw-7Qx",
            TypedBy::NotAgent,
        );
        // The page appends to it.
        type_into(&values, TabId(2), TabId(2), "agent-pw-8Rz", TypedBy::Agent);
        enter(
            &values,
            TabId(2),
            TabId(2),
            "agent-pw-8Rz!",
            TypedBy::NotAgent,
        );
        // The page takes its last character back.
        type_into(&values, TabId(3), TabId(3), "agent-pw-9Sy!", TypedBy::Agent);
        enter(
            &values,
            TabId(3),
            TabId(3),
            "agent-pw-9Sy",
            TypedBy::NotAgent,
        );
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        read_fields(&values, TabId(4), TabId(4), 8);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "agent-pw-7Qx agent-pw-8Rz! agent-pw-9Sy"),
            "[private] [private] [private]",
            "each field's value is the agent's, which no page value pushes out"
        );
    }

    /// A read reports what a page put in its fields, and the journal reports as `NotAgent` all
    /// typing but the agent's: the page's own scripts, autofill, the owner in a takeover. Both
    /// are the page's, which later page values push out, oldest first.
    #[tokio::test(start_paused = true)]
    async fn a_swept_value_or_one_the_agent_did_not_type_registers_as_the_pages() {
        let (secrets, values, _closes) = typed_values_keeping(4);
        read(&values, TabId(1), TabId(1), &["page-pw-7Qx"]);
        type_into(
            &values,
            TabId(2),
            TabId(2),
            "other-pw-7Qx",
            TypedBy::NotAgent,
        );
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "page-pw-7Qx other-pw-7Qx"),
            "[private] [private]",
            "both fields went quiet"
        );
        read_fields(&values, TabId(3), TabId(3), 4);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "page-pw-7Qx other-pw-7Qx"),
            "page-pw-7Qx other-pw-7Qx",
            "four later page values pushed out the four the set kept, these two first"
        );
    }

    /// A set full of values that no page or agent value may push out refuses the next one
    /// (`Registration::Full`), and nothing masks that value from then on. Every run going held
    /// it, as every run's held set holds every held value: one going since before it was typed,
    /// and one that began while it was held. Each is lost, so its records and its realm get
    /// `neo / output_withheld` in place of its output. A run that begins after the refusal never
    /// held the value, and loses nothing until a read finds the value again and the set refuses
    /// it again.
    #[tokio::test(start_paused = true)]
    async fn a_value_the_set_refuses_loses_every_run_that_held_it() {
        let (secrets, values) = full_of_host_and_agent_values().await;
        let before = run(&values);
        type_into(&values, TabId(1), TabId(1), "agent-pw-7Qx", TypedBy::Agent);
        let during = run(&values);
        assert_eq!(
            withheld(&*during),
            None,
            "the value is held, so it masks the run"
        );
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            masked(&secrets, "agent-pw-7Qx"),
            "agent-pw-7Qx",
            "the set refused the value, so it masks nothing"
        );
        for (which, going) in [("before", &before), ("during", &during)] {
            assert_eq!(
                withheld(&**going),
                Some((ErrorOwner::Neo, ErrorCode::OUTPUT_WITHHELD)),
                "the run going {which} the typing held the value, which nothing masks now"
            );
        }
        drop((before, during));

        let late = run(&values);
        assert_eq!(
            withheld(&*late),
            None,
            "a run that began after the refusal never held the value"
        );
        read(&values, TabId(1), TabId(1), &["agent-pw-7Qx"]);
        let held = late.held();
        assert_eq!(
            reply_masked(&secrets, &held, "agent-pw-7Qx"),
            "[private]",
            "a read found the value again: it is held, so it masks the run"
        );
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            withheld(&*late),
            Some((ErrorOwner::Neo, ErrorCode::OUTPUT_WITHHELD)),
            "the set refused it again, so the run that held it is lost"
        );
    }

    /// While a run that went on as a refused value was taken goes, what was taken stays kept for
    /// that run's sweeps, and its ops report the value again, as a union, before each send. A
    /// read that finds the value again, or a prefix of it, keeps that for masking only: it masks
    /// the runs going, a run that began after the refusal included, and never goes to the set
    /// again. Held again instead, the value would be refused at every quiet period, and each
    /// refusal would lose every run going.
    #[tokio::test(start_paused = true)]
    async fn a_refused_value_read_again_masks_its_reader_and_is_never_offered_again() {
        let (secrets, values) = full_of_host_and_agent_values().await;
        let early = run(&values);
        type_into(&values, TabId(1), TabId(1), "agent-pw-7Qx", TypedBy::Agent);
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert!(withheld(&*early).is_some(), "the set refused the value");
        let late = run(&values);
        read(&values, TabId(1), TabId(1), &["agent-pw", "agent-pw-7Qx"]);
        assert_eq!(
            reply_masked(&secrets, &late.held(), "agent-pw or agent-pw-7Qx"),
            "[private] or [private]",
            "the read found the value and a prefix of it again, and both mask the run"
        );
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(1)).await;
        assert_eq!(
            withheld(&*late),
            None,
            "kept for masking only, the value never went to the set again, so no refusal lost \
             the run"
        );
        assert_eq!(
            reply_masked(&secrets, &late.held(), "agent-pw-7Qx"),
            "[private]",
            "it still masks the run that read it"
        );
    }

    /// A value on its way to the set masks every run's held set, one that began after the
    /// promotion took the value included. So that run held it too, and once the set refuses the
    /// value, it is lost as well: losing only the runs going as the value was taken would leave
    /// it masked by nothing.
    #[tokio::test(start_paused = true)]
    async fn a_run_that_began_while_a_refused_value_was_on_its_way_is_lost_too()
    -> Result<(), Box<dyn Error>> {
        let runtime = Handle::current();
        let (secrets, values) = full_of_host_and_agent_values().await;
        enter(&values, TabId(1), TabId(1), "agent-pw-7Qx", TypedBy::Agent);
        let late = thread::scope(|scope| -> Result<Box<dyn HeldWatch>, Box<dyn Error>> {
            // Dropped as this returns, so the promotion never waits on a test that failed.
            let paused = values.pause_next_promotion();
            scope.spawn(|| {
                let _runtime = runtime.enter();
                values.register_all();
            });
            paused.reached.recv_timeout(Duration::from_secs(10))?;
            let late = run(&values);
            assert_eq!(
                reply_masked(&secrets, &late.held(), "agent-pw-7Qx"),
                "[private]",
                "on its way to the set, the value masks a run that began after it was taken"
            );
            paused.resume.send(())?;
            Ok(late)
        })?;
        assert_eq!(
            withheld(&*late),
            Some((ErrorOwner::Neo, ErrorCode::OUTPUT_WITHHELD)),
            "the run held the value the set refused"
        );
        Ok(())
    }

    /// A value shorter than the set's minimum is no secret, by the set's rule: a full set keeps
    /// nothing of it, as any set does, and no run loses its output for it.
    #[tokio::test(start_paused = true)]
    async fn a_value_too_short_to_be_a_secret_loses_no_run() {
        let (_secrets, values) = full_of_host_and_agent_values().await;
        let going = run(&values);
        enter(&values, TabId(1), TabId(1), "pin", TypedBy::Agent);
        values.register_all();
        assert_eq!(
            withheld(&*going),
            None,
            "the set kept nothing of `pin` because it is short, not because it is full"
        );
    }

    /// The operator's one sign that runs lost their output to a full set: a warning for each
    /// promotion the set refused values in, with how many and the runs it lost. A value never
    /// reaches the log (STYLE P4).
    #[tokio::test(start_paused = true)]
    async fn a_refusal_is_logged_once_with_the_runs_it_lost_and_never_the_value() {
        let log = test_log();
        let (_secrets, values) = full_of_host_and_agent_values().await;
        let (first, second) = (RunId::new(), RunId::new());
        let _runs = (
            Arc::clone(&values).watch(first),
            Arc::clone(&values).watch(second),
        );
        enter(&values, TabId(1), TabId(1), "agent-pw-7Qx", TypedBy::Agent);
        enter(
            &values,
            TabId(2),
            TabId(2),
            "page-pw-8Rz",
            TypedBy::NotAgent,
        );
        values.register_all();
        let written = String::from_utf8_lossy(&log.lock().unwrap_or_else(PoisonError::into_inner))
            .into_owned();
        let first = first.to_string();
        let lines: Vec<&str> = written
            .lines()
            .filter(|line| line.contains(&first))
            .collect();
        assert_eq!(
            lines.len(),
            1,
            "one promotion refused both values and logged once: {lines:?}"
        );
        let line = lines[0];
        assert!(
            line.contains("WARN")
                && line.contains("refused=2")
                && line.contains(&second.to_string()),
            "a warning says how many values the set refused and names both runs lost: {line}"
        );
        assert!(
            !line.contains("-pw-"),
            "no value, nor any part of one, is logged: {line}"
        );
    }

    /// Each promotion takes its values from what is held under the lock, and the set learns them
    /// after the lock is released, since `SecretSet::register_from` rebuilds its matcher. A run's
    /// held set asked in between must still have them: the door masks a reply with that set first
    /// and with the process's set after, so a value in neither reaches the script raw.
    #[tokio::test(start_paused = true)]
    async fn a_value_on_its_way_to_the_set_masks_the_runs_going() -> Result<(), Box<dyn Error>> {
        let runtime = Handle::current();
        let mut shown = Vec::new();
        for promotion in [
            Promotion::Quiet,
            Promotion::TabClosed,
            Promotion::Everything,
            Promotion::KeystrokePastTheCap,
            Promotion::ReadPastTheCap,
        ] {
            let (secrets, values, _closes) = typed_values();
            let going = run(&values);
            hold_for(&values, promotion);
            thread::scope(|scope| -> Result<(), Box<dyn Error>> {
                // Dropped as this returns, so the promotion never waits on a test that failed.
                let paused = values.pause_next_promotion();
                scope.spawn(|| {
                    // On the test's paused clock, as every other entry is.
                    let _runtime = runtime.enter();
                    promote_by(&values, promotion);
                });
                paused.reached.recv_timeout(Duration::from_secs(10))?;
                let reply = reply_masked(&secrets, &going.held(), "hunter2");
                if reply != "[private]" {
                    shown.push((promotion, reply));
                }
                paused.resume.send(())?;
                Ok(())
            })?;
            assert_eq!(
                masked(&secrets, "hunter2"),
                "[private]",
                "{promotion:?}: the set learned the value once the promotion went on"
            );
        }
        assert!(
            shown.is_empty(),
            "a reply masked after the promotion took the value and before the set learned it \
             showed it: {shown:?}"
        );
        Ok(())
    }

    /// A page can hold more sensitive fields than a secret set keeps by default, and a read
    /// reports every one: what is past [`Defaults::MAX_TYPED`] leaves for the process's set in
    /// one promotion. A run's held set built while those are on their way holds every value the
    /// run holds, the one typed a moment ago included, which the process's set has not learned.
    #[tokio::test(start_paused = true)]
    async fn every_value_a_run_holds_masks_its_reply_however_many_a_read_finds()
    -> Result<(), Box<dyn Error>> {
        let runtime = Handle::current();
        // A process's set that keeps one value: the promotion registers thousands, and each
        // registration rebuilds the set's matcher over every value it keeps. The run's held set
        // is a set of its own.
        let secrets = Arc::new(SecretSet::new(SecretSetConfig {
            max_entries: 1,
            ..SecretSetConfig::default()
        }));
        let (_closes, closed) = broadcast::channel(16);
        let values = TypedValues::start(Arc::clone(&secrets), Defaults::TYPED_QUIET, closed);
        let going = run(&values);
        enter(&values, TabId(1), TabId(1), "hunter2", TypedBy::NotAgent);
        let fields: Vec<String> = (0..SecretSetConfig::default().max_entries + Defaults::MAX_TYPED)
            .map(|field| format!("field-{field:05}"))
            .collect();
        let found: Vec<&str> = fields.iter().map(String::as_str).collect();
        let everything_held = format!("hunter2 {}", found.join(" "));
        let reply = thread::scope(|scope| -> Result<String, Box<dyn Error>> {
            // Dropped as this returns, so the promotion never waits on a test that failed.
            let paused = values.pause_next_promotion();
            scope.spawn(|| {
                let _runtime = runtime.enter();
                read(&values, TabId(2), TabId(2), &found);
            });
            paused.reached.recv_timeout(Duration::from_secs(10))?;
            let reply = reply_masked(&secrets, &going.held(), &everything_held);
            paused.resume.send(())?;
            Ok(reply)
        })?;
        let shown: Vec<&str> = reply
            .split(' ')
            .filter(|word| *word != "[private]")
            .collect();
        assert!(
            shown.is_empty(),
            "the run's held set keeps all {} values the run holds, so none reaches its reply raw: \
             {} did, first {:?}",
            found.len() + 1,
            shown.len(),
            shown.first()
        );
        Ok(())
    }

    /// The cost of the set each record of a run builds ([`TypedValues::held_since`]) at the cap,
    /// of the one matcher build behind it (`SecretSet::with_values`) past the cap and past what a
    /// secret set keeps by default, and of what a promotion pays for each value it hands a full
    /// process's set. Run it in debug and release:
    /// `cargo test -p neo-server --lib [--release] held_set_build_cost -- --ignored --nocapture`.
    #[tokio::test(start_paused = true)]
    #[ignore = "a measurement, not a check: it prints build times"]
    #[expect(clippy::print_stdout, reason = "the measurement is what it prints")]
    async fn held_set_build_cost() {
        let (_secrets, values, _closes) = typed_values();
        let full = run(&values);
        supersede_pairs(&values, TabId(1), 0..Defaults::MAX_SUPERSEDED);
        for page in (2_u32..).take(Defaults::MAX_TYPED) {
            enter(
                &values,
                TabId(i64::from(page)),
                TabId(1),
                &format!("Pw-{page:05}-x7!Qz"),
                TypedBy::NotAgent,
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
        for count in [64_usize, 256, 320, 1024, 4096, 16384] {
            let began = Instant::now();
            let set = SecretSet::with_values(
                SecretSetConfig {
                    max_entries: count,
                    ..SecretSetConfig::default()
                },
                (0..count).map(|value| SecretValue::new(format!("Pw-{value:05}-x7!Qz"))),
            );
            assert_eq!(set.count(), count, "every value was kept");
            println!("{count:5} values: {:?}", began.elapsed());
        }
        let full = SecretSet::with_values(
            SecretSetConfig::default(),
            (0..SecretSetConfig::default().max_entries)
                .map(|value| SecretValue::new(format!("Pw-{value:05}-x7!Qz"))),
        );
        let began = Instant::now();
        let _registration = full.register_from(
            SecretValue::new("Pw-promoted-x7!Qz".to_owned()),
            None,
            SecretSource::Page,
        );
        println!(
            "one value promoted into a full process's set: {:?}",
            began.elapsed()
        );
    }
}
