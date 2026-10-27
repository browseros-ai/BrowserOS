//! Every tool the server offers is wrapped by a command, or skipped on purpose.
//!
//! A wrapper nobody can trust is worse than no wrapper, and the way a wrapper
//! stops being trustworthy is slow drift: a tool is added to the server and
//! the CLI never learns about it. This test makes that fail CI until the CLI
//! either wraps it or records a reason not to.

use std::collections::BTreeSet;

use clap::Parser;
use claw_server::cli::grammar::Cli;
use claw_server::cli::plan::{Plan, build};
use serde_json::Value;

/// Every argv worth planning, which between them must touch every tool.
///
/// Kept as argv rather than as a tool list so adding a command here also
/// proves it parses.
const PROBES: &[&[&str]] = &[
    &["browseros", "tab", "list"],
    &["browseros", "open", "https://x.test"],
    &["browseros", "group", "list"],
    &["browseros", "window", "list"],
    &["browseros", "history"],
    &["browseros", "goto", "https://x.test"],
    &["browseros", "snapshot"],
    &["browseros", "diff"],
    &["browseros", "click", "@e1"],
    &["browseros", "download", "@e1"],
    &["browseros", "upload", "@e1", "f.txt"],
    &["browseros", "read"],
    &["browseros", "grep", "x"],
    &["browseros", "screenshot"],
    &["browseros", "pdf"],
    &["browseros", "wait", "--time", "10"],
    &["browseros", "eval", "return 1"],
    &["browseros", "run", "return 1"],
    &["browseros", "profile", "list"],
    &["browseros", "credentials", "list"],
    &["browseros", "cards", "list"],
    &["browseros", "target", "get"],
    &["browseros", "session", "name", "price check"],
    &["browseros", "skill", "save", "x", "--description", "d"],
    &["browseros", "skill", "ran", "x"],
    &["browseros", "pursue", "do the thing"],
    &["browseros", "help", "request", "sign in"],
    &["browseros", "help", "wait"],
];

fn wrapped() -> BTreeSet<String> {
    let mut tools = BTreeSet::new();
    for argv in PROBES {
        let cli = Cli::try_parse_from(*argv).unwrap_or_else(|error| panic!("{argv:?}: {error}"));
        match build(&cli.command, Some("source")) {
            Ok(Plan::Tool(plan)) => {
                tools.insert(plan.tool.to_string());
            }
            Ok(Plan::Local(_)) => {}
            Err(error) => panic!("{argv:?}: {error}"),
        }
    }
    tools
}

fn snapshot() -> Value {
    let raw = include_str!("fixtures/cli-tools.json");
    serde_json::from_str(raw).unwrap_or_else(|error| panic!("fixtures/cli-tools.json: {error}"))
}

fn names(snapshot: &Value, key: &str) -> Vec<String> {
    snapshot
        .get(key)
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn every_tool_in_the_snapshot_is_wrapped_or_skipped_with_a_reason() {
    let snapshot = snapshot();
    let wrapped = wrapped();
    let skipped = snapshot
        .get("skipped")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut unaccounted = Vec::new();
    for key in ["catalog", "server", "hostedOnly"] {
        for tool in names(&snapshot, key) {
            if wrapped.contains(&tool) || skipped.contains_key(&tool) {
                continue;
            }
            unaccounted.push(tool);
        }
    }
    assert!(
        unaccounted.is_empty(),
        "these server tools have no command and no recorded skip: {unaccounted:?}\n\
         Add a command, or add an entry to tests/fixtures/cli-tools.json explaining why not."
    );
}

/// A tool the CLI wraps only part of must say which part and why.
///
/// `credentials` and `payment_cards` each have a safe half that works by
/// reference and a half that carries a password or a card number. The CLI
/// offers the first and not the second, and that is a decision worth being
/// unable to lose.
#[test]
fn a_partly_wrapped_tool_records_which_part_and_why() {
    let snapshot = snapshot();
    let wrapped = wrapped();
    let partial = snapshot
        .get("partial")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    assert!(!partial.is_empty(), "the partial list should not be empty");

    for (tool, reason) in &partial {
        assert!(
            wrapped.contains(tool),
            "{tool} is recorded as partly wrapped but no command calls it"
        );
        let reason = reason.as_str().unwrap_or_default();
        assert!(
            reason.len() > 60,
            "{tool}'s partial reason is too thin to be a decision: {reason}"
        );
    }
}

/// The dangerous halves must stay unreachable. If a future change adds
/// `credentials add` or a card number flag, this fails.
#[test]
fn no_command_can_put_a_password_or_a_card_number_on_the_command_line() {
    for argv in [
        vec!["browseros", "credentials", "add"],
        vec!["browseros", "credentials", "fill", "--password", "hunter2"],
        vec![
            "browseros",
            "credentials",
            "fill",
            "--username",
            "a",
            "--password",
            "b",
        ],
        vec!["browseros", "cards", "add"],
        vec!["browseros", "cards", "fill", "--number", "4111111111111111"],
        vec!["browseros", "cards", "fill", "--cvc", "123"],
    ] {
        assert!(
            Cli::try_parse_from(&argv).is_err(),
            "{argv:?} must not parse: a CLI argument lands in shell history and ps output"
        );
    }
}

/// And the safe halves must stay reachable, carrying only an opaque id.
#[test]
fn a_saved_credential_is_reachable_by_reference_with_no_secret() {
    let cli = Cli::try_parse_from(["browseros", "credentials", "fill", "--id", "cred-7"])
        .unwrap_or_else(|error| panic!("{error}"));
    let Ok(Plan::Tool(plan)) = build(&cli.command, None) else {
        panic!("it should plan a tool call");
    };
    assert_eq!(plan.tool, "credentials");
    assert_eq!(plan.args["action"], "fill");
    assert_eq!(plan.args["credentialId"], "cred-7");
    assert!(plan.needs_page, "filling targets a page's login form");
    for secret in ["password", "username", "number", "cvc"] {
        assert!(
            plan.args.get(secret).is_none(),
            "{secret} must never be sent"
        );
    }

    let cli = Cli::try_parse_from(["browseros", "cards", "fill", "--id", "card-2"])
        .unwrap_or_else(|error| panic!("{error}"));
    let Ok(Plan::Tool(plan)) = build(&cli.command, None) else {
        panic!("it should plan a tool call");
    };
    assert_eq!(plan.tool, "payment_cards");
    assert_eq!(plan.args["cardId"], "card-2");
    assert!(plan.args.get("card").is_none(), "no supplied card");
}

#[test]
fn every_skip_names_a_real_tool_and_gives_a_reason() {
    let snapshot = snapshot();
    let known: BTreeSet<String> = ["catalog", "server", "hostedOnly"]
        .iter()
        .flat_map(|key| names(&snapshot, key))
        .collect();
    let skipped = snapshot
        .get("skipped")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    assert!(!skipped.is_empty(), "the skip list should not be empty");

    for (tool, reason) in &skipped {
        assert!(known.contains(tool), "{tool} is not a tool this server has");
        let reason = reason.as_str().unwrap_or_default();
        assert!(
            reason.len() > 40,
            "{tool}'s skip reason is too thin to be a decision: {reason}"
        );
    }
}

#[test]
fn a_skipped_tool_is_never_also_wrapped() {
    let snapshot = snapshot();
    let wrapped = wrapped();
    let skipped = snapshot
        .get("skipped")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for tool in skipped.keys() {
        assert!(
            !wrapped.contains(tool),
            "{tool} is both wrapped and recorded as skipped"
        );
    }
}

#[test]
fn no_command_invents_a_tool_the_server_does_not_have() {
    let snapshot = snapshot();
    let known: BTreeSet<String> = ["catalog", "server", "hostedOnly"]
        .iter()
        .flat_map(|key| names(&snapshot, key))
        .collect();
    for tool in wrapped() {
        assert!(
            known.contains(&tool),
            "the CLI plans a call to `{tool}`, which is not in the recorded tool list"
        );
    }
}
