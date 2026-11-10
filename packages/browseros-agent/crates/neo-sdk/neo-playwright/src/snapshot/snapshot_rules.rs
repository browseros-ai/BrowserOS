//! Neo's rule on top of Playwright's snapshot (design D20, decisions.md): a node whose role
//! ARIA prohibits naming never prints a quoted name, so every name an agent reads in a
//! snapshot is one `getByRole(role, { name })` can find.

use super::aria_snapshot::{AriaChild, AriaNode};

/// The roles ARIA prohibits naming: `getByRole(role, { name })` never matches a name on them.
/// The snapshot still names a role-less element (which it prints as `generic`) from its
/// `aria-label` or `title`.
///
/// A copy of the `elementProhibitsNaming` literal in vendored `roleUtils.ts`.
/// `tests/engine_sources.rs` reads both and fails when they differ, so an upgrade cannot change
/// one side alone.
pub(crate) const NAME_PROHIBITED_ROLES: &[&str] = &[
    "caption",
    "code",
    "definition",
    "deletion",
    "emphasis",
    "generic",
    "insertion",
    "mark",
    "paragraph",
    "presentation",
    "strong",
    "subscript",
    "suggestion",
    "superscript",
    "term",
    "time",
];

/// Applies the rule to `node` and every node below it. A named node with a prohibited role:
/// - with no `text` and no `children`, the name was its only text (upstream's distiller drops
///   a lone text child that repeats the name), so the name becomes the text:
///   `- generic [ref=e5]: Chrome 1`;
/// - otherwise the name is dropped and its content stays.
///
/// The ref stays in both cases, so the node can still be acted on.
pub(crate) fn drop_prohibited_names(node: &mut AriaNode) {
    if NAME_PROHIBITED_ROLES.contains(&node.role.as_str()) {
        let name = node.name.take();
        if node.text.is_none() && node.children.is_empty() {
            node.text = name;
        }
    }
    for child in &mut node.children {
        if let AriaChild::Node(child) = child {
            drop_prohibited_names(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::drop_prohibited_names;
    use crate::snapshot::aria_snapshot::AriaNode;
    use crate::snapshot::aria_snapshot::tests::{find, recorded};

    /// The recorded top frame, after the rule. Upstream printed `generic "Chrome 1" [ref=e4]`
    /// (`<div aria-label="Chrome 1">Chrome 1</div>`) and `generic "Status" [ref=e5]:` with a
    /// text and a button under it.
    fn ruled_top_frame() -> Result<Vec<AriaNode>, Box<dyn Error>> {
        let mut nodes = recorded()?.frames[0].json.clone();
        for node in &mut nodes {
            drop_prohibited_names(node);
        }
        Ok(nodes)
    }

    #[test]
    fn tests_that_a_generic_named_by_its_own_text_prints_that_text() -> Result<(), Box<dyn Error>> {
        let nodes = ruled_top_frame()?;
        let chrome = find(&nodes, "e4");
        assert_eq!(chrome.map(|node| node.name.as_deref()), Some(None));
        assert_eq!(
            chrome.map(|node| node.text.as_deref()),
            Some(Some("Chrome 1")),
            "`- generic [ref=e4]: Chrome 1`"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_generic_name_over_other_content_is_dropped_and_the_content_kept()
    -> Result<(), Box<dyn Error>> {
        let nodes = ruled_top_frame()?;
        let status = find(&nodes, "e5");
        assert!(status.is_some_and(|node| node.name.is_none() && node.text.is_none()));
        assert_eq!(
            status.map(|node| node.children.len()),
            Some(2),
            "the text and the button stay"
        );
        assert!(
            find(&nodes, "e6").is_some(),
            "the button inside keeps its ref"
        );
        Ok(())
    }

    #[test]
    fn tests_that_allowed_roles_and_unnamed_nodes_are_left_alone() -> Result<(), Box<dyn Error>> {
        let recorded = recorded()?;
        let nodes = ruled_top_frame()?;
        let own = |node: &AriaNode| (node.role.clone(), node.name.clone(), node.text.clone());
        for r#ref in ["e1", "e2", "e3", "e6", "e7", "e8", "e9"] {
            assert_eq!(
                find(&nodes, r#ref).map(own),
                find(&recorded.frames[0].json, r#ref).map(own),
                "{ref} has an allowed role or no name"
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_a_prohibited_name_beside_text_is_dropped_deep_in_the_tree()
    -> Result<(), Box<dyn Error>> {
        let mut node: AriaNode = serde_json::from_value(json!({
            "role": "list",
            "ref": "e1",
            "children": [{
                "role": "listitem",
                "name": "Step",
                "ref": "e2",
                "children": [
                    { "role": "code", "name": "Run tests", "ref": "e3", "text": "npm test" },
                    { "role": "strong", "name": "Note", "ref": "e4" },
                ],
            }],
        }))?;
        drop_prohibited_names(&mut node);
        assert_eq!(
            serde_json::to_value(&node)?,
            json!({
                "role": "list",
                "ref": "e1",
                "children": [{
                    "role": "listitem",
                    "name": "Step",
                    "ref": "e2",
                    "children": [
                        { "role": "code", "ref": "e3", "text": "npm test" },
                        { "role": "strong", "ref": "e4", "text": "Note" },
                    ],
                }],
            })
        );
        Ok(())
    }
}
