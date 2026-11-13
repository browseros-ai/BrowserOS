//! The value a host types privately: text that cannot be printed, logged, cloned or serialized.

use std::error::Error;
use std::{fmt, hint, mem};

use serde::de::{self, Deserialize, Deserializer};

/// The longest value one box takes, in characters (design section 4.2).
pub const MAX_PRIVATE_CHARS: usize = 256;

/// A secret the host types into a page: a password, a one-time code, a card number.
///
/// It exists for the length of one entry. The host's route turns the request body's value into
/// one at its edge and hands it to [`crate::Playwright::enter_privately`], which types it
/// character by character and drops it. It has no `Serialize`, `Display` or `Clone`, so it
/// cannot reach an operation's recorded input, an event or a reply, and its `Debug` prints
/// `[private]`. Its bytes are zeroed when it drops, as far as safe Rust allows.
///
/// The value must not reach any JSON that is stored, so this does not compile:
///
/// ```compile_fail
/// use neo_playwright::PrivateText;
/// fn record(value: &PrivateText) -> serde_json::Value {
///     serde_json::to_value(value).unwrap_or_default()
/// }
/// ```
///
/// Nor does a copy:
///
/// ```compile_fail
/// use neo_playwright::PrivateText;
/// fn copy(value: &PrivateText) -> PrivateText {
///     value.clone()
/// }
/// ```
pub struct PrivateText {
    /// Never empty, at most [`MAX_PRIVATE_CHARS`] characters, no control character.
    text: String,
}

/// Why a value cannot be typed privately. Each message is a fixed sentence: none quotes the
/// value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateTextRefused {
    /// The value has no character.
    Empty,
    /// The value has more than [`MAX_PRIVATE_CHARS`] characters.
    TooLong,
    /// The value holds a control character. A newline would press Enter, a tab would move the
    /// focus to another box.
    ControlCharacter,
}

impl PrivateTextRefused {
    /// The fixed sentence for this refusal.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Empty => "a private value has at least one character",
            Self::TooLong => "a private value has at most 256 characters",
            Self::ControlCharacter => "a private value holds no control character",
        }
    }
}

impl fmt::Display for PrivateTextRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl Error for PrivateTextRefused {}

impl PrivateText {
    /// Takes `text` as a private value, or refuses it. On a refusal `text` is zeroed too.
    pub fn new(text: String) -> Result<Self, PrivateTextRefused> {
        // Wrapped first, so a refused value is zeroed by the same drop as an accepted one.
        let value = Self { text };
        let mut count = 0_usize;
        for character in value.text.chars() {
            if character.is_control() {
                return Err(PrivateTextRefused::ControlCharacter);
            }
            count += 1;
        }
        match count {
            0 => Err(PrivateTextRefused::Empty),
            n if n > MAX_PRIVATE_CHARS => Err(PrivateTextRefused::TooLong),
            _ => Ok(value),
        }
    }

    /// How many characters the value has: what the trace may record, and what `typed` counts.
    #[must_use]
    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// The value's characters, in order. Crate-private: only the entry reads the value, and
    /// only character by character (ruling R12: a string slice at a bad byte index panics
    /// with the text in its message, and Neo writes panics to a file).
    pub(crate) fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.text.chars()
    }
}

impl Drop for PrivateText {
    fn drop(&mut self) {
        // The whole allocation, its spare capacity included, which can hold an earlier copy
        // from the string's growth. `black_box` keeps the compiler from dropping the writes as
        // dead stores; without `unsafe` (denied here) this is as far as Rust allows.
        let mut bytes = mem::take(&mut self.text).into_bytes();
        let capacity = bytes.capacity();
        bytes.clear();
        bytes.resize(capacity, 0);
        hint::black_box(&mut bytes);
    }
}

impl fmt::Debug for PrivateText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[private]")
    }
}

/// A host's route reads the value straight from its request body. The refusal is a fixed
/// sentence, never the value.
impl<'de> Deserialize<'de> for PrivateText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::new(text).map_err(|refused| de::Error::custom(refused.message()))
    }
}

#[cfg(test)]
mod tests {
    use super::{PrivateText, PrivateTextRefused};

    #[test]
    fn tests_that_a_private_value_prints_as_private() -> Result<(), PrivateTextRefused> {
        let value = PrivateText::new("hunter2".to_owned())?;
        assert_eq!(format!("{value:?}"), "[private]");
        assert_eq!(format!("{:?}", Some(&value)), "Some([private])");
        assert_eq!(value.char_count(), 7);
        Ok(())
    }

    #[test]
    fn tests_that_a_value_is_refused_by_a_fixed_sentence() {
        let refused = |text: &str| PrivateText::new(text.to_owned()).err();
        assert_eq!(refused(""), Some(PrivateTextRefused::Empty));
        assert_eq!(
            refused("pass\nword"),
            Some(PrivateTextRefused::ControlCharacter)
        );
        assert_eq!(
            refused("pass\tword"),
            Some(PrivateTextRefused::ControlCharacter)
        );
        assert_eq!(refused(&"7".repeat(257)), Some(PrivateTextRefused::TooLong));
        assert_eq!(refused(&"é".repeat(256)), None, "characters, not bytes");
        assert_eq!(refused("4242 4242 4242 4242"), None);
    }

    #[test]
    fn tests_that_a_body_that_does_not_fit_never_quotes_its_value() {
        let error = serde_json::from_str::<PrivateText>(r#""CANARY-pw\n""#)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(
            error.starts_with("a private value holds no control character"),
            "{error}"
        );
        assert!(!error.contains("CANARY"), "{error}");
    }

    #[test]
    fn tests_that_a_body_value_becomes_private_text() -> Result<(), serde_json::Error> {
        let value: PrivateText = serde_json::from_str(r#""246813""#)?;
        assert_eq!(value.char_count(), 6);
        assert_eq!(value.chars().collect::<String>(), "246813");
        Ok(())
    }
}
