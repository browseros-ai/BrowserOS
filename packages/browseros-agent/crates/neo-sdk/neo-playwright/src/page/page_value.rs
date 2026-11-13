//! Values in Playwright's in-page serialized format, read strictly from the page's text: the one
//! way a main-world script's result leaves an op.
//!
//! ```text
//!  page    assets/main-world.js: serializeAsCallArgument(value) ─▶ writeJson ─▶ one string
//!  here    depth_of ─▶ strict parse ─▶ PageValue ─▶ into_json, the format again
//!  agent   the prelude's vendored parseEvaluationResultValue ─▶ the script's value
//! ```
//!
//! The format is `SerializedValue` in the vendored `isomorphic/utilityScriptSerializers.ts`: bare
//! strings, numbers and booleans, and a tagged object for everything else (`{v}`, `{d}`, `{bi}`,
//! `{a, id}`, `{o: [{k, v}], id}`, `{ref}`, ...). The page hands it over as text rather than by
//! value: the format nests about three JSON levels per object level, and browseros-cdp drops a
//! reply nested deeper than 128. Rust reads it, rather than passing the text through, to give
//! upstream's server's own failures: a result deeper than its CDP limit, and a `{fn}` it cannot
//! serialize again ([`PageValue::binding_position`]).
//!
//! The page owns the globals the serializer runs on and can tamper with them, so the parse admits
//! only the shapes the format has. Anything else fails as the page's, and nothing of it leaves.
//! The parse renumbers every `id` in order of appearance, as upstream's server writes them
//! again, and rewrites every `ref` to match; a `ref` to an id not seen yet fails too.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Map, Number, Value, json};

use crate::listed_enum::listed_enum;

/// The deepest JSON nesting of a result upstream returns from a main-world script. Measured
/// (spike S-depth, Playwright 1.64.1 on BrowserOS neo): 99 nested objects, 297 levels of the
/// format, come back, and 149 nested arrays, 298, fail with [`TOO_DEEP`], because Chromium nests
/// a CDP message at most 300 levels deep and the reply wraps the value in three. A deeper result
/// is refused here as upstream's is, and the cap bounds every recursion over a value in Rust.
pub(crate) const MAX_DEPTH: usize = 297;

/// upstream's message for a result deeper than [`MAX_DEPTH`] (`rewriteError`, PW:36883).
pub(crate) const TOO_DEEP: &str = "Cannot serialize result: object reference chain is too long.";

/// One value of Playwright's in-page format. No `Debug`: it is page text, which no log line
/// carries (STYLE P4).
pub(crate) enum PageValue {
    /// A bare string, number or boolean, as the format keeps them.
    Str(String),
    Num(Number),
    Bool(bool),
    /// `{v}`: what JSON cannot spell.
    Special(Special),
    /// `{d}`: a Date's ISO text; `None` for an invalid Date, whose `toJSON` is `null`.
    Date(Option<String>),
    /// `{u}`: a URL's href.
    Url(String),
    /// `{bi}`: a `BigInt`'s decimal digits.
    BigInt(String),
    /// `{r: {p, f}}`.
    RegExp {
        source: String,
        flags: String,
    },
    /// `{e: {n, m, s}}`.
    Error {
        name: String,
        message: String,
        stack: String,
    },
    /// `{ta: {b, k}}`: the bytes in base64 as the page wrote them, whole elements of `kind`.
    TypedArray {
        base64: String,
        kind: TypedKind,
    },
    /// `{a, id}` and `{o: [{k, v}], id}`. `id` is the serializer's counter, which `{ref}` names
    /// for cycles and shared objects.
    Array {
        items: Vec<PageValue>,
        id: u64,
    },
    Object {
        fields: Vec<(String, PageValue)>,
        id: u64,
    },
    Ref(u64),
    /// `{fn}`: a function whose name starts with `__pw_fn_`, which the serializer writes as its
    /// name.
    Function(String),
    /// A value the serializer dropped, a function: JSON writes it as `null` in an array and
    /// leaves `v` out of an object's member, and the agent reads `null` and `undefined` there.
    Dropped,
}

listed_enum! {
    /// `{v}`'s values.
    pub(crate) enum Special {
        Undefined,
        Null,
        NaN,
        Infinity,
        NegativeInfinity,
        NegativeZero,
    }
}

impl Special {
    const fn tag(self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::Null => "null",
            Self::NaN => "NaN",
            Self::Infinity => "Infinity",
            Self::NegativeInfinity => "-Infinity",
            Self::NegativeZero => "-0",
        }
    }
}

listed_enum! {
    /// `{ta}`'s `k`: the serializer's names for the typed array constructors.
    pub(crate) enum TypedKind {
        I8,
        Ui8,
        Ui8c,
        I16,
        Ui16,
        I32,
        Ui32,
        F32,
        F64,
        Bi64,
        Bui64,
    }
}

impl TypedKind {
    /// How many bytes one element takes.
    const fn width(self) -> usize {
        match self {
            Self::I8 | Self::Ui8 | Self::Ui8c => 1,
            Self::I16 | Self::Ui16 => 2,
            Self::I32 | Self::Ui32 | Self::F32 => 4,
            Self::F64 | Self::Bi64 | Self::Bui64 => 8,
        }
    }

    const fn tag(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::Ui8 => "ui8",
            Self::Ui8c => "ui8c",
            Self::I16 => "i16",
            Self::Ui16 => "ui16",
            Self::I32 => "i32",
            Self::Ui32 => "ui32",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Bi64 => "bi64",
            Self::Bui64 => "bui64",
        }
    }
}

impl PageValue {
    /// The value in Playwright's format, as the prelude reads it back.
    pub(crate) fn into_json(self) -> Value {
        written(self)
    }

    /// Where the first `{fn}` is, as upstream's server names it when it cannot serialize the
    /// result again: the server reads `{fn}` back as a binding's caller, a function, and its
    /// `serializeValue` refuses a function (`Attempting to serialize unexpected value at
    /// position "…"`, PW:12751). `Some("")` for the value itself, `None` when there is none.
    /// The walk is the server's: items and members in order, a ref not entered again.
    pub(crate) fn binding_position(&self) -> Option<String> {
        let mut chain = Vec::new();
        binding_chain(self, &mut chain).then(|| {
            let at = chain
                .iter()
                .enumerate()
                .map(|(place, access)| match access {
                    Access::Item(index) => format!("[{index}]"),
                    Access::Member(key) if place == 0 => (*key).to_owned(),
                    Access::Member(key) => format!(".{key}"),
                })
                .collect::<Vec<_>>()
                .concat();
            if at.is_empty() {
                at
            } else {
                format!(" at position \"{at}\"")
            }
        })
    }
}

/// One step of upstream's access chain.
enum Access<'a> {
    Item(usize),
    Member(&'a str),
}

/// Whether `value` holds a `{fn}`, with `chain` left at the first one's place.
fn binding_chain<'a>(value: &'a PageValue, chain: &mut Vec<Access<'a>>) -> bool {
    match value {
        PageValue::Function(_) => true,
        PageValue::Array { items, .. } => items.iter().enumerate().any(|(index, item)| {
            chain.push(Access::Item(index));
            binding_chain(item, chain) || chain.pop().is_none()
        }),
        PageValue::Object { fields, .. } => fields.iter().any(|(key, value)| {
            chain.push(Access::Member(key));
            binding_chain(value, chain) || chain.pop().is_none()
        }),
        _ => false,
    }
}

/// Why the page's text is not a value this module reads.
pub(crate) enum Unreadable {
    /// Deeper than [`MAX_DEPTH`]: upstream's [`TOO_DEEP`].
    TooDeep,
    /// A shape the format does not have, or text that is not JSON. Only a page that tampered
    /// with the serializer's globals gets here, so it fails as `website / page_error`.
    Shape,
}

impl PageValue {
    /// Parses the page's text, failing on anything the format does not have.
    ///
    /// `{m}`, `{s}` and `{h}` are refused: a result is serialized without extended types or
    /// handles. `{fn}` is read: the serializer writes one for any function named `__pw_fn_…`. `{ab}` reads as `{o: [], id: 0}`, because upstream's server→client protocol
    /// step has no `ArrayBuffer` and gives the agent `{}` (PW:12700-12760); id 0 is the
    /// serializer's own anonymous id, which no `{ref}` names.
    pub(crate) fn parse(text: &str) -> Result<Self, Unreadable> {
        if depth_of(text) > MAX_DEPTH {
            return Err(Unreadable::TooDeep);
        }
        let mut reader = serde_json::Deserializer::from_str(text);
        // The scan above bounds the nesting, which serde's default limit of 128 would cut short.
        reader.disable_recursion_limit();
        let json = Value::deserialize(&mut reader).map_err(|_not_json| Unreadable::Shape)?;
        reader.end().map_err(|_trailing| Unreadable::Shape)?;
        Self::value(json)?.canonical(&mut Ids::default())
    }

    /// A result with no text: the page's `undefined`, which upstream's jsonValue reports as no
    /// value at all.
    pub(crate) fn undefined() -> Self {
        Self::Special(Special::Undefined)
    }

    /// The value with its ids renumbered from 1 in order of appearance, each id given as its
    /// array or object is entered, as the serializer counts them (`++lastId` before the items),
    /// and each ref rewritten to its object's new id. A ref to an id not entered yet, or an id
    /// two objects claim, is no value the serializer writes. Id 0 is the anonymous id an
    /// `ArrayBuffer` stands in with (see [`PageValue::parse`]): it takes a new id, and no ref may
    /// name it.
    fn canonical(self, ids: &mut Ids) -> Result<Self, Unreadable> {
        Ok(match self {
            Self::Array { items, id } => {
                let id = ids.enter(id)?;
                Self::Array {
                    items: items
                        .into_iter()
                        .map(|item| item.canonical(ids))
                        .collect::<Result<_, _>>()?,
                    id,
                }
            }
            Self::Object { fields, id } => {
                let id = ids.enter(id)?;
                Self::Object {
                    fields: fields
                        .into_iter()
                        .map(|(key, value)| Ok((key, value.canonical(ids)?)))
                        .collect::<Result<_, _>>()?,
                    id,
                }
            }
            Self::Ref(id) => Self::Ref(*ids.seen.get(&id).ok_or(Unreadable::Shape)?),
            leaf => leaf,
        })
    }

    /// A value in any place but an array's item, where JSON's `null` cannot stand.
    fn value(json: Value) -> Result<Self, Unreadable> {
        match json {
            Value::String(text) => Ok(Self::Str(text)),
            Value::Number(number) => Ok(Self::Num(number)),
            Value::Bool(flag) => Ok(Self::Bool(flag)),
            Value::Object(tagged) => Self::tagged(tagged),
            Value::Null | Value::Array(_) => Err(Unreadable::Shape),
        }
    }

    /// An array's item, where `null` is a value the serializer dropped.
    fn item(json: Value) -> Result<Self, Unreadable> {
        match json {
            Value::Null => Ok(Self::Dropped),
            other => Self::value(other),
        }
    }

    /// A tagged object, admitted only with exactly the keys its tag has: each tag's reader
    /// checks them again with [`record`] as it takes its values.
    fn tagged(map: Map<String, Value>) -> Result<Self, Unreadable> {
        let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let read: fn(Value) -> Result<Self, Unreadable> = match keys.as_slice() {
            ["ref"] => read_ref,
            ["v"] => read_special,
            ["d"] => read_date,
            ["u"] => read_url,
            ["bi"] => read_bigint,
            ["e"] => read_error,
            ["r"] => read_regexp,
            ["ta"] => read_typed_array,
            ["ab"] => read_array_buffer,
            ["fn"] => read_function,
            ["a", "id"] => read_array,
            ["id", "o"] => read_object,
            _ => return Err(Unreadable::Shape),
        };
        read(Value::Object(map))
    }
}

/// The ids met so far, the page's to the new ones.
#[derive(Default)]
struct Ids {
    seen: HashMap<u64, u64>,
    last: u64,
}

impl Ids {
    /// The next id, for an object the page numbered `page`.
    fn enter(&mut self, page: u64) -> Result<u64, Unreadable> {
        self.last += 1;
        if page != 0 && self.seen.insert(page, self.last).is_some() {
            return Err(Unreadable::Shape);
        }
        Ok(self.last)
    }
}

fn read_ref(json: Value) -> Result<PageValue, Unreadable> {
    let [id] = record(json, ["ref"])?;
    Ok(PageValue::Ref(counter(&id)?))
}

fn read_special(json: Value) -> Result<PageValue, Unreadable> {
    let [special] = record(json, ["v"])?;
    let special = string(special)?;
    Special::ALL
        .iter()
        .copied()
        .find(|known| known.tag() == special)
        .map(PageValue::Special)
        .ok_or(Unreadable::Shape)
}

fn read_date(json: Value) -> Result<PageValue, Unreadable> {
    let [iso] = record(json, ["d"])?;
    Ok(PageValue::Date(match iso {
        Value::Null => None,
        iso => Some(string(iso)?),
    }))
}

fn read_url(json: Value) -> Result<PageValue, Unreadable> {
    let [href] = record(json, ["u"])?;
    Ok(PageValue::Url(string(href)?))
}

/// `BigInt.prototype.toString()`'s digits only, so the prelude's `BigInt()` reads them back.
fn read_bigint(json: Value) -> Result<PageValue, Unreadable> {
    let [digits] = record(json, ["bi"])?;
    let digits = string(digits)?;
    let unsigned = digits.strip_prefix('-').unwrap_or(&digits);
    if unsigned.is_empty() || !unsigned.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Unreadable::Shape);
    }
    Ok(PageValue::BigInt(digits))
}

fn read_error(json: Value) -> Result<PageValue, Unreadable> {
    let [error] = record(json, ["e"])?;
    let [message, name, stack] = record(error, ["m", "n", "s"])?;
    Ok(PageValue::Error {
        name: string(name)?,
        message: string(message)?,
        stack: string(stack)?,
    })
}

fn read_regexp(json: Value) -> Result<PageValue, Unreadable> {
    let [regexp] = record(json, ["r"])?;
    let [flags, source] = record(regexp, ["f", "p"])?;
    Ok(PageValue::RegExp {
        source: string(source)?,
        flags: string(flags)?,
    })
}

fn read_typed_array(json: Value) -> Result<PageValue, Unreadable> {
    let [typed] = record(json, ["ta"])?;
    let [base64, kind] = record(typed, ["b", "k"])?;
    let base64 = string(base64)?;
    let kind = string(kind)?;
    let kind = TypedKind::ALL
        .iter()
        .copied()
        .find(|known| known.tag() == kind)
        .ok_or(Unreadable::Shape)?;
    let bytes = decode_base64(&base64).ok_or(Unreadable::Shape)?;
    if !bytes.len().is_multiple_of(kind.width()) {
        return Err(Unreadable::Shape);
    }
    Ok(PageValue::TypedArray { base64, kind })
}

fn read_function(json: Value) -> Result<PageValue, Unreadable> {
    let [name] = record(json, ["fn"])?;
    Ok(PageValue::Function(string(name)?))
}

/// `{ab}` as the agent receives it: `{}` (see [`PageValue::parse`]).
fn read_array_buffer(json: Value) -> Result<PageValue, Unreadable> {
    let [buffer] = record(json, ["ab"])?;
    let [base64] = record(buffer, ["b"])?;
    decode_base64(&string(base64)?).ok_or(Unreadable::Shape)?;
    Ok(PageValue::Object {
        fields: Vec::new(),
        id: 0,
    })
}

fn read_array(json: Value) -> Result<PageValue, Unreadable> {
    let [items, id] = record(json, ["a", "id"])?;
    let Value::Array(items) = items else {
        return Err(Unreadable::Shape);
    };
    Ok(PageValue::Array {
        items: items
            .into_iter()
            .map(PageValue::item)
            .collect::<Result<_, _>>()?,
        id: counter(&id)?,
    })
}

fn read_object(json: Value) -> Result<PageValue, Unreadable> {
    let [id, members] = record(json, ["id", "o"])?;
    let Value::Array(members) = members else {
        return Err(Unreadable::Shape);
    };
    Ok(PageValue::Object {
        fields: members.into_iter().map(member).collect::<Result<_, _>>()?,
        id: counter(&id)?,
    })
}

/// One `{k, v}` of an object, `v` absent where the serializer dropped the member's value.
fn member(json: Value) -> Result<(String, PageValue), Unreadable> {
    let Value::Object(mut fields) = json else {
        return Err(Unreadable::Shape);
    };
    let key = string(fields.remove("k").ok_or(Unreadable::Shape)?)?;
    let value = match fields.remove("v") {
        None => PageValue::Dropped,
        Some(value) => PageValue::value(value)?,
    };
    if !fields.is_empty() {
        return Err(Unreadable::Shape);
    }
    Ok((key, value))
}

/// The values of `json`, an object with exactly the keys `names`, in that order.
fn record<const N: usize>(json: Value, names: [&str; N]) -> Result<[Value; N], Unreadable> {
    let Value::Object(mut fields) = json else {
        return Err(Unreadable::Shape);
    };
    if fields.len() != N {
        return Err(Unreadable::Shape);
    }
    let values: Vec<Value> = names
        .iter()
        .map(|name| fields.remove(*name).ok_or(Unreadable::Shape))
        .collect::<Result<_, _>>()?;
    values.try_into().map_err(|_short| Unreadable::Shape)
}

fn string(json: Value) -> Result<String, Unreadable> {
    match json {
        Value::String(text) => Ok(text),
        _ => Err(Unreadable::Shape),
    }
}

/// An id or a ref: the serializer's counter, a whole number from 0.
fn counter(json: &Value) -> Result<u64, Unreadable> {
    json.as_u64().ok_or(Unreadable::Shape)
}

/// The value as JSON in the format, the inverse of the parse.
fn written(value: PageValue) -> Value {
    match value {
        PageValue::Str(text) => Value::String(text),
        PageValue::Num(number) => Value::Number(number),
        PageValue::Bool(flag) => Value::Bool(flag),
        PageValue::Special(special) => json!({ "v": special.tag() }),
        PageValue::Date(iso) => json!({ "d": iso }),
        PageValue::Url(href) => json!({ "u": href }),
        PageValue::BigInt(digits) => json!({ "bi": digits }),
        PageValue::RegExp { source, flags } => json!({ "r": { "p": source, "f": flags } }),
        PageValue::Error {
            name,
            message,
            stack,
        } => json!({ "e": { "n": name, "m": message, "s": stack } }),
        PageValue::TypedArray { base64, kind } => {
            json!({ "ta": { "b": base64, "k": kind.tag() } })
        }
        PageValue::Array { items, id } => {
            json!({ "a": items.into_iter().map(written).collect::<Vec<_>>(), "id": id })
        }
        PageValue::Object { fields, id } => {
            let members: Vec<Value> = fields
                .into_iter()
                .map(|(key, value)| match value {
                    PageValue::Dropped => json!({ "k": key }),
                    value => json!({ "k": key, "v": written(value) }),
                })
                .collect();
            json!({ "o": members, "id": id })
        }
        PageValue::Ref(id) => json!({ "ref": id }),
        PageValue::Function(name) => json!({ "fn": name }),
        // An array's item; an object's member leaves its `v` out above.
        PageValue::Dropped => Value::Null,
    }
}

/// `Number.prototype.toString()`: the shortest round-trip digits, in exponent form only from
/// 1e21 up and below 1e-6, with an explicit `+` on a positive exponent, and JavaScript's names
/// for the values that have no digits.
pub(crate) fn js_number(number: f64) -> String {
    let magnitude = number.abs();
    if number.is_nan() {
        "NaN".to_owned()
    } else if number.is_infinite() {
        if number > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        }
        .to_owned()
    } else if number == 0.0 {
        "0".to_owned()
    } else if magnitude >= 1e21 {
        format!("{number:e}").replacen('e', "e+", 1)
    } else if magnitude < 1e-6 {
        format!("{number:e}")
    } else {
        format!("{number}")
    }
}

/// The deepest `[`/`{` nesting in `text` outside its strings, in one pass over the bytes. A
/// multi-byte character never holds a quote, a backslash or a bracket byte.
pub(crate) fn depth_of(text: &str) -> usize {
    let (mut depth, mut deepest) = (0_usize, 0_usize);
    let (mut in_string, mut escaped) = (false, false);
    for byte in text.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

/// Standard base64 with padding, as `btoa` and `Uint8Array.prototype.toBase64` write it.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let digits = text.trim_end_matches('=');
    if !text.len().is_multiple_of(4) || text.len() - digits.len() > 2 {
        return None;
    }
    let mut bytes = Vec::with_capacity(digits.len() * 3 / 4);
    let (mut bits, mut held) = (0_u32, 0_u32);
    for digit in digits.bytes() {
        let sextet = match digit {
            b'A'..=b'Z' => digit - b'A',
            b'a'..=b'z' => digit - b'a' + 26,
            b'0'..=b'9' => digit - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(sextet);
        held += 6;
        if held >= 8 {
            held -= 8;
            bytes.push(u8::try_from((bits >> held) & 0xff).unwrap_or_default());
            bits &= (1 << held) - 1;
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests;
