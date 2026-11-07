//! Policy regressions that would turn missing coverage or ignored return differences green.
use neo_playwright_tests::model::{Pair, ResultRecord, Verdict};
use neo_playwright_tests::report;
use serde_json::{Value, json};

fn fixture() -> Value {
    let call = json!({"method":"Page.goto","state":"returned","value":{"tag":"null"}});
    let phase = json!({
        "runner":{"outcomes":[{"key":"case-1","rawStatus":"passed","expectedStatus":"passed","errors":[]}],"errors":[]},
        "journal":{"calls":[call],"requirements":["Page.goto"],"missing":[],"transportErrors":[],"capabilities":{"Page":["goto"]},"nativeOutcome":{"status":"ok","pages":[{"id":"owned-target"}]}},
        "observation":{"status":"complete","data":{}},"pageId":"owned-target","infrastructureErrors":[],
    });
    json!({"entry":{"key":"case-1","name":"case_1","file":"tests/page/page-goto.spec.ts","titles":["variant 1"],"mode":"default"},"phases":{"reference":phase,"neo":phase}})
}

fn classify(value: Value) -> Result<ResultRecord, serde_json::Error> {
    serde_json::from_value::<Pair>(value).map(|pair| report::classify(&pair))
}

#[test]
fn an_ignored_goto_response_is_a_real_failure() {
    let mut value = fixture();
    value["phases"]["reference"]["journal"]["calls"][0]["value"] =
        json!({"tag":"object","ref":{"epoch":"reference","id":8,"family":"Response"}});
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
}

#[test]
fn matching_raw_failures_and_expected_failures_cannot_pass() {
    for expected in ["passed", "failed"] {
        let mut value = fixture();
        for engine in ["reference", "neo"] {
            value["phases"][engine]["runner"]["outcomes"][0]["rawStatus"] = json!("failed");
            value["phases"][engine]["runner"]["outcomes"][0]["expectedStatus"] = json!(expected);
        }
        assert_eq!(
            classify(value).expect("valid boundary fixture").verdict,
            Verdict::ReferenceFailure,
            "the reference must really pass its assertions"
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
fn dropped_observation_missing_target_pending_values_and_caught_transport_debt_never_pass() {
    for defect in ["observer", "target", "pending", "transport"] {
        let mut value = fixture();
        match defect {
            "observer" => value["phases"]["neo"]["observation"]["status"] = json!("incomplete"),
            "target" => value["phases"]["neo"]["journal"]["nativeOutcome"]["pages"] = json!([]),
            "pending" => value["phases"]["neo"]["journal"]["calls"][0]["state"] = json!("pending"),
            "transport" => {
                value["phases"]["neo"]["journal"]["transportErrors"] =
                    json!(["caught unsupported callback transport"]);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            classify(value).expect("valid boundary fixture").verdict,
            Verdict::HarnessError,
            "incomplete {defect} evidence must not pass"
        );
    }
}

#[test]
fn logical_object_aliases_are_compared_without_comparing_transport_ids() {
    let mut value = fixture();
    for (engine, epoch, id) in [("reference", "pw-epoch", 12), ("neo", "native-epoch", 1)] {
        let call = json!({"method":"BrowserContext.pages","state":"returned","value":{"tag":"object","ref":{"epoch":epoch,"id":id,"family":"Page"}}});
        value["phases"][engine]["journal"]["calls"] = json!([call.clone(), call]);
        value["phases"][engine]["journal"]["requirements"] = json!(["BrowserContext.pages"]);
        value["phases"][engine]["journal"]["capabilities"] = json!({"BrowserContext":["pages"]});
    }
    assert_eq!(
        classify(value.clone())
            .expect("valid boundary fixture")
            .verdict,
        Verdict::Pass,
        "transport IDs are private and unrelated"
    );
    value["phases"]["neo"]["journal"]["calls"][1]["value"]["ref"]["id"] = json!(2);
    assert_eq!(
        classify(value).expect("valid boundary fixture").verdict,
        Verdict::Fail,
        "a different second Page breaks actual identity"
    );
}

#[test]
fn exact_error_wording_and_lone_surrogate_values_remain_real_differences() {
    for (expected, actual) in [
        (
            json!({"tag":"string","units":[0xd800]}),
            json!({"tag":"string","units":[0xfffd]}),
        ),
        (
            json!({"tag":"string","units":[65]}),
            json!({"tag":"string","units":[66]}),
        ),
    ] {
        let mut value = fixture();
        for (engine, error) in [("reference", expected), ("neo", actual)] {
            value["phases"][engine]["journal"]["calls"][0] =
                json!({"method":"Page.goto","state":"threw","error":error});
        }
        assert_eq!(
            classify(value).expect("valid boundary fixture").verdict,
            Verdict::Fail,
            "public wording and UTF-16 cannot be normalized away"
        );
    }
}
