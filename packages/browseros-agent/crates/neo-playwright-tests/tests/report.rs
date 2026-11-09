//! Neo: Policy regressions that would turn missing coverage or ignored return differences green.
use std::slice;

use neo_playwright_tests::model::{Pair, ResultRecord, Verdict};
use neo_playwright_tests::report;
use serde_json::{Value, json};

fn object(engine: &str, id: u64, family: &str) -> Value {
    json!({"tag":"object","ref":{"epoch":engine,"id":id,"family":family}})
}

fn call(
    engine: &str,
    method: &str,
    receiver: (u64, &str),
    line: u64,
    occurrence: u64,
    ordinal: u64,
    value: Value,
) -> Value {
    let mut call = json!({"method":method,"receiver":object(engine,receiver.0,receiver.1)["ref"],"site":{"file":"tests/page/page-goto.spec.ts","line":line,"column":1},"occurrence":occurrence,"ordinal":ordinal,"state":"returned"});
    call["value"] = value;
    call
}

fn encode(value: &Value) -> Value {
    match value {
        Value::Null => json!({"tag":"null"}),
        Value::Bool(value) => json!({"tag":"boolean","value":value}),
        Value::Number(value) => json!({"tag":"number","value":value}),
        Value::String(value) => {
            json!({"tag":"string","units":value.encode_utf16().collect::<Vec<_>>()})
        }
        Value::Array(values) => {
            json!({"tag":"array","values":values.iter().map(encode).collect::<Vec<_>>()})
        }
        Value::Object(entries) => {
            json!({"tag":"record","nullPrototype":false,"entries":entries.iter().map(|(key,value)|(key.encode_utf16().collect::<Vec<_>>(),encode(value))).collect::<Vec<_>>()})
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "The helper mutates only this test module's known tagged fixture fields."
)]
fn projection(value: &mut Value, engine: &str, field: &str, replacement: &Value) {
    let entries = value["phases"][engine]["observation"]["data"]["entries"]
        .as_array_mut()
        .expect("tagged observation record");
    let key = json!(field.encode_utf16().collect::<Vec<_>>());
    let entry = entries
        .iter_mut()
        .find(|entry| entry[0] == key)
        .expect("projection field");
    entry[1] = encode(replacement);
}

fn fixture() -> Value {
    let mut value = json!({"entry":{"key":"case-1","name":"case_1","file":"tests/page/page-goto.spec.ts","titles":["variant 1"],"mode":"default"},"phases":{}});
    for engine in ["reference", "neo"] {
        let observation = encode(
            &json!({"status":"complete","incomplete":[],"pages":[{"ordinal":0,"closed":false}],"events":[],"activation":{"status":"complete","tabs":[{"page":0,"window":0,"index":0,"active":true}]}}),
        );
        value["phases"][engine] = json!({
            "exit":{"code":0,"signal":null},
            "runner":{"status":"passed","outcomes":[{"key":"case-1","rawStatus":"passed","expectedStatus":"passed","errors":[]}],"errors":[]},
            "journal":{"calls":[call(engine,"Page.goto",(1,"Page"),1,0,0,json!({"tag":"null"}))],"roots":{"page":object(engine,1,"Page"),"context":object(engine,2,"BrowserContext"),"mainFrame":object(engine,3,"Frame")},"requirements":["Page.goto"],"missing":[],"transportErrors":[],"censusGaps":[],"correlationGaps":[],"capabilities":{"Page":["goto","title"],"BrowserContext":["pages","newPage"]},"nativeOutcome":{"status":"ok","pages":[{"id":"owned-target"}]}},
            "observation":{"status":"complete","data":observation},"pageId":"owned-target","infrastructureErrors":[],
        });
    }
    value
}

fn classify(value: Value) -> Result<ResultRecord, serde_json::Error> {
    serde_json::from_value::<Pair>(value).map(|pair| report::classify(&pair))
}

fn outcome(value: &mut Value, engine: &str, raw: &str, expected: &str) {
    value["phases"][engine]["runner"]["outcomes"][0]["rawStatus"] = json!(raw);
    value["phases"][engine]["runner"]["outcomes"][0]["expectedStatus"] = json!(expected);
    let failed = raw != "skipped" && raw != expected;
    value["phases"][engine]["runner"]["status"] = json!(if failed { "failed" } else { "passed" });
    value["phases"][engine]["exit"]["code"] = json!(u8::from(failed));
}

#[test]
fn an_ignored_goto_response_is_a_real_failure() {
    let mut value = fixture();
    value["phases"]["reference"]["journal"]["calls"][0]["value"] =
        object("reference", 8, "Response");
    let result = classify(value).expect("valid boundary fixture");
    assert_eq!(
        result.verdict,
        Verdict::Fail,
        "a discarded Response still differs from null"
    );
    assert!(
        !result.differences.is_empty(),
        "the return gap must be retained"
    );
    assert!(
        result.comparable,
        "a complete unequal result is comparable evidence"
    );
}

#[test]
fn matching_raw_failures_and_expected_failures_cannot_pass() {
    for expected in ["passed", "failed"] {
        let mut value = fixture();
        for engine in ["reference", "neo"] {
            outcome(&mut value, engine, "failed", expected);
        }
        value["phases"]["reference"]["runner"]["outcomes"][0]["errors"] = json!([encode(
            &json!({"message":"Expected button to be visible\nActual hidden button"})
        )]);
        let result = classify(value).expect("valid boundary fixture");
        assert_eq!(
            result.verdict,
            Verdict::ReferenceFailure,
            "the reference must really pass its assertions"
        );
        assert!(
            result
                .reason
                .contains("Expected button to be visible\nActual hidden button"),
            "reference failure includes the actual full upstream assertion reason"
        );
    }
}

#[test]
fn complete_reference_census_ranks_later_missing_methods_and_retains_prefix_gaps() {
    let mut value = fixture();
    value["phases"]["reference"]["journal"]["requirements"] = json!([
        "Page.goto",
        "Page.setContent",
        "Page.evaluate",
        "Page.setContent"
    ]);
    value["phases"]["reference"]["journal"]["calls"][0]["value"] =
        json!({"tag":"string","units":[65]});
    value["phases"]["neo"]["journal"]["missing"] = json!(["Page.setContent"]);
    let result = classify(value).expect("valid boundary fixture");
    assert_eq!(
        result.verdict,
        Verdict::Unsupported,
        "missing APIs are the owner's primary category"
    );
    assert!(
        !result.differences.is_empty(),
        "unsupported cannot erase a preceding return gap"
    );
    let ranking = report::missing_methods(&[result.clone(), result]);
    assert_eq!(
        ranking.len(),
        2,
        "later reference requirements must be counted"
    );
    assert_eq!(
        ranking[0].method, "Page.evaluate",
        "ties use stable method names"
    );
    assert_eq!(
        ranking[0].blocked_cases.len(),
        1,
        "repeated calls and repeated records count a case once"
    );
}

#[test]
fn unreadable_or_incomplete_evidence_never_passes() {
    for defect in [
        "observer",
        "embedded-status",
        "embedded-gap",
        "projection",
        "activation",
        "target",
        "pending",
        "transport",
        "census",
        "correlation",
        "site",
        "root",
    ] {
        let mut value = fixture();
        match defect {
            "observer" => value["phases"]["neo"]["observation"]["status"] = json!("incomplete"),
            "embedded-status" => projection(&mut value, "neo", "status", &json!("incomplete")),
            "embedded-gap" => projection(
                &mut value,
                "neo",
                "incomplete",
                &json!(["lost input events"]),
            ),
            "projection" => projection(&mut value, "neo", "events", &Value::Null),
            "activation" => {
                projection(
                    &mut value,
                    "neo",
                    "activation",
                    &json!({"status":"incomplete","tabs":[]}),
                );
            }
            "target" => value["phases"]["neo"]["journal"]["nativeOutcome"]["pages"] = json!([]),
            "pending" => value["phases"]["neo"]["journal"]["calls"][0]["state"] = json!("pending"),
            "transport" => {
                value["phases"]["neo"]["journal"]["transportErrors"] =
                    json!(["caught unsupported callback transport"]);
            }
            "census" => {
                value["phases"]["neo"]["journal"]["censusGaps"] =
                    json!(["namespace not instrumented"]);
            }
            "correlation" => {
                value["phases"]["neo"]["journal"]["correlationGaps"] =
                    json!(["callback receiver not bound"]);
            }
            "site" => value["phases"]["neo"]["journal"]["calls"][0]["site"] = Value::Null,
            "root" => {
                value["phases"]["neo"]["journal"]["roots"]["mainFrame"] = json!({"tag":"null"});
            }
            _ => unreachable!(),
        }
        let result = classify(value).expect("valid boundary fixture");
        assert_eq!(
            result.verdict,
            Verdict::HarnessError,
            "incomplete {defect} evidence must not pass"
        );
        assert!(
            !result.evidence_incomplete.is_empty(),
            "the {defect} facet must be visible"
        );
    }
}

#[test]
fn reordered_callback_invocations_match_their_source_sites() {
    let mut value = fixture();
    for engine in ["reference", "neo"] {
        let mut calls = vec![
            call(
                engine,
                "Page.title",
                (1, "Page"),
                10,
                0,
                0,
                json!({"tag":"string","units":[65]}),
            ),
            call(
                engine,
                "Page.title",
                (1, "Page"),
                20,
                0,
                1,
                json!({"tag":"string","units":[66]}),
            ),
        ];
        if engine == "neo" {
            calls.reverse();
        }
        value["phases"][engine]["journal"]["calls"] = json!(calls);
    }
    assert_eq!(
        classify(value).expect("valid boundary fixture").verdict,
        Verdict::Pass,
        "callback scheduling cannot turn corresponding values into mismatches"
    );
}

#[test]
fn repeated_source_invocations_keep_their_per_receiver_occurrences() {
    let mut value = fixture();
    for engine in ["reference", "neo"] {
        let mut calls = vec![
            call(
                engine,
                "Page.title",
                (1, "Page"),
                10,
                0,
                0,
                json!({"tag":"string","units":[65]}),
            ),
            call(
                engine,
                "Page.title",
                (1, "Page"),
                10,
                1,
                1,
                json!({"tag":"string","units":[66]}),
            ),
        ];
        if engine == "neo" {
            calls.reverse();
        }
        value["phases"][engine]["journal"]["calls"] = json!(calls);
    }
    assert_eq!(
        classify(value.clone()).expect("valid fixture").verdict,
        Verdict::Pass,
        "repeated invocations correlate by occurrence rather than array position"
    );
    value["phases"]["neo"]["journal"]["calls"][0]["occurrence"] = json!(0);
    let duplicate = classify(value).expect("valid fixture");
    assert_ne!(
        duplicate.verdict,
        Verdict::Pass,
        "duplicate source occurrences cannot silently overwrite comparable calls"
    );
    assert!(
        !duplicate.evidence_incomplete.is_empty(),
        "the ambiguous occurrence remains explicit"
    );
}

#[test]
fn the_same_return_on_a_different_receiver_is_a_real_difference() {
    let mut value = fixture();
    for engine in ["reference", "neo"] {
        let receiver = if engine == "reference" { 11 } else { 12 };
        value["phases"][engine]["journal"]["calls"] = json!([
            call(
                engine,
                "BrowserContext.newPage",
                (2, "BrowserContext"),
                2,
                0,
                0,
                object(engine, 11, "Page")
            ),
            call(
                engine,
                "BrowserContext.newPage",
                (2, "BrowserContext"),
                3,
                0,
                1,
                object(engine, 12, "Page")
            ),
            call(
                engine,
                "Page.title",
                (receiver, "Page"),
                9,
                0,
                2,
                json!({"tag":"null"})
            ),
        ]);
    }
    assert_eq!(
        classify(value).expect("valid boundary fixture").verdict,
        Verdict::Fail,
        "equal return values do not make different receivers equivalent"
    );
}

#[test]
fn logical_aliases_are_stable_across_callback_order_and_private_ids() {
    let mut value = fixture();
    for (engine, id) in [("reference", 12), ("neo", 50)] {
        let mut calls = vec![
            call(
                engine,
                "BrowserContext.pages",
                (2, "BrowserContext"),
                20,
                0,
                0,
                object(engine, id, "Page"),
            ),
            call(
                engine,
                "BrowserContext.pages",
                (2, "BrowserContext"),
                10,
                0,
                1,
                object(engine, id, "Page"),
            ),
            call(
                engine,
                "Page.title",
                (id, "Page"),
                30,
                0,
                2,
                json!({"tag":"null"}),
            ),
        ];
        if engine == "neo" {
            calls.reverse();
        }
        value["phases"][engine]["journal"]["calls"] = json!(calls);
    }
    assert_eq!(
        classify(value.clone())
            .expect("valid boundary fixture")
            .verdict,
        Verdict::Pass,
        "aliases choose stable source origins rather than first encountered tokens"
    );
    value["phases"]["neo"]["journal"]["calls"][2]["value"] = object("neo", 51, "Page");
    assert_eq!(
        classify(value).expect("valid boundary fixture").verdict,
        Verdict::Fail,
        "a different Page breaks the actual alias relationship"
    );
}

#[test]
fn roots_win_over_return_aliases_and_forward_origins_are_resolved() {
    let mut value = fixture();
    for engine in ["reference", "neo"] {
        value["phases"][engine]["journal"]["calls"] = json!([
            call(
                engine,
                "Page.title",
                (10, "Page"),
                30,
                0,
                2,
                json!({"tag":"null"})
            ),
            call(
                engine,
                "BrowserContext.pages",
                (2, "BrowserContext"),
                10,
                0,
                0,
                object(engine, 1, "Page")
            ),
            call(
                engine,
                "BrowserContext.newPage",
                (2, "BrowserContext"),
                20,
                0,
                1,
                object(engine, 10, "Page")
            ),
        ]);
    }
    assert_eq!(
        classify(value).expect("valid boundary fixture").verdict,
        Verdict::Pass,
        "return aliases preserve root identity even when receiver calls appear first"
    );
}

#[test]
fn unrooted_or_cyclic_receivers_are_incomplete_instead_of_arbitrarily_paired() {
    let mut value = fixture();
    for engine in ["reference", "neo"] {
        value["phases"][engine]["journal"]["calls"] = json!([call(
            engine,
            "Page.title",
            (9, "Page"),
            10,
            0,
            0,
            object(engine, 9, "Page")
        )]);
    }
    let result = classify(value).expect("valid boundary fixture");
    assert_eq!(
        result.verdict,
        Verdict::HarnessError,
        "private token coincidence cannot prove a receiver"
    );
    assert!(
        result.differences.is_empty(),
        "unprovable receiver identity must not invent a Neo gap"
    );
    let mut value = fixture();
    value["phases"]["neo"]["journal"]["calls"][0] = call(
        "neo",
        "Page.goto",
        (9, "Page"),
        1,
        1,
        0,
        json!({"tag":"null"}),
    );
    let result = classify(value).expect("valid fixture");
    assert_eq!(
        result.verdict,
        Verdict::HarnessError,
        "an unknown receiver cannot establish that a known invocation is missing"
    );
    assert!(
        result.differences.is_empty(),
        "per-receiver occurrences cannot be paired before receiver identity is proven"
    );
}

#[test]
fn missing_unknown_and_invalid_call_outcomes_are_unreadable() {
    for defect in [
        "state",
        "value",
        "error",
        "occurrence",
        "ordinal",
        "receiver",
    ] {
        let mut value = fixture();
        let call = value["phases"]["neo"]["journal"]["calls"][0]
            .as_object_mut()
            .expect("fixture call object");
        match defect {
            "state" => {
                call.insert("state".into(), json!("succeeded"));
            }
            "error" => {
                call.insert("state".into(), json!("threw"));
                call.remove("value");
            }
            "occurrence" => {
                call.insert("occurrence".into(), json!(-1));
            }
            _ => {
                call.remove(defect);
            }
        }
        assert!(
            classify(value).is_err(),
            "invalid {defect} cannot deserialize into comparable evidence"
        );
    }
    let mut value = fixture();
    value["phases"]["neo"]["observation"]["data"] = json!({"tag":"number"});
    assert!(
        classify(value).is_err(),
        "outer complete cannot hide an unreadable tagged effect projection"
    );
}

#[test]
fn abnormal_or_inconsistent_exit_after_passing_reports_never_passes() {
    for defect in [
        "signal",
        "missing",
        "unknown",
        "code",
        "status",
        "inconsistent",
        "post-report-timeout",
    ] {
        let mut value = fixture();
        match defect {
            "signal" => value["phases"]["neo"]["exit"] = json!({"code":null,"signal":"SIGKILL"}),
            "missing" => {
                value["phases"]["neo"]
                    .as_object_mut()
                    .expect("phase object")
                    .remove("exit");
            }
            "unknown" => value["phases"]["neo"]["exit"] = json!({"code":null,"signal":null}),
            "code" => value["phases"]["neo"]["exit"]["code"] = json!(1),
            "status" => value["phases"]["neo"]["runner"]["status"] = json!("interrupted"),
            "inconsistent" => {
                value["phases"]["neo"]["runner"]["status"] = json!("failed");
                value["phases"]["neo"]["exit"]["code"] = json!(1);
            }
            "post-report-timeout" => {
                value["phases"]["neo"]["runner"]["status"] = json!("timedout");
                value["phases"]["neo"]["exit"]["code"] = json!(1);
            }
            _ => unreachable!(),
        }
        let result = classify(value).expect("valid boundary fixture");
        assert_eq!(
            result.verdict,
            Verdict::HarnessError,
            "{defect} cannot be hidden by passing files"
        );
        assert!(
            !result.comparable,
            "abnormal lifetime cannot be fully comparable"
        );
    }
}

#[test]
fn legitimate_assertion_failure_and_upstream_skip_keep_their_categories() {
    let mut value = fixture();
    outcome(&mut value, "neo", "failed", "passed");
    assert_eq!(
        classify(value).expect("valid boundary fixture").verdict,
        Verdict::Fail,
        "ordinary exit1 assertion failure is a real Neo failure"
    );
    let mut value = fixture();
    for engine in ["reference", "neo"] {
        outcome(&mut value, engine, "skipped", "skipped");
        value["phases"][engine]["journal"] = Value::Null;
        value["phases"][engine]["observation"] = Value::Null;
    }
    let result = classify(value).expect("valid boundary fixture");
    assert_eq!(
        result.verdict,
        Verdict::Skip,
        "skips legitimately produce no executed-body evidence"
    );
    assert!(
        result.evidence_incomplete.is_empty(),
        "valid skips are exclusions rather than partial measurements"
    );
}

#[test]
fn exact_error_name_message_and_lone_surrogates_remain_real_differences() {
    for (expected, actual) in [
        (
            json!({"tag":"string","units":[0xd800]}),
            json!({"tag":"string","units":[0xfffd]}),
        ),
        (
            json!({"tag":"record","nullPrototype":false,"entries":[[[110,97,109,101],{"tag":"string","units":[65]}],[[109,101,115,115,97,103,101],{"tag":"string","units":[66]}]]}),
            json!({"tag":"record","nullPrototype":false,"entries":[[[110,97,109,101],{"tag":"string","units":[67]}],[[109,101,115,115,97,103,101],{"tag":"string","units":[66]}]]}),
        ),
        (
            json!({"tag":"string","units":[65]}),
            json!({"tag":"string","units":[66]}),
        ),
    ] {
        let mut value = fixture();
        for (engine, error) in [("reference", expected), ("neo", actual)] {
            let call = value["phases"][engine]["journal"]["calls"][0]
                .as_object_mut()
                .expect("fixture call");
            call.insert("state".into(), json!("threw"));
            call.remove("value");
            call.insert("error".into(), error);
        }
        assert_eq!(
            classify(value).expect("valid boundary fixture").verdict,
            Verdict::Fail,
            "public names, wording and UTF-16 cannot be normalized away"
        );
    }
}

#[test]
fn measured_differences_survive_incomplete_effects_and_target_witness() {
    for defect in ["observer", "target"] {
        let mut value = fixture();
        value["phases"]["neo"]["journal"]["calls"][0]["value"] =
            json!({"tag":"boolean","value":true});
        if defect == "observer" {
            value["phases"]["neo"]["observation"]["status"] = json!("incomplete");
        } else {
            value["phases"]["neo"]["journal"]["nativeOutcome"]["pages"] = json!([]);
        }
        let result = classify(value).expect("valid boundary fixture");
        assert_eq!(
            result.verdict,
            Verdict::Fail,
            "an independent measured gap survives {defect} loss"
        );
        assert!(
            !result.comparable && !result.evidence_incomplete.is_empty(),
            "partial failures remain explicitly partial"
        );
    }
}

#[test]
fn partial_reference_census_does_not_inflate_confirmed_missing_ranking() {
    let mut value = fixture();
    value["phases"]["reference"]["journal"]["censusGaps"] = json!(["unwrapped namespace"]);
    value["phases"]["reference"]["journal"]["requirements"] =
        json!(["Page.evaluate", "Page.setContent"]);
    value["phases"]["neo"]["journal"]["missing"] = json!(["Page.setContent"]);
    let result = classify(value).expect("valid boundary fixture");
    let ranking = report::missing_methods(&[result.clone(), result]);
    let reference_only = ranking
        .iter()
        .find(|row| row.method == "Page.evaluate")
        .expect("reference-only method");
    assert!(
        reference_only.blocked_cases.is_empty() && reference_only.reference_path_cases.is_empty(),
        "a partial reference does not confirm a blocked case"
    );
    assert_eq!(
        reference_only.partial_reference_cases.len(),
        1,
        "partial requirements retain the exact generated identity"
    );
    let observed = ranking
        .iter()
        .find(|row| row.method == "Page.setContent")
        .expect("observed method");
    assert_eq!(
        observed.blocked_cases.len(),
        1,
        "actual caught native missing use is retained independently"
    );
    assert_eq!(
        observed.observed_neo_cases.len(),
        1,
        "observed use is never mislabeled a complete reference path"
    );
}

#[test]
fn fixture_limitations_are_separate_from_native_missing_methods() {
    let mut value = fixture();
    value["phases"]["reference"]["journal"]["harnessLimitations"] =
        json!(["nondefault context option is not bootstrapped"]);
    let result = classify(value).expect("valid boundary fixture");
    assert_eq!(
        result.verdict,
        Verdict::HarnessError,
        "different setup conditions cannot establish a native API gap"
    );
    assert!(
        !result.reference_valid && !result.harness_limitations.is_empty(),
        "setup validity is independent of raw passing assertions"
    );
    assert!(
        report::missing_methods(&[result]).is_empty(),
        "fixture limitations never enter native method ranking"
    );
    let mut value = fixture();
    value["phases"]["neo"]["journal"]["harnessLimitations"] =
        json!(["private helper is not transported"]);
    value["phases"]["neo"]["journal"]["missing"] = json!(["Page.evaluate"]);
    let result = classify(value).expect("valid fixture");
    assert_eq!(
        result.verdict,
        Verdict::Unsupported,
        "a valid reference's actual missing API keeps primary status"
    );
    assert!(
        !result.harness_limitations.is_empty(),
        "the independent harness limitation is retained"
    );
}

#[test]
fn coverage_distinguishes_zero_supported_partial_failures_and_skips() {
    let mut unsupported = fixture();
    unsupported["phases"]["neo"]["journal"]["missing"] = json!(["Page.evaluate"]);
    let unsupported = classify(unsupported).expect("valid fixture");
    let zero = report::summarize(slice::from_ref(&unsupported));
    assert_eq!(
        (
            zero.totals.compatibility.numerator,
            zero.totals.compatibility.denominator
        ),
        (0, 0),
        "all unsupported does not manufacture a compatibility rate"
    );
    assert_eq!(
        (
            zero.totals.coverage.numerator,
            zero.totals.coverage.denominator
        ),
        (0, 1),
        "the unsupported eligible case remains visible in coverage"
    );
    assert_eq!(
        report::summarize(&[]).totals.coverage.denominator,
        0,
        "empty selections have no denominator"
    );
    let pass = classify(fixture()).expect("valid fixture");
    let mut partial = fixture();
    partial["entry"]["feature"] = json!("navigation");
    partial["phases"]["neo"]["observation"]["status"] = json!("incomplete");
    partial["phases"]["neo"]["journal"]["calls"][0]["value"] =
        json!({"tag":"boolean","value":true});
    let partial = classify(partial).expect("valid fixture");
    let mut skip = fixture();
    for engine in ["reference", "neo"] {
        outcome(&mut skip, engine, "skipped", "skipped");
    }
    let skip = classify(skip).expect("valid fixture");
    let summary = report::summarize(&[pass, partial, unsupported, skip]);
    assert_eq!(
        (
            summary.totals.compatibility.numerator,
            summary.totals.compatibility.denominator
        ),
        (1, 2),
        "partial failures remain real compatibility failures"
    );
    assert_eq!(
        (
            summary.totals.coverage.numerator,
            summary.totals.coverage.denominator
        ),
        (1, 3),
        "partial failures and missing APIs are not fully comparable"
    );
    assert_eq!(
        summary.totals.partial_fail_count, 1,
        "partial failure count accompanies the pass denominator"
    );
    assert_eq!(
        summary.totals.counts[&Verdict::Skip],
        1,
        "upstream skips remain explicit exclusions"
    );
    assert_eq!(
        summary.features["navigation"].partial_fail_count, 1,
        "feature totals retain partial evidence"
    );
    assert_eq!(
        summary.features["unclassified"].completed_count, 3,
        "offline fixtures have an explicit feature default"
    );
    assert_eq!(
        summary.suites["tests/page/page-goto.spec.ts"].completed_count, 4,
        "suite totals preserve all categories"
    );
}
