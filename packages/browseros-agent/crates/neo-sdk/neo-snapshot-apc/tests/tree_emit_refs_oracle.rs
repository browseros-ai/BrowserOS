//! Offline parity checks of [`neo_snapshot_apc::tree::SnapshotTree::aria`],
//! [`neo_snapshot_apc::refs::TabRefTable`] and [`neo_snapshot::render`] against the 16 recorded
//! Playwright oracles (`tests/fixtures/<name>.{pb,oracle.json}`).
//!
//! Every recording carries contract v4. Each page is emitted in both modes, rendered in every
//! form, and checked for canary bytes, refs that resolve and an unchanged repeat delta. Its
//! ai-mode tree is then compared with the oracle's: ref-worthiness on every joined element, and
//! role, name and state on every node both sides print. Those disagreements must match the parity
//! ledger (`support/parity_ledger.rs`). The printed textboxes' values and the collapsed selects'
//! options must equal the oracle's outright, and so must the default-mode tree, except on the
//! pages [`DEFAULT_TREE_DIFFERS`] names. The edge pages then get the checks only they can carry:
//! `ua_shadow` folding, masking, inert marks and frame refs.

#[path = "support/fixture_pages.rs"]
#[expect(
    dead_code,
    reason = "only FIXTURES and CANARY_PREFIX are read here; page loaders serve the recorder"
)]
mod fixture_pages;
#[path = "support/fixture_server.rs"]
#[expect(
    dead_code,
    reason = "fixture_pages names fixture_server::Page; the server serves the recorder"
)]
mod fixture_server;
#[path = "support/oracle.rs"]
#[expect(
    dead_code,
    reason = "oracle::recorded_url serves tests/oracles.rs; only Oracle types are read here"
)]
mod oracle;
#[path = "support/parity_ledger.rs"]
#[expect(
    dead_code,
    reason = "this test measures ref and the printed nodes' role, name and states; description \
              and the other tree facts belong to the tree oracle tests"
)]
mod parity_ledger;

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::PathBuf;
use std::{fs, mem};

use fixture_pages::{CANARY_PREFIX, FIXTURES};
use neo_snapshot::{AriaChild, AriaNode, Baseline, Call, Earlier, Passed};
use neo_snapshot_apc::proto::AnnotatedPageContent;
use neo_snapshot_apc::refs::{RefLookup, RefTarget, TabRefTable};
use neo_snapshot_apc::tree::{
    AriaAsk, AriaTree, BuildOptions, DocId, DomNodeId, MAX_JSON_DEPTH, NodeKey, Role, SnapshotTree,
    TreeNode,
};
use oracle::{Oracle, OracleFrame};
use parity_ledger::{Field, Measured};
use serde::Deserialize;
use serde_json::{Deserializer, Number, Value};

/// The first contract version whose recordings carry roles, states and redaction decisions.
const CONTRACT_V4: u32 = 4;
const NO_CHANGE_LINE: &str = "# no change since your last snapshot of this tab";
/// Roles whose name ACCNAME prohibits (D20): Neo prints such a name as the node's text instead.
const NAME_PROHIBITED_ROLES: &[&str] = &[
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
/// The printed-node facts compared with the oracle's ai tree.
const AI_NODE_FIELDS: [Field; 10] = [
    Field::Role,
    Field::Name,
    Field::Checked,
    Field::Disabled,
    Field::Expanded,
    Field::Active,
    Field::Invalid,
    Field::Level,
    Field::Pressed,
    Field::Selected,
];

/// Pages whose default-mode tree differs from the oracle's, and why. A page that starts to match
/// fails until it leaves the list.
const DEFAULT_TREE_DIFFERS: &[(&str, &str)] = &[
    (
        "hung-subframe",
        "the hung frame has no document in the .pb (ledger Join, fixture design), so the tree \
         holds a partial frame where the oracle holds the frame's content",
    ),
    (
        "states",
        "a nested aria-disabled=\"false\" does not stop inheritance (ledger Disabled, G2)",
    ),
];
/// Pages whose ai tree pairs zero-size controls with their labels (design 3.5), and how many.
const CLICK_PROXIES: &[(&str, usize)] = &[("zero-size-checkbox", 2), ("zero-size-controls", 4)];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn load_pb(name: &str) -> Result<AnnotatedPageContent, Box<dyn Error>> {
    let path = fixtures_dir().join(format!("{name}.pb"));
    let bytes = fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(neo_snapshot_apc::decode_protobuf(&bytes)?)
}

fn load_oracle(name: &str) -> Result<Oracle, Box<dyn Error>> {
    let path = fixtures_dir().join(format!("{name}.oracle.json"));
    let text = fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut reader = Deserializer::from_str(&text);
    reader.disable_recursion_limit();
    let oracle =
        Oracle::deserialize(&mut reader).map_err(|err| format!("{}: {err}", path.display()))?;
    reader.end()?;
    Ok(oracle)
}

fn parse_oracle_aria(value: &Value) -> Result<Vec<AriaNode>, Box<dyn Error>> {
    let text = serde_json::to_string(value)?;
    let mut reader = Deserializer::from_str(&text);
    reader.disable_recursion_limit();
    let nodes = Vec::<AriaNode>::deserialize(&mut reader)?;
    reader.end()?;
    Ok(nodes)
}

/// Applies D20 to the oracle's tree as the emitter applies it to Neo's.
fn normalize_oracle_d20(nodes: &mut [AriaNode]) {
    let mut stack: Vec<&mut AriaNode> = nodes.iter_mut().collect();
    while let Some(node) = stack.pop() {
        if NAME_PROHIBITED_ROLES.contains(&node.role.as_str()) {
            let taken = node.name.take();
            if node.text.is_none() && node.children.is_empty() {
                node.text = taken;
            }
        }
        for child in &mut node.children {
            if let AriaChild::Node(child_node) = child {
                stack.push(child_node);
            }
        }
    }
}

/// Hangs each child frame's oracle tree under its `iframe` node, in the order the recorder lists
/// frames, which is the order their iframes appear in the stitched tree.
fn stitch_oracle_trees(mut per_frame: Vec<Vec<AriaNode>>) -> Result<Vec<AriaNode>, Box<dyn Error>> {
    if per_frame.is_empty() {
        return Ok(Vec::new());
    }
    let mut cursor = 1_usize;
    let mut root_nodes = per_frame
        .first_mut()
        .map(mem::take)
        .ok_or("missing root frame nodes")?;

    let mut stack: Vec<&mut AriaNode> = root_nodes.iter_mut().rev().collect();
    while let Some(node) = stack.pop() {
        if node.role == "iframe" {
            let child_nodes = per_frame
                .get_mut(cursor)
                .map(mem::take)
                .ok_or_else(|| format!("missing child oracle frame at preorder index {cursor}"))?;
            cursor += 1;
            node.children = child_nodes
                .into_iter()
                .map(|child| AriaChild::Node(Box::new(child)))
                .collect();
        }
        for child in node.children.iter_mut().rev() {
            if let AriaChild::Node(child_node) = child {
                stack.push(child_node);
            }
        }
    }

    if cursor != per_frame.len() {
        return Err(format!(
            "{} oracle frames, but the stitched tree has {cursor} documents",
            per_frame.len()
        )
        .into());
    }
    Ok(root_nodes)
}

/// A recorded page: the tree built from its `.pb`, its oracle, and the tree document behind each
/// oracle frame. Frames join by their `owner` `<iframe>`, elements by `backendNodeId`.
struct RecordedPage {
    name: &'static str,
    tree: SnapshotTree,
    oracle: Oracle,
    frame_to_doc: HashMap<String, DocId>,
    /// Oracle frames with no document in the tree: their capture timed out or was redacted.
    partial_frames: usize,
}

impl RecordedPage {
    fn load(name: &'static str) -> Result<Self, Box<dyn Error>> {
        let apc = load_pb(name)?;
        let recorded = apc
            .browseros
            .as_ref()
            .and_then(|meta| meta.contract_version);
        assert!(
            recorded >= Some(CONTRACT_V4),
            "{name}: recorded at contract {recorded:?}; every recording carries v4 \
             (tests/fixtures/README.md)"
        );
        let tree = SnapshotTree::build(apc, BuildOptions::default())?;
        let oracle = load_oracle(name)?;
        let mut page = Self {
            name,
            tree,
            oracle,
            frame_to_doc: HashMap::new(),
            partial_frames: 0,
        };
        page.map_frames()?;
        Ok(page)
    }

    fn map_frames(&mut self) -> Result<(), Box<dyn Error>> {
        let main_frame = self
            .oracle
            .frames
            .first()
            .ok_or_else(|| format!("{}: oracle has no main frame", self.name))?;
        self.frame_to_doc
            .insert(main_frame.frame.clone(), DocId::MAIN);

        for frame in self.oracle.frames.iter().skip(1) {
            let parent_frame = frame
                .parent
                .as_deref()
                .ok_or_else(|| format!("{}: child frame {} lacks parent", self.name, frame.url))?;
            let owner = frame
                .owner
                .ok_or_else(|| format!("{}: child frame {} lacks owner", self.name, frame.url))?;
            let Some(&parent_doc_id) = self.frame_to_doc.get(parent_frame) else {
                self.partial_frames += 1;
                continue;
            };
            let parent_doc = self
                .tree
                .document(parent_doc_id)
                .ok_or("mapped parent doc id missing from SnapshotTree")?;
            let key = NodeKey::new(
                parent_doc.token().clone(),
                DomNodeId::new(i32::try_from(owner)?),
            );
            let iframe = self.tree.node_by_key(&key).ok_or_else(|| {
                format!(
                    "{}: owner iframe {owner} for {} not found in parent doc",
                    self.name, frame.url
                )
            })?;
            if let Some(child_doc) = iframe.child_doc() {
                self.frame_to_doc.insert(frame.frame.clone(), child_doc);
            } else {
                self.partial_frames += 1;
            }
        }
        Ok(())
    }

    /// The tree key of `backend_node_id` in the document behind `frame`, when it has one.
    fn key(
        &self,
        frame: &OracleFrame,
        backend_node_id: u64,
    ) -> Result<Option<NodeKey>, Box<dyn Error>> {
        let Some(&doc_id) = self.frame_to_doc.get(&frame.frame) else {
            return Ok(None);
        };
        let doc = self
            .tree
            .document(doc_id)
            .ok_or("mapped doc id missing from SnapshotTree")?;
        Ok(Some(NodeKey::new(
            doc.token().clone(),
            DomNodeId::new(i32::try_from(backend_node_id)?),
        )))
    }

    /// Each frame's oracle ai tree under D20, with its printed nodes indexed by `backendNodeId`.
    fn oracle_ai_frames(&self) -> Result<Vec<Vec<AriaNode>>, Box<dyn Error>> {
        self.oracle
            .frames
            .iter()
            .map(|frame| {
                let mut nodes = parse_oracle_aria(&frame.aria.ai)?;
                normalize_oracle_d20(&mut nodes);
                Ok(nodes)
            })
            .collect()
    }
}

/// The parity ledger's key for an oracle frame: `None` for the main frame, the only one with no
/// `parent`, else the child frame's recorded URL.
fn ledger_frame(frame: &OracleFrame) -> Option<String> {
    frame.parent.as_ref().map(|_| frame.url.clone())
}

fn collect_all_nodes(nodes: &[AriaNode]) -> Vec<&AriaNode> {
    let mut out = Vec::new();
    let mut stack: Vec<&AriaNode> = nodes.iter().rev().collect();
    while let Some(node) = stack.pop() {
        out.push(node);
        for child in node.children.iter().rev() {
            if let AriaChild::Node(child_node) = child {
                stack.push(child_node);
            }
        }
    }
    out
}

fn collect_printed_refs(nodes: &[AriaNode]) -> Vec<String> {
    collect_all_nodes(nodes)
        .into_iter()
        .filter_map(|node| node.r#ref.clone())
        .collect()
}

type EmittedByKey<'a> = HashMap<NodeKey, (&'a AriaNode, RefTarget)>;

/// Every printed node of an ai tree by the key its ref resolves to in `refs`.
fn index_emitted_by_key<'a>(
    nodes: &'a [AriaNode],
    refs: &TabRefTable,
) -> Result<EmittedByKey<'a>, Box<dyn Error>> {
    let mut by_key = HashMap::new();
    for node in collect_all_nodes(nodes) {
        let Some(ref_str) = node.r#ref.as_deref() else {
            continue;
        };
        let RefLookup::Key(target) = refs.resolve_str(ref_str) else {
            return Err(format!("emitted ref {ref_str:?} failed to resolve to a live key").into());
        };
        by_key.insert(target.key.clone(), (node, target));
    }
    Ok(by_key)
}

fn assert_no_canary(context: &str, label: &str, text: &str) {
    assert!(
        !text.contains(CANARY_PREFIX),
        "{context}: canary leaked into {label}"
    );
}

struct RenderedBundle {
    ai_playwright: String,
    ai_full: String,
    ai_page: String,
    default_playwright: String,
}

/// Renders both trees in every form and checks each output, and every `Debug` form that holds
/// page state, for canary bytes. Returns the texts and the baseline a later delta compares with.
fn render_and_check_safety(
    context: &str,
    tree: &SnapshotTree,
    ai: &AriaTree,
    default: AriaTree,
    refs: &TabRefTable,
) -> Result<(RenderedBundle, Baseline), Box<dyn Error>> {
    assert!(
        neo_snapshot::json_depth(ai.nodes()) < MAX_JSON_DEPTH,
        "{context}: ai json_depth {} must stay below {MAX_JSON_DEPTH}",
        neo_snapshot::json_depth(ai.nodes())
    );
    assert!(
        neo_snapshot::json_depth(default.nodes()) < MAX_JSON_DEPTH,
        "{context}: default json_depth {} must stay below {MAX_JSON_DEPTH}",
        neo_snapshot::json_depth(default.nodes())
    );
    assert!(
        collect_printed_refs(default.nodes()).is_empty(),
        "{context}: default mode must emit zero refs"
    );

    let ai_json = serde_json::to_string(ai.nodes())?;
    let default_json = serde_json::to_string(default.nodes())?;
    assert_no_canary(context, "ai JSON", &ai_json);
    assert_no_canary(context, "default JSON", &default_json);
    assert_no_canary(context, "SnapshotTree Debug", &format!("{tree:?}"));
    assert_no_canary(context, "ai AriaTree Debug", &format!("{ai:?}"));
    assert_no_canary(context, "default AriaTree Debug", &format!("{default:?}"));
    assert_no_canary(context, "TabRefTable Debug", &format!("{refs:?}"));

    let passed = Passed::default();
    let ai_snap = ai.to_snapshot();
    let default_snap = default.into_snapshot();

    let ai_playwright = neo_snapshot::render(&ai_snap, Call::Playwright).into_text();
    let ai_full = neo_snapshot::render(&ai_snap, Call::Full { max_tokens: None }).into_text();
    let ai_page_answer = neo_snapshot::render(&ai_snap, Call::Page(&passed));
    let pending = ai_page_answer
        .pending()
        .ok_or_else(|| format!("{context}: Call::Page must return a pending baseline"))?;
    let baseline = Baseline {
        text: pending.text.to_owned(),
        form: pending.form,
    };
    let ai_page = ai_page_answer.into_text();

    let default_playwright = neo_snapshot::render(&default_snap, Call::Playwright).into_text();
    let default_full =
        neo_snapshot::render(&default_snap, Call::Full { max_tokens: None }).into_text();
    let default_page = neo_snapshot::render(&default_snap, Call::Page(&passed)).into_text();

    for (label, text) in [
        ("ai Call::Playwright", ai_playwright.as_str()),
        ("ai Call::Full", ai_full.as_str()),
        ("ai Call::Page", ai_page.as_str()),
        ("default Call::Playwright", default_playwright.as_str()),
        ("default Call::Full", default_full.as_str()),
        ("default Call::Page", default_page.as_str()),
    ] {
        assert_no_canary(context, label, text);
    }
    assert!(
        !ai_playwright.is_empty() && !default_playwright.is_empty(),
        "{context}: rendered Playwright text must be non-empty"
    );

    Ok((
        RenderedBundle {
            ai_playwright,
            ai_full,
            ai_page,
            default_playwright,
        },
        baseline,
    ))
}

/// A second capture of the same page through the same table keeps every ref and renders as an
/// unchanged delta.
fn assert_idempotent_delta(
    context: &str,
    tree: &SnapshotTree,
    first_ai: &AriaTree,
    baseline: &Baseline,
    refs: &mut TabRefTable,
) -> Result<(), Box<dyn Error>> {
    let first_refs = collect_printed_refs(first_ai.nodes());
    let second_ai = tree.aria(AriaAsk::ai(refs))?;
    let second_refs = collect_printed_refs(second_ai.nodes());
    assert_eq!(
        second_refs, first_refs,
        "{context}: repeated ai emission with the same TabRefTable must preserve identical refs"
    );
    let passed = Passed::default();
    let second_snap = second_ai.into_snapshot();
    let delta = neo_snapshot::render(
        &second_snap,
        Call::Delta {
            passed: &passed,
            earlier: Earlier::Text(baseline),
        },
    )
    .into_text();
    assert_no_canary(context, "ai Call::Delta", &delta);
    assert_eq!(
        delta, NO_CHANGE_LINE,
        "{context}: repeated identical ai capture must render as unchanged delta"
    );
    Ok(())
}

/// Every printed ref resolves to a live node of the tree. Returns how many resolved.
fn assert_all_refs_resolve_in_tree(
    context: &str,
    tree: &SnapshotTree,
    ai: &AriaTree,
    refs: &TabRefTable,
) -> Result<usize, Box<dyn Error>> {
    let mut resolved_count = 0;
    for node in collect_all_nodes(ai.nodes()) {
        let Some(ref_str) = node.r#ref.as_deref() else {
            continue;
        };
        let lookup = refs.resolve_str(ref_str);
        let RefLookup::Key(target) = lookup else {
            return Err(format!(
                "{context}: emitted ref {ref_str:?} resolved to {lookup:?} instead of RefLookup::Key"
            )
            .into());
        };
        assert!(
            tree.node_by_key(&target.key).is_some(),
            "{context}: emitted ref {ref_str:?} points to missing node {:?}",
            target.key
        );
        resolved_count += 1;
    }
    Ok(resolved_count)
}

/// What one page's comparison covered, printed per page so a run shows where parity stands.
#[derive(Debug, Default)]
struct PageCounts {
    /// Oracle elements with a node in the tree.
    joined: usize,
    /// Joined elements whose role can carry a ref, compared on ref-worthiness.
    ref_compared: usize,
    /// Of those, the ones the tree pairs with a label (`ClickProxy`).
    proxied: usize,
    /// Nodes both ai trees print, compared on role, name and state.
    printed_compared: usize,
    /// Of those, the textboxes whose printed value was compared.
    values_compared: usize,
    /// Of those, the comboboxes whose option children were compared.
    selects_compared: usize,
    /// Printed refs that resolve to a live node.
    resolved_refs: usize,
}

impl PageCounts {
    fn add(&mut self, other: &Self) {
        self.joined += other.joined;
        self.ref_compared += other.ref_compared;
        self.proxied += other.proxied;
        self.printed_compared += other.printed_compared;
        self.values_compared += other.values_compared;
        self.selects_compared += other.selects_compared;
        self.resolved_refs += other.resolved_refs;
    }
}

/// Ref-worthiness of every joined element whose role can carry a ref: the tree's rule against the
/// oracle's refs, which list every element the ai snapshot gave a ref, printed or distilled away.
/// A `ClickProxy` pairing (design 3.5) is the one place the tree gives a ref on purpose where
/// Playwright gives none.
fn measure_refs(
    page: &RecordedPage,
    counts: &mut PageCounts,
    measured: &mut Vec<Measured>,
) -> Result<(), Box<dyn Error>> {
    for frame in &page.oracle.frames {
        let oracle_refs: HashSet<u64> = frame.refs.values().copied().collect();
        for element in &frame.elements {
            let Some(key) = page.key(frame, element.backend_node_id)? else {
                continue;
            };
            let Some(node) = page.tree.node_by_key(&key) else {
                continue;
            };
            counts.joined += 1;
            if matches!(element.role.as_str(), "document" | "presentation" | "none") {
                continue;
            }
            counts.ref_compared += 1;
            let proxied = page.tree.click_proxy(node.id()).is_some();
            counts.proxied += usize::from(proxied);
            assert!(
                !(proxied && oracle_refs.contains(&element.backend_node_id)),
                "{} node {}: a ClickProxy pairs only a control Playwright gives no ref",
                page.name,
                element.backend_node_id
            );
            let tree_ref = page.tree.is_ref_worthy(node.id());
            let oracle_ref = oracle_refs.contains(&element.backend_node_id) || proxied;
            if tree_ref != oracle_ref {
                measured.push(Measured {
                    fixture: page.name,
                    frame: ledger_frame(frame),
                    backend_node_id: element.backend_node_id,
                    field: Field::Ref,
                    tree: format!("{tree_ref:?}"),
                    detail: format!(
                        "{} [{}] node {} ({:?}, visible={}): Ref tree={tree_ref} oracle={oracle_ref}",
                        page.name, frame.url, element.backend_node_id, element.role, element.visible
                    ),
                });
            }
        }
    }
    Ok(())
}

/// One printed node's compared facts, in the form the tree oracle tests give the ledger's tree
/// side: the name verbatim, the role as the tree parses it, each state in `Debug` form.
fn printed_facts(node: &AriaNode) -> [(Field, String); 10] {
    let level = node.level.as_ref().and_then(Number::as_u64);
    [
        (Field::Role, format!("{:?}", Role::parse(&node.role))),
        (Field::Name, node.name.clone().unwrap_or_default()),
        (Field::Checked, format!("{:?}", node.checked)),
        (Field::Disabled, format!("{:?}", node.disabled)),
        (Field::Expanded, format!("{:?}", node.expanded)),
        (Field::Active, format!("{:?}", node.active == Some(true))),
        (Field::Invalid, format!("{:?}", node.invalid)),
        (Field::Level, format!("{level:?}")),
        (Field::Pressed, format!("{:?}", node.pressed)),
        (Field::Selected, format!("{:?}", node.selected)),
    ]
}

/// A printed node's `option` children: the options a collapsed `<select>` lists without refs.
fn printed_options(node: &AriaNode) -> Vec<&AriaNode> {
    node.children
        .iter()
        .filter_map(|child| match child {
            AriaChild::Node(option) if option.role == "option" => Some(&**option),
            _ => None,
        })
        .collect()
}

/// Role, name and each state of every node both ai trees print. The tree's node is found through
/// its printed ref, the oracle's through the recorded ref map.
fn measure_printed_nodes(
    page: &RecordedPage,
    emitted: &EmittedByKey<'_>,
    counts: &mut PageCounts,
    measured: &mut Vec<Measured>,
) -> Result<(), Box<dyn Error>> {
    for (frame, oracle_ai) in page.oracle.frames.iter().zip(page.oracle_ai_frames()?) {
        for oracle_node in collect_all_nodes(&oracle_ai) {
            let Some(&bid) = oracle_node
                .r#ref
                .as_deref()
                .and_then(|ref_str| frame.refs.get(ref_str))
            else {
                continue;
            };
            let Some(key) = page.key(frame, bid)? else {
                continue;
            };
            let Some((tree_node, _)) = emitted.get(&key) else {
                continue;
            };
            counts.printed_compared += 1;
            if tree_node.role == "textbox" {
                counts.values_compared += 1;
                assert_eq!(
                    tree_node.text, oracle_node.text,
                    "{} node {bid}: a textbox prints its value, masked where the page masker masks it",
                    page.name
                );
            }
            if tree_node.role == "combobox" {
                counts.selects_compared += 1;
                assert_eq!(
                    printed_options(tree_node),
                    printed_options(oracle_node),
                    "{} node {bid}: a collapsed select's options are synthesized from \
                     select_options (fold.rs SyntheticOption) as Playwright lists them",
                    page.name
                );
            }
            for ((field, tree), (_, oracle)) in printed_facts(tree_node)
                .into_iter()
                .zip(printed_facts(oracle_node))
            {
                if tree != oracle {
                    measured.push(Measured {
                        fixture: page.name,
                        frame: ledger_frame(frame),
                        backend_node_id: bid,
                        field,
                        detail: format!(
                            "{} [{}] node {bid} ({}): printed {field:?} tree={tree:?} oracle={oracle:?}",
                            page.name, frame.url, oracle_node.role
                        ),
                        tree,
                    });
                }
            }
        }
    }
    Ok(())
}

/// Every printed ref that carries a click proxy names the label its control is paired with, and
/// that label prints no line of its own: its text is the control's name. Returns how many.
fn assert_click_proxies(
    page: &RecordedPage,
    emitted: &EmittedByKey<'_>,
) -> Result<usize, Box<dyn Error>> {
    let mut proxied = 0;
    for (key, (_, target)) in emitted {
        let Some(label_key) = &target.click_proxy else {
            continue;
        };
        proxied += 1;
        let control = page
            .tree
            .node_by_key(key)
            .ok_or_else(|| format!("{}: proxied control {key:?} missing", page.name))?;
        assert_eq!(
            page.tree
                .click_proxy(control.id())
                .map(|proxy| &proxy.label_key),
            Some(label_key),
            "{}: the printed ref's click proxy must be the tree's pairing",
            page.name
        );
        assert!(
            page.tree.node_by_key(label_key).is_some() && !emitted.contains_key(label_key),
            "{}: the paired label {label_key:?} must fold into its control's line",
            page.name
        );
    }
    Ok(proxied)
}

/// The default-mode tree against the oracle's, child frames stitched under their iframes.
fn default_tree_matches(page: &RecordedPage, default: &AriaTree) -> Result<bool, Box<dyn Error>> {
    let frames = page
        .oracle
        .frames
        .iter()
        .map(|frame| parse_oracle_aria(&frame.aria.default))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(stitch_oracle_trees(frames).is_ok_and(|expected| default.nodes() == expected.as_slice()))
}

/// Everything the all-16 test measures across pages, checked once every page has run.
#[derive(Default)]
struct Measurements {
    refs: Vec<Measured>,
    printed: Vec<Measured>,
    default_verdicts: Vec<String>,
}

/// The default tree's verdict against [`DEFAULT_TREE_DIFFERS`]: `None` when it is as listed.
fn default_tree_verdict(
    page: &RecordedPage,
    default: &AriaTree,
) -> Result<Option<String>, Box<dyn Error>> {
    let listed = DEFAULT_TREE_DIFFERS
        .iter()
        .any(|(name, _)| *name == page.name);
    Ok(match (default_tree_matches(page, default)?, listed) {
        (true, true) => Some(format!(
            "{}: the default-mode tree now equals the oracle's; take it off DEFAULT_TREE_DIFFERS",
            page.name
        )),
        (false, false) => Some(format!(
            "{}: the default-mode tree differs from the oracle's aria.default",
            page.name
        )),
        _ => None,
    })
}

/// Emits, renders and compares one recorded page, adding what it measured to `measurements`.
fn check_recorded_page(
    name: &'static str,
    measurements: &mut Measurements,
) -> Result<PageCounts, Box<dyn Error>> {
    let page = RecordedPage::load(name)?;
    let mut refs = TabRefTable::new();
    let ai = page.tree.aria(AriaAsk::ai(&mut refs))?;
    let default = page.tree.aria(AriaAsk::default_mode())?;
    assert_eq!(
        ai.partial().len(),
        page.partial_frames,
        "{name}: ai partial count"
    );
    assert_eq!(
        default.partial().len(),
        page.partial_frames,
        "{name}: default partial count"
    );
    measurements
        .default_verdicts
        .extend(default_tree_verdict(&page, &default)?);

    let mut counts = PageCounts {
        resolved_refs: assert_all_refs_resolve_in_tree(name, &page.tree, &ai, &refs)?,
        ..PageCounts::default()
    };
    let emitted = index_emitted_by_key(ai.nodes(), &refs)?;
    let expected_proxies = CLICK_PROXIES
        .iter()
        .find(|(page_name, _)| *page_name == name)
        .map_or(0, |(_, count)| *count);
    assert_eq!(
        assert_click_proxies(&page, &emitted)?,
        expected_proxies,
        "{name}: printed refs paired with a label"
    );
    measure_refs(&page, &mut counts, &mut measurements.refs)?;
    measure_printed_nodes(&page, &emitted, &mut counts, &mut measurements.printed)?;
    assert!(
        counts.joined > 0,
        "{name}: no oracle element joined the tree"
    );

    let (rendered, baseline) = render_and_check_safety(name, &page.tree, &ai, default, &refs)?;
    assert_idempotent_delta(name, &page.tree, &ai, &baseline, &mut refs)?;
    if name == "hung-subframe" {
        assert_eq!(
            page.partial_frames, 1,
            "hung-subframe must have 1 partial frame"
        );
        assert!(
            rendered.ai_playwright.contains("# partial:")
                && rendered.ai_full.contains("# partial:")
                && rendered.ai_page.contains("# partial:")
                && rendered.default_playwright.contains("# partial:"),
            "hung-subframe partial frame must render `# partial:` across output formats"
        );
    }
    Ok(counts)
}

#[test]
fn all_16_recorded_fixtures_emit_render_and_match_the_oracles() -> Result<(), Box<dyn Error>> {
    let mut measurements = Measurements::default();
    let mut totals = PageCounts::default();
    let mut per_page = Vec::new();
    for fixture in FIXTURES {
        let counts = check_recorded_page(fixture.name, &mut measurements)?;
        totals.add(&counts);
        per_page.push(format!("{:<26} {counts:?}", fixture.name));
    }

    #[expect(
        clippy::print_stderr,
        reason = "the per-page counts show a reviewer what the comparison covered, also when it \
                  passes"
    )]
    {
        eprintln!(
            "tree_emit_refs_oracle:\n{}\ntotal {totals:?}\nledgered disagreements: {} ref, {} \
             printed",
            per_page.join("\n"),
            measurements.refs.len(),
            measurements.printed.len()
        );
    }
    assert!(
        measurements.default_verdicts.is_empty(),
        "{}",
        measurements.default_verdicts.join("\n")
    );
    parity_ledger::assert_measured_equals_ledger(&measurements.refs, &[Field::Ref]);
    parity_ledger::assert_printed_matches_ledger(&measurements.printed, &AI_NODE_FIELDS);
    assert!(
        totals.joined >= 640,
        "expected >= 640 joined oracle elements across all 16 fixtures, saw {}",
        totals.joined
    );
    assert!(
        totals.printed_compared > 0 && totals.resolved_refs > 0,
        "expected printed nodes and refs across the recordings: {totals:?}"
    );
    Ok(())
}

/// The printed node with `role` whose name, or text when it has no name, is `label`.
fn find_printed<'a>(nodes: &'a [AriaNode], role: &str, label: &str) -> Option<&'a AriaNode> {
    collect_all_nodes(nodes).into_iter().find(|node| {
        node.role == role && node.name.as_deref().or(node.text.as_deref()) == Some(label)
    })
}

#[test]
fn ua_shadow_recording_folds_browser_made_content() -> Result<(), Box<dyn Error>> {
    let page = RecordedPage::load("ua-shadow")?;
    let mut refs = TabRefTable::new();
    let ai = page.tree.aria(AriaAsk::ai(&mut refs))?;
    let emitted = index_emitted_by_key(ai.nodes(), &refs)?;

    let ua_shadow: Vec<_> = page
        .tree
        .nodes()
        .iter()
        .filter(|node| node.hidden().is_ua_shadow())
        .collect();
    assert!(
        !ua_shadow.is_empty(),
        "the recording carries browser-made shadow content"
    );
    for node in ua_shadow {
        assert!(
            !page.tree.is_ref_worthy(node.id()),
            "a ua_shadow node never gets a ref"
        );
        if let Some(key) = page.tree.node_key(node.id()) {
            assert!(!emitted.contains_key(&key), "a ua_shadow node never prints");
        }
    }
    for node in collect_all_nodes(ai.nodes()) {
        assert!(
            !matches!(node.role.as_str(), "spinbutton" | "slider"),
            "the date input's and the video's own controls stay folded: {:?}",
            node.role
        );
    }

    let select = find_printed(ai.nodes(), "combobox", "combobox: collapsed select")
        .ok_or("the collapsed select prints")?;
    assert_eq!(
        printed_options(select).len(),
        3,
        "its options are synthesized from select_options; the all-16 test compares them"
    );

    let rendered = neo_snapshot::render(&ai.into_snapshot(), Call::Playwright).into_text();
    assert!(rendered.contains("button: inside open details"));
    assert!(!rendered.contains("inside closed details"));
    Ok(())
}

#[test]
fn private_forms_recording_masks_values_as_the_page_masker_does() -> Result<(), Box<dyn Error>> {
    let page = RecordedPage::load("private-forms")?;
    let mut refs = TabRefTable::new();
    let ai = page.tree.aria(AriaAsk::ai(&mut refs))?;

    let masked: Vec<&str> = collect_all_nodes(ai.nodes())
        .into_iter()
        .filter(|node| node.role == "textbox")
        .filter_map(|node| node.text.as_deref())
        .filter(|text| !text.is_empty() && text.chars().all(|c| c == '•'))
        .collect();
    assert_eq!(
        masked.len(),
        12,
        "two password fields and five autocomplete tokens on input and on textarea print as bullets"
    );
    let public = find_printed(
        ai.nodes(),
        "textbox",
        "textbox: plain input with a public value",
    )
    .ok_or("the public field prints")?;
    assert_eq!(public.text.as_deref(), Some("plain: public note"));
    let toggle = find_printed(ai.nodes(), "button", "Show password").ok_or("the toggle prints")?;
    assert_eq!(toggle.pressed, Some(neo_snapshot::Tristate::True));
    Ok(())
}

#[test]
fn modal_inert_recording_marks_each_inert_subtree_at_its_top() -> Result<(), Box<dyn Error>> {
    let page = RecordedPage::load("modal-inert")?;
    let mut refs = TabRefTable::new();
    let ai = page.tree.aria(AriaAsk::ai(&mut refs))?;

    for (role, label) in [
        ("heading", "heading: page behind the modal"),
        ("button", "button: behind modal"),
        ("paragraph", "paragraph: page text behind the modal"),
        ("button", "button: in an inert div outside the dialog"),
        ("link", "link: inert inside the modal"),
        ("button", "button: inert inside the modal"),
        ("textbox", "textbox: inert inside the modal"),
    ] {
        let node = find_printed(ai.nodes(), role, label)
            .ok_or_else(|| format!("{role} {label:?} prints"))?;
        assert_eq!(node.inert, Some(true), "{role} {label:?} is inert");
        assert!(
            node.r#ref.is_some(),
            "inertness never strips a ref: {role} {label:?}"
        );
    }
    let dialog = collect_all_nodes(ai.nodes())
        .into_iter()
        .find(|node| node.role == "dialog")
        .ok_or("the dialog prints")?;
    assert_eq!(dialog.inert, None, "the modal dialog is live");
    for (role, label) in [
        ("heading", "heading: modal title, also the dialog name"),
        ("button", "button: modal cancel"),
    ] {
        let node = find_printed(ai.nodes(), role, label)
            .ok_or_else(|| format!("{role} {label:?} prints"))?;
        assert_eq!(node.inert, None, "{role} {label:?} is live");
    }
    let confirm =
        find_printed(ai.nodes(), "button", "button: modal confirm").ok_or("confirm prints")?;
    assert_eq!((confirm.inert, confirm.active), (None, Some(true)));

    let mut marked_below_a_mark = Vec::new();
    let mut stack: Vec<(&AriaNode, bool)> = ai.nodes().iter().map(|node| (node, false)).collect();
    while let Some((node, under_mark)) = stack.pop() {
        if under_mark && node.inert == Some(true) {
            marked_below_a_mark.push(node.role.clone());
        }
        let child_under_mark = under_mark || node.inert == Some(true);
        for child in &node.children {
            if let AriaChild::Node(child_node) = child {
                stack.push((child_node, child_under_mark));
            }
        }
    }
    assert!(
        marked_below_a_mark.is_empty(),
        "only the top-most node of an inert subtree is marked: {marked_below_a_mark:?}"
    );

    let snap = ai.into_snapshot();
    let listed = neo_snapshot::render(&snap, Call::Page(&Passed::default())).into_text();
    assert!(listed.starts_with("# not listed:"), "{listed}");
    assert!(listed.contains("button: modal confirm") && listed.contains("button: modal cancel"));
    assert!(!listed.contains("button: behind modal"));
    let with_inert = neo_snapshot::render(
        &snap,
        Call::Page(&Passed {
            inert: Some(true),
            ..Passed::default()
        }),
    )
    .into_text();
    assert!(with_inert.contains("button: behind modal"));
    Ok(())
}

#[test]
fn nested_frames_recording_names_refs_by_frame_across_all_eight() -> Result<(), Box<dyn Error>> {
    let page = RecordedPage::load("nested-cross-site-frames")?;
    assert_eq!(page.oracle.frames.len(), 8);
    assert_eq!((page.frame_to_doc.len(), page.partial_frames), (8, 0));

    // The recorder numbers frames in stitch order, as Neo's per-frame probes do: `e<n>` in the
    // main frame, `f<k>e<n>` in the k-th child frame. The emitter must name refs the same way.
    // APC sends no <html> or <body> for four of the documents, so their refs count from the
    // first element after the body Playwright prints, and the numbers inside a frame can differ.
    let frame_of_doc: HashMap<DocId, usize> = page
        .oracle
        .frames
        .iter()
        .enumerate()
        .filter_map(|(idx, frame)| Some((*page.frame_to_doc.get(&frame.frame)?, idx)))
        .collect();
    let mut refs = TabRefTable::new();
    let ai = page.tree.aria(AriaAsk::ai(&mut refs))?;
    let mut refs_per_frame = vec![0_usize; page.oracle.frames.len()];
    for ref_str in collect_printed_refs(ai.nodes()) {
        let RefLookup::Key(target) = refs.resolve_str(&ref_str) else {
            return Err(format!("printed ref {ref_str:?} does not resolve").into());
        };
        let doc = page
            .tree
            .node_by_key(&target.key)
            .map(TreeNode::doc)
            .ok_or("a printed ref names a live node")?;
        let frame_idx = *frame_of_doc
            .get(&doc)
            .ok_or("every document is an oracle frame")?;
        let prefix = if frame_idx == 0 {
            "e".to_owned()
        } else {
            format!("f{frame_idx}e")
        };
        assert!(
            ref_str
                .strip_prefix(&prefix)
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())),
            "ref {ref_str:?} of frame {frame_idx} must read {prefix}<n>"
        );
        if let Some(count) = refs_per_frame.get_mut(frame_idx) {
            *count += 1;
        }
    }
    assert!(
        refs_per_frame.iter().all(|&count| count > 0),
        "every frame prints refs: {refs_per_frame:?}"
    );
    Ok(())
}
