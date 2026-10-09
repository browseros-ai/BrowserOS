//! What a real decision costs, measured against the recorded baseline.
//!
//! Builds the actual question set this crate sends, over pages of several
//! sizes, and reports the input tokens the endpoint billed. The baseline is
//! 13,871 input tokens per decision, measured on the previous implementation.
//!
//! cargo run -p browseros-decide --example measure_step
#![allow(clippy::print_stdout)]

use browseros_decide::space::{ActionSpace, MAX_OFFERED};
use browseros_decide::view::{Control, ControlState, PageView};
use browseros_decide::{Jev, questions};

/// The figure this design has to beat, from benchmark round 5.
const BASELINE: f64 = 13_871.0;

fn control(reference: &str, role: &str, name: &str) -> Control {
    Control {
        reference: reference.to_string(),
        clickable: false,
        role: role.to_string(),
        name: name.to_string(),
        value: None,
        state: ControlState::default(),
        options: Vec::new(),
        guard: 0,
    }
}

/// A listing page of a given size, shaped like the ones the benchmarks used:
/// navigation, a brand facet, a sort select, and a lot of results.
fn listing(results: usize, text_bytes: usize) -> PageView {
    let mut controls = vec![
        control("e1", "link", "Your Account"),
        control("e2", "link", "Customer Service"),
        control("e5", "searchbox", "Search"),
    ];
    let mut sort = control("e99", "combobox", "Sort by:");
    sort.options = vec![
        "Featured".to_string(),
        "Price: Low to High".to_string(),
        "Price: High to Low".to_string(),
    ];
    controls.push(sort);
    for (index, brand) in ["Corsair", "Kingston", "Crucial", "ADATA"]
        .iter()
        .enumerate()
    {
        let mut facet = control(&format!("b{index}"), "checkbox", brand);
        facet.state.checked = Some(false);
        controls.push(facet);
    }
    for index in 0..results {
        controls.push(control(
            &format!("p{index}"),
            "link",
            &format!("Some DDR5 memory kit, model number {index}, 16GB"),
        ));
    }
    PageView {
        url: "https://www.example.in/s?k=32gb+ddr5".to_string(),
        title: "32gb ddr5".to_string(),
        text: "Brand\nCorsair\nKingston\n".repeat(text_bytes / 24),
        controls,
        fingerprint: 1,
    }
}

#[tokio::main]
async fn main() {
    let token = std::env::var("JEV_TOKEN").expect("JEV_TOKEN must be set in the environment");
    let jev = Jev::new(&token);
    let goal = "Narrow the results to the Corsair brand and sort by price, lowest first";

    println!("goal: {goal}");
    println!("offering at most {MAX_OFFERED} candidates per question\n");
    println!(
        "{:>8}  {:>10}  {:>9}  {:>9}  {:>13}  {:>9}",
        "results", "text bytes", "offered", "omitted", "input tokens", "vs base"
    );

    for (results, text_bytes) in [(10, 600), (50, 1_200), (200, 2_400), (600, 6_000)] {
        let mut view = listing(results, text_bytes);
        view.budget_text(1_500);
        let space = ActionSpace::build(&view, goal, None);
        let operations = questions::available(&space, &view, false);
        let built = questions::build(goal, &space, &operations);
        let state = questions::state(&view, &space, &[]);

        match jev.ask(&state, &built).await {
            Ok(response) => {
                let tokens = response.usage.input_tokens;
                println!(
                    "{results:>8}  {text_bytes:>10}  {:>9}  {:>9}  {tokens:>13}  {:>8.1}x",
                    space.offered.len(),
                    space.omitted,
                    BASELINE / f64::from(u32::try_from(tokens).unwrap_or(u32::MAX))
                );
            }
            Err(error) => {
                println!("{results:>8}  {text_bytes:>10}  FAILED: {error}");
                std::process::exit(1);
            }
        }
    }
    println!("\nbaseline {BASELINE:.0} input tokens per decision, benchmark round 5");
    println!("the window is 32,000 tokens and the floor for any request is about 280");
}
