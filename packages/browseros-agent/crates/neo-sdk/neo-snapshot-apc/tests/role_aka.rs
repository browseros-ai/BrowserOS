//! `select::role_aka`, the tree's `getByRole` locator for an element a strict-mode violation lists
//! (apc-engine-design 5.5): upstream's role-with-name candidate, kept only when it selects the
//! element alone in its document, the non-exact form first and the exact one after it.

use std::error::Error;

use neo_snapshot_apc::select::{RefResolver, role_aka};
use neo_snapshot_apc::tree::{
    ApcBuilder, BuildOptions, DocToken, DomNodeId, NameCache, NodeBuilder, NodeId, NodeKey,
    SnapshotTree,
};

const MAIN: &str = "doc-main";

/// No ref resolves: a role candidate has no `aka` part.
struct NoRefs;

impl RefResolver for NoRefs {
    fn resolve(&self, _body: &str) -> Option<NodeKey> {
        None
    }
}

fn button(dom: i32, label: &str) -> NodeBuilder {
    NodeBuilder::element("button")
        .dom_id(dom)
        .bounding_box(0, 0, 80, 20)
        .child(NodeBuilder::text(label))
}

fn tree(children: Vec<NodeBuilder>) -> Result<SnapshotTree, Box<dyn Error>> {
    let apc = ApcBuilder::new(MAIN)
        .root(NodeBuilder::root().children(children))
        .build();
    Ok(SnapshotTree::build(apc, BuildOptions::default())?)
}

fn node(tree: &SnapshotTree, dom: i32) -> Result<NodeId, Box<dyn Error>> {
    let token = DocToken::new(MAIN).ok_or("a test token is never empty")?;
    Ok(tree
        .node_by_key(&NodeKey::new(token, DomNodeId::new(dom)))
        .ok_or_else(|| format!("no node {dom}"))?
        .id())
}

fn aka(tree: &SnapshotTree, dom: i32) -> Result<Option<String>, Box<dyn Error>> {
    let names = NameCache::new(tree);
    Ok(role_aka(tree, &names, &NoRefs, node(tree, dom)?))
}

#[test]
fn a_unique_name_is_the_non_exact_role_locator() -> Result<(), Box<dyn Error>> {
    let page = tree(vec![button(1, "One"), button(2, "Two")])?;
    assert_eq!(
        aka(&page, 1)?.as_deref(),
        Some("getByRole('button', { name: 'One' })")
    );
    assert_eq!(
        aka(&page, 2)?.as_deref(),
        Some("getByRole('button', { name: 'Two' })")
    );
    Ok(())
}

#[test]
fn a_name_another_contains_takes_the_exact_form() -> Result<(), Box<dyn Error>> {
    let page = tree(vec![button(1, "Save"), button(2, "Save draft")])?;
    assert_eq!(
        aka(&page, 1)?.as_deref(),
        Some("getByRole('button', { name: 'Save', exact: true })"),
        "a non-exact `Save` also matches `Save draft`"
    );
    assert_eq!(
        aka(&page, 2)?.as_deref(),
        Some("getByRole('button', { name: 'Save draft' })")
    );
    Ok(())
}

#[test]
fn a_name_no_candidate_makes_unique_offers_none() -> Result<(), Box<dyn Error>> {
    let page = tree(vec![button(1, "Same"), button(2, "Same")])?;
    assert_eq!(aka(&page, 1)?, None);
    assert_eq!(aka(&page, 2)?, None);
    Ok(())
}

#[test]
fn a_nameless_or_presentational_element_offers_none() -> Result<(), Box<dyn Error>> {
    let page = tree(vec![
        NodeBuilder::element("button")
            .dom_id(1)
            .bounding_box(0, 0, 80, 20),
        NodeBuilder::element("div")
            .dom_id(2)
            .role("presentation")
            .child(NodeBuilder::text("Decor")),
    ])?;
    assert_eq!(aka(&page, 1)?, None, "a role with no name");
    assert_eq!(aka(&page, 2)?, None, "`presentation` is no role to name");
    Ok(())
}

#[test]
fn a_name_with_quotes_prints_as_as_locator_quotes_it() -> Result<(), Box<dyn Error>> {
    let page = tree(vec![button(1, "Bob's \"pick\"")])?;
    assert_eq!(
        aka(&page, 1)?.as_deref(),
        Some(r#"getByRole('button', { name: 'Bob\'s "pick"' })"#)
    );
    Ok(())
}
