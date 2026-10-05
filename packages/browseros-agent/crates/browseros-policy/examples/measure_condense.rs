// A command line measurement tool, so printing to stdout is the whole point.
#![allow(clippy::print_stdout)]

//! Measures what condensing the page saves, against the live provider.
//!
//! Sends the same crowded page twice, once with the tree in full and once
//! condensed, and reports the input tokens each costs. This is the number the
//! state-shrinking work is justified by.
//!
//! cargo run -p browseros-policy --example measure_condense

use browseros_policy::action::{ActionSpace, Element};
use browseros_policy::condense::{DEFAULT_TREE_BUDGET, condense};
use browseros_policy::decide::Decider;
use browseros_policy::questions::Observation;

/// A page shaped like a real search result: a few controls, a lot of prose.
fn crowded_page(prose_lines: usize) -> (Vec<Element>, String) {
    let elements = vec![
        Element::new("e1", "combobox", "Where from?"),
        Element::new("e2", "combobox", "Where to?"),
        Element::new("e3", "button", "Search"),
        Element::new("e4", "button", "Clear all"),
    ];
    let mut lines = vec![
        "- banner".to_string(),
        "  - heading \"Flight search\"".to_string(),
        "  - form".to_string(),
        "    - combobox \"Where from?\" [ref=e1]".to_string(),
        "    - combobox \"Where to?\" [ref=e2]: \"Zurich\"".to_string(),
        "    - button \"Search\" [ref=e3]".to_string(),
        "    - button \"Clear all\" [ref=e4]".to_string(),
    ];
    for index in 0..prose_lines {
        lines.push(format!(
            "  - text \"Travel guidance paragraph {index}: fares shown include taxes and vary by date.\""
        ));
    }
    (elements, lines.join("\n"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("JEV_TOKEN")
        .map_err(|_| "set JEV_TOKEN to the credential you want to measure with")?;
    let decider = Decider::new(token);
    let goal = "Find one-way flights from Zurich to London on 20 September 2026.";
    let (elements, tree) = crowded_page(600);

    let condensed = condense(&tree, DEFAULT_TREE_BUDGET);
    println!(
        "tree bytes   {} raw -> {} condensed ({}% saved, {} context lines dropped)",
        condensed.original_bytes,
        condensed.kept_bytes(),
        condensed.saved_percent(),
        condensed.dropped_lines
    );
    println!();

    let observation = Observation {
        url: "https://example.com/travel/flights".to_string(),
        title: "Flight search".to_string(),
        tree,
        space: ActionSpace::new(elements, true, false),
    };

    // What the loop sends now.
    let after = decider
        .decide_with_budget(goal, &observation, &[], DEFAULT_TREE_BUDGET)
        .await?;
    // What it sent before the page was condensed: the tree in full.
    let before = decider
        .decide_with_budget(goal, &observation, &[], usize::MAX)
        .await?;

    let before_in = before.input_tokens.unwrap_or(0);
    let after_in = after.input_tokens.unwrap_or(0);
    println!("{:<12} {:>9} {:>9}", "", "full", "condensed");
    println!("{:<12} {:>9} {:>9}", "input", before_in, after_in);
    println!(
        "{:<12} {:>9} {:>9}",
        "output",
        before.output_tokens.unwrap_or(0),
        after.output_tokens.unwrap_or(0)
    );
    println!(
        "{:<12} {:>8}ms {:>8}ms",
        "latency", before.latency_ms, after.latency_ms
    );
    if let Some(saved) = before_in
        .checked_sub(after_in)
        .and_then(|saved| saved.checked_mul(100))
        .and_then(|saved| saved.checked_div(before_in))
    {
        println!("\ninput tokens saved: {saved}%");
    }
    println!(
        "\nboth chose: {} / {}",
        before.decision.operation.as_str(),
        after.decision.operation.as_str()
    );
    Ok(())
}
