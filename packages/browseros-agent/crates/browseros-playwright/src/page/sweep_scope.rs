//! Which frames of a page a main-world script can reach, so the evaluate guard sweeps all of
//! them (decisions.md, "Main-world sweep scope").
//!
//! A script reads the DOM of every frame of its own origin through `frames[…]`, `parent` and
//! `top`, however many cross-site frames sit between them. Chromium keeps every document of one
//! site in one renderer process, but CDP splits a process into one session per local frame
//! root: in the nested fixture a3 has top's site and shares its process, yet sits on a session
//! of its own under two cross-site OOPIFs, and b1 and b2 are same-origin siblings on two
//! sessions. So the guard sweeps every frame of the page whose site is the script frame's, on
//! every session.
//!
//! ```text
//!  Target.getTargets (the browser; no renderer is asked)
//!     └─ the page's target, and each iframe target whose parentId chain leads to it
//!          └─ Site::may_hold(target url)? ── no: never asked anything, hung or not
//!                                         └─ yes: its session's frames are all swept
//! ```
//!
//! A target is judged by its root document's URL alone: every frame on one session shares
//! that session's renderer, which site isolation locks to one site, so the rest of its frames
//! hold that site or an opaque origin no other frame can reach. A site that cannot be read from
//! a URL (`about:blank`, `about:srcdoc`, `data:`) may be any, so such a target is swept.
//!
//! `Target.getTargets` lists every tab's targets, so its cost grows with the whole browser. The
//! guard reads it on every sweep all the same, because nothing in the page finds every
//! out-of-process frame: a session's frame tree lists none, and `window.length` leaves out the
//! frames of shadow trees (measured: an OOPIF in a closed shadow root counts 0 in both worlds,
//! yet the browser lists it under the page).

use std::collections::HashSet;

use browseros_core::{BrowserSession, CoreError, FrameId};
use serde::Deserialize;
use serde_json::json;
use url::Url;

use crate::page::document_watch::decode;

/// A site as Chromium's site isolation counts it, the boundary of a renderer process: a scheme
/// and a registrable domain (`https` and `bbc.co.uk`), or a whole host that has none (an IP
/// address, `localhost`). Ports do not count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Site {
    /// A document with a host.
    Known {
        scheme: String,
        /// Lowercase, without a trailing dot.
        domain: String,
    },
    /// An opaque or unreadable origin: a document at `about:blank` or `about:srcdoc` inherits
    /// its creator's origin, and one at `data:` has none, so it may share any frame's site.
    Unknown,
}

impl Site {
    /// The site of a document at `url` whose registrable domain its renderer reported as
    /// `domain_and_registry` (`Page.Frame.domainAndRegistry`, empty when the host has none).
    pub(crate) fn of(url: &str, domain_and_registry: Option<&str>) -> Self {
        let Some((scheme, host)) = scheme_and_host(url) else {
            return Self::Unknown;
        };
        let domain = domain_and_registry
            .map(normalized)
            .filter(|domain| within(&host, domain))
            .unwrap_or(host);
        Self::Known { scheme, domain }
    }

    /// Whether a document at `url` may be of this site. Its host is of the site when it is the
    /// registrable domain or a name under it. That can only widen the site a public suffix
    /// list would draw (`a.s3.amazonaws.com` under `amazonaws.com`), never narrow it, so the
    /// sweep may cover a frame more, never one less. An unreadable URL may be of any site, and
    /// any URL may be of an unknown one.
    pub(crate) fn may_hold(&self, url: &str) -> bool {
        let Self::Known { scheme, domain } = self else {
            return true;
        };
        scheme_and_host(url).is_none_or(|(other, host)| other == *scheme && within(&host, domain))
    }
}

/// `host` is `domain` or a name under it.
fn within(host: &str, domain: &str) -> bool {
    host.strip_suffix(domain)
        .is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
}

fn normalized(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// The scheme and host a URL's document belongs to, a `blob:` URL's being its creator's;
/// `None` for a URL with no host.
fn scheme_and_host(url: &str) -> Option<(String, String)> {
    let parsed = Url::parse(url).ok()?;
    let parsed = if parsed.scheme() == "blob" {
        Url::parse(parsed.path()).ok()?
    } else {
        parsed
    };
    let host = parsed.host_str().map(normalized)?;
    (!host.is_empty()).then(|| (parsed.scheme().to_owned(), host))
}

/// One target of the page: the page's own (whose id is its main frame's), or an OOPIF (whose id
/// is its root frame's).
#[derive(Debug)]
pub(crate) struct PageTarget {
    pub(crate) frame: FrameId,
    /// Its root document's URL, as the browser reports it.
    pub(crate) url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Targets {
    target_infos: Vec<TargetInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfo {
    target_id: String,
    url: String,
    /// The target that embeds an iframe target: the one whose session lists the frame that
    /// holds it. `None` for a page.
    #[serde(default)]
    parent_id: Option<String>,
}

/// Every page and iframe target of the browser. One browser-side call, so no renderer, hung or
/// not, is asked.
async fn browser_targets(browser: &BrowserSession) -> Result<Targets, CoreError> {
    browser
        .cdp(
            "Target.getTargets",
            json!({ "filter": [{ "type": "page" }, { "type": "iframe" }] }),
            None,
        )
        .await
        .and_then(decode)
}

/// Whether the browser still lists `target`. Only this says a target left: a session lost to
/// another client's detach leaves its frame in the page.
pub(crate) async fn is_listed(
    browser: &BrowserSession,
    target: &FrameId,
) -> Result<bool, CoreError> {
    Ok(browser_targets(browser)
        .await?
        .target_infos
        .iter()
        .any(|listed| listed.target_id == target.as_str()))
}

/// The targets of the page whose main frame is `main`: its own, then each iframe target whose
/// `parentId` chain leads to it.
pub(crate) async fn page_targets(
    browser: &BrowserSession,
    main: &FrameId,
) -> Result<Vec<PageTarget>, CoreError> {
    let listed = browser_targets(browser).await?;
    let mut ours = HashSet::from([main.as_str().to_owned()]);
    let (mut found, mut rest): (Vec<_>, Vec<_>) = listed
        .target_infos
        .into_iter()
        .partition(|target| target.target_id == main.as_str());
    // Each round takes the targets embedded in one found so far, so it ends within as many
    // rounds as there are targets.
    loop {
        let (embedded, others): (Vec<_>, Vec<_>) = rest.into_iter().partition(|target| {
            target
                .parent_id
                .as_ref()
                .is_some_and(|parent| ours.contains(parent))
        });
        rest = others;
        if embedded.is_empty() {
            break;
        }
        ours.extend(embedded.iter().map(|target| target.target_id.clone()));
        found.extend(embedded);
    }
    Ok(found
        .into_iter()
        .map(|target| PageTarget {
            frame: FrameId(target.target_id),
            url: target.url,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::Site;

    #[test]
    fn tests_that_a_site_is_the_scheme_and_registrable_domain_without_the_port() {
        let bbc = Site::of("https://www.bbc.co.uk:8443/news", Some("bbc.co.uk"));
        assert!(bbc.may_hold("https://pay.bbc.co.uk/frame"));
        assert!(bbc.may_hold("https://bbc.co.uk/"));
        assert!(
            bbc.may_hold("https://BBC.co.uk./"),
            "case and a trailing dot"
        );
        assert!(
            !bbc.may_hold("http://www.bbc.co.uk/"),
            "another scheme is another site"
        );
        assert!(
            !bbc.may_hold("https://notbbc.co.uk/"),
            "a name under it, not one ending so"
        );
        assert!(!bbc.may_hold("https://other.co.uk/"));
        assert!(
            bbc.may_hold("blob:https://pay.bbc.co.uk/0b2e"),
            "a blob URL's creator"
        );
    }

    #[test]
    fn tests_that_a_host_without_a_registrable_domain_is_its_own_site() {
        let ip = Site::of("http://127.0.0.1:41000/nest/top.html", Some(""));
        assert!(
            ip.may_hold("http://127.0.0.1:52000/nest/a3.html"),
            "ports do not count"
        );
        assert!(!ip.may_hold("http://localhost:41000/nest/b1.html"));
        assert!(!ip.may_hold("http://[::1]:41000/nest/c2.html"));
        let v6 = Site::of("http://[::1]:41000/nest/c2.html", None);
        assert!(v6.may_hold("http://[::1]:41000/nest/c4.html"));
        assert!(!v6.may_hold("http://hang.localtest.me:41000/nest/hang.html"));
        assert_eq!(
            Site::of("https://example.com/", Some("unrelated.org")),
            Site::of("https://example.com/", None),
            "a registrable domain that is not the host's own is ignored"
        );
    }

    #[test]
    fn tests_that_a_url_without_a_host_may_hold_any_site() {
        let ip = Site::of("http://127.0.0.1:41000/", None);
        for url in [
            "about:blank",
            "about:srcdoc",
            "data:text/html,x",
            "not a url",
        ] {
            assert!(ip.may_hold(url), "{url} may be of the script's site");
            assert_eq!(Site::of(url, Some("")), Site::Unknown, "{url}");
        }
        assert!(Site::Unknown.may_hold("https://anything.example/"));
    }
}
