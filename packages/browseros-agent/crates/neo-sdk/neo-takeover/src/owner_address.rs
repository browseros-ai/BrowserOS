//! Resolves the owner's browser-bar text into a web address or a search, without browser I/O.
//!
//! Text that names a scheme or carries credentials is never searched, so a password typed as
//! `alice:secret@bank.example` cannot reach a search engine's logs: it opens only as an http or
//! https address without credentials, and is refused otherwise.

use crate::limits::{MAX_ADDRESS_LEN, SEARCH_URL};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

/// What the owner's typed text resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Address {
    /// Open this URL: a web address as typed (`https://` added when it names no scheme), or a
    /// search for any other text.
    Open(String),
    /// A non-http(s) scheme, credentials, an address that does not parse, or nothing.
    Refused,
}

/// Go's `url.QueryEscape`: every byte but `A-Z a-z 0-9 - _ . ~` escaped, then spaces as `+`.
const QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

pub(crate) fn owner_address(typed: &str) -> Address {
    let typed = typed.trim();
    let scheme = typed.split(':').next().unwrap_or_default();
    let host = typed.split('/').next().unwrap_or_default();
    // A scheme is a word and a colon with no space after it. A host with a port
    // (`example.com:8443`) starts the same way, but its word is dotted.
    let address = if (names_scheme(typed) && !scheme.contains('.')) || signed_in(typed) {
        // As a browser does, spaces are escaped; anything else that will not parse is refused.
        typed.replace(' ', "%20")
    } else if typed.is_empty() {
        return Address::Refused;
    } else if typed.contains(char::is_whitespace) || !host.contains('.') {
        let query = utf8_percent_encode(typed, QUERY)
            .to_string()
            .replace("%20", "+");
        return Address::Open(format!("{SEARCH_URL}{query}"));
    } else {
        format!("https://{typed}")
    };
    if is_web_address(&address) {
        Address::Open(address)
    } else {
        Address::Refused
    }
}

/// `^[A-Za-z][A-Za-z0-9+.-]*:\S`: a scheme, a colon, then something that is not a space.
fn names_scheme(typed: &str) -> bool {
    let mut chars = typed.chars();
    if !chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
    {
        return false;
    }
    for symbol in chars.by_ref() {
        match symbol {
            ':' => return chars.next().is_some_and(|next| !next.is_whitespace()),
            'a'..='z' | 'A'..='Z' | '0'..='9' | '+' | '.' | '-' => {}
            _ => return false,
        }
    }
    false
}

/// `^[^\s/@]+@`: a first word that carries credentials (`alice:pw@bank.example`, `bob@x.com`).
fn signed_in(typed: &str) -> bool {
    typed
        .find(|symbol: char| symbol.is_whitespace() || symbol == '/' || symbol == '@')
        .is_some_and(|at| at > 0 && typed[at..].starts_with('@'))
}

/// Accepts `about:blank`, or an http(s) URL with a host and no credentials, at most
/// 2000 bytes. A `%` that does not start an escape is refused as Go's parser refuses it; the
/// WHATWG parser would keep it.
fn is_web_address(address: &str) -> bool {
    if address == "about:blank" {
        return true;
    }
    let escapes_valid = address.split('%').skip(1).all(|rest| {
        rest.as_bytes()
            .get(..2)
            .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
    });
    address.len() <= MAX_ADDRESS_LEN
        && escapes_valid
        && Url::parse(address).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some_and(|host| !host.is_empty())
                && url.username().is_empty()
                && url.password().is_none()
        })
}

#[cfg(test)]
mod tests {
    use super::{Address, owner_address};

    /// Addresses, searches and malformed input all preserve the no-credentials rule.
    #[test]
    fn typed_text_opens_as_an_address_or_a_search_and_never_searches_credentials() {
        for (typed, want) in [
            (
                "https://www.doordash.com/home",
                "https://www.doordash.com/home",
            ),
            (
                "  doordash.com/store?id=4 ",
                "https://doordash.com/store?id=4",
            ),
            (
                "pizza near me",
                "https://www.google.com/search?q=pizza+near+me",
            ),
            (
                "what is 2.5 kg in lbs",
                "https://www.google.com/search?q=what+is+2.5+kg+in+lbs",
            ),
            ("http://example.com/a b", "http://example.com/a%20b"),
            ("example.com:8443/x", "https://example.com:8443/x"),
            ("example.org", "https://example.org"),
            ("localhost", "https://www.google.com/search?q=localhost"),
            (
                "c++ & rust~*",
                "https://www.google.com/search?q=c%2B%2B+%26+rust~%2A",
            ),
            ("about:blank", "about:blank"),
            ("javascript:alert(1)", ""),
            ("file:///etc/passwd", ""),
            ("https://user:pw@bank.example", ""),
            // Credentials never leak into a search, even in an address with a space or one
            // that will not parse.
            ("https://alice:secret@bank.example/a b", ""),
            ("alice:secret@bank.example/a b", ""),
            ("alice.smith:secret@bank.example/a b", ""),
            ("bob@example.com", ""),
            ("https://bank.example/%zz", ""),
            ("   ", ""),
        ] {
            let got = match owner_address(typed) {
                Address::Open(url) => url,
                Address::Refused => String::new(),
            };
            assert_eq!(got, want, "{typed:?}");
        }
        let long = format!("https://example.com/{}", "a".repeat(2000));
        assert_eq!(owner_address(&long), Address::Refused, "over 2000 bytes");
    }
}
