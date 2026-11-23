//! The span privacy dial (`CLAW_TELEMETRY_SITE_DETAIL`, [`SiteDetail`]): what of an op's page
//! address its span may keep. Spans only: no metric ever sees a URL, whatever the dial
//! says.
//!
//! `none` (the default) keeps nothing. `host` keeps `url.domain`. `url` adds `url.full`, with
//! userinfo, query string and fragment removed. Only web addresses (`http`, `https`) are kept at
//! all: a `data:` URL is page content rather than an address and a `file:` path names the
//! machine's users, so any other scheme keeps nothing whatever the dial says. A URL that does
//! not parse keeps nothing too: it is never copied raw.

use url::Url;

use crate::SiteDetail;

/// The longest `url.full` a span keeps. A longer one is left out rather than cut: a cut URL would
/// name a page that does not exist.
const FULL_MAX_BYTES: usize = 2048;

/// The dial as an op span applies it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Privacy {
    detail: SiteDetail,
}

/// What of one page address a span keeps: the `url.domain` and `url.full` attributes, each only
/// when the dial allows it. A plain data bag.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Site {
    /// `url.domain`: the host name, or the IP address in its URL spelling (`[::1]`).
    pub(crate) domain: Option<String>,
    /// `url.full`: the address without userinfo, query string or fragment.
    pub(crate) full: Option<String>,
}

impl Privacy {
    pub(crate) const fn new(detail: SiteDetail) -> Self {
        Self { detail }
    }

    /// What a span keeps of `url`, the op's raw page URL (`None` when the door had none, or the
    /// op was the host's private typing).
    pub(crate) fn site(self, url: Option<&str>) -> Site {
        let with_full = match self.detail {
            SiteDetail::None => return Site::default(),
            SiteDetail::Host => false,
            SiteDetail::Url => true,
        };
        let Some(mut url) = url.and_then(|raw| Url::parse(raw).ok()) else {
            return Site::default();
        };
        if !matches!(url.scheme(), "http" | "https") {
            return Site::default();
        }
        let domain = url.host_str().map(str::to_owned);
        let full = with_full
            .then(|| {
                url.set_query(None);
                url.set_fragment(None);
                // Both succeed on an http(s) URL, the only kind that gets here.
                let _ = url.set_username("");
                let _ = url.set_password(None);
                let full: String = url.into();
                full
            })
            .filter(|full| full.len() <= FULL_MAX_BYTES);
        Site { domain, full }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://alice:s3cret@shop.example.com:8443/cart/items?session=abc#pay";

    #[test]
    fn none_keeps_nothing_at_all() {
        assert_eq!(
            Privacy::new(SiteDetail::None).site(Some(PAGE)),
            Site::default(),
            "the default dial: no URL, no host, nothing"
        );
    }

    #[test]
    fn host_keeps_only_the_domain() {
        assert_eq!(
            Privacy::new(SiteDetail::Host).site(Some(PAGE)),
            Site {
                domain: Some("shop.example.com".to_owned()),
                full: None,
            },
            "`host` keeps url.domain and nothing of the path, query, user or port"
        );
    }

    #[test]
    fn url_keeps_the_address_without_userinfo_query_or_fragment() {
        assert_eq!(
            Privacy::new(SiteDetail::Url).site(Some(PAGE)),
            Site {
                domain: Some("shop.example.com".to_owned()),
                full: Some("https://shop.example.com:8443/cart/items".to_owned()),
            },
            "`url` keeps scheme, host, port and path; the credentials, the query and the fragment go"
        );
        assert_eq!(
            Privacy::new(SiteDetail::Url).site(Some("http://[::1]:3000/?q=1")),
            Site {
                domain: Some("[::1]".to_owned()),
                full: Some("http://[::1]:3000/".to_owned()),
            },
            "an IP address is the domain, in its URL spelling"
        );
    }

    #[test]
    fn anything_but_a_parsable_web_address_keeps_nothing() {
        for detail in [SiteDetail::Host, SiteDetail::Url] {
            for raw in [
                "data:text/html,<p>secret page</p>",
                "file:///Users/alice/notes.html",
                "about:blank",
                "chrome://settings",
                "javascript:alert(1)",
                "not a url",
                "",
            ] {
                assert_eq!(
                    Privacy::new(detail).site(Some(raw)),
                    Site::default(),
                    "`{raw}` is not a web address, so a span keeps none of it"
                );
            }
            assert_eq!(
                Privacy::new(detail).site(None),
                Site::default(),
                "no URL (a protected op): nothing"
            );
        }
    }

    #[test]
    fn an_overlong_address_keeps_its_domain_but_not_a_cut_url() {
        let long = format!("https://example.com/{}", "a".repeat(FULL_MAX_BYTES));
        assert_eq!(
            Privacy::new(SiteDetail::Url).site(Some(&long)),
            Site {
                domain: Some("example.com".to_owned()),
                full: None,
            },
            "url.full is whole or absent, never cut"
        );
    }
}
