use super::space;

/// `!isNaN(Number(s))`, the number test in the vendored `isomorphic/yaml.ts`
/// `yamlStringNeedsQuotes`. `Number(s)` is ECMA-262's `StringToNumber`, which is `NaN` exactly
/// when `s` is no `StringNumericLiteral`:
/// - JavaScript's whitespace is trimmed from both ends, and an empty or all-space string is 0;
/// - decimal digits with an optional sign, `5.` and `.5` included, then an optional exponent
///   with an optional sign;
/// - `Infinity` with an optional sign, case-sensitive;
/// - `0x`, `0o` or `0b` (either case) with at least one digit of that radix and no sign;
/// - no `_` separators, and `NaN` is not a number.
///
/// Rust's `f64::from_str` is wrong in both directions: it takes `inf`, `NaN` and `infinity` in
/// any case, and refuses spaces, the empty string and `0x1F`.
pub(crate) fn is_number(s: &str) -> bool {
    let literal = space::trim(s);
    if literal.is_empty() {
        return true;
    }
    if let Some((radix, digits)) = radix_prefixed(literal) {
        return !digits.is_empty() && digits.chars().all(|digit| digit.is_digit(radix));
    }
    let unsigned = literal.strip_prefix(['+', '-']).unwrap_or(literal);
    unsigned == "Infinity" || is_decimal(unsigned)
}

/// A `NonDecimalIntegerLiteral`'s radix and the text after its prefix.
fn radix_prefixed(literal: &str) -> Option<(u32, &str)> {
    let rest = literal.strip_prefix('0')?;
    if let Some(digits) = rest.strip_prefix(['x', 'X']) {
        return Some((16, digits));
    }
    if let Some(digits) = rest.strip_prefix(['o', 'O']) {
        return Some((8, digits));
    }
    rest.strip_prefix(['b', 'B']).map(|digits| (2, digits))
}

/// `StrUnsignedDecimalLiteral` without `Infinity`: digits with an optional `.` and fraction, at
/// least one digit on either side of it, then an optional exponent.
fn is_decimal(literal: &str) -> bool {
    let (whole, after_whole) = digits(literal);
    let (fraction, rest) = match after_whole.strip_prefix('.') {
        Some(after_point) => digits(after_point),
        None => (0, after_whole),
    };
    if whole == 0 && fraction == 0 {
        return false;
    }
    match rest.strip_prefix(['e', 'E']) {
        None => rest.is_empty(),
        Some(exponent) => {
            let (count, after) = digits(exponent.strip_prefix(['+', '-']).unwrap_or(exponent));
            count > 0 && after.is_empty()
        }
    }
}

/// How many ASCII digits `text` starts with, and the text after them.
fn digits(text: &str) -> (usize, &str) {
    let after = text.trim_start_matches(|character: char| character.is_ascii_digit());
    (text.len() - after.len(), after)
}
