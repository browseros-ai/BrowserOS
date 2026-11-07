//! Classifies raw outcomes and ranks distinct blocked cases. No matching failures count as pass.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

use crate::model::{Call, CallOutcome, Case, JsValue, Pair, Phase, RawTest, ResultRecord, Verdict};

#[must_use]
pub fn harness_error(case: Case, reason: String) -> ResultRecord {
    ResultRecord {
        case,
        verdict: Verdict::HarnessError,
        reason,
        differences: Vec::new(),
        missing_reference: BTreeSet::new(),
        missing_observed: BTreeSet::new(),
        reference_complete: false,
        evidence_incomplete: BTreeSet::new(),
    }
}

fn raw<'a>(phase: &'a Phase, key: &str) -> Option<&'a RawTest> {
    let runner = phase.runner.as_ref()?;
    if !runner.errors.is_empty() || runner.outcomes.len() != 1 {
        return None;
    }
    runner.outcomes.iter().find(|test| test.key == key)
}

fn canonicalize(value: &mut JsValue, objects: &mut BTreeMap<(String, u64, String), u64>) {
    match value {
        JsValue::Object { reference } => {
            let next = objects.len() as u64 + 1;
            reference.id = *objects
                .entry((
                    reference.epoch.clone(),
                    reference.id,
                    reference.family.clone(),
                ))
                .or_insert(next);
            reference.epoch.clear();
        }
        JsValue::Array { values } => {
            for value in values {
                canonicalize(value, objects);
            }
        }
        JsValue::Record { entries, .. } => {
            for (_, value) in entries {
                canonicalize(value, objects);
            }
        }
        JsValue::Regexp { last_index, .. } => canonicalize(last_index, objects),
        _ => {}
    }
}

fn canonical_calls(calls: &[Call]) -> Vec<Call> {
    let mut result = calls.to_vec();
    let mut objects = BTreeMap::new();
    for call in &mut result {
        match &mut call.outcome {
            CallOutcome::Returned { value } | CallOutcome::Threw { error: value } => {
                canonicalize(value, &mut objects);
            }
            CallOutcome::Pending | CallOutcome::Untransportable { .. } => {}
        }
    }
    result
}

fn absent(method: &str, capabilities: &BTreeMap<String, BTreeSet<String>>) -> bool {
    let Some((family, member)) = method.split_once('.') else {
        return true;
    };
    let member = if method == "Frame.expect" {
        "_expect"
    } else {
        member
    };
    !capabilities
        .get(family)
        .is_some_and(|methods| methods.contains(member))
}

fn passed(test: &RawTest) -> bool {
    test.raw_status == "passed" && test.expected_status == "passed" && test.errors.is_empty()
}

fn complete(observation: Option<&Value>) -> bool {
    observation.is_some_and(|value| value["status"] == "complete")
}

fn compare_journals(result: &mut ResultRecord, reference: &Phase, neo: &Phase) {
    if let (Some(reference_journal), Some(neo_journal)) = (&reference.journal, &neo.journal) {
        result.missing_reference = reference_journal
            .requirements
            .iter()
            .filter(|method| absent(method, &neo_journal.capabilities))
            .cloned()
            .collect();
        result.missing_observed.clone_from(&neo_journal.missing);
        let reference_calls = canonical_calls(&reference_journal.calls);
        let neo_calls = canonical_calls(&neo_journal.calls);
        for (index, (expected, actual)) in reference_calls.iter().zip(&neo_calls).enumerate() {
            if expected != actual {
                result.differences.push(format!(
                    "call {index} {} differs in method, return value or exact public error",
                    expected.method
                ));
            }
        }
        if reference_calls.len() != neo_calls.len() {
            result.differences.push(format!(
                "API call counts differ ({} / {})",
                reference_calls.len(),
                neo_calls.len()
            ));
        }
    }
}

fn native_witness(neo: &Phase) -> Result<(), String> {
    let native = neo
        .journal
        .as_ref()
        .and_then(|journal| journal.native_outcome.as_ref())
        .ok_or("No native RunOutcome proves target execution")?;
    if native["status"] != "ok" || native["callsDropped"].is_object() {
        return Err("Native run failed, stopped or dropped evidence".into());
    }
    let target = neo.page_id.as_deref();
    if target.is_none()
        || native["pages"]
            .as_array()
            .is_none_or(|pages| !pages.iter().any(|page| page["id"].as_str() == target))
    {
        return Err("Native run did not witness the owned target".into());
    }
    Ok(())
}

fn transport_failure(reference: &Phase, neo: &Phase) -> Option<String> {
    let errors: Vec<_> = reference
        .journal
        .iter()
        .chain(&neo.journal)
        .flat_map(|journal| journal.transport_errors.iter())
        .collect();
    (!errors.is_empty()).then(|| format!("Harness transport: {errors:?}"))
}

#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "Verdict precedence stays visible as one ordered policy; only complete evidence can pass."
)]
pub fn classify(pair: &Pair) -> ResultRecord {
    let mut result = harness_error(pair.entry.clone(), "Paired evidence is incomplete".into());
    let (Some(reference), Some(neo)) = (pair.phases.get("reference"), pair.phases.get("neo"))
    else {
        return result;
    };
    let (Some(reference_test), Some(neo_test)) =
        (raw(reference, &pair.entry.key), raw(neo, &pair.entry.key))
    else {
        result.reason = "Runner did not finish exactly the requested upstream case".into();
        return result;
    };
    let reference_pass = passed(reference_test);
    result.reference_complete = reference_pass;
    compare_journals(&mut result, reference, neo);
    for (engine, phase) in [("reference", reference), ("neo", neo)] {
        if !complete(phase.observation.as_ref()) {
            result
                .evidence_incomplete
                .insert(format!("{engine}: read-only observation incomplete"));
        }
    }
    let infrastructure: Vec<_> = reference
        .infrastructure_errors
        .iter()
        .chain(&neo.infrastructure_errors)
        .collect();
    if let Some(error) = infrastructure.first() {
        result.reason = format!("Infrastructure: {error}");
        return result;
    }
    if reference_test.raw_status == "skipped" && neo_test.raw_status == "skipped" {
        result.verdict = Verdict::Skip;
        result.reason =
            "Original upstream skip/fixme under actual headless Chromium conditions".into();
        return result;
    }
    if !reference_pass {
        result.verdict = Verdict::ReferenceFailure;
        result.reason = format!(
            "Reference raw status {} (expected {})",
            reference_test.raw_status, reference_test.expected_status
        );
        return result;
    }
    let (Some(reference_journal), Some(neo_journal)) = (&reference.journal, &neo.journal) else {
        result.reason = "The executed case lacks an engine journal".into();
        return result;
    };
    if let Some(error) = transport_failure(reference, neo) {
        result.reason = error;
        return result;
    }
    if !result.missing_reference.is_empty() || !result.missing_observed.is_empty() {
        result.verdict = Verdict::Unsupported;
        result.reason = format!(
            "missing {}",
            result
                .missing_reference
                .union(&result.missing_observed)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        return result;
    }
    if reference_journal
        .calls
        .iter()
        .chain(&neo_journal.calls)
        .any(|call| {
            matches!(
                call.outcome,
                CallOutcome::Pending | CallOutcome::Untransportable { .. }
            )
        })
    {
        result.reason = "A return value or pending call could not be fully observed".into();
        return result;
    }
    if let Err(error) = native_witness(neo) {
        result.reason = error;
        return result;
    }
    // An already measured unequal return is a real gap even if a later observer fence
    // lost evidence. The independent facet remains visible; only Pass needs every fence.
    if !result.differences.is_empty() {
        result.verdict = Verdict::Fail;
        result.reason.clone_from(&result.differences[0]);
        return result;
    }
    if !result.evidence_incomplete.is_empty() {
        result.reason = "Read-only browser observation is incomplete".into();
        return result;
    }
    if reference.observation != neo.observation {
        result
            .differences
            .push("Final DOM, delivered input or document lifecycle differs".into());
    }
    if !passed(neo_test) {
        result.differences.push(format!(
            "Neo upstream raw status {} (expected {})",
            neo_test.raw_status, neo_test.expected_status
        ));
    }
    if reference_journal.calls.is_empty() || neo_journal.calls.is_empty() {
        result.reason = "No measured browser API calls in the executed body".into();
        return result;
    }
    result.verdict = if result.differences.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    result.reason = result.differences.first().cloned().unwrap_or_else(|| {
        "Upstream assertions, every return/error and observed effects agree".into()
    });
    result
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingMethod {
    pub method: String,
    pub blocked_cases: BTreeSet<String>,
    pub reference_path_cases: BTreeSet<String>,
    pub observed_neo_cases: BTreeSet<String>,
    pub partial_reference_cases: BTreeSet<String>,
}

#[must_use]
pub fn missing_methods(records: &[ResultRecord]) -> Vec<MissingMethod> {
    let mut ranking = BTreeMap::new();
    for record in records {
        for method in record.missing_reference.union(&record.missing_observed) {
            let row = ranking
                .entry(method.clone())
                .or_insert_with(|| MissingMethod {
                    method: method.clone(),
                    blocked_cases: BTreeSet::new(),
                    reference_path_cases: BTreeSet::new(),
                    observed_neo_cases: BTreeSet::new(),
                    partial_reference_cases: BTreeSet::new(),
                });
            if record.reference_complete || record.missing_observed.contains(method) {
                row.blocked_cases.insert(record.case.key.clone());
            }
            if record.missing_reference.contains(method) {
                if record.reference_complete {
                    row.reference_path_cases.insert(record.case.key.clone());
                } else {
                    row.partial_reference_cases.insert(record.case.key.clone());
                }
            }
            if record.missing_observed.contains(method) {
                row.observed_neo_cases.insert(record.case.key.clone());
            }
        }
    }
    let mut rows: Vec<_> = ranking.into_values().collect();
    rows.sort_by(|left, right| {
        right
            .blocked_cases
            .len()
            .cmp(&left.blocked_cases.len())
            .then_with(|| left.method.cmp(&right.method))
    });
    rows
}
