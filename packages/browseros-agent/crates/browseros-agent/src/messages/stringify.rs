//! JSON embedded as tool-result text must follow JavaScript's property enumeration and
//! number spelling. Numeric index keys precede insertion-ordered ordinary keys; serde's
//! float spelling alone differs at integer floats and the 1e-6/1e21 exponent cutoffs.
use serde_json::Value;

pub(crate) fn js_stringify(value: &Value) -> String {
    match value {
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(js_stringify)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(map) => {
            let mut indexed = map
                .iter()
                .filter_map(|(key, value)| index(key).map(|id| (id, key, value)))
                .collect::<Vec<_>>();
            indexed.sort_by_key(|(id, _, _)| *id);
            let fields = indexed
                .into_iter()
                .map(|(_, k, v)| (k, v))
                .chain(map.iter().filter(|(key, _)| index(key).is_none()));
            format!(
                "{{{}}}",
                fields
                    .map(|(key, value)| format!("{}:{}", quote(key), js_stringify(value)))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Number(number) => number
            .as_f64()
            .map(number_string)
            .unwrap_or_else(|| number.to_string()),
        Value::String(text) => quote(text),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".into(),
    }
}
fn quote(text: &str) -> String {
    // A JSON string is always serializable; the fallback avoids a panic if serde changes.
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}
fn index(key: &str) -> Option<u32> {
    let index = key.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == key).then_some(index)
}
fn number_string(number: f64) -> String {
    if number == 0.0 {
        return "0".into();
    }
    let raw = number.to_string();
    let (sign, raw) = raw
        .strip_prefix('-')
        .map_or(("", raw.as_str()), |rest| ("-", rest));
    let (mantissa, exponent) = raw
        .split_once(['e', 'E'])
        .map_or((raw, 0), |(m, e)| (m, e.parse::<i32>().unwrap_or_default()));
    let dot = mantissa.find('.').unwrap_or(mantissa.len()) as i32;
    let digits = mantissa.replace('.', "");
    let leading = digits.bytes().take_while(|b| *b == b'0').count();
    let digits = digits[leading..].trim_end_matches('0');
    let decimal = dot + exponent - leading as i32;
    let exponent = decimal - 1;
    if !(-6..21).contains(&exponent) {
        let suffix = if digits.len() > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        format!(
            "{sign}{}{suffix}e{}{exponent}",
            &digits[..1],
            if exponent >= 0 { "+" } else { "" }
        )
    } else if decimal <= 0 {
        format!("{sign}0.{}{digits}", "0".repeat((-decimal) as usize))
    } else if decimal as usize >= digits.len() {
        format!(
            "{sign}{digits}{}",
            "0".repeat(decimal as usize - digits.len())
        )
    } else {
        format!(
            "{sign}{}.{}",
            &digits[..decimal as usize],
            &digits[decimal as usize..]
        )
    }
}
