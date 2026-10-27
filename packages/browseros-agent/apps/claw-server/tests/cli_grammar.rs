//! Command line in, tool call out.
//!
//! This is the main test. The mapping is a pure function, needs no browser and
//! no server, and is where a surface of two dozen tools and forty-odd verbs
//! actually breaks. Everything asserted here is the exact JSON that reaches
//! the wire, because every tool rejects unknown fields and a wrong name fails
//! only at runtime otherwise.

use clap::Parser;
use claw_server::cli::grammar::Cli;
use claw_server::cli::plan::{External, Local, Plan, needs_external};
use serde_json::{Value, json};

/// Parses an argv and returns the tool name plus its arguments.
fn call(argv: &[&str]) -> (String, Value) {
    let cli = Cli::try_parse_from(argv).unwrap_or_else(|error| panic!("{argv:?}: {error}"));
    match claw_server::cli::plan::build(&cli.command, None) {
        Ok(Plan::Tool(plan)) => (plan.tool.to_string(), Value::Object(plan.args)),
        Ok(Plan::Local(local)) => panic!("{argv:?} planned a local action: {local:?}"),
        Err(error) => panic!("{argv:?}: {error}"),
    }
}

/// Parses an argv that is expected to be handled without the server.
fn local(argv: &[&str]) -> Local {
    let cli = Cli::try_parse_from(argv).unwrap_or_else(|error| panic!("{argv:?}: {error}"));
    match claw_server::cli::plan::build(&cli.command, None) {
        Ok(Plan::Local(local)) => local,
        other => panic!("{argv:?} did not plan a local action: {other:?}"),
    }
}

/// The usage error for an argv the CLI should refuse before sending anything.
fn refused(argv: &[&str]) -> String {
    let Ok(cli) = Cli::try_parse_from(argv) else {
        return "clap refused it".to_string();
    };
    match claw_server::cli::plan::build(&cli.command, None) {
        Err(error) => error.message,
        Ok(plan) => panic!("{argv:?} was accepted: {plan:?}"),
    }
}

#[test]
fn open_is_one_call_because_tabs_new_already_navigates() {
    let (tool, args) = call(&["browseros", "open", "https://example.com"]);
    assert_eq!(tool, "tabs");
    assert_eq!(args["action"], "new");
    assert_eq!(args["url"], "https://example.com");
    // No page: the tab does not exist yet.
    assert!(args.get("page").is_none(), "{args}");
}

#[test]
fn navigating_verbs_map_onto_one_tool_with_an_action() {
    for (argv, action) in [
        (vec!["browseros", "back"], "back"),
        (vec!["browseros", "forward"], "forward"),
        (vec!["browseros", "reload"], "reload"),
    ] {
        let (tool, args) = call(&argv);
        assert_eq!(tool, "navigate", "{argv:?}");
        assert_eq!(args["action"], action, "{argv:?}");
    }
    let (tool, args) = call(&["browseros", "goto", "https://x.test"]);
    assert_eq!(tool, "navigate");
    assert_eq!(args["action"], "url");
    assert_eq!(args["url"], "https://x.test");
}

#[test]
fn every_acting_verb_becomes_one_act_call_with_its_kind() {
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec!["browseros", "click", "@e1"], "click"),
        (vec!["browseros", "fill", "@e1", "x"], "fill"),
        (vec!["browseros", "type", "x"], "type"),
        (vec!["browseros", "press", "Enter"], "press"),
        (vec!["browseros", "hover", "@e1"], "hover"),
        (vec!["browseros", "focus", "@e1"], "focus"),
        (vec!["browseros", "check", "@e1"], "check"),
        (vec!["browseros", "uncheck", "@e1"], "uncheck"),
        (vec!["browseros", "select", "@e1", "Large"], "select"),
        (vec!["browseros", "scroll", "down"], "scroll"),
        (vec!["browseros", "drag", "@e1", "@e2"], "drag"),
        (vec!["browseros", "dialog", "accept"], "dialog_accept"),
        (vec!["browseros", "dialog", "dismiss"], "dialog_dismiss"),
    ];
    for (argv, kind) in cases {
        let (tool, args) = call(&argv);
        assert_eq!(tool, "act", "{argv:?}");
        assert_eq!(args["kind"], kind, "{argv:?}");
    }
}

#[test]
fn coordinates_switch_the_kind_rather_than_adding_a_flag() {
    let (_, args) = call(&["browseros", "click", "--at", "120,340"]);
    assert_eq!(args["kind"], "click_at");
    assert_eq!(args["x"], 120.0);
    assert_eq!(args["y"], 340.0);
    assert!(args.get("ref").is_none(), "{args}");

    let (_, args) = call(&["browseros", "hover", "--at", "5,6"]);
    assert_eq!(args["kind"], "hover_at");
}

#[test]
fn all_three_ref_spellings_reach_the_wire_as_one() {
    for spelling in ["e12", "@e12", "[ref=e12]"] {
        let (_, args) = call(&["browseros", "click", spelling]);
        assert_eq!(args["ref"], "e12", "{spelling}");
    }
}

#[test]
fn a_whole_form_goes_in_one_call_which_is_what_the_tool_asks_for() {
    let (tool, args) = call(&[
        "browseros",
        "fill",
        "--fields",
        "@e1=Jane",
        "@e2=jane@example.com",
    ]);
    assert_eq!(tool, "act");
    assert_eq!(args["kind"], "fill");
    assert_eq!(
        args["fields"],
        json!([
            { "ref": "e1", "value": "Jane" },
            { "ref": "e2", "value": "jane@example.com" }
        ])
    );
    // clear defaults to true, which is the tool's own default for fill.
    assert_eq!(args["clear"], true);
}

#[test]
fn append_is_the_only_way_to_stop_fill_replacing() {
    let (_, args) = call(&["browseros", "fill", "@e1", "x", "--append"]);
    assert_eq!(args["clear"], false);
}

#[test]
fn the_diff_flag_accepts_both_halves_of_the_tools_union() {
    for (raw, expected) in [
        ("none", json!("none")),
        ("summary", json!("summary")),
        ("full", json!("full")),
        ("500", json!(500.0)),
    ] {
        let (_, args) = call(&["browseros", "click", "@e1", "--diff", raw]);
        assert_eq!(args["diff"], expected, "{raw}");
    }
    assert!(refused(&["browseros", "click", "@e1", "--diff", "loud"]).contains("--diff takes"));
}

#[test]
fn fill_keeps_its_diff_flag_despite_building_its_plan_differently() {
    let (_, args) = call(&["browseros", "fill", "@e1", "x", "--diff", "summary"]);
    assert_eq!(args["diff"], "summary");
}

#[test]
fn a_boolean_that_is_false_is_never_sent_because_every_tool_denies_unknown_fields() {
    let (_, args) = call(&["browseros", "read"]);
    assert_eq!(args["format"], "markdown");
    for absent in ["viewportOnly", "includeLinks", "includeImages", "selector"] {
        assert!(
            args.get(absent).is_none(),
            "{absent} should be absent: {args}"
        );
    }
}

#[test]
fn read_formats_are_mutually_exclusive_and_console_wins_nothing_by_accident() {
    for (flag, format) in [
        ("--text", "text"),
        ("--links", "links"),
        ("--console", "console"),
    ] {
        let (_, args) = call(&["browseros", "read", flag]);
        assert_eq!(args["format"], format, "{flag}");
    }
}

#[test]
fn snapshot_interactive_is_a_mode_not_a_separate_tool() {
    let (tool, args) = call(&["browseros", "snapshot", "-i"]);
    assert_eq!(tool, "snapshot");
    assert_eq!(args["mode"], "interactive");
    let (_, plain) = call(&["browseros", "snapshot"]);
    assert_eq!(plain["mode"], "full");
}

#[test]
fn wait_picks_the_signal_from_the_flag_that_was_given() {
    let (tool, args) = call(&["browseros", "wait", "--text", "Order placed"]);
    assert_eq!(tool, "wait");
    assert_eq!(args["for"], "text");
    assert_eq!(args["value"], "Order placed");

    let (_, args) = call(&["browseros", "wait", "--selector", ".row"]);
    assert_eq!(args["for"], "selector");

    let (_, args) = call(&["browseros", "wait", "--time", "1500"]);
    assert_eq!(args["for"], "time");
    assert_eq!(args["value"], 1500.0);
}

#[test]
fn values_the_server_would_reject_are_refused_locally() {
    // Each of these otherwise costs a round trip to learn.
    assert!(refused(&["browseros", "wait", "--time", "200000"]).contains("90000"));
    assert!(
        refused(&[
            "browseros",
            "wait",
            "--selector",
            ".x",
            "--timeout",
            "60000"
        ])
        .contains("30000")
    );
    assert!(refused(&["browseros", "snapshot", "--depth", "500"]).contains("1..=100"));
    assert!(refused(&["browseros", "grep", "x", "--over", "dom"]).contains("\"ax\""));
    assert!(refused(&["browseros", "eval", "x", "--max-chars", "999999"]).contains("UTF-8 bytes"));
    assert!(refused(&["browseros", "screenshot", "--quality", "900"]).contains("0..=100"));
    assert!(
        refused(&[
            "browseros",
            "session",
            "name",
            "x",
            "--summary",
            &"a".repeat(201)
        ])
        .contains("200")
    );
}

#[test]
fn a_run_timeout_over_the_cap_is_clamped_with_a_notice_rather_than_refused() {
    let cli = Cli::try_parse_from(["browseros", "run", "x", "--script-timeout", "120000"])
        .unwrap_or_else(|error| panic!("{error}"));
    let Ok(Plan::Tool(plan)) = claw_server::cli::plan::build(&cli.command, None) else {
        panic!("run should plan a tool call");
    };
    assert_eq!(plan.args["timeout"], 30000.0);
    assert!(
        plan.notices
            .iter()
            .any(|note| note.contains("clamped to 30000")),
        "{:?}",
        plan.notices
    );
}

#[test]
fn run_wants_the_page_bound_and_no_page_opts_out() {
    let cli = Cli::try_parse_from(["browseros", "run", "return 1"])
        .unwrap_or_else(|error| panic!("{error}"));
    let Ok(Plan::Tool(plan)) = claw_server::cli::plan::build(&cli.command, None) else {
        panic!("run should plan a tool call");
    };
    assert!(plan.wrap_page);
    assert_eq!(plan.args["code"], "return 1");

    let cli = Cli::try_parse_from(["browseros", "run", "return 1", "--no-page"])
        .unwrap_or_else(|error| panic!("{error}"));
    let Ok(Plan::Tool(plan)) = claw_server::cli::plan::build(&cli.command, None) else {
        panic!("run should plan a tool call");
    };
    assert!(!plan.wrap_page);
}

#[test]
fn helpers_are_run_scripts_because_they_only_work_inside_run() {
    let (tool, args) = call(&["browseros", "helpers", "list"]);
    assert_eq!(tool, "run");
    let code = args["code"].as_str().unwrap_or_default();
    assert!(code.contains("listHelpers"), "{code}");

    let (tool, args) = call(&["browseros", "helpers", "read", "findSearch"]);
    assert_eq!(tool, "run");
    assert!(
        args["code"]
            .as_str()
            .unwrap_or_default()
            .contains("\"findSearch\""),
        "the name must be JSON encoded so a quote in it cannot break the script"
    );
}

#[test]
fn a_secret_has_a_route_that_keeps_it_out_of_argv() {
    // A CLI argument lands in shell history and in ps output, which an MCP
    // argument does not, so the CLI has to offer somewhere else to put it.
    assert_eq!(
        needs_external(
            &Cli::try_parse_from(["browseros", "fill", "@e1", "--value-stdin"])
                .unwrap_or_else(|error| panic!("{error}"))
                .command
        ),
        Some(External::Stdin)
    );
    assert_eq!(
        needs_external(
            &Cli::try_parse_from(["browseros", "fill", "@e1", "--value-env", "APP_TOKEN"])
                .unwrap_or_else(|error| panic!("{error}"))
                .command
        ),
        Some(External::Env("APP_TOKEN".into()))
    );
    assert!(refused(&["browseros", "fill", "@e1"]).contains("shell history"));
}

#[test]
fn sensitive_reaches_the_tool_that_knows_how_to_redact_it() {
    let (_, args) = call(&["browseros", "fill", "@e1", "x", "--sensitive"]);
    assert_eq!(args["sensitive"], true);
}

#[test]
fn a_script_read_from_a_file_or_stdin_is_announced_before_the_plan_is_built() {
    let command = Cli::try_parse_from(["browseros", "run", "-f", "s.js"])
        .unwrap_or_else(|error| panic!("{error}"))
        .command;
    assert_eq!(
        needs_external(&command),
        Some(External::File("s.js".into()))
    );

    let command = Cli::try_parse_from(["browseros", "run", "-"])
        .unwrap_or_else(|error| panic!("{error}"))
        .command;
    assert_eq!(needs_external(&command), Some(External::Stdin));
}

#[test]
fn run_with_no_script_names_the_heredoc_form() {
    let message = refused(&["browseros", "run"]);
    assert!(message.contains("<<'JS'"), "{message}");
}

#[test]
fn pursue_turns_pairs_into_the_shapes_the_tool_declares() {
    let (tool, args) = call(&[
        "browseros",
        "pursue",
        "narrow these results to Corsair",
        "--value",
        "pincode=600042",
        "--destination",
        "docs=https://docs.test",
    ]);
    assert_eq!(tool, "pursue");
    assert_eq!(args["values"], json!({ "pincode": "600042" }));
    assert_eq!(
        args["destinations"],
        json!([{ "label": "docs", "url": "https://docs.test" }])
    );
    assert!(refused(&["browseros", "pursue", "g", "--value", "nope"]).contains("name=value"));
}

#[test]
fn locals_never_become_tool_calls() {
    assert_eq!(local(&["browseros", "status"]), Local::Status);
    assert_eq!(local(&["browseros", "use", "7"]), Local::Use(7));
    assert_eq!(local(&["browseros", "session", "list"]), Local::SessionList);
    assert_eq!(
        local(&["browseros", "skills", "get", "core", "--full"]),
        Local::SkillsGet {
            name: "core".into(),
            full: true
        }
    );
    // session status is the same report as status, deliberately.
    assert_eq!(local(&["browseros", "session", "status"]), Local::Status);
}

#[test]
fn session_end_leaves_tabs_alone_unless_asked() {
    assert_eq!(
        local(&["browseros", "session", "end"]),
        Local::SessionEnd {
            close_tabs: false,
            all: false
        }
    );
    assert_eq!(
        local(&["browseros", "session", "end", "--close-tabs"]),
        Local::SessionEnd {
            close_tabs: true,
            all: false
        }
    );
}

#[test]
fn the_group_is_carried_on_tab_creation_and_nowhere_else() {
    let cli = Cli::try_parse_from(["browseros", "open", "https://x.test"])
        .unwrap_or_else(|error| panic!("{error}"));
    let Ok(Plan::Tool(plan)) = claw_server::cli::plan::build(&cli.command, None) else {
        panic!("open should plan a tool call");
    };
    assert!(plan.with_group, "work must stay in one group");
    assert!(plan.remember_page, "open sets the current page");

    let cli = Cli::try_parse_from(["browseros", "goto", "https://x.test"])
        .unwrap_or_else(|error| panic!("{error}"));
    let Ok(Plan::Tool(plan)) = claw_server::cli::plan::build(&cli.command, None) else {
        panic!("goto should plan a tool call");
    };
    assert!(!plan.with_group);
}

#[test]
fn a_screenshot_is_written_to_a_file_because_a_terminal_cannot_render_one() {
    let cli = Cli::try_parse_from(["browseros", "screenshot", "shot.png"])
        .unwrap_or_else(|error| panic!("{error}"));
    let Ok(Plan::Tool(plan)) = claw_server::cli::plan::build(&cli.command, None) else {
        panic!("screenshot should plan a tool call");
    };
    assert_eq!(plan.save_image, Some(Some("shot.png".to_string())));
}

#[test]
fn page_scoped_commands_ask_for_a_page_and_page_free_ones_do_not() {
    let needs: Vec<Vec<&str>> = vec![
        vec!["browseros", "snapshot"],
        vec!["browseros", "diff"],
        vec!["browseros", "read"],
        vec!["browseros", "grep", "x"],
        vec!["browseros", "click", "@e1"],
        vec!["browseros", "goto", "https://x.test"],
        vec!["browseros", "screenshot"],
        vec!["browseros", "download", "@e1"],
        vec!["browseros", "wait", "--time", "100"],
        vec!["browseros", "eval", "return 1"],
    ];
    for argv in needs {
        let cli = Cli::try_parse_from(&argv).unwrap_or_else(|error| panic!("{argv:?}: {error}"));
        let Ok(Plan::Tool(plan)) = claw_server::cli::plan::build(&cli.command, None) else {
            panic!("{argv:?} should plan a tool call");
        };
        assert!(plan.needs_page, "{argv:?} must carry a page");
    }

    let free: Vec<Vec<&str>> = vec![
        vec!["browseros", "tab", "list"],
        vec!["browseros", "history"],
        vec!["browseros", "window", "list"],
        vec!["browseros", "group", "list"],
        vec!["browseros", "run", "return 1"],
        vec!["browseros", "open", "https://x.test"],
    ];
    for argv in free {
        let cli = Cli::try_parse_from(&argv).unwrap_or_else(|error| panic!("{argv:?}: {error}"));
        let Ok(Plan::Tool(plan)) = claw_server::cli::plan::build(&cli.command, None) else {
            panic!("{argv:?} should plan a tool call");
        };
        assert!(!plan.needs_page, "{argv:?} must not carry a page");
    }
}
