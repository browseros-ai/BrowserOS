//! End-to-end checks for the three held findings, against the real endpoint.
//!
//! Each one drives the real question builder and the real gate, so the
//! behaviour checked here is the behaviour a run gets, not a scripted
//! approximation.
//!
//! cargo run -p browseros-decide --example blocker_checks
#![allow(clippy::print_stdout)]

use browseros_decide::operations::Operation;
use browseros_decide::space::ActionSpace;
use browseros_decide::view::{Control, ControlState, PageView};
use browseros_decide::{Jev, Verdict, gate, questions};

fn control(reference: &str, role: &str, name: &str) -> Control {
    Control {
        reference: reference.to_string(),
        role: role.to_string(),
        name: name.to_string(),
        value: None,
        state: ControlState::default(),
        options: Vec::new(),
        guard: 0,
    }
}

fn cart(text: &str) -> PageView {
    PageView::from_snapshot_parts(
        "https://example.com/cart",
        "Cart",
        text,
        vec![
            control("e1", "button", "Continue"),
            control("e2", "link", "Home"),
        ],
    )
}

/// The three of the eight that the plan said need a live check rather than only
/// a unit test: the retry and deadline contract against a real call, the key's
/// confidence against a real press decision, and the indeterminate state
/// against a page that has one.
async fn remaining_checks(jev: &Jev, failures: &mut u32) {
    // The deadline. A budget smaller than any real round trip must give up
    // rather than wait, and it must say it timed out rather than something else.
    println!("\nthe deadline: does an impossible budget give up?");
    let mut trivial = std::collections::BTreeMap::new();
    trivial.insert(
        "q".to_string(),
        browseros_decide::Question::Noul {
            instructions: serde_json::json!("Is this a test?"),
            criteria: None,
        },
    );
    let started = std::time::Instant::now();
    match jev
        .ask_within(
            &serde_json::json!("a state"),
            &trivial,
            std::time::Duration::from_millis(1),
        )
        .await
    {
        Err(browseros_decide::JevError::TimedOut(_)) => {
            println!(
                "  timed out after {:?}, as a timeout rather than a transport error",
                started.elapsed()
            );
        }
        Err(other) => {
            println!("  gave up as {other}, which is not a timeout");
            *failures += 1;
        }
        Ok(_) => {
            println!("  it answered within a millisecond, so this check proves nothing");
        }
    }

    // And a real budget still works, so the deadline bounds rather than breaks.
    println!("\nthe deadline: does a real budget still answer?");
    match jev
        .ask_within(
            &serde_json::json!("The sky is blue."),
            &trivial,
            std::time::Duration::from_secs(30),
        )
        .await
    {
        Ok(response) => println!(
            "  answered, input {} tokens, model {}",
            response.usage.input_tokens, response.model
        ),
        Err(error) => {
            println!("  FAILED: {error}");
            *failures += 1;
        }
    }

    // The key's confidence. A page whose control cannot be clicked is where a
    // press is the right answer, so this asks for one and reads every part.
    println!("\nthe key: is every judgement of a press reported?");
    let mut covered = control("e9", "combobox", "Sort by:");
    covered.options = vec!["Featured".to_string(), "Price: Low to High".to_string()];
    let page = PageView::from_snapshot_parts(
        "https://example.com/s",
        "Results",
        "Sort by: Featured",
        vec![covered, control("e1", "link", "Home")],
    );
    let goal = "Open the sort dropdown using the keyboard, because a click is blocked";
    let space = ActionSpace::build(&page, goal, None);
    // Press is offered once something has refused a click, which is this case.
    let operations = questions::available(&space, &page, true);
    let built = questions::build(goal, &space, &operations);
    let state = questions::state(&page, &space, &[]);
    match jev.ask(&state, &built).await {
        Ok(response) => {
            let decision = gate::read(&response).expect("reads");
            let (weakest, confidence) = decision.weakest();
            println!(
                "  chose {} key {:?}; weakest judgement was {} at {confidence:.2}",
                decision.operation.as_str(),
                decision.key,
                weakest.as_str()
            );
            if decision.operation == Operation::Press && decision.key_confidence.is_none() {
                println!("  FAILED: a press came back with no confidence for its key");
                *failures += 1;
            }
        }
        Err(error) => {
            println!("  FAILED: {error}");
            *failures += 1;
        }
    }

    // The indeterminate state. The model has to be told "some of this group",
    // not "none of it".
    println!("\nthe mixed checkbox: is a partial selection described as one?");
    let mut parent = control("e2", "checkbox", "All brands");
    parent.state.indeterminate = true;
    let page = PageView::from_snapshot_parts(
        "https://example.com/s",
        "Results",
        "Brand\nAll brands\nCorsair\nKingston",
        vec![parent, control("e3", "checkbox", "Corsair")],
    );
    let goal = "Clear every brand filter so no brand is selected";
    let space = ActionSpace::build(&page, goal, None);
    let described = space.offered.iter().any(|candidate| {
        candidate
            .descriptor
            .get("current")
            .and_then(|value| value.as_str())
            .is_some_and(|state| state.contains("partially"))
    });
    println!("  the descriptor says partially selected: {described}");
    if !described {
        println!("  FAILED: the model would be told nothing is selected");
        *failures += 1;
    }
    let operations = questions::available(&space, &page, false);
    let built = questions::build(goal, &space, &operations);
    let state = questions::state(&page, &space, &[]);
    match jev.ask(&state, &built).await {
        Ok(response) => {
            let decision = gate::read(&response).expect("reads");
            println!(
                "  asked to clear a partial selection, it chose {} on {:?}",
                decision.operation.as_str(),
                decision.target
            );
        }
        Err(error) => {
            println!("  FAILED: {error}");
            *failures += 1;
        }
    }
}

#[tokio::main]
async fn main() {
    let token = std::env::var("JEV_TOKEN").expect("JEV_TOKEN must be set in the environment");
    let jev = Jev::new(&token);
    let mut failures = 0;

    // Blocker 3: the text a decision was read from is part of the page's
    // identity, so a page whose copy changed is not the page a claim was made
    // about.
    println!("blocker 3: does changed page text invalidate a completion claim?");
    let before = cart("Item added to your cart. Your order is complete.");
    let after = cart("Out of stock. Nothing was added.");
    println!(
        "  url and controls identical: {}",
        before.url == after.url && before.controls == after.controls
    );
    let fresh = after.is_fresh_for(before.fingerprint);
    println!("  after the text changed, still the same page: {fresh}");
    if fresh {
        println!("  FAILED: the claim would be accepted against text it was not read from");
        failures += 1;
    }
    let guard_held =
        after.control_is_fresh(&before.controls[0].reference, before.controls[0].guard);
    println!("  and the control's own guard still holds: {guard_held}");
    if !guard_held {
        println!("  FAILED: unrelated copy moved a control's identity, which would refuse work");
        failures += 1;
    }

    // Blocker 1: a completion claim arrives with the confidence behind it, and
    // is reported rather than refused.
    println!("\nblocker 1: does a real completion claim carry its confidence?");
    let done = cart("Item added to your cart. Your order is complete.");
    let space = ActionSpace::build(&done, "Confirm the item is in the cart", None);
    let operations = questions::available(&space, &done, false);
    let built = questions::build("Confirm the item is in the cart", &space, &operations);
    let state = questions::state(&done, &space, &[]);
    match jev.ask(&state, &built).await {
        Ok(response) => {
            let decision = gate::read(&response).expect("reads");
            println!(
                "  chose {} at confidence {:.2}, input {}",
                decision.operation.as_str(),
                decision.operation_confidence,
                response.usage.input_tokens
            );
            if decision.operation.is_terminal() {
                println!("  terminal, so it is reported with its number and never gated");
            } else {
                println!("  not terminal on this page, which is a fair answer; the gate applies");
            }
        }
        Err(error) => {
            println!("  FAILED: {error}");
            failures += 1;
        }
    }

    // Blocker 2: when nothing offered can advance the goal, the run hands back
    // instead of circling.
    println!("\nblocker 2: does an impossible goal hand back rather than circle?");
    let page = cart("Your cart is empty.");
    let goal = "Tick the Corsair brand filter checkbox in the left sidebar";
    let space = ActionSpace::build(&page, goal, None);
    let operations = questions::available(&space, &page, false);
    let built = questions::build(goal, &space, &operations);
    let state = questions::state(&page, &space, &[]);
    match jev.ask(&state, &built).await {
        Ok(response) => {
            let decision = gate::read(&response).expect("reads");
            let target_name = decision
                .target
                .as_deref()
                .and_then(|reference| page.control(reference))
                .map(|control| control.name.clone());
            println!(
                "  chose {} target {:?}, none_of_these={}",
                decision.operation.as_str(),
                decision.target,
                decision.none_of_these
            );
            if decision.operation == Operation::Blocked {
                println!(
                    "  BLOCKED at the operation, which is the coherent answer when nothing on the"
                );
                println!(
                    "  page can advance the goal, and is the common route to the caller deciding."
                );
                println!(
                    "  The loop ends there and reports what the decision could see. The gate is"
                );
                println!("  not consulted: a terminal answer is a report, not an action.");
            } else {
                let verdict = gate::verdict(&decision, target_name.as_deref());
                println!("  verdict: {verdict:?}");
                if !matches!(verdict, Verdict::HandBack(_)) {
                    println!("  NOTE: it chose to act, so this page did not reach the escape path");
                }
            }
        }
        Err(error) => {
            println!("  FAILED: {error}");
            failures += 1;
        }
    }

    remaining_checks(&jev, &mut failures).await;

    println!(
        "\n{}",
        if failures == 0 {
            "all checks passed"
        } else {
            "CHECKS FAILED"
        }
    );
    if failures > 0 {
        std::process::exit(1);
    }
}
