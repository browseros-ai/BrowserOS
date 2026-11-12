//! Web globals for the agent's realm: URL parsing and form encoding use Rust's `url` crate;
//! the JS factory owns `WebIDL` conversion, live search parameters and the base64 arity checks.
//! These pure string natives carry no bridge, browser or host authority and never become globals.
//! Abort signals dispatch in JavaScript; their timeout factory reuses the run-bounded clock.

use std::io;

use rquickjs::context::EvalOptions;
use rquickjs::function::Func;
use rquickjs::{Ctx, Error, Function, Object, Value};
use serde::{Serialize, Serializer};
use url::{Url, form_urlencoded, quirks};

/// Installs once per fresh realm, before the bootstrap and every prelude. The factory captures
/// its natives so only the standard interface objects remain on the global object.
pub(super) fn install_web_globals(ctx: &Ctx<'_>, memory_bytes: usize) -> rquickjs::Result<()> {
    let budget = NativeBudget(memory_bytes);
    let natives = Object::new(ctx.clone())?;
    natives.set(
        "parse",
        Func::from(move |input: String, base: Option<String>| {
            parse(&input, base.as_deref(), budget)
        }),
    )?;
    natives.set(
        "set",
        Func::from(move |href: String, setter: String, value: String| {
            set(&href, &setter, &value, budget)
        }),
    )?;
    natives.set(
        "formParse",
        Func::from(move |query: String| form_parse(&query, budget)),
    )?;
    natives.set(
        "formSerialize",
        Func::from(move |name: String, value: String| form_serialize(&name, &value, budget)),
    )?;
    // Own its evaluation settings as well as its natives: component-test realms include this
    // same installer without depending on the server's bridge or orchestration module.
    let mut options = EvalOptions::default();
    options.global = true;
    options.strict = true;
    options.filename = Some("web_globals.js".to_owned());
    let factory: Function<'_> =
        ctx.eval_with_options(include_str!("js/web_globals.js"), options)?;
    factory.call::<_, ()>((natives,))?;
    // Cancellation is a realm interface, rather than an agent function parameter, so
    // standard scripts and libraries can construct signals before calling an SDK wait.
    let factory: Function<'_> = ctx.eval(include_str!("js/abort_globals.js"))?;
    let sleep: Value<'_> = ctx.globals().get("__neo_sleep")?;
    factory.call((sleep,))
}

/// The array order is shared with `web_globals.js`'s HREF..HASH constants. `quirks` implements
/// the DOM interface's getters, including empty delimiters and opaque origins.
fn components(url: &Url, budget: NativeBudget) -> rquickjs::Result<String> {
    // Budget the serialized URL and origin while JSON is written. Opaque "null" can be
    // longer than a very short href; a tuple origin is at most the href's byte length.
    let href_bytes = url.as_str().len();
    let output = budget.remaining(href_bytes)?.remaining(href_bytes.max(4))?;
    json(
        &[
            quirks::href(url),
            &quirks::origin(url),
            quirks::protocol(url),
            quirks::username(url),
            quirks::password(url),
            quirks::host(url),
            quirks::hostname(url),
            quirks::port(url),
            quirks::pathname(url),
            quirks::search(url),
            quirks::hash(url),
        ],
        output,
    )
}

fn parse(
    input: &str,
    base: Option<&str>,
    budget: NativeBudget,
) -> rquickjs::Result<Option<String>> {
    let input_bytes = input
        .len()
        .checked_add(base.map_or(0, str::len))
        .ok_or(Error::Allocation)?;
    let budget = budget.for_url(input_bytes)?;
    // Even an absolute input must reject an invalid explicitly supplied base, per API parse.
    let Ok(base) = base.map(Url::parse).transpose() else {
        return Ok(None);
    };
    let parsed = Url::options().base_url(base.as_ref()).parse(input);
    drop(base);
    match parsed {
        Ok(url) => components(&url, budget).map(Some),
        Err(_) => Ok(None),
    }
}

/// Translate the JS protocol's setter name once; subsequent dispatch is exhaustive.
enum UrlSetter {
    Href,
    Protocol,
    Username,
    Password,
    Host,
    Hostname,
    Port,
    Pathname,
    Search,
    Hash,
}

impl UrlSetter {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "href" => Some(Self::Href),
            "protocol" => Some(Self::Protocol),
            "username" => Some(Self::Username),
            "password" => Some(Self::Password),
            "host" => Some(Self::Host),
            "hostname" => Some(Self::Hostname),
            "port" => Some(Self::Port),
            "pathname" => Some(Self::Pathname),
            "search" => Some(Self::Search),
            "hash" => Some(Self::Hash),
            _ => None,
        }
    }

    fn apply(self, url: &mut Url, value: &str) -> Option<()> {
        // DOM setters ignore invalid values, except href, whose failure JS turns into TypeError.
        match self {
            Self::Href => quirks::set_href(url, value).ok()?,
            Self::Protocol => {
                let _ = quirks::set_protocol(url, value);
            }
            Self::Username => {
                let _ = quirks::set_username(url, value);
            }
            Self::Password => {
                let _ = quirks::set_password(url, value);
            }
            Self::Host => {
                let _ = quirks::set_host(url, value);
            }
            Self::Hostname => {
                let _ = quirks::set_hostname(url, value);
            }
            Self::Port => {
                let _ = quirks::set_port(url, value);
            }
            Self::Pathname => quirks::set_pathname(url, value),
            Self::Search => quirks::set_search(url, value),
            Self::Hash => quirks::set_hash(url, value),
        }
        Some(())
    }
}

fn set(
    href: &str,
    setter: &str,
    value: &str,
    budget: NativeBudget,
) -> rquickjs::Result<Option<String>> {
    let input_bytes = href
        .len()
        .checked_add(value.len())
        .and_then(|bytes| bytes.checked_add(setter.len()))
        .ok_or(Error::Allocation)?;
    let budget = budget.for_url(input_bytes)?;
    let (Ok(mut url), Some(setter)) = (Url::parse(href), UrlSetter::parse(setter)) else {
        return Ok(None);
    };
    if setter.apply(&mut url, value).is_none() {
        return Ok(None);
    }
    components(&url, budget).map(Some)
}

fn form_parse(query: &str, budget: NativeBudget) -> rquickjs::Result<String> {
    // The Rust UTF-8 argument already exists outside the JS heap. Scan the borrowed input
    // before decoding anything: empty fields are skipped, and only a pair with '+' or '%'
    // can need copies for plus replacement, percent decoding and lossy UTF-8 repair.
    let output = budget.remaining(query.len())?;
    let mut pairs = 0_usize;
    let mut largest_encoded_pair = 0;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        pairs += 1;
        if pair
            .as_bytes()
            .iter()
            .any(|byte| matches!(byte, b'+' | b'%'))
        {
            largest_encoded_pair = largest_encoded_pair.max(pair.len());
        }
    }
    // The decoder keeps only one pair: two input-sized byte buffers and up to twice the
    // input size for the UTF-8 repair buffer's growth. Plain UTF-8 fields are borrowed.
    let decode = largest_encoded_pair
        .checked_mul(4)
        .ok_or(Error::Allocation)?;
    let output = output.remaining(decode)?;
    // Even empty name/value pairs emit ["",""]; refuse pair-dense JSON before growing it.
    let minimum = if pairs == 0 {
        2
    } else {
        pairs
            .checked_mul(8)
            .and_then(|bytes| bytes.checked_add(1))
            .ok_or(Error::Allocation)?
    };
    output.remaining(minimum)?;
    json(&FormPairs(query), output)
}

/// Serializes one decoded pair at a time; serde never sees a pair-count-sized Rust collection.
struct FormPairs<'a>(&'a str);

impl Serialize for FormPairs<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(form_urlencoded::parse(self.0.as_bytes()))
    }
}

fn form_serialize(name: &str, value: &str, budget: NativeBudget) -> rquickjs::Result<String> {
    let input_bytes = name
        .len()
        .checked_add(value.len())
        .ok_or(Error::Allocation)?;
    let mut output = NativeBuffer::new(budget.remaining(input_bytes)?);
    // Only primitive strings cross for one pair. The JS heap owns the list and the final join;
    // neither a JSON decoder nor user-defined toJSON hooks can touch this internal format.
    for part in form_urlencoded::byte_serialize(name.as_bytes()) {
        output.append(part)?;
    }
    output.append("=")?;
    for part in form_urlencoded::byte_serialize(value.as_bytes()) {
        output.append(part)?;
    }
    output.finish()
}

/// Accounts native string work against the run's existing allowance, outside the JS heap.
/// rquickjs emits QuickJS's OOM exception for `Error::Allocation`; the run then reports
/// the same `script/out_of_memory` outcome as exhausting the JS heap.
#[derive(Clone, Copy)]
struct NativeBudget(usize);

impl NativeBudget {
    fn remaining(self, bytes: usize) -> rquickjs::Result<Self> {
        self.0.checked_sub(bytes).map(Self).ok_or(Error::Allocation)
    }

    fn for_url(self, input_bytes: usize) -> rquickjs::Result<Self> {
        // Percent encoding can triple bytes. Keep room for both a base and the parsed/updated
        // URL during normalization, plus the incoming string copies, before invoking url.
        self.remaining(input_bytes.checked_mul(7).ok_or(Error::Allocation)?)?;
        self.remaining(input_bytes)
    }
}

/// A growable native result whose requested capacity never exceeds its remaining allowance.
/// Both JSON and form encoding stream here; allocation/limit failures become QuickJS OOM.
struct NativeBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl NativeBuffer {
    fn new(budget: NativeBudget) -> Self {
        Self {
            bytes: Vec::new(),
            limit: budget.0,
        }
    }

    fn append(&mut self, text: &str) -> rquickjs::Result<()> {
        use std::io::Write;
        if self.write_all(text.as_bytes()).is_err() {
            return Err(Error::Allocation);
        }
        Ok(())
    }

    fn finish(self) -> rquickjs::Result<String> {
        String::from_utf8(self.bytes)
            .map_err(|error| Error::new_into_js_message("web global", "UTF-8", error.to_string()))
    }
}

impl io::Write for NativeBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let required = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|length| *length <= self.limit)
            .ok_or(io::ErrorKind::OutOfMemory)?;
        if required > self.bytes.capacity() {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(256)
                .max(required)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|error| io::Error::new(io::ErrorKind::OutOfMemory, error))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json(value: &impl Serialize, budget: NativeBudget) -> rquickjs::Result<String> {
    let mut output = NativeBuffer::new(budget);
    serde_json::to_writer(&mut output, value).map_err(|error| {
        // This writer performs no I/O: its only I/O-shaped failures are allocation limits.
        if error.is_io() {
            Error::Allocation
        } else {
            Error::new_into_js_message("web global", "JSON", error.to_string())
        }
    })?;
    output.finish()
}

#[cfg(test)]
mod tests {
    use super::{Error, NativeBudget, form_parse, form_serialize, parse};

    #[test]
    fn dense_pairs_are_rejected_with_a_bounded_native_allowance() -> rquickjs::Result<()> {
        let dense = "=&".repeat(512);
        let one_pair = "x".repeat(dense.len());
        assert!(
            matches!(
                form_parse(&dense, NativeBudget(4096)),
                Err(Error::Allocation)
            ),
            "pair count, not just input length, determines whether the result can fit"
        );
        // A same-sized ordinary query fits: amplification, not input length alone, is bounded.
        form_parse(&one_pair, NativeBudget(4096))?;
        assert!(
            matches!(
                form_serialize(" ", " ", NativeBudget(4)),
                Err(Error::Allocation)
            ),
            "form encoding also bounds its result while it expands input"
        );
        assert!(
            matches!(
                form_parse(&"\0".repeat(1024), NativeBudget(4096)),
                Err(Error::Allocation)
            ),
            "JSON escaping is streamed into the bounded writer after the minimum-size check"
        );
        assert!(
            matches!(
                parse(
                    &format!("https://h/{}", "x".repeat(768)),
                    None,
                    NativeBudget(4096)
                ),
                Err(Error::Allocation)
            ),
            "URL normalization also rejects oversized work before library allocation"
        );
        Ok(())
    }
}
