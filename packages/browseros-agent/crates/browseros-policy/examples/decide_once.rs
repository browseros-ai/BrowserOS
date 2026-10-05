//! Asks for one decision and prints what it cost, without touching a browser.
//!
//! The measurement tool behind the mode's latency and token figures. Reads the
//! credential from `JEV_TOKEN` and makes exactly one request.
//!
//! cargo run -p browseros-policy --example decide_once

use browseros_policy::action::{ActionSpace, Element};
use browseros_policy::decide::Decider;
use browseros_policy::questions::Observation;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("JEV_TOKEN")
        .map_err(|_| "set JEV_TOKEN to the credential you want to measure with")?;

    // A flight search with the destination already filled, so the correct next
    // step is typing the origin rather than submitting.
    let elements = vec![
        Element::new("e1", "combobox", "Where from?"),
        Element::new("e2", "combobox", "Where to?"),
        Element::new("e3", "button", "Search"),
        Element::new("e4", "button", "Clear all"),
    ];
    let observation = Observation {
        url: "https://example.com/travel/flights".to_string(),
        title: "Flight search".to_string(),
        tree: concat!(
            "- combobox \"Where from?\" [ref=e1]\n",
            "- combobox \"Where to?\" [ref=e2]: \"Zurich\"\n",
            "- button \"Search\" [ref=e3]\n",
            "- button \"Clear all\" [ref=e4]"
        )
        .to_string(),
        space: ActionSpace::new(elements, true, false),
    };

    let step = Decider::new(token)
        .decide(
            "Find one-way flights from Zurich to London on 20 September 2026.",
            &observation,
            &[],
        )
        .await?;

    println!("model      {}", step.model);
    println!("latency    {} ms", step.latency_ms);
    println!(
        "tokens     {} in, {} out",
        step.input_tokens.unwrap_or(0),
        step.output_tokens.unwrap_or(0)
    );
    if step.dropped_targets > 0 {
        println!("dropped    {} targets over the cap", step.dropped_targets);
    }
    println!();
    println!(
        "operation  {} (confidence {:.3})",
        step.decision.operation.as_str(),
        step.decision.operation_confidence
    );
    println!(
        "target     {:?} (confidence {:?})",
        step.decision.target, step.decision.target_confidence
    );
    println!("satisfied  P(true) {:.3}", step.decision.satisfied);
    println!("progress   {:.3} of 2", step.decision.progress);
    let mut ranked: Vec<_> = step.decision.operation_probabilities.iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    println!();
    for (operation, probability) in ranked {
        println!("  {operation:<12} {probability:.3}");
    }
    Ok(())
}
