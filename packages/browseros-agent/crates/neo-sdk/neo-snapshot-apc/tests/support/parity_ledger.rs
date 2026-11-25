//! The parity ledger: every oracle element where [`neo_snapshot_apc::tree`] and the recorded
//! Playwright oracle still disagree on the contract-v4 recordings, with the reason.
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

/// A group of elements that disagree on one field for one reason.
pub(crate) struct KnownMismatch {
    pub(crate) field: Field,
    pub(crate) cause: Cause,
    pub(crate) why: &'static str,
    pub(crate) nodes: &'static [OracleNode],
}

const NESTED: &str = "nested-cross-site-frames";

const IMAGE_INPUTS: &[OracleNode] = &[
    main("roles", 262),
    main("names", 20),
    main("names", 153),
    main("names", 155),
    main("names", 157),
];

/// Recorded on BrowserOS Dev 158.0.8127.18 at contract v4 (`tests/fixtures/README.md`).
pub(crate) const KNOWN_MISMATCHES: &[KnownMismatch] = &[
    KnownMismatch {
        field: Field::Join,
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
        cause: Cause::ContractFact,
        why: "APC emits no node for these documents' <html> element, which holds Playwright's \
              `document` role",
        nodes: &[
            child(NESTED, "http://localhost/nested/b1.html", 61),
            child(NESTED, "http://[::1]/nested/c2.html", 18),
            child(NESTED, "http://127.0.0.1/nested/a3.html", 69),
            child(NESTED, "http://[::1]/nested/c4.html", 34),
            child("shadow-roots", "about:srcdoc", 72),
            main("modal-inert", 6),
        ],
    },
    KnownMismatch {
        field: Field::Join,
        cause: Cause::RustRule,
        why: "a collapsed <select>'s options have no box and no APC node; Rust's option synthesis \
              from select_options (apc-engine-design 3.5) is not built, and a synthesized option \
              has no DOM id to join by",
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
        cause: Cause::ContractFact,
        why: "a `display: contents` element has no layout object, so no APC node until contract \
              P11 (field 33) lands",
        nodes: &[
            main("roles", 647),
            main("roles", 649),
            main("roles", 651),
            main("roles", 653),
            main("roles", 654),
        ],
    },
    KnownMismatch {
        field: Field::Join,
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
        field: Field::Role,
        cause: Cause::RustRule,
        why: "APC sends <input type=image> as an Image node with no FormControlData; the role \
              rule reads only form_control_type and falls back to textbox, though tag `input` \
              plus ContentData::Image identify it",
        nodes: IMAGE_INPUTS,
    },
    KnownMismatch {
        field: Field::Pressed,
        cause: Cause::RustRule,
        why: "follows the image input's textbox role, which is not offered pressed",
        nodes: IMAGE_INPUTS,
    },
    KnownMismatch {
        field: Field::Invalid,
        cause: Cause::RustRule,
        why: "follows the image input's textbox role, which is offered invalid",
        nodes: IMAGE_INPUTS,
    },
    KnownMismatch {
        field: Field::Disabled,
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
        field: Field::Name,
        cause: Cause::RustRule,
        why: "follows the image input's textbox role: alt and the `Submit` default are skipped",
        nodes: &[main("roles", 262), main("names", 153), main("names", 157)],
    },
    KnownMismatch {
        field: Field::Name,
        cause: Cause::RustRule,
        why: "a list box's native <option> text arrives stamped ua_shadow, which the name rule \
              skips; the select's select_options carries every label",
        nodes: &[
            main("roles", 305),
            main("roles", 306),
            main("roles", 313),
            main("roles", 314),
            main("roles", 543),
            main("states", 82),
            main("states", 83),
            main("states", 84),
            main("states", 222),
            main("states", 223),
            main("states", 224),
        ],
    },
    KnownMismatch {
        field: Field::Name,
        cause: Cause::ContractFact,
        why: "an inline <svg>'s <title> child reaches APC only through stock `label`, which Rust \
              never reads (contract assumption 3)",
        nodes: &[main("roles", 398)],
    },
    KnownMismatch {
        field: Field::Name,
        cause: Cause::ContractFact,
        why: "the generated text of a `display: block` ::after has no break_before, so it joins \
              the previous word without a space",
        nodes: &[main("names", 197)],
    },
    KnownMismatch {
        field: Field::ErrorMessage,
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
    pub(crate) detail: String,
}

/// The identity both sides of the comparison reduce to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    fixture: &'static str,
    frame: Option<String>,
    backend_node_id: u64,
    field: Field,
}

impl Key {
    fn of_ledger(node: &OracleNode, field: Field) -> Self {
        Self {
            fixture: node.fixture,
            frame: node.frame.map(str::to_owned),
            backend_node_id: node.backend_node_id,
            field,
        }
    }

    fn of_measured(entry: &Measured) -> Self {
        Self {
            fixture: entry.fixture,
            frame: entry.frame.clone(),
            backend_node_id: entry.backend_node_id,
            field: entry.field,
        }
    }
}

/// Fails unless the disagreements measured on `fields` equal the ledger's entries for them,
/// naming each new disagreement with its values and each closed one with its recorded reason.
pub(crate) fn assert_measured_equals_ledger(measured: &[Measured], fields: &[Field]) {
    let ledger: BTreeSet<Key> = KNOWN_MISMATCHES
        .iter()
        .filter(|known| fields.contains(&known.field))
        .flat_map(|known| {
            known
                .nodes
                .iter()
                .map(|node| Key::of_ledger(node, known.field))
        })
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
            if !measured_keys.contains(&Key::of_ledger(node, known.field)) {
                report.push(format!(
                    "closed, remove from the ledger: {node:?} {:?} ({}: {})",
                    known.field, known.cause, known.why
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
