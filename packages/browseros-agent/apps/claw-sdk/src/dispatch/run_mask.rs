//! What Neo masks a run's texts with: what it hands the script's realm, and the texts the script
//! chose that Neo keeps. One place decides both, and cuts a text only once it is masked.
//!
//! ```text
//!  into the realm ─┬ OperationGuard::settle: a script's op reply, its value, error and pages
//!                  └ RunService::open_pages: the run's init pages
//!  kept by Neo ────┬ OperationGuard::settle: a main-world answer's numbers, in its record too
//!                  ├ Dispatcher::admit: an unknown name in its refusal, cut at 128 characters
//!                  ├ RefusalBrand::refusal: a prelude refusal's reason, cut at TEXT_CHARS
//!                  └ run_script::thrown_error: what the script threw, cut at TEXT_CHARS
//! ```
//!
//! A mask holds the process's secret set and the values its run holds that the set has not
//! learned yet ([`super::HeldValues`]). Once the run lost one of those values, nothing can mask
//! that one any more, so the mask withholds instead: no text it is asked to mask comes through.

use std::mem;
use std::sync::Arc;

use browseros_trace::SecretSet;
use serde_json::Value;

use super::{Held, OperationError, WITHHELD};

/// How many characters of a script-chosen text the door keeps, once masked: a thrown error's
/// name and its message, and a refusal's reason (FR/RC C4). One constant, so they cannot drift.
pub(crate) const TEXT_CHARS: usize = 4096;

/// The mask for one text's way out of the script or into it ([`RunMask::of`]). Built per use
/// from a [`Held`] the caller already has, so it builds no secret set of its own.
pub(crate) struct RunMask<'a>(Masking<'a>);

enum Masking<'a> {
    /// Every text is masked with the process's set and, when the run holds any, its held values.
    Masks {
        secrets: &'a SecretSet,
        held: Option<Arc<SecretSet>>,
    },
    /// The run lost a held value: every text is withheld, and an op's outcome gives way to this
    /// error, which says why.
    Withholds(OperationError),
}

impl<'a> RunMask<'a> {
    /// The process's set and every value `held` keeps, or, for a run that lost one of them
    /// ([`Held::Lost`]), a mask that withholds.
    #[must_use]
    pub(crate) fn of(secrets: &'a SecretSet, held: &Held) -> Self {
        Self(match held.withheld() {
            Some(withheld) => Masking::Withholds(withheld),
            None => Masking::Masks {
                secrets,
                held: held.set(),
            },
        })
    }

    /// Masks `text` with both sets at once, as the trace writer masks a record's texts, and
    /// returns whether it changed. Each set masks its own copy: one after the other would not do,
    /// since a match of the first can cut a value of the second so that it no longer matches, and
    /// keep the rest of it. A set keeps where its matches are to itself, so a text both sets match
    /// in becomes [`WITHHELD`] whole, which never hides less than the union of their matches would.
    /// A mask that withholds makes every text [`WITHHELD`].
    pub(super) fn mask_str(&self, text: &mut String) -> bool {
        match &self.0 {
            Masking::Masks {
                secrets,
                held: None,
            } => secrets.mask_str(text),
            Masking::Masks {
                secrets,
                held: Some(held),
            } => {
                let mut by_held = text.clone();
                let held_matched = held.mask_str(&mut by_held);
                let secrets_matched = secrets.mask_str(text);
                match (secrets_matched, held_matched) {
                    (true, true) => WITHHELD.clone_into(text),
                    (false, true) => *text = by_held,
                    (_, false) => {}
                }
                secrets_matched || held_matched
            }
            Masking::Withholds(_) => {
                WITHHELD.clone_into(text);
                true
            }
        }
    }

    /// Masks every string in `value` and every object key, each as [`RunMask::mask_str`] masks
    /// one text, and returns whether anything changed. browseros-trace's own walker is private to
    /// it. A mask that withholds makes the whole value [`WITHHELD`], since withheld keys would
    /// collide.
    pub(super) fn mask_json(&self, value: &mut Value) -> bool {
        if let Masking::Withholds(_) = self.0 {
            *value = Value::String(WITHHELD.to_owned());
            return true;
        }
        match value {
            Value::String(text) => self.mask_str(text),
            // Every item and key is masked: stopping at the first change would leak the rest.
            Value::Array(items) => {
                let mut changed = false;
                for item in items {
                    changed |= self.mask_json(item);
                }
                changed
            }
            Value::Object(fields) => {
                let mut changed = false;
                *fields = mem::take(fields)
                    .into_iter()
                    .map(|(mut key, mut field)| {
                        changed |= self.mask_str(&mut key);
                        changed |= self.mask_json(&mut field);
                        (key, field)
                    })
                    .collect();
                changed
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => false,
        }
    }

    /// Masks every number in `value` whose text holds a value of either set, as
    /// browseros-playwright masks a number in a main-world answer: the number gives way to its
    /// text, masked as [`RunMask::mask_str`] masks one, as a string, since a number cannot hold
    /// `[private]`. Strings and keys are left to [`RunMask::mask_json`] and the trace writer.
    pub(super) fn mask_numbers(&self, value: &mut Value) {
        match value {
            Value::Number(number) => {
                let mut text = number.to_string();
                if self.mask_str(&mut text) {
                    *value = Value::String(text);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.mask_numbers(item);
                }
            }
            Value::Object(fields) => {
                for field in fields.values_mut() {
                    self.mask_numbers(field);
                }
            }
            Value::Null | Value::Bool(_) | Value::String(_) => {}
        }
    }

    /// `text` as the door keeps or shows it: masked, then cut to `chars` characters with `…`
    /// marking the cut. In that order, since a cut through a known value leaves a prefix that no
    /// mask recognizes.
    #[must_use]
    pub(crate) fn shown(&self, mut text: String, chars: usize) -> String {
        self.mask_str(&mut text);
        if let Some((cut, _)) = text.char_indices().nth(chars) {
            text.truncate(cut);
            text.push('…');
        }
        text
    }

    /// An op's outcome as the script's realm receives it: its value, or its error's message,
    /// masked. From a run that lost a held value, nothing the op produced comes through, only the
    /// error that says why; the op's delivery and id still reach the script with it.
    pub(super) fn outcome(
        &self,
        outcome: Result<Value, OperationError>,
    ) -> Result<Value, OperationError> {
        match (&self.0, outcome) {
            (Masking::Withholds(withheld), _) => Err(withheld.clone()),
            (Masking::Masks { .. }, Ok(mut value)) => {
                self.mask_json(&mut value);
                Ok(value)
            }
            (Masking::Masks { .. }, Err(mut error)) => {
                error.mask(self);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use browseros_trace::{SecretSet, SecretSetConfig, SecretSource, SecretValue};
    use serde_json::json;

    use super::RunMask;
    use crate::dispatch::Held;

    /// Decision 35z's stopgap. A number cannot hold `[private]`, so one whose text holds a value
    /// the run holds, or one of the process's, gives way to the string of its masked text,
    /// wherever it nests. Every other number stays a number, the ones next to a value included.
    #[test]
    fn a_number_that_spells_a_masked_value_gives_way_to_its_masked_text() {
        let secrets = SecretSet::new(SecretSetConfig::default());
        let _host = secrets.register_from(
            SecretValue::new("4242".to_owned()),
            None,
            SecretSource::Host,
        );
        let held = Held::Kept(Some(Arc::new(SecretSet::with_values(
            SecretSetConfig::default(),
            [SecretValue::new("734921".to_owned())],
        ))));
        let mut value = json!({
            "held": 734_921,
            "known": 4242,
            "inside": [{ "longer": 17_349_210 }, -734_921.5],
            "near": [734_922, 73_492, 424],
        });
        RunMask::of(&secrets, &held).mask_numbers(&mut value);
        assert_eq!(
            value,
            json!({
                "held": "[private]",
                "known": "[private]",
                "inside": [{ "longer": "1[private]0" }, "-[private].5"],
                "near": [734_922, 73_492, 424],
            }),
            "a number is masked as its text would be, and no number without a value changes"
        );
    }
}
