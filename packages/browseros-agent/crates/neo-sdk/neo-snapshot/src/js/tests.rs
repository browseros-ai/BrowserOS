//! Each `js` function against QuickJS, the engine the TypeScript renderer runs in: the same
//! inputs through the JavaScript expression the function stands for, one script per batch.

use std::error::Error;
use std::num::TryFromIntError;

use rquickjs::{CatchResultExt, Context, Runtime};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::split_mix64::SplitMix64;

use super::{
    is_number, is_space, json_quote, math_round, number_text, prefix_as_javascript,
    prefix_between_characters, skip_units, trim, trim_end, trim_start, units,
};

/// The seed of every random batch, so a failure names doubles that reproduce.
const SEED: u64 = 0x5eed_cafe_f00d_0001;

/// Characters next to the space set that JavaScript does not trim: U+0085, which Rust's
/// `char::is_whitespace` takes, and look-alikes Unicode once listed or that render blank.
const NEAR_SPACES: [char; 9] = [
    '\0', '\u{1f}', '\u{85}', '\u{ad}', '\u{180e}', '\u{200b}', '\u{2060}', '\u{fffe}', 'x',
];

/// Runs `script` in a fresh QuickJS context, with `input` as JSON text in the global `input`, and
/// reads the JSON text the script evaluates to.
fn quickjs<T: DeserializeOwned>(input: &impl Serialize, script: &str) -> Result<T, Box<dyn Error>> {
    let input = serde_json::to_string(input)?;
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let output = context.with(|ctx| -> Result<String, String> {
        ctx.globals()
            .set("input", input)
            .catch(&ctx)
            .map_err(|caught| caught.to_string())?;
        ctx.eval::<String, _>(script)
            .catch(&ctx)
            .map_err(|caught| caught.to_string())
    })?;
    Ok(serde_json::from_str(&output)?)
}

/// A double's bits as the two big-endian halves a `DataView` writes, so it crosses into the
/// script exactly.
fn halves(double: f64) -> Result<[u32; 2], TryFromIntError> {
    let bits = double.to_bits();
    Ok([
        u32::try_from(bits >> 32)?,
        u32::try_from(bits & 0xffff_ffff)?,
    ])
}

/// The double two big-endian halves hold.
fn from_halves([high, low]: [u32; 2]) -> f64 {
    f64::from_bits((u64::from(high) << 32) | u64::from(low))
}

/// 2 to the power `exponent`, built from its bits: a subnormal below -1022.
fn power_of_two(exponent: i32) -> Result<f64, TryFromIntError> {
    Ok(if exponent < -1022 {
        f64::from_bits(1 << u32::try_from(exponent + 1074)?)
    } else {
        f64::from_bits(u64::try_from(exponent + 1023)? << 52)
    })
}

/// The units QuickJS's `/\s/` matches, of all 65,536.
fn javascript_spaces() -> Result<Vec<u32>, Box<dyn Error>> {
    quickjs(
        &(),
        r"JSON.stringify(Array.from({ length: 0x10000 }, (_, unit) => unit)
            .filter((unit) => /\s/.test(String.fromCharCode(unit))))",
    )
}

#[test]
fn tests_that_is_space_is_javascripts_on_every_utf16_unit() -> Result<(), Box<dyn Error>> {
    let spaces = javascript_spaces()?;
    for unit in 0..=0xffff_u32 {
        let javascript = spaces.contains(&unit);
        match char::from_u32(unit) {
            Some(character) => assert_eq!(is_space(character), javascript, "U+{unit:04X}"),
            None => assert!(
                !javascript,
                "U+{unit:04X} is half a surrogate pair, which JavaScript never calls a space and \
                 no Rust string holds"
            ),
        }
    }
    Ok(())
}

#[test]
fn tests_that_trim_takes_exactly_javascripts_spaces_off_either_end() -> Result<(), Box<dyn Error>> {
    let spaces: Vec<char> = javascript_spaces()?
        .into_iter()
        .filter_map(char::from_u32)
        .collect();
    let every: String = spaces.iter().collect();
    let mut cases: Vec<String> = spaces
        .iter()
        .chain(&NEAR_SPACES)
        .map(|edge| format!("{edge}{edge}a{edge}b{edge}{edge}"))
        .collect();
    cases.extend([format!("{every}a\u{85}b{every}"), every, String::new()]);
    let trimmed: Vec<[String; 3]> = quickjs(
        &cases,
        "JSON.stringify(JSON.parse(input).map((s) => [s.trim(), s.trimStart(), s.trimEnd()]))",
    )?;
    assert_eq!(trimmed.len(), cases.len(), "one answer per case");
    for (case, [both, start, end]) in cases.iter().zip(&trimmed) {
        assert_eq!(trim(case), both, "{case:?}.trim()");
        assert_eq!(trim_start(case), start, "{case:?}.trimStart()");
        assert_eq!(trim_end(case), end, "{case:?}.trimEnd()");
    }
    Ok(())
}

#[test]
fn tests_that_json_quote_is_json_stringify() -> Result<(), Box<dyn Error>> {
    let mut cases: Vec<String> = (0..0x100_u32)
        .filter_map(char::from_u32)
        .map(String::from)
        .collect();
    cases.extend(
        [
            "\u{2028}",
            "\u{2029}",
            "\u{1f600}",
            "a \"quoted\" name\\ with\n/slashes/\u{7f}\u{9f}\u{1f600}",
        ]
        .map(String::from),
    );
    let quoted: Vec<String> = quickjs(
        &cases,
        "JSON.stringify(JSON.parse(input).map((s) => JSON.stringify(s)))",
    )?;
    assert_eq!(quoted.len(), cases.len(), "one answer per case");
    for (case, javascript) in cases.iter().zip(&quoted) {
        let mut rust = String::from("key ");
        json_quote(case, &mut rust);
        assert_eq!(
            rust,
            format!("key {javascript}"),
            "JSON.stringify({case:?})"
        );
    }
    Ok(())
}

#[test]
fn tests_that_is_number_is_not_isnan_of_number() -> Result<(), Box<dyn Error>> {
    let mut cases: Vec<String> = [
        "",
        " ",
        "\t\n\r",
        "\u{feff}",
        "\u{a0}",
        "\u{3000}\u{2028}",
        "\u{85}",
        "\u{85}1",
        "\u{feff}12\u{a0}",
        " 12 ",
        "12",
        "+5",
        "-5",
        "-.5",
        "+.5",
        "5.",
        ".",
        "-.",
        "1e",
        "1e+",
        "1e-",
        "1e5",
        "1E5",
        "1e+5",
        "1e-5",
        "1.e5",
        ".e5",
        "e5",
        "1e5.5",
        "1e1e1",
        "0e",
        "5e+0",
        ".5e-3",
        "0x1F",
        "0X1f",
        "0x",
        "0xG",
        "0x1.8",
        "-0x1",
        "+0x1",
        "0b101",
        "0B1",
        "0b2",
        "0b",
        "0o17",
        "0O7",
        "0o8",
        "0o",
        "00",
        "007",
        "010",
        "0.0",
        "-0",
        "+0",
        "1_000",
        "1,000",
        "12px",
        "1 2",
        "+-5",
        "--5",
        "5-",
        "-",
        "+",
        "Infinity",
        "-Infinity",
        "+Infinity",
        "infinity",
        "INFINITY",
        "Infinityx",
        "-Infinity ",
        "NaN",
        "-NaN",
        "nan",
        "inf",
        "1e400",
        "-1e400",
        "1e-400",
        "\u{661}\u{662}",
        "\u{ff11}",
        "\0",
        "1\0",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    cases.push(format!("{}.5", "9".repeat(400)));
    cases.push("0xFFFFFFFFFFFFFFFFFFFF".to_owned());
    let numbers: Vec<bool> = quickjs(
        &cases,
        "JSON.stringify(JSON.parse(input).map((s) => !isNaN(Number(s))))",
    )?;
    assert_eq!(numbers.len(), cases.len(), "one answer per case");
    for (case, javascript) in cases.iter().zip(&numbers) {
        assert_eq!(is_number(case), *javascript, "!isNaN(Number({case:?}))");
    }
    Ok(())
}

#[test]
fn tests_that_number_text_is_string_of_the_double() -> Result<(), Box<dyn Error>> {
    let mut random = SplitMix64(SEED);
    let mut doubles: Vec<f64> = random
        .by_ref()
        .map(f64::from_bits)
        .filter(|double| double.is_finite())
        .take(20_000)
        .collect();
    // Exponents from about 1e-9 to 4e22: where the plain form meets the exponent form.
    for _ in 0..5_000 {
        let bits = random.next().ok_or("splitmix64 never ends")?;
        let exponent = 993 + bits % 105;
        doubles.push(f64::from_bits((exponent << 52) | (bits >> 12)));
    }
    for exponent in -1074..=1023 {
        let power = power_of_two(exponent)?;
        doubles.extend([power, -power, power.next_up(), power.next_down()]);
    }
    doubles.extend([
        0.0,
        -0.0,
        f64::from_bits(1),
        f64::MIN_POSITIVE,
        f64::MAX,
        -f64::MAX,
        1e21,
        1e-6,
        1e-7,
        123_456_789_012_345_680_000.0,
        0.1 + 0.2,
        // -167581363823776.125 exactly: two 17-digit forms tie, and JavaScript takes the even.
        -(167_581_363_823_776.0 + 0.125),
        9_007_199_254_740_992.0,
        9_007_199_254_740_994.0,
        1e20,
        2.5,
        -1.5,
        100.0,
    ]);
    let bits: Vec<[u32; 2]> = doubles
        .iter()
        .map(|double| halves(*double))
        .collect::<Result<_, _>>()?;
    let texts: Vec<String> = quickjs(
        &bits,
        "const view = new DataView(new ArrayBuffer(8));
         JSON.stringify(JSON.parse(input).map(([high, low]) => {
           view.setUint32(0, high);
           view.setUint32(4, low);
           return String(view.getFloat64(0));
         }))",
    )?;
    assert_eq!(texts.len(), doubles.len(), "one answer per double");
    for (double, javascript) in doubles.iter().zip(&texts) {
        let number = Number::from_f64(*double).ok_or("every double here is finite")?;
        assert_eq!(
            number_text(&number),
            *javascript,
            "String(x) of the double with bits {:#018x}",
            double.to_bits()
        );
    }
    Ok(())
}

/// JSON texts as `JSON.parse` reads them: integers past 2^53 and past `u64`, `-0`, a fraction
/// written with its zero, and exponents.
#[test]
fn tests_that_number_text_prints_a_json_number_as_string_of_json_parse()
-> Result<(), Box<dyn Error>> {
    let cases = [
        "0",
        "-0",
        "7",
        "-7",
        "1.0",
        "-0.0",
        "2.5",
        "9007199254740991",
        "9007199254740992",
        "9007199254740993",
        "-9007199254740993",
        "9223372036854775807",
        "-9223372036854775808",
        "18446744073709551615",
        "18446744073709551616",
        "123456789012345680000",
        "1e21",
        "1E-7",
        "0.000001",
        "5e-324",
        "1.7976931348623157e308",
    ];
    let texts: Vec<String> = quickjs(
        &cases,
        "JSON.stringify(JSON.parse(input).map((text) => String(JSON.parse(text))))",
    )?;
    assert_eq!(texts.len(), cases.len(), "one answer per case");
    for (case, javascript) in cases.iter().zip(&texts) {
        let number: Number = serde_json::from_str(case)?;
        assert_eq!(
            number_text(&number),
            *javascript,
            "String(JSON.parse({case:?}))"
        );
    }
    Ok(())
}

#[test]
fn tests_that_math_round_is_javascripts_to_the_bit() -> Result<(), Box<dyn Error>> {
    let mut doubles: Vec<f64> = SplitMix64(SEED ^ 1)
        .map(f64::from_bits)
        .take(5_000)
        .collect();
    for whole in -1_000..=1_000 {
        let half = f64::from(whole) + 0.5;
        doubles.extend([half, half.next_up(), half.next_down(), f64::from(whole)]);
    }
    doubles.extend([
        0.0,
        -0.0,
        0.5_f64.next_down(),
        0.5,
        -0.5,
        -(0.5_f64.next_down()),
        -0.4,
        -0.7,
        2.5,
        -2.5,
        f64::from_bits(1),
        -f64::from_bits(1),
        4_503_599_627_370_495.5,
        4_503_599_627_370_497.0,
        9_007_199_254_740_994.0,
        f64::MAX,
        f64::MIN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ]);
    let bits: Vec<[u32; 2]> = doubles
        .iter()
        .map(|double| halves(*double))
        .collect::<Result<_, _>>()?;
    let rounded: Vec<[u32; 2]> = quickjs(
        &bits,
        "const view = new DataView(new ArrayBuffer(8));
         JSON.stringify(JSON.parse(input).map(([high, low]) => {
           view.setUint32(0, high);
           view.setUint32(4, low);
           view.setFloat64(0, Math.round(view.getFloat64(0)));
           return [view.getUint32(0), view.getUint32(4)];
         }))",
    )?;
    assert_eq!(rounded.len(), doubles.len(), "one answer per double");
    for (double, javascript) in doubles.iter().zip(&rounded) {
        let (rust, javascript) = (math_round(*double), from_halves(*javascript));
        assert!(
            rust.to_bits() == javascript.to_bits() || (rust.is_nan() && javascript.is_nan()),
            "Math.round({double:e}): JavaScript {javascript:e} ({:#018x}), Rust {rust:e} ({:#018x})",
            javascript.to_bits(),
            rust.to_bits()
        );
    }
    Ok(())
}

/// What QuickJS answers for one string and one cut, each string as its UTF-16 units.
#[derive(Deserialize)]
struct Cut {
    length: usize,
    clip: Vec<u16>,
    prefix: Vec<u16>,
    rest: Vec<u16>,
}

#[test]
fn tests_that_utf16_lengths_and_cuts_are_javascripts_at_every_unit() -> Result<(), Box<dyn Error>> {
    let strings = [
        "",
        "abc",
        "\u{1f600}",
        "a\u{1f600}b",
        "\u{1f600}\u{1f600}",
        "ab\u{1f600}c\u{1f600}\u{1f600}d",
        "\u{e9}\u{10ffff}\u{fc}\u{1f600}",
        "\u{ffff}\u{1f600}\u{10000}",
    ];
    let mut cases = Vec::new();
    for string in strings {
        for cut in 0..=units(string) + 2 {
            cases.push((string, cut));
        }
    }
    let javascript: Vec<Cut> = quickjs(
        &cases,
        "const codes = (s) => Array.from({ length: s.length }, (_, index) => s.charCodeAt(index));
         const clip = (s, length) => {
           const last = s.charCodeAt(length - 1);
           return s.slice(0, last >= 0xd800 && last <= 0xdbff ? length - 1 : length);
         };
         JSON.stringify(JSON.parse(input).map(([s, cut]) => ({
           length: s.length,
           clip: codes(clip(s, cut)),
           prefix: codes(s.slice(0, cut)),
           rest: codes(s.slice(cut)),
         })))",
    )?;
    assert_eq!(javascript.len(), cases.len(), "one answer per case");
    for ((string, cut), answer) in cases.iter().zip(&javascript) {
        assert_eq!(units(string), answer.length, "{string:?}.length");
        assert_eq!(
            prefix_between_characters(string, *cut),
            String::from_utf16(&answer.clip)?,
            "clip's cut of {string:?} at {cut} never splits a pair"
        );
        // A lone half on the JavaScript side is U+FFFD on Rust's, one for one.
        assert_eq!(
            prefix_as_javascript(string, *cut),
            String::from_utf16_lossy(&answer.prefix),
            "{string:?}.slice(0, {cut})"
        );
        assert_eq!(
            skip_units(string, *cut),
            String::from_utf16_lossy(&answer.rest),
            "{string:?}.slice({cut})"
        );
    }
    Ok(())
}
