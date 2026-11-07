//! Evidence is independent of verdict. Missing methods cannot erase an earlier real mismatch.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Case {
    pub key: String,
    pub name: String,
    pub file: String,
    pub titles: Vec<String>,
    pub mode: String,
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

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ObjectRef {
    pub epoch: String,
    pub id: u64,
    pub family: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Call {
    pub method: String,
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

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub calls: Vec<Call>,
    #[serde(default)]
    pub requirements: BTreeSet<String>,
    #[serde(default)]
    pub missing: BTreeSet<String>,
    #[serde(default)]
    pub transport_errors: Vec<String>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, BTreeSet<String>>,
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
    pub outcomes: Vec<RawTest>,
    pub errors: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Phase {
    pub runner: Option<Runner>,
    pub journal: Option<Journal>,
    pub observation: Option<Value>,
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
    Skip,
    NotApplicable,
    ReferenceFailure,
    HarnessError,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultRecord {
    pub case: Case,
    pub verdict: Verdict,
    pub reason: String,
    pub differences: Vec<String>,
    pub missing_reference: BTreeSet<String>,
    pub missing_observed: BTreeSet<String>,
    pub reference_complete: bool,
    pub evidence_incomplete: BTreeSet<String>,
}
