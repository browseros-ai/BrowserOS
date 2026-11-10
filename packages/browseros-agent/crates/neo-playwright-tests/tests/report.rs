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
fn error_classes_and_unnamed_throws_remain_real_differences() {
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
            "different classes and unnamed throws cannot be normalized away"
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
            Verdict::HarnessError,
            "an independent measured gap survives {defect} loss"
        );
        assert!(
            !result.differences.is_empty(),
            "infrastructure does not erase measured gaps"
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
    partial["phases"]["neo"]["journal"]["censusGaps"] = json!(["unwrapped namespace"]);
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

#[expect(
    clippy::expect_used,
    reason = "Only the known fixture call is mutated."
)]
fn throws(value: &mut Value, engine: &str, name: &str, message: &str) {
    let call = value["phases"][engine]["journal"]["calls"][0]
        .as_object_mut()
        .expect("fixture call");
    call.insert("state".into(), json!("threw"));
    call.remove("value");
    call.insert(
        "error".into(),
        encode(&json!({"name":name,"message":message})),
    );
}

#[test]
fn tests_that_error_names_match_and_wording_is_counted_once_per_case() {
    let mut value = fixture();
    throws(&mut value, "reference", "TimeoutError", "reference wording");
    throws(&mut value, "neo", "TimeoutError", "native wording");
    for engine in ["reference", "neo"] {
        let mut next = value["phases"][engine]["journal"]["calls"][0].clone();
        next["ordinal"] = json!(1);
        next["site"]["line"] = json!(2);
        value["phases"][engine]["journal"]["calls"]
            .as_array_mut()
            .expect("calls")
            .push(next);
    }
    let result = classify(value).expect("valid fixture");
    assert_eq!(
        result.verdict,
        Verdict::Pass,
        "message text is not behavior"
    );
    assert!(
        result.differences.is_empty(),
        "wording is separate from differences"
    );
    let serialized = serde_json::to_value(&result).expect("record");
    assert_eq!(
        serialized["wordingNotes"].as_array().expect("notes").len(),
        2,
        "one note per differing message"
    );
    let summary = serde_json::to_value(report::summarize(&[result])).expect("summary");
    for totals in [
        &summary["totals"],
        &summary["features"]["unclassified"],
        &summary["suites"]["tests/page/page-goto.spec.ts"],
    ] {
        assert_eq!(
            totals["wordingCount"], 1,
            "wording cases appear in every grouping"
        );
    }
}

#[test]
fn tests_that_ansi_is_ignored_in_values_errors_and_effects_without_mutating_evidence() {
    for errors in [false, true] {
        let mut value = fixture();
        if errors {
            throws(&mut value, "reference", "Error", "plain");
            throws(
                &mut value,
                "neo",
                "\u{1b}[1;31mError\u{1b}[m",
                "\u{1b}[32mplain\u{1b}[0m",
            );
        } else {
            value["phases"]["reference"]["journal"]["calls"][0]["value"] =
                encode(&json!({"key":["plain"]}));
            value["phases"]["neo"]["journal"]["calls"][0]["value"] =
                encode(&json!({"\u{1b}[31mkey\u{1b}[m":["\u{1b}[32mplain\u{1b}[0m"]}));
        }
        projection(&mut value, "reference", "events", &json!(["plain"]));
        projection(
            &mut value,
            "neo",
            "events",
            &json!(["\u{1b}[31mplain\u{1b}[m"]),
        );
        let pair: Pair = serde_json::from_value(value).expect("pair");
        let original = pair.phases["neo"]
            .journal
            .as_ref()
            .expect("journal")
            .calls
            .clone();
        let result = report::classify(&pair);
        assert_eq!(
            result.verdict,
            Verdict::Pass,
            "ANSI presentation is ignored"
        );
        assert_eq!(
            pair.phases["neo"].journal.as_ref().expect("journal").calls,
            original,
            "raw journal stays unchanged"
        );
        assert_eq!(
            serde_json::to_value(result).expect("record")["wordingNotes"],
            json!([]),
            "ANSI alone is not wording"
        );
    }
}

#[test]
fn tests_that_upstream_assertion_diagnostics_strip_ansi() {
    let mut value = fixture();
    outcome(&mut value, "reference", "failed", "passed");
    value["phases"]["reference"]["runner"]["outcomes"][0]["errors"] = json!([encode(
        &json!({"message":"\u{1b}[31mExpected text\u{1b}[0m"})
    )]);
    let result = classify(value).expect("fixture");
    assert!(
        result.reason.ends_with("Expected text"),
        "readable assertion text"
    );
    assert!(
        !result.reason.contains('\u{1b}'),
        "ANSI is stripped in assertion diagnostics"
    );
}

#[test]
fn tests_that_unproven_upstream_wording_assertions_stay_failures() {
    let mut value = fixture();
    throws(&mut value, "reference", "Error", "reference message");
    throws(&mut value, "neo", "Error", "native message");
    outcome(&mut value, "neo", "failed", "passed");
    value["phases"]["neo"]["runner"]["outcomes"][0]["errors"] = json!([encode(
        &json!({"message":"expect(received).toContain(expected)\nReceived: native message"})
    )]);
    assert_eq!(
        classify(value).expect("fixture").verdict,
        Verdict::Fail,
        "rendered assertion text does not prove the received value"
    );
}

fn bridge_fixture() -> Value {
    let mut value = fixture();
    value["phases"]["neo"]["journal"]["harnessLimitations"] =
        json!(["bridge limit: host callback Page.on"]);
    value
}

#[test]
fn tests_that_bridge_limits_and_transport_errors_never_become_failures() {
    for transport in ["label", "journal", "call"] {
        let mut value = bridge_fixture();
        if transport != "label" {
            value["phases"]["neo"]["journal"]["harnessLimitations"] = json!([]);
        }
        if transport == "journal" {
            value["phases"]["neo"]["journal"]["transportErrors"] =
                json!(["cannot encode callback"]);
        } else if transport == "call" {
            value["phases"]["neo"]["journal"]["calls"][0]["state"] = json!("untransportable");
            value["phases"]["neo"]["journal"]["calls"][0]["transportError"] =
                json!("unsupported return");
        }
        outcome(&mut value, "neo", "failed", "passed");
        let result = classify(value).expect("fixture");
        assert_eq!(
            serde_json::to_value(&result).expect("record")["verdict"],
            "BridgeLimit",
            "transport is not an SDK failure"
        );
        assert!(
            !result.comparable,
            "bridge-limited evidence cannot enter compatibility"
        );
        let summary = report::summarize(&[result]);
        assert_eq!(
            summary.totals.compatibility.denominator, 0,
            "bridge limits have no pass rate"
        );
    }
}

#[test]
fn tests_that_unsupported_and_invalid_references_take_precedence_over_bridge_limits() {
    let mut value = bridge_fixture();
    value["phases"]["neo"]["journal"]["missing"] = json!(["Page.evaluate"]);
    assert_eq!(
        classify(value).expect("fixture").verdict,
        Verdict::Unsupported,
        "actual missing APIs take precedence"
    );
    let mut value = bridge_fixture();
    outcome(&mut value, "reference", "failed", "passed");
    assert_eq!(
        classify(value).expect("fixture").verdict,
        Verdict::ReferenceFailure,
        "reference must pass"
    );
}

#[test]
fn tests_that_infrastructure_errors_outrank_bridge_limits_and_unsupported() {
    for defect in ["observer", "exit", "worker", "witness"] {
        for unsupported in [false, true] {
            let mut value = bridge_fixture();
            if unsupported {
                value["phases"]["neo"]["journal"]["missing"] = json!(["Page.evaluate"]);
            }
            match defect {
                "observer" => value["phases"]["neo"]["observation"]["status"] = json!("incomplete"),
                "exit" => value["phases"]["neo"]["exit"]["code"] = json!(1),
                "worker" => value["phases"]["neo"]["infrastructureErrors"] = json!(["worker died"]),
                "witness" => {
                    value["phases"]["neo"]["journal"]["nativeOutcome"]["status"] = json!("error");
                }
                _ => unreachable!(),
            }
            assert_eq!(
                classify(value).expect("fixture").verdict,
                Verdict::HarnessError,
                "{defect} remains infrastructure"
            );
        }
    }
}

#[test]
fn tests_that_bridge_limit_counts_only_proven_earlier_differences() {
    for earlier_difference in [false, true] {
        let mut value = bridge_fixture();
        if earlier_difference {
            value["phases"]["neo"]["journal"]["calls"][0]["value"] = encode(&json!(true));
        }
        for engine in ["reference", "neo"] {
            let mut next = call(
                engine,
                "Page.title",
                (1, "Page"),
                2,
                0,
                1,
                encode(&json!("title")),
            );
            if engine == "neo" {
                next["state"] = json!("pending");
            }
            value["phases"][engine]["journal"]["calls"]
                .as_array_mut()
                .expect("calls")
                .push(next);
        }
        // A reference tail after the blocked call is evidence, not a proven earlier difference.
        value["phases"]["reference"]["journal"]["calls"]
            .as_array_mut()
            .expect("calls")
            .push(call(
                "reference",
                "Page.title",
                (1, "Page"),
                3,
                0,
                2,
                encode(&json!("later")),
            ));
        let result = classify(value).expect("fixture");
        assert!(!result.differences.is_empty(), "retain the incomplete tail");
        let summary = serde_json::to_value(report::summarize(&[result])).expect("summary");
        for totals in [
            &summary["totals"],
            &summary["features"]["unclassified"],
            &summary["suites"]["tests/page/page-goto.spec.ts"],
        ] {
            assert_eq!(
                totals["bridgeLimitWithEarlierDifferenceCount"],
                usize::from(earlier_difference),
                "only proven earlier differences count"
            );
        }
    }
}

#[test]
fn tests_that_invalid_reference_transport_cannot_establish_a_bridge_verdict() {
    for missing in [false, true] {
        let mut value = bridge_fixture();
        if missing {
            value["phases"]["reference"]["journal"] = Value::Null;
        } else {
            value["phases"]["reference"]["journal"]["transportErrors"] =
                json!(["reference serialization failed"]);
        }
        assert_eq!(
            classify(value).expect("fixture").verdict,
            Verdict::HarnessError,
            "unreadable reference transport is a harness error"
        );
    }
}

#[test]
fn tests_that_ansi_normalization_preserves_object_origins_and_lone_surrogates() {
    let mut value = fixture();
    for engine in ["reference", "neo"] {
        let key = if engine == "neo" {
            "\u{1b}[31mchild\u{1b}[m"
        } else {
            "child"
        };
        value["phases"][engine]["journal"]["calls"][0]["value"] = json!({"tag":"record","nullPrototype":false,"entries":[[key.encode_utf16().collect::<Vec<_>>(),object(engine,10,"Page")]]});
        let units = if engine == "neo" {
            vec![0xd800, 0x1b, 0x5b, 0x6d, 65]
        } else {
            vec![0xd800, 65]
        };
        value["phases"][engine]["journal"]["calls"]
            .as_array_mut()
            .expect("calls")
            .push(call(
                engine,
                "Page.title",
                (10, "Page"),
                2,
                0,
                1,
                json!({"tag":"string","units":units}),
            ));
    }
    assert_eq!(
        classify(value.clone()).expect("fixture").verdict,
        Verdict::Pass,
        "ANSI in returned keys cannot change child receiver identities"
    );
    value["phases"]["neo"]["journal"]["calls"][1]["value"]["units"] = json!([0xfffd, 65]);
    assert_eq!(
        classify(value).expect("fixture").verdict,
        Verdict::Fail,
        "lone surrogates cannot collapse into replacement characters"
    );
}
