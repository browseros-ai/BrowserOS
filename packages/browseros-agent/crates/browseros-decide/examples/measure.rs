//! Measures one real run of the loop against a real page.
//!
//! The loop is abstracted over [`Driver`], so the only thing missing outside the
//! server is a browser. This supplies one by shelling out to `agent-browser`,
//! which means a change to the question payload can be measured end to end
//! without building the browser or standing up the MCP surface.
//!
//! What it prints is what the benchmark scores: the ending, how many decisions
//! and question batches it took to get there, the tokens and the wall clock, and
//! every decision's margin against the bar it had to clear.
//!
//! ```text
//! JEV_TOKEN=... cargo run --example measure -- \
//!     --url "https://example.com/results" \
//!     --goal "add the first widget to the cart" \
//!     --value "search term=widget"
//! ```

use browseros_decide::client::Jev;
use browseros_decide::drive::{ActError, Driver, Journey, Move, Performed};
use browseros_decide::offer::{Destination, NamedValue};
use browseros_decide::pursue::advance;
use browseros_decide::run::{Budget, Offered, Run};
use browseros_decide::view::{Control, ControlState, PageView};
use browseros_decide::vocabulary::{Direction, How, Operation};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::process::Command;
use std::time::Instant;

/// One browser, driven by the `agent-browser` command line.
struct Shell {
    session: String,
}

impl Shell {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let mut argv = vec!["--session", self.session.as_str()];
        argv.extend_from_slice(args);
        let output = Command::new("agent-browser")
            .args(&argv)
            .output()
            .map_err(|error| error.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if output.status.success() {
            Ok(stdout)
        } else {
            Err(format!(
                "{}{}",
                stdout,
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    fn get(&self, what: &str) -> String {
        self.run(&["get", what])
            .unwrap_or_default()
            .trim()
            .to_string()
    }
}

/// Reads one `[key=value]` or bare `[key]` attribute out of a snapshot line.
fn attribute(line: &str, key: &str) -> Option<String> {
    let brackets = line.rfind('[')?;
    let inside = &line[brackets + 1..line.rfind(']')?];
    for part in inside.split(", ") {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(key) {
            if rest.is_empty() {
                return Some("true".to_string());
            }
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Parses the indented snapshot the `agent-browser` renderer writes.
///
/// Document order is kept, because the order controls arrive in is the only
/// thing that says which price belongs to which button.
fn controls_from(snapshot: &str) -> (Vec<Control>, usize) {
    let mut controls: Vec<Control> = Vec::new();
    let mut unlabelled_images = 0usize;
    for line in snapshot.lines() {
        let trimmed = line.trim_start();
        let Some(body) = trimmed.strip_prefix("- ") else {
            continue;
        };
        let role = body
            .split([' ', ' '])
            .next()
            .unwrap_or_default()
            .trim_end_matches(':')
            .to_string();
        let name = body
            .find('"')
            .and_then(|start| {
                let rest = &body[start + 1..];
                rest.find('"').map(|end| rest[..end].to_string())
            })
            .unwrap_or_default();
        if (role == "img" || role == "image") && name.is_empty() {
            unlabelled_images += 1;
        }
        let Some(reference) = attribute(trimmed, "ref") else {
            continue;
        };
        // A combobox prints its options as children; the parser keeps them on
        // the parent, which is where the loop expects selectable values.
        if role == "option" {
            if let Some(parent) = controls.last_mut()
                && !name.is_empty()
            {
                parent.options.push(name);
            }
            continue;
        }
        let value = body.rsplit_once(": ").map(|(_, after)| after.to_string());
        let state = ControlState {
            checked: attribute(trimmed, "checked").map(|held| held == "true"),
            expanded: attribute(trimmed, "expanded").map(|held| held == "true"),
            selected: attribute(trimmed, "selected").map(|held| held == "true"),
            disabled: attribute(trimmed, "disabled").is_some(),
            required: attribute(trimmed, "required").is_some(),
            editable: matches!(
                role.as_str(),
                "textbox" | "searchbox" | "spinbutton" | "combobox"
            ),
            indeterminate: false,
        };
        let mut hasher = DefaultHasher::new();
        (&reference, &role, &name).hash(&mut hasher);
        controls.push(Control {
            reference,
            clickable: matches!(
                role.as_str(),
                "button" | "link" | "menuitem" | "tab" | "option" | "checkbox" | "radio"
            ),
            role,
            name,
            value,
            state,
            options: Vec::new(),
            guard: hasher.finish(),
            group: None,
        });
    }
    (controls, unlabelled_images)
}

impl Driver for Shell {
    fn stopped(&self) -> bool {
        false
    }

    async fn observe(&self) -> Result<PageView, String> {
        // Settle by watching the snapshot stop growing. Waiting for the network
        // to go idle never returns on a commercial page.
        let mut last = 0usize;
        let mut snapshot = String::new();
        for _ in 0..8 {
            snapshot = self.run(&["snapshot", "-i"])?;
            if snapshot.len() == last {
                break;
            }
            last = snapshot.len();
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let (controls, unlabelled_images) = controls_from(&snapshot);
        let text = self.run(&["read"]).unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        snapshot.hash(&mut hasher);
        Ok(PageView {
            url: self.get("url"),
            title: self.get("title"),
            text: text.chars().take(4000).collect(),
            controls,
            fingerprint: hasher.finish(),
            arrangement: hasher.finish(),
            unlabelled_images,
        })
    }

    async fn perform(&self, act: &Move) -> Result<Performed, ActError> {
        let before = self.run(&["snapshot", "-i"]).unwrap_or_default();
        let outcome = match act {
            Move::Settle => self.run(&["wait", "1200"]),
            Move::Travel(journey) => match journey {
                Journey::To(destination) => self.run(&["open", destination.url.as_str()]),
                Journey::Back => self.run(&["back"]),
                Journey::Again => self.run(&["reload"]),
                Journey::Tab(_) => return Err(ActError::NotOffered(Operation::Tabs)),
            },
            Move::Act(decision) => {
                let target = decision.target.clone().unwrap_or_default();
                let aim = format!("@{target}");
                match decision.how {
                    Some(How::Click) => self.run(&["click", &aim]),
                    Some(How::Fill) => {
                        let value = decision.value.clone().unwrap_or_default();
                        self.run(&["fill", &aim, value.as_str()])
                    }
                    Some(How::Submit) => self.run(&["press", "Enter"]),
                    Some(How::Select) => {
                        let value = decision.value.clone().unwrap_or_default();
                        self.run(&["select", &aim, value.as_str()])
                    }
                    Some(How::Check) => self.run(&["check", &aim]),
                    Some(How::Uncheck) => self.run(&["uncheck", &aim]),
                    Some(How::Hover) => self.run(&["hover", &aim]),
                    Some(How::Press) => {
                        let key = decision.key.clone().unwrap_or_else(|| "Enter".to_string());
                        self.run(&["press", key.as_str()])
                    }
                    Some(How::Scroll) => {
                        let way = match decision.direction {
                            Some(Direction::Up) => "up",
                            _ => "down",
                        };
                        self.run(&["scroll", way, "600"])
                    }
                    None => return Err(ActError::NotOffered(Operation::Act)),
                }
            }
        };
        outcome.map_err(ActError::Refused)?;
        std::thread::sleep(std::time::Duration::from_millis(900));
        let after = self.run(&["snapshot", "-i"]).unwrap_or_default();
        if after == before {
            Ok(Performed::Unmoved)
        } else {
            Ok(Performed::Moved)
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut goal = String::new();
    let mut url = String::new();
    let mut session = "jevbench".to_string();
    let mut values: Vec<NamedValue> = Vec::new();
    let mut destinations: Vec<Destination> = Vec::new();
    let mut max_steps = 12u32;
    let mut label = "run".to_string();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let next = args.get(index + 1).cloned().unwrap_or_default();
        match flag {
            "--goal" => goal = next,
            "--url" => url = next,
            "--session" => session = next,
            "--label" => label = next,
            "--max-steps" => max_steps = next.parse().unwrap_or(12),
            "--value" => {
                if let Some((name, text)) = next.split_once('=') {
                    values.push(NamedValue::new(name, text));
                }
            }
            "--dest" => {
                if let Some((name, href)) = next.split_once('=') {
                    destinations.push(Destination {
                        label: name.to_string(),
                        url: href.to_string(),
                        new_tab: false,
                    });
                }
            }
            other => eprintln!("ignoring {other}"),
        }
        index += 2;
    }

    let token = std::env::var("JEV_TOKEN").unwrap_or_default();
    if token.trim().is_empty() {
        eprintln!("JEV_TOKEN is not set");
        std::process::exit(2);
    }
    let shell = Shell { session };
    if !url.is_empty()
        && let Err(error) = shell.run(&["open", url.as_str()])
    {
        eprintln!("could not open the page: {error}");
        std::process::exit(2);
    }

    let oracle = Jev::new(token);
    let run = Run::new(
        &goal,
        Budget {
            max_steps,
            max_seconds: 180,
        },
    )
    .offering(Offered {
        destinations,
        values,
        tabs: Vec::new(),
        unavailable: vec![Operation::Tabs],
    });

    let started = Instant::now();
    let outcome = advance(&oracle, &shell, run).await;
    let seconds = started.elapsed().as_secs_f64();

    println!(
        "LABEL={label} STATUS={} actions={} changed={} decisions={} batches={} offered={} \
         omitted={} retired={} tokens={} seconds={seconds:.1} goalMet={:.2}",
        outcome.status.as_str(),
        outcome.trail.len(),
        outcome
            .trail
            .iter()
            .filter(|step| step.page_changed)
            .count(),
        outcome.decisions,
        outcome.batches,
        outcome.offered,
        outcome.omitted,
        outcome.retired,
        outcome.input_tokens,
        outcome.goal_met.unwrap_or(0.0),
    );
    println!("  url  {}", outcome.url_after);
    for decided in &outcome.decided {
        println!(
            "  {:>11} {:<7} {:<6} b{} margin={} needed={} share={} {} {}",
            decided.operation,
            decided.how.unwrap_or(""),
            decided.verdict,
            decided.batch,
            decided
                .margin
                .map_or("-".to_string(), |held| format!("{held:.2}")),
            decided
                .needed
                .map_or("-".to_string(), |held| format!("{held:.2}")),
            decided
                .share
                .map_or("-".to_string(), |held| format!("{held:.2}")),
            decided.target_name.clone().unwrap_or_default(),
            decided.why.clone().unwrap_or_default(),
        );
    }
    for step in &outcome.trail {
        for moved in &step.moved {
            println!(
                "  moved {:?} {:?} -> {:?}",
                moved.reference, moved.was, moved.now
            );
        }
    }
}
