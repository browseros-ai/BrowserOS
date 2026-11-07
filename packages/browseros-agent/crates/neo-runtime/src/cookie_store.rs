//! Playwright: `server/cookieStore.ts` `CookieStore`
//! Playwright: `server/cookieStore.ts` `parseRawCookie`
//! Playwright: `server/fetch.ts` `APIRequestContext._parseSetCookieHeader`
//!
//! The cookies of API requests (`api_request.rs`): what a response's `Set-Cookie` sets, and the
//! jar of a `playwright.request.newContext()` context, which keeps its own (a browser context's
//! `request` uses the browser's cookies instead). Which cookies a URL is sent is
//! `session_state.rs`'s `matches_url`, the port of Playwright's `filterCookies` both use.

use neo_runtime_spec::serve::{NetworkCookie, NetworkCookieSameSite};
use url::Url;

use crate::session_state::{MAX_COOKIE_EXPIRES_SECONDS, domain_matches};

/// Playwright: `server/cookieStore.ts` `RawCookie`
/// What one `Set-Cookie` header says.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RawCookie {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) domain: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) expires: Option<f64>,
    pub(crate) http_only: bool,
    pub(crate) secure: bool,
    pub(crate) same_site: Option<NetworkCookieSameSite>,
}

/// Playwright: `server/cookieStore.ts` `parseCookieNameValue`
pub(crate) fn parse_cookie_name_value(pair: &str) -> (String, String) {
    match pair.split_once('=') {
        None => (String::new(), pair.trim().to_string()),
        Some((name, value)) => (name.trim().to_string(), value.trim().to_string()),
    }
}

/// Playwright: `server/cookieStore.ts` `serializeCookieNameValue`: `${cookie.name}=${cookie.value}`
pub(crate) fn serialize_cookie_name_value(name: &str, value: &str) -> String {
    if name.is_empty() {
        value.to_string()
    } else {
        format!("{name}={value}")
    }
}

/// Playwright: `server/cookieStore.ts` `parseRawCookie`: `const [nameValue, ...attributes] = header.split(';');`
/// `now` is the current time in seconds since the epoch, for `Max-Age`.
pub(crate) fn parse_raw_cookie(header: &str, now: f64) -> Option<RawCookie> {
    let mut parts = header.split(';');
    let (name, value) = parse_cookie_name_value(parts.next().unwrap_or_default());
    if name.is_empty() && value.is_empty() {
        return None;
    }
    let mut cookie = RawCookie {
        name,
        value,
        ..RawCookie::default()
    };
    let mut max_age_expires = None;
    for part in parts.filter(|part| !part.trim().is_empty()) {
        let (key, value) = match part.split_once('=') {
            None => (part.trim(), ""),
            Some((key, value)) => (key.trim(), value.trim()),
        };
        match key.to_lowercase().as_str() {
            "expires" => {
                if let Some(expires_ms) = parse_expires(value) {
                    cookie.expires = Some(if expires_ms <= 0.0 {
                        0.0
                    } else {
                        (expires_ms / 1000.0).min(MAX_COOKIE_EXPIRES_SECONDS)
                    });
                }
            }
            // Playwright: `server/cookieStore.ts` `parseRawCookie`: `if (/^-?\d+$/.test(value)) {`
            "max-age" => {
                let digits = value.strip_prefix('-').unwrap_or(value);
                if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
                    let seconds = value.parse::<f64>().unwrap_or(f64::MAX);
                    max_age_expires = Some(if seconds <= 0.0 {
                        0.0
                    } else {
                        (now + seconds).min(MAX_COOKIE_EXPIRES_SECONDS)
                    });
                }
            }
            "domain" => {
                let mut domain = value.to_lowercase();
                if !domain.is_empty() && !domain.starts_with('.') && domain.contains('.') {
                    domain.insert(0, '.');
                }
                cookie.domain = Some(domain);
            }
            "path" => cookie.path = Some(value.to_string()),
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            "samesite" => match value.to_lowercase().as_str() {
                "none" => cookie.same_site = Some(NetworkCookieSameSite::None),
                "lax" => cookie.same_site = Some(NetworkCookieSameSite::Lax),
                "strict" => cookie.same_site = Some(NetworkCookieSameSite::Strict),
                _ => {}
            },
            _ => {}
        }
    }
    // Playwright: `server/cookieStore.ts` `parseRawCookie`: `// Max-Age takes precedence over Expires, regardless of the order of attributes.`
    if max_age_expires.is_some() {
        cookie.expires = max_age_expires;
    }
    Some(cookie)
}

/// Playwright: `server/fetch.ts` `APIRequestContext._parseSetCookieHeader`
/// The cookies one
/// response's `Set-Cookie` headers set for `response_url`, each defaulted as Playwright's
/// `parseCookie` does (`sameSite: 'Lax'`). A cookie for a domain the URL does not match is
/// dropped.
pub(crate) fn parse_set_cookie_headers(
    response_url: &Url,
    headers: &[&str],
    now: f64,
) -> Vec<NetworkCookie> {
    let hostname = response_url.host_str().unwrap_or_default();
    let default_path = default_path(response_url.path());
    let mut cookies = Vec::new();
    for header in headers {
        let Some(raw) = parse_raw_cookie(header, now) else {
            continue;
        };
        // Playwright: `server/fetch.ts` `APIRequestContext._parseSetCookieHeader`: `cookie.domain = url.hostname;`
        let domain = match raw.domain {
            Some(domain) if !domain.is_empty() => domain,
            _ => hostname.to_string(),
        };
        if !domain_matches(hostname, &domain) {
            continue;
        }
        let path = match raw.path {
            Some(path) if path.starts_with('/') => path,
            _ => default_path.clone(),
        };
        cookies.push(NetworkCookie {
            name: raw.name,
            value: raw.value,
            domain,
            path,
            expires: raw.expires.unwrap_or(-1.0),
            http_only: raw.http_only,
            secure: raw.secure,
            same_site: raw.same_site.unwrap_or(NetworkCookieSameSite::Lax),
            partition_key: None,
            cr_has_cross_site_ancestor: None,
        });
    }
    cookies
}

/// Playwright: `server/fetch.ts` `APIRequestContext._parseSetCookieHeader`: `const defaultPath = '/' + url.pathname.substr(1).split('/').slice(0, -1).join('/');`
fn default_path(pathname: &str) -> String {
    let rest = pathname.strip_prefix('/').unwrap_or(pathname);
    let parts: Vec<&str> = rest.split('/').collect();
    format!("/{}", parts[..parts.len().saturating_sub(1)].join("/"))
}

/// Playwright: `server/cookieStore.ts` `CookieStore`
/// Cookies by name, in the order the names
/// came, each name's cookies in the order they were set. A cookie replaces the one of the same
/// name, domain and path; expired cookies go whenever their name is read or written.
#[derive(Debug, Default)]
pub(crate) struct CookieStore {
    names: Vec<(String, Vec<NetworkCookie>)>,
}

/// Playwright: `server/cookieStore.ts` `CookieStore`
impl CookieStore {
    /// Playwright: `server/cookieStore.ts` `CookieStore.addCookies`
    pub(crate) fn add_cookies(&mut self, cookies: Vec<NetworkCookie>, now: f64) {
        for cookie in cookies {
            self.add_cookie(cookie, now);
        }
    }

    /// Playwright: `server/cookieStore.ts` `CookieStore.allCookies`
    pub(crate) fn all_cookies(&mut self, now: f64) -> Vec<NetworkCookie> {
        let mut all = Vec::new();
        for (_, cookies) in &mut self.names {
            prune_expired(cookies, now);
            all.extend(cookies.iter().cloned());
        }
        self.names.retain(|(_, cookies)| !cookies.is_empty());
        all
    }

    /// Playwright: `server/cookieStore.ts` `CookieStore._addCookie`
    fn add_cookie(&mut self, cookie: NetworkCookie, now: f64) {
        let index = self
            .names
            .iter()
            .position(|(name, _)| *name == cookie.name)
            .unwrap_or_else(|| {
                self.names.push((cookie.name.clone(), Vec::new()));
                self.names.len() - 1
            });
        let cookies = &mut self.names[index].1;
        // Playwright: `server/cookieStore.ts` `CookieStore._addCookie`: `// https://datatracker.ietf.org/doc/html/rfc6265#section-5.3`
        cookies.retain(|other| !(other.domain == cookie.domain && other.path == cookie.path));
        cookies.push(cookie);
        prune_expired(cookies, now);
    }
}

/// Playwright: `server/cookieStore.ts` `CookieStore.pruneExpired`
fn prune_expired(cookies: &mut Vec<NetworkCookie>, now: f64) {
    cookies.retain(|cookie| !expired(cookie, now));
}

/// Playwright: `server/cookieStore.ts` `Cookie._expired`: `return this._raw.expires * 1000 < Date.now();`
/// A session cookie's `expires` is -1, the only negative one a context accepts.
fn expired(cookie: &NetworkCookie, now: f64) -> bool {
    cookie.expires >= 0.0 && cookie.expires < now
}

/// Deviation: `server/cookieStore.ts` `parseRawCookie`: `const expiresMs = (+new Date(value));`
/// Playwright reads `Expires` with JavaScript's lenient `Date`. Neo reads it as Chromium does,
/// with RFC 6265's cookie-date algorithm (every HTTP date form, two-digit years), then as an ISO
/// 8601 date or UTC date-time; a form only `Date` accepts leaves the cookie a session cookie.
/// The time in milliseconds since the epoch.
fn parse_expires(value: &str) -> Option<f64> {
    cookie_date(value).or_else(|| iso_date(value))
}

/// Neo: RFC 6265 section 5.1.1, the cookie-date algorithm.
fn cookie_date(value: &str) -> Option<f64> {
    let delimiter = |byte: u8| {
        byte == 0x09
            || (0x20..=0x2f).contains(&byte)
            || (0x3b..=0x40).contains(&byte)
            || (0x5b..=0x60).contains(&byte)
            || (0x7b..=0x7e).contains(&byte)
    };
    let (mut time, mut day, mut month, mut year) = (None, None, None, None);
    for token in value.as_bytes().split(|byte| delimiter(*byte)) {
        if token.is_empty() {
            continue;
        }
        if time.is_none()
            && let Some(found) = time_token(token)
        {
            time = Some(found);
        } else if day.is_none()
            && let Some(found) = digits_token(token, 1, 2)
        {
            day = Some(found);
        } else if month.is_none()
            && let Some(found) = month_token(token)
        {
            month = Some(found);
        } else if year.is_none()
            && let Some(found) = digits_token(token, 2, 4)
        {
            year = Some(found);
        }
    }
    let (hour, minute, second) = time?;
    let (day, month, mut year) = (day?, month?, year?);
    if (70..=99).contains(&year) {
        year += 1900;
    } else if year <= 69 {
        year += 2000;
    }
    if !(1..=31).contains(&day) || year < 1601 || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    utc_millis(year, month, day, hour, minute, second, 0)
}

/// Neo: `1*2DIGIT ":" 1*2DIGIT ":" 1*2DIGIT ( non-digit *OCTET )`.
fn time_token(token: &[u8]) -> Option<(u32, u32, u32)> {
    let mut fields = [0u32; 3];
    let mut rest = token;
    for (index, field) in fields.iter_mut().enumerate() {
        let digits = rest.iter().take_while(|byte| byte.is_ascii_digit()).count();
        if !(1..=2).contains(&digits) {
            return None;
        }
        *field = ascii_number(&rest[..digits]);
        rest = &rest[digits..];
        if index < 2 {
            rest = rest.strip_prefix(b":")?;
        }
    }
    (rest.first().is_none_or(|byte| !byte.is_ascii_digit()))
        .then_some((fields[0], fields[1], fields[2]))
}

/// Neo: `min*maxDIGIT ( non-digit *OCTET )`.
fn digits_token(token: &[u8], min: usize, max: usize) -> Option<u32> {
    let digits = token
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    (min..=max)
        .contains(&digits)
        .then(|| ascii_number(&token[..digits]))
}

/// Neo: a month's first three letters, any case.
fn month_token(token: &[u8]) -> Option<u32> {
    /// Neo: RFC 6265's month names, by their first three letters.
    const MONTHS: [&[u8]; 12] = [
        b"jan", b"feb", b"mar", b"apr", b"may", b"jun", b"jul", b"aug", b"sep", b"oct", b"nov",
        b"dec",
    ];
    let prefix = token.get(..3)?.to_ascii_lowercase();
    let index = MONTHS
        .iter()
        .position(|month| *month == prefix.as_slice())?;
    u32::try_from(index + 1).ok()
}

/// Neo: `YYYY-MM-DD`, and `YYYY-MM-DDTHH:MM[:SS[.sss]]Z`, as JavaScript's `Date` reads them (UTC).
fn iso_date(value: &str) -> Option<f64> {
    let value = value.trim();
    let (date, time) = match value.split_once('T') {
        Some((date, time)) => (date, Some(time.strip_suffix('Z')?)),
        None => (value, None),
    };
    let mut date_fields = date.split('-');
    let year = number(date_fields.next()?, 4)?;
    let month = number(date_fields.next()?, 2)?;
    let day = number(date_fields.next()?, 2)?;
    if date_fields.next().is_some() {
        return None;
    }
    let (hour, minute, second, millis) = match time {
        None => (0, 0, 0, 0),
        Some(time) => {
            let (clock, millis) = match time.split_once('.') {
                Some((clock, fraction)) => (clock, number(fraction, 3)?),
                None => (time, 0),
            };
            let mut clock_fields = clock.split(':');
            let hour = number(clock_fields.next()?, 2)?;
            let minute = number(clock_fields.next()?, 2)?;
            let second = clock_fields
                .next()
                .map_or(Some(0), |field| number(field, 2))?;
            if clock_fields.next().is_some() || hour > 23 || minute > 59 || second > 59 {
                return None;
            }
            (hour, minute, second, millis)
        }
    };
    utc_millis(year, month, day, hour, minute, second, millis)
}

/// Neo: exactly `width` ASCII digits.
fn number(field: &str, width: usize) -> Option<u32> {
    (field.len() == width && field.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| ascii_number(field.as_bytes()))
}

/// Neo: the number some ASCII digits write, saturating.
fn ascii_number(digits: &[u8]) -> u32 {
    digits.iter().fold(0u32, |total, digit| {
        total
            .saturating_mul(10)
            .saturating_add(u32::from(digit - b'0'))
    })
}

/// Neo: the milliseconds since the epoch of a UTC date and time, `None` for a day the month
/// does not have (days-from-civil, proleptic Gregorian).
fn utc_millis(
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    millis: u32,
) -> Option<f64> {
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&month)
        || day == 0
        || day > month_days[usize::try_from(month - 1).ok()?]
    {
        return None;
    }
    let (year, month, day) = (i64::from(year), i64::from(month), i64::from(day));
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds =
        days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    #[expect(
        clippy::cast_precision_loss,
        reason = "a cookie date's milliseconds stay far inside f64's exact integers"
    )]
    let millis = (seconds * 1_000 + i64::from(millis)) as f64;
    Some(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_700_000_000.0;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap_or_else(|error| panic!("{text}: {error}"))
    }

    fn cookie(name: &str, domain: &str, path: &str, expires: f64) -> NetworkCookie {
        NetworkCookie {
            name: name.to_string(),
            value: format!("{name}-value"),
            domain: domain.to_string(),
            path: path.to_string(),
            expires,
            http_only: false,
            secure: false,
            same_site: NetworkCookieSameSite::Lax,
            partition_key: None,
            cr_has_cross_site_ancestor: None,
        }
    }

    #[test]
    fn tests_that_a_set_cookie_header_parses_as_playwrights_does() {
        let raw = parse_raw_cookie(
            " a = b ; Domain=Example.COM; Path=/p; Secure; HttpOnly; SameSite=strict; Max-Age=60",
            NOW,
        )
        .unwrap_or_default();
        assert_eq!(raw.name, "a");
        assert_eq!(raw.value, "b");
        assert_eq!(raw.domain.as_deref(), Some(".example.com"));
        assert_eq!(raw.path.as_deref(), Some("/p"));
        assert!(raw.secure && raw.http_only);
        assert_eq!(raw.same_site, Some(NetworkCookieSameSite::Strict));
        assert_eq!(raw.expires, Some(NOW + 60.0));
        // A nameless cookie, and Max-Age over Expires whatever their order.
        let nameless = parse_raw_cookie(
            "token; Max-Age=0; Expires=Wed, 21 Oct 2037 07:28:00 GMT",
            NOW,
        )
        .unwrap_or_default();
        assert_eq!(
            (nameless.name.as_str(), nameless.value.as_str()),
            ("", "token")
        );
        assert_eq!(nameless.expires, Some(0.0));
        assert!(
            parse_raw_cookie(" ; Path=/", NOW).is_none(),
            "no name and no value"
        );
        // A Max-Age that is not an optional minus and digits is ignored.
        assert_eq!(
            parse_raw_cookie("a=b; Max-Age=1.5", NOW)
                .unwrap_or_default()
                .expires,
            None
        );
        assert_eq!(
            parse_raw_cookie("a=b; Domain=localhost", NOW)
                .unwrap_or_default()
                .domain
                .as_deref(),
            Some("localhost"),
            "a domain without a dot gets no leading dot"
        );
    }

    #[test]
    fn tests_that_expires_reads_every_http_date_form_and_iso() {
        let at = |value: &str| {
            parse_raw_cookie(&format!("a=b; Expires={value}"), NOW)
                .unwrap_or_default()
                .expires
        };
        let expected = Some(1_445_412_480.0);
        assert_eq!(at("Wed, 21 Oct 2015 07:28:00 GMT"), expected);
        assert_eq!(at("Wednesday, 21-Oct-15 07:28:00 GMT"), expected);
        assert_eq!(at("Wed, 21-Oct-2015 07:28:00 GMT"), expected);
        assert_eq!(at("Wed Oct 21 07:28:00 2015"), expected);
        assert_eq!(at("2015-10-21T07:28:00Z"), expected);
        assert_eq!(at("2015-10-21"), Some(1_445_385_600.0));
        assert_eq!(
            at("Thu, 01 Jan 1970 00:00:00 GMT"),
            Some(0.0),
            "the epoch expires at once"
        );
        assert_eq!(
            at("Fri, 31 Dec 9999 23:59:59 GMT"),
            Some(MAX_COOKIE_EXPIRES_SECONDS)
        );
        assert_eq!(
            at("Sat, 01 Jan 10000 00:00:00 GMT"),
            None,
            "a five-digit year is not one"
        );
        assert_eq!(
            at("Fri, 30 Feb 2015 07:28:00 GMT"),
            None,
            "a day the month lacks"
        );
        assert_eq!(at("not a date"), None);
    }

    #[test]
    fn tests_that_set_cookie_defaults_domain_and_path_from_the_response_url() {
        let from = url("http://www.example.com/a/b/c?q");
        let cookies = parse_set_cookie_headers(
            &from,
            &[
                "one=1",
                "two=2; Path=relative",
                "three=3; Domain=example.com; Path=/x",
                "four=4; Domain=other.com",
            ],
            NOW,
        );
        let summary: Vec<(&str, &str, &str, f64, NetworkCookieSameSite)> = cookies
            .iter()
            .map(|cookie| {
                (
                    cookie.name.as_str(),
                    cookie.domain.as_str(),
                    cookie.path.as_str(),
                    cookie.expires,
                    cookie.same_site,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "one",
                    "www.example.com",
                    "/a/b",
                    -1.0,
                    NetworkCookieSameSite::Lax
                ),
                (
                    "two",
                    "www.example.com",
                    "/a/b",
                    -1.0,
                    NetworkCookieSameSite::Lax
                ),
                (
                    "three",
                    ".example.com",
                    "/x",
                    -1.0,
                    NetworkCookieSameSite::Lax
                ),
            ],
            "another site's domain is dropped"
        );
        assert_eq!(default_path("/"), "/");
        assert_eq!(default_path("/a"), "/");
    }

    #[test]
    fn tests_that_the_jar_replaces_same_cookies_keeps_order_and_prunes_expired() {
        let mut jar = CookieStore::default();
        jar.add_cookies(
            vec![
                cookie("a", "example.com", "/", -1.0),
                cookie("c", "example.com", "/", NOW + 10.0),
                cookie("a", "example.com", "/x", -1.0),
            ],
            NOW,
        );
        let order = |jar: &mut CookieStore, now: f64| -> Vec<(String, String)> {
            jar.all_cookies(now)
                .into_iter()
                .map(|cookie| (cookie.name, cookie.path))
                .collect()
        };
        assert_eq!(
            order(&mut jar, NOW),
            [
                ("a".into(), "/".into()),
                ("a".into(), "/x".into()),
                ("c".into(), "/".into())
            ],
            "by name in the order the names came"
        );
        // The same name, domain and path replaces; an expired one removes.
        jar.add_cookies(
            vec![NetworkCookie {
                value: "new".into(),
                ..cookie("a", "example.com", "/", -1.0)
            }],
            NOW,
        );
        jar.add_cookies(vec![cookie("a", "example.com", "/x", 0.0)], NOW);
        let all = jar.all_cookies(NOW);
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].name.as_str(), all[0].value.as_str()), ("a", "new"));
        assert_eq!(
            order(&mut jar, NOW + 11.0),
            [("a".into(), "/".into())],
            "c expired"
        );
    }
}
