//! Scrubbing for everything that leaves this computer in a skill draft digest. The digest is built
//! from task history the agent already recorded, but page titles, URLs and step text can still
//! carry an email, a phone number or an order id, so every kept string passes through here first.

use std::sync::LazyLock;

use percent_encoding::percent_decode_str;
use regex::Regex;
use url::Url;

/// What a removed value reads as in the digest, so the model knows something was there.
pub const REMOVED: &str = "[removed]";

static EMAIL: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}").ok());
/// IBAN-like account numbers: two letters, two digits, then 10 to 30 letters and digits.
static IBAN: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\b[A-Z]{2}\d{2}(?:[ ]?[A-Z0-9]){10,30}\b").ok());
/// Runs of seven or more digits, with the spaces, dots and dashes phone and card numbers use.
static LONG_NUMBER: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\+?\d(?:[ ().-]?\d){6,}").ok());
/// Tokens, ids and keys: 20 or more letters and digits that mix both.
static TOKEN: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9_-]{20,}\b").ok());

/// Removes emails, phone and card numbers, account numbers and long tokens, collapses whitespace
/// and cuts the result to `max` characters. Fails closed: if a pattern is unavailable the text is
/// dropped rather than sent unscrubbed.
#[must_use]
pub fn scrub_text(raw: &str, max: usize) -> String {
    let (Some(email), Some(iban), Some(long_number), Some(token)) = (
        EMAIL.as_ref(),
        IBAN.as_ref(),
        LONG_NUMBER.as_ref(),
        TOKEN.as_ref(),
    ) else {
        return String::new();
    };
    let text = email.replace_all(raw, REMOVED);
    let text = iban.replace_all(&text, REMOVED);
    let text = long_number.replace_all(&text, REMOVED);
    let text = token.replace_all(&text, |found: &regex::Captures| {
        let token = &found[0];
        let digits = token.bytes().any(|byte| byte.is_ascii_digit());
        let letters = token.bytes().any(|byte| byte.is_ascii_alphabetic());
        if digits && letters {
            REMOVED.to_string()
        } else {
            token.to_string()
        }
    });
    cut(&text.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

/// The first `max` characters of `text`, trimmed.
#[must_use]
pub fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.trim().to_string();
    }
    text.chars()
        .take(max)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// A page's host (lowercase, no `www.`) and its path with the query and fragment dropped and any
/// segment that looks like an id replaced by `{id}`.
#[must_use]
pub fn host_and_path(raw: &str) -> Option<(String, String)> {
    let url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = bare_host(url.host_str()?)?;
    let segments = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .map(|segment| {
                    // Scrub the decoded text: an email arrives here as `jane%40example.com`.
                    let segment = percent_decode_str(segment).decode_utf8_lossy();
                    if is_id_like(&segment) {
                        "{id}".to_string()
                    } else {
                        scrub_text(&segment, 60)
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let path = format!("/{}", segments.join("/"));
    Some((host, cut(&path, 300)))
}

/// A host as the worker accepts it: lowercase letters, digits, dashes and dots, without `www.`.
#[must_use]
pub fn bare_host(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let valid = host.contains('.')
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        });
    valid.then_some(host)
}

/// The search term a site put in its own URL (`?q=`, `?query=`, ...), scrubbed. This is the one
/// typed value the digest carries: it is what a person changes between runs, and the site already
/// made it part of the page address.
#[must_use]
pub fn search_term(raw: &str) -> Option<String> {
    const KEYS: [&str; 7] = ["q", "query", "search", "k", "keyword", "keywords", "term"];
    let url = Url::parse(raw).ok()?;
    url.query_pairs()
        .find(|(key, value)| KEYS.contains(&key.as_ref()) && !value.trim().is_empty())
        .map(|(_, value)| scrub_text(&value, 200))
        .filter(|value| !value.is_empty() && value != REMOVED)
}

fn is_id_like(segment: &str) -> bool {
    let digits = segment.bytes().filter(u8::is_ascii_digit).count();
    digits >= 4
        || (segment.len() >= 16 && segment.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || (segment.len() >= 20 && digits > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_text_removes_personal_values_and_keeps_words() {
        let raw = "Sent the order 4111 1111 1111 1111 to jane.doe@example.com, call +91 98765 43210 \
                   or ref GB82WEST12345698765432 token a1b2c3d4e5f6g7h8i9j0k1";
        let clean = scrub_text(raw, 300);
        assert!(!clean.contains("4111"), "{clean}");
        assert!(!clean.contains('@'), "{clean}");
        assert!(!clean.contains("98765"), "{clean}");
        assert!(!clean.contains("GB82"), "{clean}");
        assert!(!clean.contains("a1b2c3"), "{clean}");
        assert!(clean.starts_with("Sent the order"), "{clean}");
    }

    #[test]
    fn scrub_text_keeps_prices_and_long_plain_words() {
        assert_eq!(
            scrub_text("Set the price filter to 30000 on internationalization", 100),
            "Set the price filter to 30000 on internationalization"
        );
    }

    #[test]
    fn host_and_path_drops_query_fragment_and_ids() {
        assert_eq!(
            host_and_path("https://www.Flipkart.com/apple-iphone/p/itm1234567?pid=MOB9&q=x#top"),
            Some((
                "flipkart.com".to_string(),
                "/apple-iphone/p/{id}".to_string()
            ))
        );
        assert!(host_and_path("chrome://newtab").is_none());
    }

    #[test]
    fn host_and_path_scrubs_encoded_personal_values() {
        let (_, path) = host_and_path(
            "https://example.com/users/jane%40example.com/orders/jane%2Bdoe%40mail.co",
        )
        .unwrap_or_default();
        assert!(!path.contains("jane"), "{path}");
        assert!(!path.contains("%40"), "{path}");
        assert!(path.starts_with("/users/[removed]/orders/"), "{path}");
    }

    #[test]
    fn search_term_reads_only_search_keys() {
        assert_eq!(
            search_term("https://www.flipkart.com/search?q=phones+under+30000").as_deref(),
            Some("phones under 30000")
        );
        assert_eq!(
            search_term("https://example.com/login?email=a%40b.co"),
            None
        );
        assert_eq!(
            search_term("https://example.com/s?q=jane%40example.com"),
            None
        );
    }

    #[test]
    fn bare_host_matches_the_worker_rule() {
        assert_eq!(bare_host("WWW.Amazon.in").as_deref(), Some("amazon.in"));
        assert_eq!(bare_host("localhost"), None);
        assert_eq!(bare_host("bad_host.com"), None);
    }
}
