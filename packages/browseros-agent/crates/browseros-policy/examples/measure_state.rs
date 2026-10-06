// A command line measurement tool, so printing to stdout is the whole point.
#![allow(clippy::print_stdout)]

//! Measures where a decision request's bytes actually go.
//!
//! A run on one article was observed costing roughly 13k input tokens per
//! decision, and a shopping search page 131k to 145k, against a tree budget of
//! a few thousand bytes. Only the tree was budgeted, so the question was which
//! part of the state the rest of it was. This answers that per page size, with
//! no provider and no network, so it can be rerun after any change to the
//! state's shape.
//!
//! cargo run -p browseros-policy --example measure_state

use browseros_policy::action::{ActionSpace, Element, MAX_TARGET_OPTIONS};
use browseros_policy::questions::{Observation, state};

/// A page shaped like a search result: many named links, many unnamed nodes.
fn page(actionables: usize) -> Observation {
    let elements = (1..=actionables)
        .map(|index| {
            if index % 3 == 0 {
                // Image links and footnote markers, which carry no usable name.
                Element::new(format!("e{index}"), "link", "")
            } else {
                Element::new(
                    format!("e{index}"),
                    "link",
                    format!("Corsair Vengeance DDR5 32GB kit {index}"),
                )
            }
        })
        .collect::<Vec<_>>();
    let tree = (1..=actionables)
        .map(|index| format!("  - link \"item {index}\" [ref=e{index}]"))
        .collect::<Vec<_>>()
        .join("\n");
    Observation {
        url: "https://example.com/search?q=32gb+ddr5".to_string(),
        title: "Search results".to_string(),
        tree: format!("- main\n{tree}"),
        space: ActionSpace::new(elements, true, false),
    }
}

/// Rough token count, the same four-bytes-per-token rule the crate uses
/// elsewhere. Good enough to compare parts of one request against each other.
fn tokens(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

fn main() {
    println!("state size by page size (cap is {MAX_TARGET_OPTIONS} target options)\n");
    println!(
        "{:>10}  {:>10}  {:>10}  {:>10}  {:>10}  {:>8}",
        "actionable", "offered", "total B", "tree B", "elements B", "~tokens"
    );

    for actionables in [10, 50, 100, 250, 500, 1000] {
        let observation = page(actionables);
        let offered = observation.space.offered_elements().len();
        let state = state("Open the cheapest 32GB DDR5 kit.", &observation, &[]);

        let total = serde_json::to_string(&state)
            .map(|json| json.len())
            .unwrap_or(0);
        let tree = state["page"]["tree"].as_str().unwrap_or_default().len();
        let elements = serde_json::to_string(&state["elements"])
            .map(|json| json.len())
            .unwrap_or(0);

        println!(
            "{actionables:>10}  {offered:>10}  {total:>10}  {tree:>10}  {elements:>10}  {:>8}",
            tokens(total)
        );
    }

    println!(
        "\nThe element list is the part that used to grow without a bound. It is now\n\
         capped at the offered candidates, so the state stops growing once a page\n\
         passes {MAX_TARGET_OPTIONS} actionables."
    );
}
