//! Neo: Evidence is independent of verdict. Missing methods cannot erase an earlier real mismatch.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The full pinned title path identifies each generated upstream case. Feature labels only
/// group reporting and never change that identity or the runner's exact case selection.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Case {
    pub key: String,
    pub name: String,
    pub file: String,
    pub titles: Vec<String>,
    pub mode: String,
    #[serde(default = "unclassified")]
    pub feature: String,
}

fn unclassified() -> String {
    "unclassified".into()
}

/// UTF-16 code units and explicit tags retain lone surrogates, holes and every JS number.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "tag", rename_all = "camelCase")]
pub enum JsValue {
    Undefined,
    Null,
    Hole,
    NegativeZero,
    Nan,
    Infinity,
    NegativeInfinity,
    Boolean {
        value: bool,
    },
    Number {
        value: f64,
    },
    String {
        units: Vec<u16>,
    },
    Bigint {
        decimal: String,
    },
    Array {
        values: Vec<Self>,
    },
    Record {
        #[serde(rename = "nullPrototype")]
        null_prototype: bool,
        entries: Vec<(Vec<u16>, Self)>,
    },
    Regexp {
        #[serde(rename = "sourceUnits")]
        source_units: Vec<u16>,
        flags: String,
        #[serde(rename = "lastIndex")]
        last_index: Box<Self>,
    },
    Object {
        #[serde(rename = "ref")]
        reference: ObjectRef,
    },
}

/// A membrane interns objects inside one engine epoch. Transport IDs are private to that phase;
/// the report binds them to roots and return origins before comparing identities across engines.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ObjectRef {
    pub epoch: String,
    pub id: u64,
    pub family: String,
}

/// Original pinned source coordinates identify an invocation independently of callback order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct SourceSite {
    pub file: String,
    pub line: u64,
    pub column: u64,
}

/// An outer public invocation retains its original source and owning receiver. Occurrence is
/// scoped to that pair; global ordinal and source stack are diagnostics, not correspondence keys.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    pub method: String,
    pub receiver: ObjectRef,
    pub site: Option<SourceSite>,
    pub occurrence: u64,
    pub ordinal: u64,
    pub source_stack: Option<String>,
    #[serde(flatten)]
    pub outcome: CallOutcome,
}

/// A returned value is mandatory, including explicit undefined. Missing or unknown states
/// are unreadable evidence rather than two absent values that could accidentally agree.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CallOutcome {
    Pending,
    Returned {
        value: JsValue,
    },
    Threw {
        error: JsValue,
    },
    Untransportable {
        #[serde(rename = "transportError")]
        transport_error: String,
    },
}

/// One engine membrane owns this journal. Requirements describe its measured path, while
/// missing records actual Neo use even when caught. Roots and gaps determine whether the
/// consumer can prove call correspondence; a readable journal alone is not complete evidence.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub calls: Vec<Call>,
    #[serde(default)]
    pub roots: BTreeMap<String, JsValue>,
    #[serde(default)]
    pub requirements: BTreeSet<String>,
    #[serde(default)]
    pub missing: BTreeSet<String>,
    #[serde(default)]
    pub transport_errors: Vec<String>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    pub census_gaps: Vec<String>,
    #[serde(default)]
    pub correlation_gaps: Vec<String>,
    #[serde(default)]
    pub harness_limitations: Vec<String>,
    pub native_outcome: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTest {
    pub key: String,
    pub raw_status: String,
    pub expected_status: String,
    #[serde(default)]
    pub errors: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Runner {
    pub status: Option<String>,
    pub outcomes: Vec<RawTest>,
    pub errors: Vec<Value>,
}

/// The supervisor records the actual child exit separately from the reporter's final status.
/// Both must agree before evidence can enter compatibility eligibility.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerExit {
    pub code: Option<i32>,
    pub signal: Option<String>,
}

/// Read-only browser effects cross JSON through the same lossless codec as call returns.
/// The outer status is a producer claim; Rust also validates the mandatory tagged projection.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Observation {
    pub status: String,
    pub data: JsValue,
}

/// The supervisor owns one private browser and runner phase. Exit and infrastructure fields
/// validate the producer's lifetime independently of assertions, journals and read-only effects.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Phase {
    pub exit: Option<RunnerExit>,
    pub runner: Option<Runner>,
    pub journal: Option<Journal>,
    pub observation: Option<Observation>,
    #[serde(default)]
    pub infrastructure_errors: Vec<String>,
    pub page_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Pair {
    pub entry: Case,
    pub phases: BTreeMap<String, Phase>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Verdict {
    Pass,
    Fail,
    Unsupported,
    BridgeLimit,
    CleanupBlur,
    Skip,
    NotApplicable,
    ReferenceFailure,
    HarnessError,
}

/// Rust owns the verdict over retained evidence. Missing APIs, proven differences and incomplete
/// evidence remain independent facets. A partial Fail is a real gap but is not fully comparable;
/// only a valid reference with complete census contributes confirmed reference-path blockers.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Reference validity, completeness and earlier differences are independent evidence facets."
)]
pub struct ResultRecord {
    pub case: Case,
    pub verdict: Verdict,
    pub reason: String,
    pub differences: Vec<String>,
    pub wording_notes: Vec<String>,
    pub cleanup_blur_notes: Vec<String>,
    pub bridge_limit_with_earlier_difference: bool,
    pub missing_reference: BTreeSet<String>,
    pub missing_observed: BTreeSet<String>,
    pub reference_valid: bool,
    pub reference_complete: bool,
    pub comparable: bool,
    pub evidence_incomplete: BTreeSet<String>,
    pub harness_limitations: BTreeSet<String>,
}
