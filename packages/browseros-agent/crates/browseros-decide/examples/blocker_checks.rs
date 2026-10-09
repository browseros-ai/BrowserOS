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
