//! The parity ledger: every oracle element where [`neo_snapshot_apc::tree`] and the recorded
//! Playwright oracle still disagree on the contract-v6 recordings, with the reason.
//!
//! `tree_roles_states_oracle.rs` (join, role, hidden, box visibility, each state) and
//! `tree_names_oracle.rs` (name,
//! description, error message) compare every joined element of all 16 fixtures and require the
//! disagreements to equal this ledger exactly. A change that fixes a rule removes its entries; one
//! that breaks a rule fails with the new disagreement spelled out. Each entry names its owner:
//! a Rust rule, a contract fact Chromium does not send, or an accepted gap
//! (`apc-contract-design` section 10, G1 to G9; `decisions.md` D2).

use std::collections::BTreeSet;
use std::fmt;

/// The oracle fact a ledger entry disagrees on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Field {
    /// The oracle element has no node in the `SnapshotTree` (no APC node with its id).
    Join,
    Role,
    Hidden,
    /// Has a box and is not `visibility: hidden` (`isElementVisible`).
    BoxVisible,
    // One field per state `toAriaNode` offers, plus `active`.
    Checked,
    Pressed,
    Expanded,
    Selected,
    Disabled,
    Invalid,
    Level,
    Active,
    Name,
    Description,
    ErrorMessage,
}

/// Who owns a disagreement, so its follow-up goes to the right side of the contract.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Cause {
    /// The contract carries the facts; a Rust rule reads them differently from Playwright.
    RustRule,
    /// The capture lacks a fact Rust would need, and no accepted gap names it.
    ContractFact,
    /// A drift the designs accept: `G1`..`G9` or `D2`.
    AcceptedGap(&'static str),
    /// The fixture exists to produce it.
    FixtureDesign,
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RustRule => f.write_str("Rust rule"),
            Self::ContractFact => f.write_str("contract fact"),
            Self::AcceptedGap(gap) => write!(f, "accepted gap {gap}"),
            Self::FixtureDesign => f.write_str("fixture design"),
        }
    }
}

/// One oracle element: its fixture, its frame (`None` for the main frame, else the child
/// frame's recorded URL), and its `backendNodeId` in that frame's document.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OracleNode {
    pub(crate) fixture: &'static str,
    pub(crate) frame: Option<&'static str>,
    pub(crate) backend_node_id: u64,
}

const fn main(fixture: &'static str, backend_node_id: u64) -> OracleNode {
    OracleNode {
        fixture,
        frame: None,
        backend_node_id,
    }
}

const fn child(fixture: &'static str, frame: &'static str, backend_node_id: u64) -> OracleNode {
    OracleNode {
        fixture,
        frame: Some(frame),
        backend_node_id,
    }
}

/// A group of elements that disagree on one field for one reason, with the same tree value.
pub(crate) struct KnownMismatch {
    pub(crate) field: Field,
    /// The tree's side on every node of the group, as the test prints it: a name, description
    /// or error message verbatim, any other fact in `Debug` form, empty for [`Field::Join`]. The
    /// oracle's side is recorded data, so pinning the tree's side means a known disagreement
    /// cannot change into a different wrong value unnoticed.
    pub(crate) tree: &'static str,
    pub(crate) cause: Cause,
    pub(crate) why: &'static str,
    pub(crate) nodes: &'static [OracleNode],
}

/// Recorded on BrowserOS Dev 158.0.8127.18 at contract v6 (`tests/fixtures/README.md`).
pub(crate) const KNOWN_MISMATCHES: &[KnownMismatch] = &[
    KnownMismatch {
        field: Field::Join,
        tree: "",
        cause: Cause::FixtureDesign,
        why: "the hung frame spins past APC's subframe timeout, so the .pb has no document for \
              it; the oracle read it before the spin",
        nodes: &[
            child("hung-subframe", "http://[::1]/hung-subframe/hung.html", 2),
            child("hung-subframe", "http://[::1]/hung-subframe/hung.html", 4),
            child("hung-subframe", "http://[::1]/hung-subframe/hung.html", 6),
        ],
    },
    KnownMismatch {
        field: Field::Join,
        tree: "",
        cause: Cause::RustRule,
        why: "a collapsed <select>'s options have no box and no APC node. Chunk 5 \
              (tree-emit-refs) synthesizes them from select_options as fold.rs SyntheticOption \
              (apc-engine-design 3.5); one has no DOM id, so a follow-up joins it to the oracle \
              by select and index and removes these entries",
        nodes: &[
            main("nosize", 72),
            main("nosize", 74),
            main("ua-shadow", 132),
            main("ua-shadow", 135),
            main("ua-shadow", 138),
            main("roles", 531),
            main("roles", 533),
            main("names", 307),
            main("names", 309),
            main("names", 340),
            main("states", 374),
            main("states", 456),
        ],
    },
    KnownMismatch {
        field: Field::Join,
        tree: "",
        cause: Cause::AcceptedGap("D2"),
        why: "hidden for ARIA with no box (display: none or the hidden attribute), so no APC node",
        nodes: &[
            main("shadow-roots", 147),
            main("nosize", 15),
            main("roles", 451),
            main("roles", 521),
            main("roles", 522),
            main("roles", 523),
            main("roles", 656),
            main("roles", 657),
            main("names", 348),
            main("names", 350),
        ],
    },
    KnownMismatch {
        field: Field::BoxVisible,
        tree: "true",
        cause: Cause::ContractFact,
        why: "APC repairs a zero-size <input type=file>'s outer box from its browser-made button, \
              so the tree sees a box Playwright does not; apc-engine-design 5.2 keeps the \
              `visible` filter in the page for this reason",
        nodes: &[
            main("zero-size-controls", 43),
            main("zero-size-controls", 44),
        ],
    },
    KnownMismatch {
        field: Field::Disabled,
        tree: "Some(true)",
        cause: Cause::AcceptedGap("G2"),
        why: "a nested aria-disabled=\"false\" does not stop aria-disabled inheritance",
        nodes: &[
            main("states", 167),
            main("states", 169),
            main("states", 307),
            main("states", 309),
        ],
    },
    KnownMismatch {
        field: Field::ErrorMessage,
        tree: "",
        cause: Cause::AcceptedGap("G6"),
        why: "validity.valid gates the error message when aria-invalid is absent",
        nodes: &[main("names", 17)],
    },
];

/// One disagreement a test measured: which element, which field, and both values.
#[derive(Debug)]
pub(crate) struct Measured {
    pub(crate) fixture: &'static str,
    pub(crate) frame: Option<String>,
    pub(crate) backend_node_id: u64,
    pub(crate) field: Field,
    /// The tree's side, printed as [`KnownMismatch::tree`] documents.
    pub(crate) tree: String,
    pub(crate) detail: String,
}

/// The identity both sides of the comparison reduce to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    fixture: &'static str,
    frame: Option<String>,
    backend_node_id: u64,
    field: Field,
    tree: String,
}

impl Key {
    fn of_ledger(node: &OracleNode, known: &KnownMismatch) -> Self {
        Self {
            fixture: node.fixture,
            frame: node.frame.map(str::to_owned),
            backend_node_id: node.backend_node_id,
            field: known.field,
            tree: known.tree.to_owned(),
        }
    }

    fn of_measured(entry: &Measured) -> Self {
        Self {
            fixture: entry.fixture,
            frame: entry.frame.clone(),
            backend_node_id: entry.backend_node_id,
            field: entry.field,
            tree: entry.tree.clone(),
        }
    }
}

/// Fails unless the disagreements measured on `fields` equal the ledger's entries for them, tree
/// values included, naming each new disagreement with its values and each entry no longer
/// measured with its recorded reason. A known disagreement whose tree value changed shows as both.
pub(crate) fn assert_measured_equals_ledger(measured: &[Measured], fields: &[Field]) {
    let ledger: BTreeSet<Key> = KNOWN_MISMATCHES
        .iter()
        .filter(|known| fields.contains(&known.field))
        .flat_map(|known| known.nodes.iter().map(|node| Key::of_ledger(node, known)))
        .collect();
    let measured_keys: BTreeSet<Key> = measured.iter().map(Key::of_measured).collect();

    let mut report = Vec::new();
    for entry in measured {
        if !ledger.contains(&Key::of_measured(entry)) {
            report.push(format!("new disagreement: {}", entry.detail));
        }
    }
    for known in KNOWN_MISMATCHES
        .iter()
        .filter(|known| fields.contains(&known.field))
    {
        for node in known.nodes {
            if !measured_keys.contains(&Key::of_ledger(node, known)) {
                report.push(format!(
                    "closed or changed, update the ledger: {node:?} {:?} tree={:?} ({}: {})",
                    known.field, known.tree, known.cause, known.why
                ));
            }
        }
    }
    assert!(
        report.is_empty(),
        "the tree's disagreements with the Playwright oracles must equal the parity ledger \
         (tests/support/parity_ledger.rs):\n{}",
        report.join("\n")
    );
}
