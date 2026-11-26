//! Privacy layer 2 (design 8.1): the boxes the host typed into that the page did not declare
//! private, keyed by DOM node id and site.
//!
//! Layer 1 is Blink's own redaction of password-like and sensitive `autocomplete` fields, read by
//! the mask pass from `redaction_decision`. A value the host typed into any other box arrives in
//! clear in `field_value`. The private entry marks such a box in its frame's isolated world (the
//! page holds the record), the tab's mirror in `neo-playwright` copies each mark into a
//! [`PrivateNodes`], and every [`SnapshotTree::build`](super::SnapshotTree::build) masks through
//! that set.

use std::collections::{HashMap, HashSet};
use std::fmt;

use super::node_key::DomNodeId;
use super::site_key::SiteKey;
use super::tree_doc::TreeDoc;

/// The host-private marks a build masks: a node masks when its id is marked and its document's
/// site is a site the id was marked on, or either site is unknown.
///
/// The key is the node id plus the [`SiteKey`], not the `NodeKey`: a box the page moves into
/// another same-origin document keeps its id and changes its document token, and must stay
/// masked, while a renderer of another site may reuse the number for an unrelated node. An unknown
/// site fails closed on both sides (a mark made where the site was unreadable masks its id in
/// every document; a document with no readable origin masks every marked id), so a false match
/// prints bullets, never a value. Its `Debug` prints counts only.
#[derive(Clone, Default)]
pub struct PrivateNodes {
    marks: HashMap<DomNodeId, NodeMarks>,
}

/// The sites one node id is marked on.
#[derive(Clone)]
enum NodeMarks {
    /// Marked where the site was unknown: the id masks in every document, so a later site mark
    /// adds nothing.
    Anywhere,
    /// Marked on these sites only. Never empty once [`PrivateNodes::mark`] returns.
    On(HashSet<SiteKey>),
}

impl NodeMarks {
    fn cover(&self, site: Option<&SiteKey>) -> bool {
        match (self, site) {
            (Self::Anywhere, _) | (Self::On(_), None) => true,
            (Self::On(sites), Some(site)) => sites.contains(site),
        }
    }
}

impl PrivateNodes {
    /// Creates an empty set, under which a build masks only what Blink redacted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks `node` private on `site`. `None`: the mark's site is unknown, so `node` masks in
    /// every document.
    pub fn mark(&mut self, site: Option<SiteKey>, node: DomNodeId) {
        let marks = self
            .marks
            .entry(node)
            .or_insert_with(|| NodeMarks::On(HashSet::new()));
        match site {
            None => *marks = NodeMarks::Anywhere,
            Some(site) => {
                if let NodeMarks::On(sites) = marks {
                    sites.insert(site);
                }
            }
        }
    }

    /// The one rule: `node` is marked, and the sites are equal or either side is unknown.
    /// `site` is the site of the document `node` is read in.
    #[must_use]
    pub fn is_private(&self, site: Option<&SiteKey>, node: DomNodeId) -> bool {
        self.marks.get(&node).is_some_and(|marks| marks.cover(site))
    }

    /// [`Self::is_private`] for a node of `doc`. Looks the id up first, so a document's site is
    /// parsed only for a marked id; [`SiteKey::of_doc`] gives the document's site.
    #[must_use]
    pub fn is_private_in(&self, doc: &TreeDoc, node: DomNodeId) -> bool {
        self.marks
            .get(&node)
            .is_some_and(|marks| marks.cover(SiteKey::of_doc(doc).as_ref()))
    }

    /// Whether no node is marked, so a build can skip the host-private pass.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }
}

impl fmt::Debug for PrivateNodes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut site_marks = 0_usize;
        let mut anywhere_marks = 0_usize;
        for marks in self.marks.values() {
            match marks {
                NodeMarks::Anywhere => anywhere_marks += 1,
                NodeMarks::On(sites) => site_marks += sites.len(),
            }
        }
        f.debug_struct("PrivateNodes")
            .field("marked_ids", &self.marks.len())
            .field("site_marks", &site_marks)
            .field("anywhere_marks", &anywhere_marks)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::super::apc_builder::{ApcBuilder, NodeBuilder};
    use super::super::node_key::DomNodeId;
    use super::super::site_key::SiteKey;
    use super::super::snapshot_tree::{BuildOptions, SnapshotTree};
    use super::PrivateNodes;

    fn site(origin: &str) -> Result<SiteKey, String> {
        SiteKey::from_origin(origin).ok_or_else(|| format!("{origin:?} must read as an origin"))
    }

    #[test]
    fn is_private_follows_the_one_rule_over_every_mark_and_document_site()
    -> Result<(), Box<dyn Error>> {
        struct Row<'s> {
            node: DomNodeId,
            doc_site: Option<&'s SiteKey>,
            masks: bool,
            why: &'static str,
        }
        let x = site("https://x.test")?;
        let y = site("https://y.test")?;
        let unmarked = DomNodeId::new(1);
        let on_x = DomNodeId::new(2);
        let anywhere = DomNodeId::new(3);
        let mut set = PrivateNodes::new();
        set.mark(Some(x.clone()), on_x);
        set.mark(None, anywhere);

        let table = [
            Row {
                node: unmarked,
                doc_site: Some(&x),
                masks: false,
                why: "an unmarked id is clear on its site",
            },
            Row {
                node: unmarked,
                doc_site: Some(&y),
                masks: false,
                why: "an unmarked id is clear on another site",
            },
            Row {
                node: unmarked,
                doc_site: None,
                masks: false,
                why: "an unmarked id is clear in an origin-less document",
            },
            Row {
                node: on_x,
                doc_site: Some(&x),
                masks: true,
                why: "a mark masks its id on its own site",
            },
            Row {
                node: on_x,
                doc_site: Some(&y),
                masks: false,
                why: "another site's renderer may reuse the id",
            },
            Row {
                node: on_x,
                doc_site: None,
                masks: true,
                why: "an origin-less document fails closed for a marked id",
            },
            Row {
                node: anywhere,
                doc_site: Some(&x),
                masks: true,
                why: "an unknown-site mark masks on every site",
            },
            Row {
                node: anywhere,
                doc_site: Some(&y),
                masks: true,
                why: "an unknown-site mark masks on every site",
            },
            Row {
                node: anywhere,
                doc_site: None,
                masks: true,
                why: "an unknown-site mark masks in an origin-less document",
            },
        ];
        for row in table {
            assert_eq!(
                set.is_private(row.doc_site, row.node),
                row.masks,
                "{}",
                row.why
            );
        }
        Ok(())
    }

    #[test]
    fn an_unknown_site_mark_absorbs_site_marks_before_and_after_it() -> Result<(), Box<dyn Error>> {
        let x = site("https://x.test")?;
        let y = site("https://y.test")?;
        let before = DomNodeId::new(1);
        let after = DomNodeId::new(2);
        let mut set = PrivateNodes::new();
        set.mark(Some(x.clone()), before);
        set.mark(None, before);
        set.mark(None, after);
        set.mark(Some(x), after);
        for node in [before, after] {
            assert!(
                set.is_private(Some(&y), node),
                "once an id is marked on an unknown site, no site mark narrows it back"
            );
        }
        Ok(())
    }

    #[test]
    fn is_private_in_reads_the_documents_origin_not_its_url() -> Result<(), Box<dyn Error>> {
        let node = DomNodeId::new(5);
        let mut set = PrivateNodes::new();
        assert!(set.is_empty(), "a new set marks nothing");
        set.mark(Some(site("https://x.test")?), node);
        assert!(!set.is_empty(), "a mark makes the set non-empty");

        let doc = |url: &str, origin: &str| -> Result<SnapshotTree, Box<dyn Error>> {
            let apc = ApcBuilder::new("doc-in")
                .main_url(url)
                .main_origin(origin, false)
                .root(NodeBuilder::root().dom_id(1))
                .build();
            Ok(SnapshotTree::build(apc, BuildOptions::new())?)
        };
        let on_x = doc("https://y.test/page", "https://x.test")?;
        let on_y = doc("https://x.test/page", "https://y.test")?;
        assert!(
            set.is_private_in(on_x.main_document().ok_or("main doc")?, node),
            "the document's origin is x.test, whatever its URL says"
        );
        assert!(
            !set.is_private_in(on_y.main_document().ok_or("main doc")?, node),
            "the document's origin is y.test, whatever its URL says"
        );
        Ok(())
    }

    #[test]
    fn debug_prints_counts_and_never_an_origin_nonce_or_id() -> Result<(), Box<dyn Error>> {
        let opaque_tree = SnapshotTree::build(
            ApcBuilder::new("doc-debug")
                .main_origin("nonce-secret-777", true)
                .root(NodeBuilder::root().dom_id(1))
                .build(),
            BuildOptions::new(),
        )?;
        let opaque = SiteKey::of_doc(opaque_tree.main_document().ok_or("main doc")?);
        let mut set = PrivateNodes::new();
        set.mark(
            Some(site("https://tenant-secret.example:8443")?),
            DomNodeId::new(4242),
        );
        set.mark(opaque, DomNodeId::new(4242));
        set.mark(None, DomNodeId::new(9191));

        let debug = format!("{set:?}");
        for forbidden in ["tenant-secret", "8443", "nonce-secret-777", "4242", "9191"] {
            assert!(
                !debug.contains(forbidden),
                "PrivateNodes Debug leaked {forbidden:?}: {debug}"
            );
        }
        assert_eq!(
            debug, "PrivateNodes { marked_ids: 2, site_marks: 2, anywhere_marks: 1 }",
            "Debug reports how many ids and marks the set holds"
        );
        Ok(())
    }
}
