//! Live gate for one complete step.
//!
//! Sends the whole request, operation plus every argument head, and checks the
//! three things that cannot be checked locally: that every question comes back
//! answered, that confidence is present on a Choice and absent on a Noul, and
//! that a native select can actually be driven by value.
//!
//! cargo run -p browseros-decide --example full_step
#![allow(clippy::print_stdout)]

use browseros_decide::operations::Operation;
use browseros_decide::space::ActionSpace;
use browseros_decide::view::{Control, ControlState, PageView};
use browseros_decide::{Jev, Question, gate, questions};
use serde_json::json;

fn control(reference: &str, role: &str, name: &str) -> Control {
    Control {
        reference: reference.to_string(),
        role: role.to_string(),
        name: name.to_string(),
        value: None,
        state: ControlState::default(),
        options: Vec::new(),
        guard: 7,
    }
}

/// A listing page with the sort dropdown that the previous implementation could
/// never operate, because its options are not clickable.
fn page() -> PageView {
    let mut sort = control("e99", "combobox", "Sort by:");
    sort.options = vec![
        "Featured".to_string(),
        "Price: Low to High".to_string(),
        "Price: High to Low".to_string(),
        "Newest Arrivals".to_string(),
    ];
    let mut corsair = control("e34", "checkbox", "Corsair");
    corsair.state.checked = Some(false);
    let mut kingston = control("e35", "checkbox", "Kingston");
    kingston.state.checked = Some(true);

    PageView {
        url: "https://www.example.in/s?k=32gb+ddr5".to_string(),
        title: "32gb ddr5".to_string(),
        text: "Brand\nCorsair\nKingston\nResults 1-16 of 200".to_string(),
        controls: vec![
            control("e1", "link", "Your Account"),
            control("e5", "searchbox", "Search"),
            sort,
            corsair,
            kingston,
            control("e40", "button", "Place your order"),
        ],
        fingerprint: 1,
    }
}

async fn step(jev: &Jev, goal: &str, expect: Operation) {
    let view = page();
    let space = ActionSpace::build(&view, goal, None);
    let operations = questions::available(&space, &view);
    let built = questions::build(goal, &space, &operations);
    let state = questions::state(&view, &space, &[]);

    println!("\ngoal: {goal}");
    println!(
        "  offered {} operations, {} questions",
        operations.len(),
        built.len()
    );

    match jev.ask(&state, &built).await {
        Ok(response) => {
            for id in built.keys() {
                assert!(
                    response.answers.contains_key(id),
                    "question {id} came back unanswered"
                );
            }
            let decision = gate::read(&response).expect("the answer reads");
            let target_name = decision
                .target
                .as_deref()
                .and_then(|reference| view.control(reference))
                .map(|control| control.name.clone());
            let verdict = gate::verdict(&decision, target_name.as_deref());
            println!(
                "  chose {:<12} target {:<6} value {:<22} confidence {:.2}  input {}",
                decision.operation.as_str(),
                decision.target.clone().unwrap_or_else(|| "-".to_string()),
                decision.value.clone().unwrap_or_else(|| "-".to_string()),
                decision.confidence(),
                decision.input_tokens
            );
            println!(
                "  verdict {verdict:?}  {}",
                if decision.operation == expect {
                    "AS EXPECTED"
                } else {
                    "NOT what was expected"
                }
            );
        }
        Err(error) => {
            println!("  FAILED: {error}");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    let token = std::env::var("JEV_TOKEN").expect("JEV_TOKEN must be set in the environment");
    let jev = Jev::new(&token);

    // A Noul must come back without a confidence, which is what the docs say
    // and what our parsing relies on.
    let mut noul = std::collections::BTreeMap::new();
    noul.insert(
        "sure".to_string(),
        Question::Noul {
            instructions: json!("Is the sky blue?"),
            criteria: None,
        },
    );
    let response = jev
        .ask(&json!("The sky is blue."), &noul)
        .await
        .expect("asks");
    let answer = &response.answers["sure"];
    assert!(answer.noul.is_some(), "a noul returns a probability");
    assert!(
        answer.confidence.is_none(),
        "a noul has no confidence, and parsing must not invent one"
    );
    println!("noul returns a probability and no confidence, as documented");

    // The dropdown, which is the failure this whole design is aimed at.
    step(
        &jev,
        "Sort these results by price, lowest first",
        Operation::Select,
    )
    .await;
    // A filter that is not yet applied.
    step(
        &jev,
        "Narrow the results to the Corsair brand",
        Operation::Check,
    )
    .await;
    // One already in the requested state: the model should not undo it.
    step(
        &jev,
        "Make sure Kingston is included in the results",
        Operation::Done,
    )
    .await;
}
