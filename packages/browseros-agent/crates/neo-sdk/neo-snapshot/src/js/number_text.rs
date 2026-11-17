use std::num::FpCategory;

use serde_json::Number;

/// Below this, every integer is a double exactly and prints as its own digits.
const EXACT_INTEGERS: u64 = 1 << 53;

/// Significant digits that always read back as the same double.
const MAX_DIGITS: usize = 17;

/// `String(x)` for the double `JSON.parse` makes of `number`: how the TypeScript renderer prints a
/// level or a box coordinate.
///
/// The double: `as_f64`, which rounds an integer past 2^53 to the nearest double, a tie to the
/// even one, as `JSON.parse` does. Then ECMA-262's `Number::toString`:
/// - `-0` prints `0`;
/// - plain digits from 1e-6 up to below 1e21 (`0.000001`, `123456789012345680000`), an exponent
///   outside that range (`1e-7`, `1e+21`);
/// - the digits are the ones QuickJS chooses, the engine the TypeScript runs in. It starts from
///   17 significant digits, each count correctly rounded with a tie to even and without trailing
///   zeros, and takes one digit fewer while the shorter digits still read back as the same
///   double, stopping at the first that do not.
///
/// That is the fewest digits that read back, the closest on a tie, except at some powers of two,
/// whose rounding interval is narrower below than above: there QuickJS stops with more digits,
/// and so does this. Rust's own shortest form (`{:e}`) breaks a tie the other way:
/// -167581363823776.125 prints `-167581363823776.12` in JavaScript and `…776.13` in Rust.
pub(crate) fn number_text(number: &Number) -> String {
    if let Some(integer) = number.as_i64()
        && integer.unsigned_abs() < EXACT_INTEGERS
    {
        return integer.to_string();
    }
    // `as_f64` is `None` only under serde_json's `arbitrary_precision`, for a number past the
    // double's range, which `JSON.parse` reads as an infinity.
    let double = number.as_f64().unwrap_or_else(|| {
        if number.to_string().starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }
    });
    double_text(double)
}

/// `Number::toString(x)` with radix 10.
fn double_text(double: f64) -> String {
    let sign = if double.is_sign_negative() { "-" } else { "" };
    match double.classify() {
        FpCategory::Nan => "NaN".to_owned(),
        FpCategory::Infinite => format!("{sign}Infinity"),
        FpCategory::Zero => "0".to_owned(),
        FpCategory::Subnormal | FpCategory::Normal => {
            let (digits, point) = quickjs_digits(double.abs());
            format!("{sign}{}", layout(&digits, point))
        }
    }
}

/// QuickJS's digits for a positive finite `double` (`js_dtoa`'s free format in its `dtoa.c`), and
/// where the decimal point goes: the value is `0.<digits>` times 10 to the power `point`.
fn quickjs_digits(double: f64) -> (String, i32) {
    let mut found = rounded(double, MAX_DIGITS);
    while found.0.len() > 1 {
        let shorter = rounded(double, found.0.len() - 1);
        if !reads_back(&shorter, double) {
            break;
        }
        found = shorter;
    }
    found
}

/// `double` correctly rounded to `precision` significant digits, a tie to the even digit (Rust's
/// exact formatting), without trailing zeros; and the decimal point's place.
fn rounded(double: f64, precision: usize) -> (String, i32) {
    let text = format!("{double:.fraction$e}", fraction = precision - 1);
    let (mantissa, exponent) = text.split_once('e').unwrap_or((text.as_str(), "0"));
    let digits: String = mantissa
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .trim_end_matches('0')
        .to_owned();
    // Rust writes the exponent as plain decimal digits, so it always parses.
    let exponent: i32 = exponent.parse().unwrap_or_default();
    (digits, exponent + 1)
}

/// Whether `0.<digits>` times 10 to the power `point` reads back as `double`. Rust's float
/// parsing rounds correctly, as `JSON.parse` and QuickJS's own check do.
fn reads_back((digits, point): &(String, i32), double: f64) -> bool {
    format!("0.{digits}e{point}")
        .parse::<f64>()
        .is_ok_and(|read| read.to_bits() == double.to_bits())
}

/// ECMA-262 `Number::toString` from step 5 on: the digits `0.<digits>` times 10 to the power
/// `point`, plain or with an exponent.
fn layout(digits: &str, point: i32) -> String {
    let count = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if (count..=21).contains(&point) {
        // k <= n <= 21: the digits, then zeros up to the point.
        format!("{digits}{}", zeros(point - count))
    } else if (1..=21).contains(&point) {
        // 0 < n <= 21: the point inside the digits.
        let split = usize::try_from(point).unwrap_or_default();
        let whole: String = digits.chars().take(split).collect();
        let fraction: String = digits.chars().skip(split).collect();
        format!("{whole}.{fraction}")
    } else if (-5..=0).contains(&point) {
        // -6 < n <= 0: zeros between the point and the digits.
        format!("0.{}{digits}", zeros(-point))
    } else {
        let exponent = point - 1;
        let sign = if exponent < 0 { '-' } else { '+' };
        let first: String = digits.chars().take(1).collect();
        let rest: String = digits.chars().skip(1).collect();
        let dot = if rest.is_empty() { "" } else { "." };
        format!("{first}{dot}{rest}e{sign}{}", exponent.unsigned_abs())
    }
}

/// `count` zeros; none for a count below one.
fn zeros(count: i32) -> String {
    "0".repeat(usize::try_from(count).unwrap_or_default())
}
