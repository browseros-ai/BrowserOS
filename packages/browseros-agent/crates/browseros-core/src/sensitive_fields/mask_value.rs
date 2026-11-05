//! How a sensitive field's value is shown wherever Neo reads it: the snapshot renderer here, and
//! `seal.js` in the sealed world, which applies the same two rules.

use std::borrow::Cow;

use super::field_rules::SensitiveKind;

/// What a non-empty sensitive value becomes. Fixed width, so the value's length does not leak
/// (Chromium's own password mask keeps one bullet per character).
pub const FIELD_MASK: &str = "••••";

/// The value as Neo shows it. Empty stays empty, so a reader can still tell a filled field from
/// an empty one. A valid card number keeps its last four digits, as a receipt does, so the agent
/// can tell which card was used.
#[must_use]
pub fn mask_value(kind: SensitiveKind, value: &str) -> Cow<'_, str> {
    if value.is_empty() {
        return Cow::Borrowed(value);
    }
    if kind == SensitiveKind::Card
        && let Some(digits) = card_digits(value)
    {
        let last_four = digits[digits.len() - 4..]
            .iter()
            .filter_map(|digit| char::from_digit(*digit, 10))
            .collect::<String>();
        return Cow::Owned(format!("{FIELD_MASK} {last_four}"));
    }
    Cow::Borrowed(FIELD_MASK)
}

/// Whether `value` is a card number: 13 to 19 digits, optionally grouped with spaces or dashes,
/// that pass the Luhn check.
#[must_use]
pub fn is_card_number(value: &str) -> bool {
    card_digits(value).is_some()
}

fn card_digits(value: &str) -> Option<Vec<u32>> {
    if !value
        .chars()
        .all(|c| c.is_ascii_digit() || c == ' ' || c == '-')
    {
        return None;
    }
    let digits = value
        .chars()
        .filter_map(|c| c.to_digit(10))
        .collect::<Vec<_>>();
    ((13..=19).contains(&digits.len()) && passes_luhn(&digits)).then_some(digits)
}

/// The Luhn checksum over a card number's digits.
pub(crate) fn passes_luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(index, &digit)| {
            if index % 2 == 1 {
                let doubled = digit * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                digit
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::{FIELD_MASK, is_card_number, mask_value};
    use crate::sensitive_fields::SensitiveKind;

    #[test]
    fn masks_are_fixed_width_and_cards_keep_their_last_four() {
        assert_eq!(mask_value(SensitiveKind::Password, "hunter22"), FIELD_MASK);
        assert_eq!(
            mask_value(SensitiveKind::Password, "••••••••••••"),
            FIELD_MASK
        );
        assert_eq!(mask_value(SensitiveKind::OneTimeCode, "7"), FIELD_MASK);
        assert_eq!(mask_value(SensitiveKind::Card, "987"), FIELD_MASK);
        assert_eq!(
            mask_value(SensitiveKind::Card, "4111111111111111"),
            "•••• 1111"
        );
        assert_eq!(
            mask_value(SensitiveKind::Card, "3782-822463-10005"),
            "•••• 0005"
        );
        assert_eq!(
            mask_value(SensitiveKind::Password, "4111111111111111"),
            FIELD_MASK
        );
        assert_eq!(mask_value(SensitiveKind::Card, ""), "");
    }

    #[test]
    fn card_numbers_need_the_right_length_and_checksum() {
        assert!(is_card_number("4111 1111 1111 1111"));
        assert!(is_card_number("4012888888881881"));
        assert!(!is_card_number("4111111111111112"));
        assert!(!is_card_number("411111111111"));
        assert!(!is_card_number("4111x111111111111"));
    }
}
