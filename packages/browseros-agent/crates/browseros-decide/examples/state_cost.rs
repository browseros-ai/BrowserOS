//! Live gate for the state's shape and cost.
//!
//! Sends a page state to the real endpoint and reports what it actually cost,
//! from the tokeniser that bills us, rather than estimating. Every budget in
//! this crate is measured against this number.
//!
//! cargo run -p browseros-decide --example state_cost
#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;

use browseros_decide::{Jev, Question};
use serde_json::{Value, json};

/// Builds a state the shape the decision layer sends: a named page object, a
/// list of candidate descriptors, and the recent actions.
fn state(controls: usize, text_bytes: usize) -> Value {
    let descriptors: Vec<Value> = (0..controls)
        .map(|index| {
            json!({
                "control": format!("link \"Corsair Vengeance DDR5 32GB kit {index}\""),
                "current": "",
                "role": "link",
            })
        })
        .collect();
    json!({
        "page": {
            "url": "https://www.example.com/s?k=32gb+ddr5",
            "title": "Search results",
            "text": "x".repeat(text_bytes),
        },
        "controls": descriptors,
        "recent": [],
    })
}

#[tokio::main]
async fn main() {
    let token = std::env::var("JEV_TOKEN").expect("JEV_TOKEN must be set in the environment");
    let jev = Jev::new(&token);

    let mut questions = BTreeMap::new();
    questions.insert(
        "actionable".to_string(),
        Question::Noul {
            instructions: json!("Does `controls` contain a control that could filter by brand?"),
            criteria: None,
        },
    );

    println!(
        "{:>9}  {:>10}  {:>13}  {:>12}",
        "controls", "text bytes", "input tokens", "model"
    );
    for (controls, text_bytes) in [(0, 0), (40, 1500), (40, 6000), (250, 6000)] {
        match jev.ask(&state(controls, text_bytes), &questions).await {
            Ok(response) => println!(
                "{controls:>9}  {text_bytes:>10}  {:>13}  {:>12}",
                response.usage.input_tokens, response.model
            ),
            Err(error) => {
                println!("{controls:>9}  {text_bytes:>10}  FAILED: {error}");
                std::process::exit(1);
            }
        }
    }
    println!(
        "\nthe 32,000 token window and the 276 token floor are the two bounds to read this against"
    );
}
