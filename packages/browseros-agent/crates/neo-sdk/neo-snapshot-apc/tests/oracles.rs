//! Offline checks of the recorded oracles (`tests/fixtures/<name>.oracle.json`): every registry
//! fixture has one beside its `.pb`, it reads as the shape the parity comparison joins on, and no
//! recorded file holds a typed secret.

#[path = "support/fixture_pages.rs"]
#[expect(
    dead_code,
    reason = "only the registry is read here; loading and capturing serve the recorder"
)]
mod fixture_pages;
#[path = "support/fixture_server.rs"]
#[expect(
    dead_code,
    reason = "the registry names fixture_server's Page; the server itself serves the recorder"
)]
mod fixture_server;
#[path = "support/oracle.rs"]
mod oracle;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use fixture_pages::{CANARY_PREFIX, FIXTURES};
use neo_snapshot::{AriaChild, AriaNode};
use neo_snapshot_apc::AnnotatedPageContent;
use neo_snapshot_apc::proto::{ContentData, ContentNode, FormControlType, IframeDataContent};
use oracle::Oracle;
use serde::Deserialize;
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn load_oracle(name: &str) -> Result<Oracle, Box<dyn Error>> {
    let path = fixtures_dir().join(format!("{name}.oracle.json"));
    let text = fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    // An ARIA tree nests two JSON levels per element; deep-nesting's passes serde's default 128.
    let mut reader = serde_json::Deserializer::from_str(&text);
    reader.disable_recursion_limit();
    let oracle =
        Oracle::deserialize(&mut reader).map_err(|error| format!("{}: {error}", path.display()))?;
    reader.end()?;
    Ok(oracle)
}

fn load_pb(name: &str) -> Result<AnnotatedPageContent, Box<dyn Error>> {
    let path = fixtures_dir().join(format!("{name}.pb"));
    let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(neo_snapshot_apc::decode_protobuf(&bytes)?)
}

/// `nodes` read as the snapshot printer's input type.
fn aria_nodes(value: &Value) -> Result<Vec<AriaNode>, serde_json::Error> {
    Vec::<AriaNode>::deserialize(value)
}

/// Every `ref` in `nodes`, children included.
fn printed_refs(nodes: &[AriaNode]) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    let mut pending: Vec<&AriaNode> = nodes.iter().collect();
    while let Some(node) = pending.pop() {
        refs.extend(node.r#ref.clone());
        for child in &node.children {
            if let AriaChild::Node(child) = child {
                pending.push(child);
            }
        }
    }
    refs
}

#[test]
fn every_registry_fixture_has_a_pb_and_an_oracle_of_its_own() -> Result<(), Box<dyn Error>> {
    for fixture in FIXTURES {
        load_pb(fixture.name)?;
        let oracle = load_oracle(fixture.name)?;
        assert_eq!(
            oracle.fixture, fixture.name,
            "an oracle names the fixture it was read from, so a misnamed file is caught"
        );
        assert!(
            !oracle.frames.is_empty(),
            "{}: every page has a main frame for the engine to read",
            fixture.name
        );
    }
    Ok(())
}

#[test]
fn oracle_frames_list_main_first_and_each_child_after_its_owned_parent()
-> Result<(), Box<dyn Error>> {
    for fixture in FIXTURES {
        let oracle = load_oracle(fixture.name)?;
        let (main, children) = oracle
            .frames
            .split_first()
            .ok_or("an oracle has at least the main frame")?;
        assert!(
            main.parent.is_none() && main.owner.is_none(),
            "{}: the main frame comes first and has no parent document to own it",
            fixture.name
        );
        let mut earlier: HashSet<&str> = HashSet::from([main.frame.as_str()]);
        for frame in children {
            let parent = frame
                .parent
                .as_deref()
                .ok_or_else(|| format!("{}: frame {} has no parent", fixture.name, frame.url))?;
            assert!(
                earlier.contains(parent),
                "{}: frame {} is listed depth first, after its parent, so seqs follow the tree",
                fixture.name,
                frame.url
            );
            assert!(
                frame.owner.is_some(),
                "{}: child frame {} has an owner, the <iframe> APC writes into its parent",
                fixture.name,
                frame.url
            );
            earlier.insert(&frame.frame);
        }
    }
    Ok(())
}

#[test]
fn backend_node_ids_are_unique_within_each_frame() -> Result<(), Box<dyn Error>> {
    for fixture in FIXTURES {
        let oracle = load_oracle(fixture.name)?;
        for frame in &oracle.frames {
            let mut seen = HashSet::new();
            for element in &frame.elements {
                assert!(
                    seen.insert(element.backend_node_id),
                    "{}: backendNodeId {} repeats in frame {}; it is the join key with the \
                     normalizer's nodes, so it must name one element",
                    fixture.name,
                    element.backend_node_id,
                    frame.url
                );
            }
        }
    }
    Ok(())
}

#[test]
fn every_oracle_element_has_a_role() -> Result<(), Box<dyn Error>> {
    for fixture in FIXTURES {
        let oracle = load_oracle(fixture.name)?;
        for frame in &oracle.frames {
            for element in &frame.elements {
                assert!(
                    !element.role.is_empty(),
                    "{}: the oracle lists only elements getAriaRole gives a role (node {})",
                    fixture.name,
                    element.backend_node_id
                );
            }
        }
    }
    Ok(())
}

#[test]
fn aria_trees_read_as_neo_snapshot_nodes_and_every_printed_ref_resolves()
-> Result<(), Box<dyn Error>> {
    for fixture in FIXTURES {
        let oracle = load_oracle(fixture.name)?;
        for frame in &oracle.frames {
            let ai = aria_nodes(&frame.aria.ai)
                .map_err(|error| format!("{}: ai tree of {}: {error}", fixture.name, frame.url))?;
            let default = aria_nodes(&frame.aria.default).map_err(|error| {
                format!("{}: default tree of {}: {error}", fixture.name, frame.url)
            })?;
            let printed = printed_refs(&ai);
            let recorded: BTreeSet<String> = frame.refs.keys().cloned().collect();
            // Not equality: the engine's distiller drops some nodes from the printed tree but
            // keeps their refs resolvable (ariaSnapshotDistiller.ts), so `refs` may hold more.
            let unresolved: Vec<&String> = printed.difference(&recorded).collect();
            assert!(
                unresolved.is_empty(),
                "{}: refs {unresolved:?} printed in {}'s ai tree have no element in `refs`, so \
                 the comparison could not join them",
                fixture.name,
                frame.url
            );
            assert!(
                printed_refs(&default).is_empty(),
                "{}: default mode mints no refs (Playwright's toInternalOptions), in {}",
                fixture.name,
                frame.url
            );
        }
    }
    Ok(())
}

#[test]
fn no_recorded_fixture_holds_a_typed_secret() -> Result<(), Box<dyn Error>> {
    let mut checked = 0;
    for entry in fs::read_dir(fixtures_dir())? {
        let path = entry?.path();
        let bytes = fs::read(&path)?;
        assert!(
            !bytes
                .windows(CANARY_PREFIX.len())
                .any(|window| window == CANARY_PREFIX.as_bytes()),
            "{} holds a typed secret: the recorder's canary guard should have refused to write it",
            path.display()
        );
        checked += 1;
    }
    assert!(
        checked >= 2 * FIXTURES.len(),
        "the search must have read every fixture's .pb and oracle"
    );
    Ok(())
}

/// One document of a decoded APC tree: its URL without the port, the DOM node ids of the
/// `<iframe>` nodes directly in it, and of its `<select>` nodes with how many options each sent.
#[derive(Debug)]
struct ApcDocument {
    url: Option<String>,
    iframe_owners: HashSet<i32>,
    selects: HashMap<i32, usize>,
}

/// The documents of `apc`, main first. An iframe node with `FrameData` holds its child
/// document's nodes; one without (a frame APC left empty) holds none.
fn apc_documents(apc: &AnnotatedPageContent) -> Vec<ApcDocument> {
    let main_url = apc
        .main_frame_data
        .as_ref()
        .and_then(|frame| frame.url.as_deref())
        .map(oracle::recorded_url);
    let mut documents = vec![ApcDocument {
        url: main_url,
        iframe_owners: HashSet::new(),
        selects: HashMap::new(),
    }];
    let mut pending: Vec<(&ContentNode, usize)> =
        apc.root_node.iter().map(|node| (node, 0)).collect();
    while let Some((node, document)) = pending.pop() {
        let mut children_document = document;
        if let Some(attrs) = node.content_attributes.as_ref()
            && let Some(ContentData::FormControl(control)) = attrs.content_data.as_ref()
            && control.form_control_type.is_some_and(|kind| {
                kind == FormControlType::SelectOne as i32
                    || kind == FormControlType::SelectMultiple as i32
            })
            && let (Some(selects), Some(id)) = (
                documents.get_mut(document).map(|doc| &mut doc.selects),
                attrs.common_ancestor_dom_node_id,
            )
        {
            selects.insert(id, control.select_options.len());
        }
        if let Some(attrs) = node.content_attributes.as_ref()
            && let Some(ContentData::Iframe(iframe)) = attrs.content_data.as_ref()
        {
            if let (Some(owners), Some(id)) = (
                documents.get_mut(document),
                attrs.common_ancestor_dom_node_id,
            ) {
                owners.iframe_owners.insert(id);
            }
            let url = match iframe.data.as_ref() {
                Some(IframeDataContent::FrameData(frame)) => {
                    frame.url.as_deref().map(oracle::recorded_url)
                }
                _ => None,
            };
            children_document = documents.len();
            documents.push(ApcDocument {
                url,
                iframe_owners: HashSet::new(),
                selects: HashMap::new(),
            });
        }
        for child in &node.children_nodes {
            pending.push((child, children_document));
        }
    }
    documents
}

#[test]
fn child_frame_owners_are_iframe_nodes_of_the_same_load_pb() -> Result<(), Box<dyn Error>> {
    for fixture in FIXTURES {
        let oracle = load_oracle(fixture.name)?;
        let documents = apc_documents(&load_pb(fixture.name)?);
        for frame in oracle.frames.iter().skip(1) {
            let (Some(parent), Some(owner)) = (frame.parent.as_deref(), frame.owner) else {
                continue;
            };
            let parent_url = oracle
                .frames
                .iter()
                .find(|candidate| candidate.frame == parent)
                .map(|candidate| candidate.url.as_str())
                .ok_or("a child frame's parent is in the same oracle")?;
            let owner = i32::try_from(owner)?;
            // DOM node ids are per renderer, so the owner is looked up in its parent's document
            // only: the same number in another process's document is another element.
            let in_parent = documents.iter().any(|document| {
                document.url.as_deref() == Some(parent_url)
                    && document.iframe_owners.contains(&owner)
            });
            assert!(
                in_parent,
                "{}: frame {}'s owner {owner} is no <iframe> node of {parent_url} in the .pb; \
                 both files come from one page load, so backendNodeIds must agree",
                fixture.name, frame.url
            );
        }
    }
    Ok(())
}

#[test]
fn every_recorded_option_place_names_a_select_of_the_same_load_pb() -> Result<(), Box<dyn Error>> {
    for fixture in FIXTURES {
        let oracle = load_oracle(fixture.name)?;
        let documents = apc_documents(&load_pb(fixture.name)?);
        for frame in &oracle.frames {
            for element in &frame.elements {
                let Some(option) = &element.option else {
                    continue;
                };
                assert!(
                    option.index < option.count,
                    "{}: option {} sits at {} of a list of {}; select.options holds it, so its \
                     index is below the list's length",
                    fixture.name,
                    element.backend_node_id,
                    option.index,
                    option.count
                );
                let select = i32::try_from(option.select)?;
                // DOM node ids are per renderer, so the select is looked up in the option's own
                // frame's document only.
                let sent = documents
                    .iter()
                    .filter(|document| document.url.as_deref() == Some(frame.url.as_str()))
                    .find_map(|document| document.selects.get(&select).copied());
                let Some(sent) = sent else {
                    panic!(
                        "{}: option {}'s select {select} is no <select> node of {} in the .pb; \
                         both files come from one page load, so backendNodeIds must agree",
                        fixture.name, element.backend_node_id, frame.url
                    );
                };
                // Chromium fills select_options from select.options (GetOptionList()), the list
                // the oracle indexes, so a select that sent its options sent this many: the
                // count the option's slot is joined by.
                assert!(
                    sent == 0 || usize::try_from(option.count)? == sent,
                    "{}: option {}'s select {select} sent {sent} options in the .pb, but the \
                     page listed {} on the same load",
                    fixture.name,
                    element.backend_node_id,
                    option.count
                );
            }
        }
    }
    Ok(())
}
