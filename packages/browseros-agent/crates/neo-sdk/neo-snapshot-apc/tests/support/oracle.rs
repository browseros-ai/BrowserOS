//! The recorded oracle: what Playwright's own ARIA engine said about a fixture page, read on the
//! same page load as that fixture's `.pb`.
//!
//! ```text
//!  record_fixtures ─▶ oracle_capture (JS engine, per frame) ─▶ Oracle ─▶ <name>.oracle.json
//!                  └▶ Page.getAnnotatedPageContent ──────────────────▶ <name>.pb
//!  tests/oracles.rs reads both; the Rust normalizer is later compared with the oracle node by
//!  node, joined on backendNodeId (an element) and on `owner` (a child frame's <iframe>).
//! ```
//!
//! The recorder serializes these types and the offline tests deserialize them, so the file's
//! shape is fixed here once. Every struct denies unknown fields: an oracle written by a newer
//! recorder fails loudly instead of being half-read.

use std::collections::BTreeMap;

use neo_snapshot::{Invalid, Tristate};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

/// One fixture's oracle file. Recorded data: it stays the parity target after the JS engine
/// leaves the bundle, so it names the engine and browser that produced it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Oracle {
    /// The fixture registry name; the `.pb` beside it has the same stem.
    pub(crate) fixture: String,
    /// injected.js's banner line naming Playwright's version and commit.
    pub(crate) engine: String,
    /// `Browser.getVersion`'s `product` of the browser both files were read from.
    pub(crate) browser: String,
    /// Main frame first, then depth first, each frame's children sorted by url then frame id.
    pub(crate) frames: Vec<OracleFrame>,
}

/// `url` as an oracle records it: without its `:port`, which the fixture server picks anew on
/// every run, so a re-recording writes the same URL and tests/oracles.rs can match it against
/// the `.pb`'s frame URLs.
pub(crate) fn recorded_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let host = match authority.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && (host.ends_with(']') || !host.contains(':')) =>
        {
            host
        }
        _ => authority,
    };
    format!("{scheme}://{host}{path}")
}

/// One frame's document as the engine read it. `frame` is the CDP frame id (the devtools frame
/// token); the normalizer finds this document in the `.pb` through `owner`, the `<iframe>` node
/// APC writes into the parent document.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OracleFrame {
    /// CDP frame id of this document's frame.
    pub(crate) frame: String,
    /// The parent frame's id; `None` only for the main frame.
    pub(crate) parent: Option<String>,
    /// backendNodeId of this frame's `<iframe>` in the parent document: the APC iframe node's
    /// `common_ancestor_dom_node_id`. `None` only for the main frame.
    pub(crate) owner: Option<u64>,
    /// The document's URL with the fixture server's ephemeral `:port` removed, so re-recording
    /// changes no byte for it.
    pub(crate) url: String,
    /// Every element the engine gives a role, in tree order (shadow roots before light children).
    pub(crate) elements: Vec<OracleElement>,
    /// ai-mode ref to backendNodeId: every ref the engine minted on this read, including those of
    /// nodes its distiller left out of the printed tree (they still resolve).
    pub(crate) refs: BTreeMap<String, u64>,
    /// The engine's `ariaSnapshotJSON` from this document's body, both modes.
    pub(crate) aria: OracleAria,
}

/// `ariaSnapshotJSON` exactly as the engine returned it. Kept as JSON so the oracle records what
/// the engine printed, not what today's `neo_snapshot::AriaNode` can read; the test checks that
/// it reads.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OracleAria {
    /// `mode: 'ai'`: Neo's snapshot form, with refs (`e<n>`, `f<seq>e<n>` in child frames).
    pub(crate) ai: Value,
    /// `mode: 'default'`: the form `toMatchAriaSnapshot` matches; no refs.
    pub(crate) default: Value,
}

/// The engine's facts about one element with a role, joined with the normalizer's node for the
/// same element by `backend_node_id`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OracleElement {
    /// The element's CDP backendNodeId, unique within its frame's document on this load.
    pub(crate) backend_node_id: u64,
    /// `getAriaRole`: explicit or implicit, `presentation` and `none` included.
    pub(crate) role: String,
    /// `getElementAccessibleName`, whitespace-normalized as the tree prints it.
    pub(crate) name: String,
    /// `getElementAccessibleDescription`, whitespace-normalized.
    pub(crate) description: String,
    /// `getElementAccessibleErrorMessage`.
    pub(crate) error_message: String,
    /// `isElementHiddenForAria`.
    pub(crate) hidden: bool,
    /// `isElementVisible`: has a box and is not `visibility: hidden`.
    pub(crate) visible: bool,
    /// The states Playwright offers this role.
    pub(crate) states: OracleStates,
}

/// The states `toAriaNode` writes, each present only when Playwright offers it to the element's
/// role (`kAria*Roles`), so an absent state and a false one stay apart.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OracleStates {
    /// `getAriaChecked`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checked: Option<Tristate>,
    /// `getAriaDisabled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) disabled: Option<bool>,
    /// `getAriaExpanded`; also absent when the role allows it but the element does not expand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expanded: Option<bool>,
    /// `getAriaInvalid`, mapped as `toAriaNode` maps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) invalid: Option<Invalid>,
    /// `getAriaLevel`; 0 when the role allows a level and none is set. A JSON number, as
    /// `neo_snapshot::AriaNode::level` is: `aria-level` may be any positive integer a script
    /// reads, past `u32` and `u64` alike.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) level: Option<Number>,
    /// `getAriaPressed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pressed: Option<Tristate>,
    /// `getAriaSelected`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected: Option<bool>,
    /// The element is its document's `activeElement` and that document has focus.
    pub(crate) active: bool,
}
