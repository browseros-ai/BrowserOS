//! Which frames a main-world script can reach, so the evaluate guard sweeps all of them
//! (decisions.md, "Main-world sweep scope").
//!
//! A script reads the DOM of every frame of its own origin through `frames[…]`, `parent` and
//! `top`, however many cross-site frames sit between them. Chromium keeps every document of one
//! site in one renderer process, but CDP splits a process into one session per local frame
//! root: in the nested fixture a3 has top's site and shares its process, yet sits on a session
//! of its own under two cross-site OOPIFs, and b1 and b2 are same-origin siblings on two
//! sessions. So the guard sweeps every frame of the page whose site is the script frame's, on
//! every session.
//!
//! A script also reaches the other tabs of its browsing context group: its opener through
//! `window.opener`, a popup through the window `window.open` returned, any of them by name,
//! and their frames through tabs of any site in between. The browser's `openerId` links draw
//! that group, so every tab they connect to the page, both ways and transitively, is swept as
//! the page is. Measured on BrowserOS 155 with raw CDP:
//! - a popup and its opener read each other's fields, and a popup reads a sibling's;
//! - a popup disowned by either side (`w.opener = null`, `window.opener = null`) is still read
//!   by its opener, and finds the opener by name;
//! - a `noopener` popup reaches no other tab and no tab reaches it, by handle or by name;
//! - when the middle tab of a chain closes, the last one's `openerId` moves to the first, which
//!   still reaches it.
//!
//! A `noopener` popup is swept all the same: the browser reports it exactly as it reports a
//! disowned one (`canAccessOpener` false, `openerFrameId` set), so leaving it out would leave
//! out a popup its opener still reads. A tab no link connects is never asked anything.
//!
//! ```text
//!  Target.getTargets (the browser; no renderer is asked)
//!     ├─ the page's target, and each iframe target whose parentId chain leads to it
//!     └─ each tab openerId links connect to the page, with its targets
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

use std::collections::HashMap;

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

/// One target of a tab: the tab's own (whose id is its main frame's), or an OOPIF (whose id is
/// its root frame's).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PageTarget {
    pub(crate) frame: FrameId,
    /// Its root document's URL, as the browser reports it.
    pub(crate) url: String,
}

/// Every target a script on one page can reach: the page's own, and those of each tab its
/// opener links connect it to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Reach {
    /// The page's targets: its own, and each iframe target whose `parentId` chain leads to it.
    pub(crate) own: Vec<PageTarget>,
    /// The other tabs of its opener-connected group, nearest link first.
    pub(crate) linked: Vec<LinkedTab>,
}

/// A tab an opener link connects to the script's page, directly or through other tabs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LinkedTab {
    /// The tab's page target, whose id is its main frame's.
    pub(crate) main: FrameId,
    /// Its targets, as [`Reach::own`] lists the page's.
    pub(crate) targets: Vec<PageTarget>,
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
    /// The tab that opened a page: its page target, also when an iframe of it called
    /// `window.open` (measured). Set for `noopener` popups too, and moved to the opener's own
    /// opener when the opener closes (measured).
    #[serde(default)]
    opener_id: Option<String>,
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

/// Every target a script on the page whose main frame is `main` can reach, from one read of
/// the browser's target list.
pub(crate) async fn reach(browser: &BrowserSession, main: &FrameId) -> Result<Reach, CoreError> {
    Ok(reach_in(browser_targets(browser).await?.target_infos, main))
}

/// [`reach`] over the browser's `listed` targets: one pass finds each target's tab, so a page
/// with no opener link reaches its own targets and asks nothing more.
fn reach_in(listed: Vec<TargetInfo>, main: &FrameId) -> Reach {
    let (group, tab_of_each) = {
        let parents: HashMap<&str, &str> = listed
            .iter()
            .filter_map(|target| Some((target.target_id.as_str(), target.parent_id.as_deref()?)))
            .collect();
        // A popup reaches its opener through `window.opener` or by name, and the opener reaches
        // it through the window `window.open` returned, so a link counts both ways.
        let mut links: HashMap<&str, Vec<&str>> = HashMap::new();
        for target in &listed {
            if let (None, Some(opener)) = (&target.parent_id, &target.opener_id) {
                let opener = tab_of(&parents, opener);
                links
                    .entry(target.target_id.as_str())
                    .or_default()
                    .push(opener);
                links
                    .entry(opener)
                    .or_default()
                    .push(target.target_id.as_str());
            }
        }
        let mut group = vec![main.as_str()];
        let mut seen = HashMap::from([(main.as_str(), 0)]);
        let mut next = 0;
        while let Some(tab) = group.get(next).copied() {
            for &linked in links.get(tab).into_iter().flatten() {
                if !seen.contains_key(linked) {
                    seen.insert(linked, group.len());
                    group.push(linked);
                }
            }
            next += 1;
        }
        let tab_of_each: Vec<Option<usize>> = listed
            .iter()
            .map(|target| seen.get(tab_of(&parents, &target.target_id)).copied())
            .collect();
        let group: Vec<FrameId> = group
            .into_iter()
            .map(|tab| FrameId(tab.to_owned()))
            .collect();
        (group, tab_of_each)
    };
    let mut tabs: Vec<Vec<PageTarget>> = group.iter().map(|_| Vec::new()).collect();
    for (target, tab) in listed.into_iter().zip(tab_of_each) {
        if let Some(targets) = tab.and_then(|tab| tabs.get_mut(tab)) {
            targets.push(PageTarget {
                frame: FrameId(target.target_id),
                url: target.url,
            });
        }
    }
    let mut tabs = group.into_iter().zip(tabs);
    let own = tabs.next().map(|(_, own)| own).unwrap_or_default();
    Reach {
        own,
        // A tab the browser lists no target of (a closed opener its popups still name) links
        // them, but holds nothing to sweep.
        linked: tabs
            .filter(|(_, targets)| !targets.is_empty())
            .map(|(main, targets)| LinkedTab { main, targets })
            .collect(),
    }
}

/// The tab `target` is in: the root of its `parentId` chain. The walk takes at most as many
/// steps as there are parents, so a cycle cannot hold it.
fn tab_of<'a>(parents: &HashMap<&'a str, &'a str>, mut target: &'a str) -> &'a str {
    for _ in 0..=parents.len() {
        match parents.get(target) {
            Some(parent) => target = parent,
            None => break,
        }
    }
    target
}

#[cfg(test)]
mod tests {
    use browseros_core::FrameId;

    use super::{LinkedTab, PageTarget, Reach, Site, TargetInfo};

    /// A target `id` at `https://<id>.test/`, embedded in `parent` or opened by `opener`.
    fn target(id: &str, parent: Option<&str>, opener: Option<&str>) -> TargetInfo {
        TargetInfo {
            target_id: id.to_owned(),
            url: format!("https://{id}.test/"),
            parent_id: parent.map(str::to_owned),
            opener_id: opener.map(str::to_owned),
        }
    }

    fn targets(ids: &[&str]) -> Vec<PageTarget> {
        ids.iter()
            .map(|id| PageTarget {
                frame: FrameId((*id).to_owned()),
                url: format!("https://{id}.test/"),
            })
            .collect()
    }

    fn tab(main: &str, ids: &[&str]) -> LinkedTab {
        LinkedTab {
            main: FrameId(main.to_owned()),
            targets: targets(ids),
        }
    }

    /// Two groups: a opened b, which opened c; d opened e. a and b each embed an OOPIF, and
    /// a's holds another.
    fn browser() -> Vec<TargetInfo> {
        vec![
            target("a", None, None),
            target("a-frame", Some("a"), None),
            target("a-inner", Some("a-frame"), None),
            target("b", None, Some("a")),
            target("b-frame", Some("b"), None),
            target("c", None, Some("b")),
            target("d", None, None),
            target("e", None, Some("d")),
        ]
    }

    #[test]
    fn tests_that_a_page_reaches_its_own_targets_and_its_opener_group_both_ways() {
        let page = |id: &str| FrameId(id.to_owned());
        assert_eq!(
            super::reach_in(browser(), &page("a")),
            Reach {
                own: targets(&["a", "a-frame", "a-inner"]),
                linked: vec![tab("b", &["b", "b-frame"]), tab("c", &["c"])],
            },
            "an opener reaches its popup, and the popup's popup"
        );
        assert_eq!(
            super::reach_in(browser(), &page("c")),
            Reach {
                own: targets(&["c"]),
                linked: vec![
                    tab("b", &["b", "b-frame"]),
                    tab("a", &["a", "a-frame", "a-inner"])
                ],
            },
            "a popup reaches its opener, and the opener's opener"
        );
        assert_eq!(
            super::reach_in(browser(), &page("e")).linked,
            vec![tab("d", &["d"])],
            "another group is out of reach"
        );
    }

    #[test]
    fn tests_that_a_page_with_no_opener_link_reaches_only_its_own_targets() {
        let alone = vec![
            target("a", None, None),
            target("a-frame", Some("a"), None),
            target("d", None, None),
            target("e", None, Some("d")),
        ];
        assert_eq!(
            super::reach_in(alone, &FrameId("a".to_owned())),
            Reach {
                own: targets(&["a", "a-frame"]),
                linked: Vec::new(),
            }
        );
    }

    #[test]
    fn tests_that_popups_of_one_opener_reach_each_other_even_when_it_closed() {
        let with_opener = vec![
            target("a", None, None),
            target("b", None, Some("a")),
            target("c", None, Some("a")),
        ];
        assert_eq!(
            super::reach_in(with_opener, &FrameId("b".to_owned())).linked,
            vec![tab("a", &["a"]), tab("c", &["c"])],
            "a popup reaches its sibling through their opener"
        );
        let closed = vec![target("b", None, Some("a")), target("c", None, Some("a"))];
        assert_eq!(
            super::reach_in(closed, &FrameId("b".to_owned())).linked,
            vec![tab("c", &["c"])],
            "an opener no longer listed still links its popups"
        );
    }

    #[test]
    fn tests_that_an_opener_named_by_its_frame_is_its_tab() {
        let listed = vec![
            target("a", None, None),
            target("a-frame", Some("a"), None),
            target("b", None, Some("a-frame")),
        ];
        assert_eq!(
            super::reach_in(listed, &FrameId("b".to_owned())).linked,
            vec![tab("a", &["a", "a-frame"])]
        );
    }

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
