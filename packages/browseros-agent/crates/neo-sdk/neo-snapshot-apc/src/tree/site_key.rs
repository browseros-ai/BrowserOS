//! The site a host-private mark is scoped to: half of privacy layer 2's key (design 8.1).
//!
//! [`PrivateNodes`](super::PrivateNodes) keys a mark by DOM node id and site rather than by
//! [`NodeKey`](super::NodeKey), because a box the page moves into another same-origin document
//! keeps its id and changes its document token. The site comes only from an origin as the browser
//! serializes one, never from a document URL, and anything that does not read as such an origin
//! is the unknown site (`None`), which masks rather than reveals.

use std::fmt;

use url::{Origin, Url};

use super::tree_doc::{DocOrigin, TreeDoc};

/// A normalized security origin or an opaque origin's nonce: the site a host-private mark was
/// made on, or the site of a document a build masks.
///
/// Two documents share a key only when the browser serialized the same origin for both, or gave
/// both the same opaque nonce. An opaque key never equals a tuple key, even for the same string.
/// Its `Debug` prints the variant only, since an origin names the site a secret was typed into.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SiteKey(Site);

#[derive(Clone, PartialEq, Eq, Hash)]
enum Site {
    /// `scheme://host[:port]`: a tuple origin as [`Origin::ascii_serialization`] writes it, or
    /// the same shape for a non-tuple scheme with an authority (`file://`, `chrome-extension://id`).
    Origin(String),
    /// An opaque origin's exact nonce.
    Opaque(String),
}

impl SiteKey {
    /// The site of `origin`, an origin as the browser serializes one: APC's `DocOrigin.value`,
    /// CDP's `securityOrigin`, or a world's `self.origin`.
    ///
    /// Returns `None` (site unknown) for blank or unparseable text (`"null"` included), an
    /// opaque-path URL (`about:`, `data:`, `javascript:`, `blob:`), and a URL with credentials, a
    /// query, a fragment or a path. A document URL is not its document's origin (an
    /// `about:srcdoc` or sandboxed `https:` document's origin is not its URL's), so a URL passed
    /// here by mistake fails closed instead of becoming a plausible wrong key.
    #[must_use]
    pub fn from_origin(origin: &str) -> Option<Self> {
        let url = Url::parse(origin.trim()).ok()?;
        let is_bare_origin = !url.cannot_be_a_base()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && matches!(url.path(), "" | "/");
        if !is_bare_origin {
            return None;
        }
        let normalized = match url.origin() {
            tuple @ Origin::Tuple(..) => tuple.ascii_serialization(),
            Origin::Opaque(_) => non_tuple_origin(&url)?,
        };
        Some(Self(Site::Origin(normalized)))
    }

    /// The site of `doc`, read from its APC security origin only, never from its URL.
    ///
    /// An opaque origin keys on its exact nonce. A document with no origin, a blank nonce, or an
    /// origin value [`Self::from_origin`] refuses has no known site.
    #[must_use]
    pub fn of_doc(doc: &TreeDoc) -> Option<Self> {
        doc.origin().and_then(Self::from_doc_origin)
    }

    fn from_doc_origin(origin: &DocOrigin) -> Option<Self> {
        if !origin.opaque {
            return Self::from_origin(&origin.value);
        }
        if origin.value.trim().is_empty() {
            return None;
        }
        Some(Self(Site::Opaque(origin.value.clone())))
    }
}

impl fmt::Debug for SiteKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Site::Origin(_) => f.write_str("SiteKey::Origin(<redacted>)"),
            Site::Opaque(_) => f.write_str("SiteKey::Opaque(<redacted>)"),
        }
    }
}

/// `scheme://lowercased-host[:port]` for a non-tuple scheme with a host, `file://` for a
/// host-less `file:` origin, and `None` for any other shape, which no browser serializes as an
/// origin.
fn non_tuple_origin(url: &Url) -> Option<String> {
    let scheme = url.scheme();
    match url.host_str() {
        Some(host) if !host.is_empty() => {
            let host = host.to_ascii_lowercase();
            Some(match url.port() {
                Some(port) => format!("{scheme}://{host}:{port}"),
                None => format!("{scheme}://{host}"),
            })
        }
        _ if scheme == "file" => Some("file://".to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::super::apc_builder::{ApcBuilder, NodeBuilder};
    use super::super::snapshot_tree::{BuildOptions, SnapshotTree};
    use super::SiteKey;

    fn site(origin: &str) -> Result<SiteKey, String> {
        SiteKey::from_origin(origin).ok_or_else(|| format!("{origin:?} must read as an origin"))
    }

    /// A one-node main document with the given URL and security origin.
    fn tree_with(
        url: Option<&str>,
        origin: Option<(&str, bool)>,
    ) -> Result<SnapshotTree, Box<dyn Error>> {
        let mut builder = ApcBuilder::new("doc-site").root(NodeBuilder::root().dom_id(1));
        if let Some(url) = url {
            builder = builder.main_url(url);
        }
        if let Some((value, opaque)) = origin {
            builder = builder.main_origin(value, opaque);
        }
        Ok(SnapshotTree::build(builder.build(), BuildOptions::new())?)
    }

    fn site_of(tree: &SnapshotTree) -> Result<Option<SiteKey>, &'static str> {
        Ok(SiteKey::of_doc(tree.main_document().ok_or("main doc")?))
    }

    #[test]
    fn tuple_origins_normalize_default_port_and_host_case() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            site("https://A.TEST:443")?,
            site("https://a.test")?,
            "the default port and host case name the same origin"
        );
        assert_eq!(
            site(" https://a.test/ ")?,
            site("https://a.test")?,
            "a trailing slash and surrounding blanks are not part of the origin"
        );
        assert_ne!(
            site("https://a.test:8443")?,
            site("https://a.test")?,
            "a non-default port is another origin"
        );
        assert_ne!(
            site("http://a.test")?,
            site("https://a.test")?,
            "the scheme is part of the origin"
        );
        Ok(())
    }

    #[test]
    fn non_tuple_origins_with_an_authority_normalize_stably() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            site("file://")?,
            site("FILE:///")?,
            "a host-less file: origin reads as file:// in any spelling"
        );
        assert_eq!(
            site("chrome-extension://ABC")?,
            site("chrome-extension://abc/")?,
            "an extension id is matched without case"
        );
        assert_ne!(
            site("chrome-extension://abc")?,
            site("chrome-extension://xyz")?,
            "two extensions are two sites"
        );
        assert_ne!(
            site("file://")?,
            site("chrome-extension://abc")?,
            "two schemes are two sites"
        );
        Ok(())
    }

    #[test]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the SDK test contract (STYLE P10) requires Result even when no step can fail"
    )]
    fn anything_but_a_bare_origin_is_an_unknown_site() -> Result<(), Box<dyn Error>> {
        for refused in [
            "",
            "  \t ",
            "null",
            "not a url",
            "about:blank",
            "about:srcdoc",
            "data:,x",
            "javascript:void(0)",
            "blob:https://a.test/x",
            "https://a.test/login",
            "https://a.test/?q",
            "https://a.test/#top",
            "https://u:p@a.test",
            "https://u@a.test",
            "file:///tmp/page.html",
        ] {
            assert!(
                SiteKey::from_origin(refused).is_none(),
                "{refused:?} is not an origin as the browser serializes one, so its site is unknown"
            );
        }
        Ok(())
    }

    #[test]
    fn of_doc_reads_the_security_origin_never_the_url() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            site_of(&tree_with(
                Some("https://b.test/form"),
                Some(("https://A.test:443", false))
            )?)?,
            Some(site("https://a.test")?),
            "a tuple origin normalizes as from_origin normalizes it, whatever the URL"
        );
        assert_eq!(
            site_of(&tree_with(Some("https://a.test/form"), None)?)?,
            None,
            "a document with a URL but no origin has no known site"
        );
        assert_eq!(
            site_of(&tree_with(None, Some(("", false)))?)?,
            None,
            "an empty origin value is no site"
        );
        assert_eq!(
            site_of(&tree_with(None, Some(("   ", true)))?)?,
            None,
            "a blank opaque nonce is no site"
        );
        Ok(())
    }

    #[test]
    fn opaque_nonces_match_exactly_and_never_a_tuple_origin() -> Result<(), Box<dyn Error>> {
        let nonce_a = site_of(&tree_with(None, Some(("https://a.test", true)))?)?;
        let nonce_a_again = site_of(&tree_with(None, Some(("https://a.test", true)))?)?;
        let nonce_b = site_of(&tree_with(None, Some(("nonce-b", true)))?)?;
        assert!(nonce_a.is_some(), "a non-blank nonce is a site");
        assert_eq!(nonce_a, nonce_a_again, "the same nonce is the same site");
        assert_ne!(nonce_a, nonce_b, "another nonce is another site");
        assert_ne!(
            nonce_a,
            Some(site("https://a.test")?),
            "an opaque nonce never equals a tuple origin spelled the same"
        );
        Ok(())
    }

    #[test]
    fn debug_prints_the_variant_only() -> Result<(), Box<dyn Error>> {
        let tuple = site("https://tenant-secret.example:8443")?;
        let opaque =
            site_of(&tree_with(None, Some(("nonce-secret-42", true)))?)?.ok_or("opaque site")?;
        assert_eq!(format!("{tuple:?}"), "SiteKey::Origin(<redacted>)");
        assert_eq!(format!("{opaque:?}"), "SiteKey::Opaque(<redacted>)");
        Ok(())
    }
}
