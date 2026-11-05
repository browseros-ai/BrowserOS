//! The page's one aria snapshot, stitched from every frame's own, as upstream's server stitches
//! it (`ariaSnapshotJSONForFrame`, server/page.ts at v1.64.1), then Neo's name rule and
//! sdk-privacy's mask. Pure: the snapshot leaf reads each frame through its probe and hands the
//! results here.
//!
//! ```text
//!  FrameSnapshot { json, sweep, children }          one per frame the leaf read
//!    ChildFrame::Read { iframe_ref, snapshot }      stitched under the iframe node `iframe_ref`
//!    ChildFrame::Partial(PartialFrame)              the iframe node stays a leaf
//!  stitch(root) ─▶ merge bottom-up ─▶ drop_prohibited_names ─▶ mask page text with every
//!                 frame's sweep ─▶ SnapshotResult { json, partial }, the op's output
//! ```
//!
//! The YAML is rendered in the agent runtime, never in a page: rendering the stitched tree in
//! the root frame's world would copy every cross-site frame's text into the embedding site's
//! renderer process, which site isolation exists to prevent.

use std::collections::HashMap;
use std::mem;

use browseros_core::sensitive_fields::FieldValues;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::snapshot_rules;

/// One node of `ariaSnapshotJSON`: upstream's `AriaNodeJSON` (isomorphic/ariaSnapshot.ts).
///
/// The strings a page controls are typed (`name`, `text`, `url`, `placeholder`, string
/// children), because the mask covers exactly those. The rest is vocabulary or numbers and
/// passes through as upstream wrote it. Decoding rejects unknown fields: a field a Playwright
/// upgrade adds could carry page text the mask would miss, so it fails until it is classified.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AriaNode {
    /// An ARIA role, or `iframe` or `text`: never free text, because roleUtils only returns
    /// roles from its own list.
    pub(crate) role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    /// `true` or `"mixed"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checked: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expanded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) active: Option<bool>,
    /// `true`, `"grammar"` or `"spelling"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) invalid: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) level: Option<Value>,
    /// `true` or `"mixed"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pressed: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) aria_hidden: Option<bool>,
    /// Minted by the engine (`e5`, `f3e5`), never page text, so the mask leaves it whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) r#ref: Option<String>,
    /// `"pointer"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) r#box: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) placeholder: Option<String>,
    /// The node's only content, when it is one string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    /// Upstream writes no `children` rather than an empty list, and so does this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) children: Vec<AriaChild>,
}

/// An item of a node's `children`: text, or a node.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum AriaChild {
    Text(String),
    Node(Box<AriaNode>),
}

/// One frame's own snapshot, as its probe returned it, and what became of each iframe it
/// printed. The snapshot leaf builds this tree as it reads the frames; `stitch` folds it.
pub(crate) struct FrameSnapshot {
    /// The frame's `ariaSnapshotJSON` nodes, its iframe nodes not stitched yet.
    pub(crate) json: Vec<AriaNode>,
    /// The frame's sensitive values (`__clawSeal.sensitiveValues()`). One frame's sweep masks
    /// echoes in every frame: a code typed in one frame can be printed by another.
    pub(crate) sweep: FieldValues,
    /// One entry per iframe the frame printed with a ref, in any order.
    pub(crate) children: Vec<ChildFrame>,
}

/// What became of the frame inside one iframe node.
pub(crate) enum ChildFrame {
    /// Read within its budget: its nodes become the iframe node's children.
    Read {
        /// The iframe node's ref in its parent frame.
        iframe_ref: String,
        snapshot: FrameSnapshot,
    },
    /// Not read (timed out, detached, session lost): the iframe node keeps its ref and gets no
    /// children, and the rest of the page is stitched as if it were empty.
    Partial(PartialFrame),
}

/// A frame the snapshot could not read. The prelude prints
/// `# partial: <frame> not read (<reason>)` under its iframe's line.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct PartialFrame {
    /// The iframe node's ref in its parent frame.
    pub(crate) r#ref: String,
    /// The frame's URL as far as Neo knows it, the iframe's `src`. Page text, so it is masked
    /// like the tree.
    pub(crate) frame: String,
    /// `timeout after 2000 ms`, `frame detached`, `target closed`.
    pub(crate) reason: String,
}

/// The snapshot ops' output, and the contract with the prelude that renders it:
///
/// ```json
/// {
///   "json": [{ "role": "generic", "ref": "e1", "children": [
///     { "role": "iframe", "ref": "e10", "children": [{ "role": "button", "name": "Go", "ref": "f1e2" }] },
///     { "role": "iframe", "ref": "e11" }
///   ] }],
///   "partial": [{ "ref": "e11", "frame": "https://pay.example/frame", "reason": "timeout after 2000 ms" }]
/// }
/// ```
///
/// `json` is upstream's `AriaSnapshotJSON` with every frame stitched in, the name rule applied
/// and swept values masked. The prelude renders it with the vendored `renderAriaSnapshotAsYaml`
/// and adds a `# partial` line under each partial iframe. `tests/data/snapshot-output.json` is
/// one recorded from real frames.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct SnapshotResult {
    pub(crate) json: Vec<AriaNode>,
    pub(crate) partial: Vec<PartialFrame>,
}

/// The page's snapshot from the root frame's tree: every read child stitched in, the name rule
/// applied, and every page string masked with the union of every frame's sweep.
pub(crate) fn stitch(root: FrameSnapshot) -> SnapshotResult {
    let mut partial = Vec::new();
    let (mut json, sweep) = merge(root, &mut partial);
    for node in &mut json {
        snapshot_rules::drop_prohibited_names(node);
    }
    if !sweep.is_empty() {
        for node in &mut json {
            mask_node(node, &sweep);
        }
        for frame in &mut partial {
            mask_text(&mut frame.frame, &sweep);
        }
    }
    SnapshotResult { json, partial }
}

/// Upstream's merge, bottom-up: each read child is stitched first, then set under the iframe
/// node with its ref. Returns the frame's stitched nodes and the union of every sweep below.
fn merge(frame: FrameSnapshot, partial: &mut Vec<PartialFrame>) -> (Vec<AriaNode>, FieldValues) {
    let FrameSnapshot {
        mut json,
        mut sweep,
        children,
    } = frame;
    let mut stitched = HashMap::new();
    for child in children {
        match child {
            ChildFrame::Read {
                iframe_ref,
                snapshot,
            } => {
                let (nodes, child_sweep) = merge(snapshot, partial);
                sweep = sweep.union(child_sweep);
                stitched.insert(iframe_ref, nodes);
            }
            ChildFrame::Partial(frame) => partial.push(frame),
        }
    }
    for node in &mut json {
        attach(node, &mut stitched);
    }
    (json, sweep)
}

/// Upstream's `mergeIframeChildren`: an iframe node with a ref takes its child's nodes, unless
/// the child came back empty, and is not looked into; any other node is walked. Version 1.64.1
/// keeps an iframe node's own `text` beside the stitched children (1.65 drops it).
fn attach(node: &mut AriaNode, stitched: &mut HashMap<String, Vec<AriaNode>>) {
    if node.role == "iframe"
        && let Some(iframe_ref) = &node.r#ref
    {
        if let Some(nodes) = stitched.remove(iframe_ref)
            && !nodes.is_empty()
        {
            node.children = nodes
                .into_iter()
                .map(|child| AriaChild::Node(Box::new(child)))
                .collect();
        }
        return;
    }
    for child in &mut node.children {
        if let AriaChild::Node(child) = child {
            attach(child, stitched);
        }
    }
}

/// Masks every string the page controls in `node` and below. Keys, roles, refs and the other
/// vocabulary are left alone: no page text reaches them, and masking one would break a ref or
/// show which word a secret equals.
fn mask_node(node: &mut AriaNode, values: &FieldValues) {
    let AriaNode {
        name,
        url,
        placeholder,
        text,
        children,
        ..
    } = node;
    for text in [name, url, placeholder, text].into_iter().flatten() {
        mask_text(text, values);
    }
    for child in children {
        match child {
            AriaChild::Text(text) => mask_text(text, values),
            AriaChild::Node(node) => mask_node(node, values),
        }
    }
}

/// `FieldValues::mask_json` on one string: a long value wherever it occurs, a short one where
/// it is the whole string.
fn mask_text(text: &mut String, values: &FieldValues) {
    let mut leaf = Value::String(mem::take(text));
    values.mask_json(&mut leaf);
    if let Value::String(masked) = leaf {
        *text = masked;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::Path;

    use browseros_core::sensitive_fields::FieldValues;
    use serde::Deserialize;
    use serde_json::{Value, json};

    use super::{AriaChild, AriaNode, ChildFrame, FrameSnapshot, PartialFrame, merge, stitch};

    /// Recorded from playwright-core 1.64.1 by scripts/capture-snapshot-fixture.mjs.
    const RECORDED: &str = include_str!("../../tests/data/snapshot-frames.json");

    /// From this crate's manifest directory.
    const OUTPUT: &str = "tests/data/snapshot-output.json";

    #[derive(Deserialize)]
    pub(crate) struct Recorded {
        pub(crate) frames: Vec<RecordedFrame>,
        stitched: Vec<AriaNode>,
    }

    #[derive(Deserialize)]
    pub(crate) struct RecordedFrame {
        pub(crate) json: Vec<AriaNode>,
        iframes: Vec<RecordedIframe>,
    }

    #[derive(Deserialize)]
    struct RecordedIframe {
        r#ref: String,
        src: String,
        frame: usize,
    }

    pub(crate) fn recorded() -> Result<Recorded, serde_json::Error> {
        serde_json::from_str(RECORDED)
    }

    /// The fixture page: 0 is the top frame; its iframe `e10` holds frame 1 (cross-site),
    /// whose iframe `f1e4` holds frame 3; its iframe `e11` holds frame 2.
    const CHILD: usize = 1;
    const SIDE: usize = 2;
    const GRANDCHILD: usize = 3;

    /// The tree the snapshot leaf would build from the recorded frames, with `sweeps` as each
    /// frame's sensitive values and the frames in `unread` past their budget.
    fn tree(
        recorded: &Recorded,
        index: usize,
        sweeps: &[(usize, Value)],
        unread: &[usize],
    ) -> FrameSnapshot {
        let frame = &recorded.frames[index];
        let sweep = sweeps
            .iter()
            .find(|(swept, _)| *swept == index)
            .map_or_else(FieldValues::default, |(_, values)| {
                FieldValues::from_sweep(values)
            });
        let children = frame
            .iframes
            .iter()
            .map(|iframe| {
                if unread.contains(&iframe.frame) {
                    ChildFrame::Partial(PartialFrame {
                        r#ref: iframe.r#ref.clone(),
                        frame: iframe.src.clone(),
                        reason: "timeout after 2000 ms".to_owned(),
                    })
                } else {
                    ChildFrame::Read {
                        iframe_ref: iframe.r#ref.clone(),
                        snapshot: tree(recorded, iframe.frame, sweeps, unread),
                    }
                }
            })
            .collect();
        FrameSnapshot {
            json: frame.json.clone(),
            sweep,
            children,
        }
    }

    /// The node with `ref`, anywhere in `nodes`.
    pub(crate) fn find<'a>(nodes: &'a [AriaNode], r#ref: &str) -> Option<&'a AriaNode> {
        nodes.iter().find_map(|node| find_below(node, r#ref))
    }

    fn find_below<'a>(node: &'a AriaNode, r#ref: &str) -> Option<&'a AriaNode> {
        if node.r#ref.as_deref() == Some(r#ref) {
            return Some(node);
        }
        node.children.iter().find_map(|child| match child {
            AriaChild::Node(child) => find_below(child, r#ref),
            AriaChild::Text(_) => None,
        })
    }

    #[test]
    fn tests_that_three_levels_of_recorded_frames_stitch_into_upstreams_own_tree()
    -> Result<(), Box<dyn Error>> {
        let recorded = recorded()?;
        let mut partial = Vec::new();
        let (json, _) = merge(tree(&recorded, 0, &[], &[]), &mut partial);
        assert!(partial.is_empty(), "every frame was read");
        assert_eq!(
            serde_json::to_value(&json)?,
            serde_json::to_value(&recorded.stitched)?,
            "the same tree upstream's server stitched from the same frames"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_partial_child_keeps_its_ref_while_the_rest_is_stitched()
    -> Result<(), Box<dyn Error>> {
        let recorded = recorded()?;
        let result = stitch(tree(&recorded, 0, &[], &[SIDE, GRANDCHILD]));
        assert_eq!(
            result.partial,
            [
                PartialFrame {
                    r#ref: "f1e4".to_owned(),
                    frame: "http://127.0.0.1:42051/snapshot/grandchild.html".to_owned(),
                    reason: "timeout after 2000 ms".to_owned(),
                },
                PartialFrame {
                    r#ref: "e11".to_owned(),
                    frame: "http://[::1]:42051/snapshot/side.html".to_owned(),
                    reason: "timeout after 2000 ms".to_owned(),
                },
            ],
            "the nested partial frame is reported too"
        );
        for unread in ["e11", "f1e4"] {
            let node = find(&result.json, unread);
            assert!(
                node.is_some_and(|node| node.children.is_empty()),
                "{unread} stays a leaf with its ref"
            );
        }
        assert!(
            find(&result.json, "f1e2").is_some(),
            "the readable child frame is still stitched"
        );
        assert!(find(&result.json, "f3e2").is_none());
        Ok(())
    }

    #[test]
    fn tests_that_a_value_swept_in_one_frame_is_masked_where_another_frame_prints_it()
    -> Result<(), Box<dyn Error>> {
        let recorded = recorded()?;
        // The code is typed in the top frame and echoed by its cross-site child; a value from
        // the deepest frame is printed by the top frame.
        let sweeps = [
            (0, json!(["482913"])),
            (GRANDCHILD, json!(["Plain paragraph"])),
        ];
        let result = stitch(tree(&recorded, 0, &sweeps, &[]));
        let echo = find(&result.json, "f1e3").and_then(|node| node.text.clone());
        assert_eq!(echo.as_deref(), Some("Confirm your code [private]"));
        let paragraph = find(&result.json, "e7").and_then(|node| node.text.clone());
        assert_eq!(paragraph.as_deref(), Some("[private]"));
        Ok(())
    }

    /// Only page strings are masked. A secret that equals a role or a ref leaves both whole:
    /// masking them would break the ref and show which word the secret is.
    #[test]
    fn tests_that_roles_and_refs_are_never_masked_but_names_and_frame_urls_are()
    -> Result<(), Box<dyn Error>> {
        let recorded = recorded()?;
        let sweeps = [(CHILD, json!(["button", "e11", "side"]))];
        let result = stitch(tree(&recorded, 0, &sweeps, &[SIDE]));
        let retry = find(&result.json, "e6");
        assert_eq!(retry.map(|node| node.role.as_str()), Some("button"));
        let child_button = find(&result.json, "f1e2").and_then(|node| node.name.clone());
        assert_eq!(child_button.as_deref(), Some("Child [private]"));
        assert!(find(&result.json, "e11").is_some(), "the ref e11 is whole");
        assert_eq!(
            result.partial.first().map(|frame| frame.frame.as_str()),
            Some("http://[::1]:42051/snapshot/[private].html"),
            "a frame's URL is page text"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_field_upstream_does_not_write_fails_to_decode() {
        let unknown = serde_json::from_value::<AriaNode>(json!({
            "role": "button",
            "description": "a field 1.64.1 does not have",
        }));
        assert!(
            unknown.is_err(),
            "an unclassified field could carry unmasked page text"
        );
    }

    /// tests/data/snapshot-output.json is what `stitch` returns for the recorded page, with the
    /// top frame's code echoed by its child and the side frame past its budget. The prelude's
    /// renderer is tested on it (packages/browseros-playwright-js), so the file must track this
    /// code: a stale one is rewritten and the test fails until it is committed.
    #[test]
    fn tests_that_the_snapshot_output_fixture_is_what_stitch_returns() -> Result<(), Box<dyn Error>>
    {
        let recorded = recorded()?;
        let result = stitch(tree(&recorded, 0, &[(0, json!(["482913"]))], &[SIDE]));
        let fresh = format!("{}\n", serde_json::to_string_pretty(&result)?);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(OUTPUT);
        if fs::read_to_string(&path).ok().as_deref() != Some(fresh.as_str()) {
            fs::write(&path, &fresh)?;
            panic!(
                "{} did not match stitch() and has been rewritten; review and commit it",
                path.display()
            );
        }
        Ok(())
    }
}
