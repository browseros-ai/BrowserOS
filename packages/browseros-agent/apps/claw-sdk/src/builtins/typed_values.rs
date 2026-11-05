//! What was typed into sensitive fields, or read from them, held until each field is done.

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

/// Values typed into sensitive fields, or found there by a read, that the secret set has not
/// learned yet: the latest value of each field. The set learns a field's value once no entry
/// came for [`Defaults::TYPED_QUIET`], or when its tab closes, and never learns its prefixes.
/// This is sdk-traces decision 32, the rule the takeover's control journal follows for the
/// owner's typing, applied to what browseros-playwright's journal and reads report.
///
/// ```text
///  PrivacyHooks::field_entered ─ note: a value that extends a held one replaces it
///  PrivacyHooks::values_swept ── sweep: a value no held one covers is held beside the rest
///    quiet for TYPED_QUIET ─────────────── every held value ─┬─▶ SecretSet::register
///    its tab closes (TabRegistry) ──────── that tab's ───────┘
///  a run ends ─ held(): every held value, as a set of its own ─▶ that run's reply masked
/// ```
///
/// The seal reports every keystroke as the field's whole value. Registering each entry made every
/// prefix of four characters or more a secret for a day, so a password that starts `https` would
/// mask `http` in every later read. A read reports each field's whole value too, and one between
/// two keystrokes finds a prefix, so what it finds waits the same way; the read has masked it out
/// of its own result already. While a value is held, the seal still masks the field itself in
/// every read. A run's reply is masked once, as the run ends, and may show a value still held,
/// so it is masked with them too ([`TypedValues::held`]). Registering them then instead would
/// make a secret of whatever another field held at that moment, a prefix the owner is still
/// typing included.
pub(crate) struct TypedValues {
    secrets: Arc<SecretSet>,
    pending: Mutex<Pending>,
    /// Wakes the timer when an entry moves its deadline, and when this is dropped.
    wake: Arc<Notify>,
}

/// The held values, a read's in front and the keystrokes in the order typed, and when they are
/// registered unless another entry comes first.
#[derive(Default)]
struct Pending {
    values: Vec<Typed>,
    quiet_until: Option<Instant>,
}

/// One field's latest value and where it was typed or read. A plain data bag.
struct Typed {
    page: PageId,
    /// The page's tab when an op had touched it, so that the tab's close registers the value.
    tab: Option<TabId>,
    /// Zeroed when dropped, which is what happens to a prefix a longer value replaces.
    value: Zeroizing<String>,
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

    /// What is held, as a secret set of its own that masks one run's reply and registers nothing
    /// in the process's set. `None` while nothing is held.
    ///
    /// The reply is masked with the process's set first, which leaves a held value that extends a
    /// registered one as `[private]` plus the rest. So each value is here as typed and as that mask
    /// leaves it, and the reply hides it whole either way.
    pub(crate) fn held(&self) -> Option<SecretSet> {
        // Copied out zeroed-on-drop, so the lock is not held while the set's matcher builds.
        let values: Vec<_> = self
            .lock()
            .values
            .iter()
            .map(|typed| typed.value.clone())
            .collect();
        if values.is_empty() {
            return None;
        }
        let held = SecretSet::new(SecretSetConfig::default());
        for mut value in values {
            let mut masked = value.clone();
            if self.secrets.mask_str(&mut masked) {
                let _registration = held.register(SecretValue::new(mem::take(&mut *masked)), None);
            }
            let _registration = held.register(SecretValue::new(mem::take(&mut *value)), None);
        }
        Some(held)
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

    fn quiet_until(&self) -> Option<Instant> {
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

    /// Outside an await, and never held while registering.
    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
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
    /// repeats one held for the same page is that field typed further, and the held one goes:
    /// it is a prefix. A value one character shorter than the value typed last, on the same page,
    /// is a backspace there. Anything else is held beside the rest. A shorter value is not taken
    /// for the same field otherwise: a security code that starts like its card number would
    /// erase a value nothing registered yet.
    fn note(&mut self, typed: Typed, now: Instant) -> Option<Typed> {
        let value = typed.value.as_str();
        let backspace = self.values.last().is_some_and(|last| {
            last.page == typed.page
                && last
                    .value
                    .strip_prefix(value)
                    .is_some_and(|removed| removed.chars().count() == 1)
        });
        if backspace {
            self.values.pop();
        }
        self.values
            .retain(|held| held.page != typed.page || !value.starts_with(held.value.as_str()));
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
    /// left out: it is held already, or the field was typed past it. Anything else is held beside
    /// the rest, in front, so that the value typed last stays last for the next backspace, and a
    /// later entry that extends it replaces it, as it does any held prefix. A read that finds
    /// nothing new does not restart the quiet period, so a page read over and over still goes
    /// quiet.
    fn sweep(&mut self, swept: Vec<Typed>, now: Instant) -> Vec<Typed> {
        let fresh: Vec<Typed> = swept
            .into_iter()
            .filter(|typed| {
                !self.values.iter().any(|held| {
                    held.page == typed.page && held.value.starts_with(typed.value.as_str())
                })
            })
            .collect();
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
        let Some(quiet_until) = values.upgrade().map(|values| values.quiet_until()) else {
            return;
        };
        tokio::select! {
            () = quiet(quiet_until) => {
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

/// Ends when the held values' quiet period does, and never while nothing is held.
async fn quiet(until: Option<Instant>) {
    match until {
        Some(until) => time::sleep_until(until).await,
        None => future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_core::sensitive_fields::{FieldValues, JournalEntry, SensitiveKind};
    use browseros_core::{PageId, TabId};
    use browseros_trace::{SecretSet, SecretSetConfig, SecretValue};
    use tokio::sync::broadcast;
    use tokio::time;

    use super::TypedValues;
    use crate::config::Defaults;

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

    /// Values over a fresh secret set, and the sender a tab registry would close tabs on.
    fn typed_values() -> (Arc<SecretSet>, Arc<TypedValues>, broadcast::Sender<TabId>) {
        let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
        let (closes, closed) = broadcast::channel(16);
        let values = TypedValues::start(Arc::clone(&secrets), closed);
        (secrets, values, closes)
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
    async fn what_is_held_masks_a_reply_whole_and_registers_nothing() -> Result<(), Box<dyn Error>>
    {
        let (secrets, values, _closes) = typed_values();
        // An old password the set knows, and a new one typed as the old one extended.
        let _registration = secrets.register(SecretValue::new("hunter2".to_owned()), None);
        type_into(&values, &PageId(1), TabId(1), "hunter2!!");
        let held = values.held().ok_or("a value is held")?;
        // The run masks its reply with the process's set first, then with what is held.
        let mut reply = "new: hunter2!!, old: hunter2".to_owned();
        secrets.mask_str(&mut reply);
        held.mask_str(&mut reply);
        assert_eq!(reply, "new: [private], old: [private]");
        assert_eq!(
            masked(&secrets, "hunter2!! hunter2!"),
            "[private]!! [private]!",
            "the held value is still held, and none of it was registered"
        );
        Ok(())
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
}
