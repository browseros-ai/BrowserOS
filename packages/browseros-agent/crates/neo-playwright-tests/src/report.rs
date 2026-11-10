//! Neo: Classifies raw outcomes and ranks distinct blocked cases. No matching failures count as pass.

use std::char;
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

use crate::model::{
    Call, CallOutcome, Case, Journal, JsValue, ObjectRef, Observation, Pair, Phase, RawTest,
    ResultRecord, SourceSite, Verdict,
};

fn record(case: Case, reason: String) -> ResultRecord {
    ResultRecord {
        case,
        verdict: Verdict::HarnessError,
        reason,
        differences: Vec::new(),
        missing_reference: BTreeSet::new(),
        missing_observed: BTreeSet::new(),
        reference_valid: false,
        reference_complete: false,
        comparable: false,
        evidence_incomplete: BTreeSet::new(),
        harness_limitations: BTreeSet::new(),
    }
}

#[must_use]
pub fn harness_error(case: Case, reason: String) -> ResultRecord {
    let mut result = record(case, reason.clone());
    result.evidence_incomplete.insert(reason);
    result
}

fn raw<'a>(phase: &'a Phase, key: &str) -> Option<&'a RawTest> {
    let runner = phase.runner.as_ref()?;
    if !runner.errors.is_empty() || runner.outcomes.len() != 1 {
        return None;
    }
    runner.outcomes.iter().find(|test| test.key == key)
}

fn exit_error(phase: &Phase, test: Option<&RawTest>) -> Option<String> {
    let Some(exit) = &phase.exit else {
        return Some("runner exit is missing".into());
    };
    if exit.signal.is_some() {
        return Some(format!("runner terminated by signal {:?}", exit.signal));
    }
    let Some(status) = phase
        .runner
        .as_ref()
        .and_then(|runner| runner.status.as_deref())
    else {
        return Some("runner final status is missing".into());
    };
    let expected_code = match status {
        "passed" => 0,
        "failed" | "timedout" => 1,
        _ => {
            return Some(format!(
                "runner final status {status:?} is not a completed run"
            ));
        }
    };
    if exit.code != Some(expected_code) {
        return Some(format!(
            "runner exit {:?} disagrees with status {status}",
            exit.code
        ));
    }
    if let Some(test) = test {
        let expected = test.raw_status == "skipped" || test.raw_status == test.expected_status;
        if (status == "passed" && !expected) || (status == "failed" && expected) {
            return Some(format!(
                "runner status {status} disagrees with raw status {} (expected {})",
                test.raw_status, test.expected_status
            ));
        }
        if status == "timedout" && test.raw_status != "timedOut" {
            return Some(format!(
                "runner timed out after reporting raw status {}",
                test.raw_status
            ));
        }
        if !matches!(
            test.raw_status.as_str(),
            "passed" | "failed" | "skipped" | "timedOut"
        ) {
            return Some(format!(
                "runner raw status {:?} is incomplete",
                test.raw_status
            ));
        }
    }
    None
}

/// A shortest origin wins over a longer alias path, then pinned source coordinates break ties.
/// This fixed-point construction ignores call array order and private transport token numbers.
/// Unrooted cycles never acquire an origin, so they cannot accidentally become comparable.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Origin {
    depth: usize,
    root: String,
    returns: Vec<ReturnOrigin>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ReturnOrigin {
    site: SourceSite,
    occurrence: u64,
    path: Vec<ValuePath>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ValuePath {
    Index(usize),
    Property(Vec<u16>),
    RegexpLastIndex,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CallKey {
    site: SourceSite,
    occurrence: u64,
    receiver: Origin,
}

/// Roots and return origins bind one engine's private references to semantic identities.
/// The index retains unresolved calls so an unknown receiver cannot masquerade as a missing call.
struct Correlation<'a> {
    objects: BTreeMap<ObjectRef, Origin>,
    calls: BTreeMap<CallKey, &'a Call>,
    unresolved: BTreeSet<Option<SourceSite>>,
    gaps: BTreeSet<String>,
}

fn source_valid(site: &SourceSite) -> bool {
    site.line > 0
        && site.column > 0
        && site.file.starts_with("tests/")
        && site
            .file
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".."))
        && !site.file.contains('\\')
}

fn value_objects<'a>(
    value: &'a JsValue,
    path: &mut Vec<ValuePath>,
    objects: &mut Vec<(&'a ObjectRef, Vec<ValuePath>)>,
) {
    match value {
        JsValue::Object { reference } => objects.push((reference, path.clone())),
        JsValue::Array { values } => {
            for (index, value) in values.iter().enumerate() {
                path.push(ValuePath::Index(index));
                value_objects(value, path, objects);
                path.pop();
            }
        }
        JsValue::Record { entries, .. } => {
            for (key, value) in entries {
                path.push(ValuePath::Property(key.clone()));
                value_objects(value, path, objects);
                path.pop();
            }
        }
        JsValue::Regexp { last_index, .. } => {
            path.push(ValuePath::RegexpLastIndex);
            value_objects(last_index, path, objects);
            path.pop();
        }
        _ => {}
    }
}

fn outcome_value(outcome: &CallOutcome) -> Option<&JsValue> {
    match outcome {
        CallOutcome::Returned { value } | CallOutcome::Threw { error: value } => Some(value),
        CallOutcome::Pending | CallOutcome::Untransportable { .. } => None,
    }
}

impl<'a> Correlation<'a> {
    fn new(journal: &'a Journal) -> Self {
        let mut result = Self {
            objects: BTreeMap::new(),
            calls: BTreeMap::new(),
            unresolved: BTreeSet::new(),
            gaps: BTreeSet::new(),
        };
        for (name, family) in [
            ("page", "Page"),
            ("context", "BrowserContext"),
            ("mainFrame", "Frame"),
        ] {
            if !matches!(journal.roots.get(name), Some(JsValue::Object { reference }) if reference.family == family)
            {
                result
                    .gaps
                    .insert(format!("missing or invalid {name} identity root"));
            }
        }
        for (name, value) in &journal.roots {
            let JsValue::Object { reference } = value else {
                result
                    .gaps
                    .insert(format!("identity root {name} is not an object"));
                continue;
            };
            let origin = Origin {
                depth: 0,
                root: name.clone(),
                returns: Vec::new(),
            };
            result.insert_origin(reference, origin);
        }
        let mut references = BTreeSet::new();
        let mut returns = Vec::new();
        for call in &journal.calls {
            references.insert(call.receiver.clone());
            let mut objects = Vec::new();
            if let Some(value) = outcome_value(&call.outcome) {
                value_objects(value, &mut Vec::new(), &mut objects);
            }
            references.extend(objects.iter().map(|(reference, _)| (*reference).clone()));
            returns.push(objects);
        }
        // At most one edge per distinct object is needed for a shortest rooted path.
        // Later passes propagate a lexically earlier alias to its descendants as well.
        for _ in 0..=references.len() {
            let mut changed = false;
            for (call, objects) in journal.calls.iter().zip(&returns) {
                let Some(site) = call.site.as_ref().filter(|site| source_valid(site)) else {
                    continue;
                };
                let Some(receiver) = result.objects.get(&call.receiver).cloned() else {
                    continue;
                };
                for (reference, path) in objects {
                    let mut origin = receiver.clone();
                    origin.depth += 1;
                    origin.returns.push(ReturnOrigin {
                        site: site.clone(),
                        occurrence: call.occurrence,
                        path: path.clone(),
                    });
                    changed |= result.insert_origin(reference, origin);
                }
            }
            if !changed {
                break;
            }
        }
        result.index_calls(journal);
        result
    }

    fn insert_origin(&mut self, reference: &ObjectRef, origin: Origin) -> bool {
        if self
            .objects
            .get(reference)
            .is_none_or(|current| origin < *current)
        {
            self.objects.insert(reference.clone(), origin);
            return true;
        }
        false
    }

    fn index_calls(&mut self, journal: &'a Journal) {
        let mut ordinals = BTreeSet::new();
        let mut occurrences = BTreeMap::<(&ObjectRef, &Option<SourceSite>), BTreeSet<u64>>::new();
        let mut duplicates = BTreeSet::new();
        for call in &journal.calls {
            if !ordinals.insert(call.ordinal) {
                self.gaps.insert(format!(
                    "duplicate diagnostic call ordinal {}",
                    call.ordinal
                ));
            }
            occurrences
                .entry((&call.receiver, &call.site))
                .or_default()
                .insert(call.occurrence);
            let site = call.site.as_ref().filter(|site| source_valid(site));
            let receiver = self.objects.get(&call.receiver);
            if let (Some(site), Some(receiver)) = (site, receiver) {
                let key = CallKey {
                    site: site.clone(),
                    occurrence: call.occurrence,
                    receiver: receiver.clone(),
                };
                if self.calls.insert(key.clone(), call).is_some() {
                    duplicates.insert(key);
                    self.gaps
                        .insert("duplicate source occurrence for one logical receiver".into());
                }
            } else {
                self.unresolved.insert(call.site.clone());
                self.gaps.insert(format!(
                    "call {} {} lacks a provable source or receiver",
                    call.ordinal, call.method
                ));
            }
        }
        for key in duplicates {
            self.calls.remove(&key);
            self.unresolved.insert(Some(key.site));
        }
        for counts in occurrences.values() {
            if counts
                .iter()
                .copied()
                .zip(0_u64..)
                .any(|(actual, expected)| actual != expected)
            {
                self.gaps
                    .insert("source occurrence sequence is incomplete".into());
            }
        }
        if ordinals
            .iter()
            .copied()
            .zip(0_u64..)
            .any(|(actual, expected)| actual != expected)
        {
            self.gaps
                .insert("global diagnostic ordinal sequence is incomplete".into());
        }
    }

    fn might_be_unresolved(&self, key: &CallKey) -> bool {
        self.unresolved.iter().any(|site| {
            site.as_ref()
                .is_none_or(|site| !source_valid(site) || site == &key.site)
        })
    }
}

fn canonicalize(value: &mut JsValue, objects: &BTreeMap<ObjectRef, Origin>) -> bool {
    match value {
        JsValue::Object { reference } => {
            let Some(origin) = objects.get(reference) else {
                return false;
            };
            // A semantic origin is kept as text only inside this comparison clone. It never
            // crosses the wire or changes raw evidence. Family remains part of exact equality.
            reference.epoch = format!("{origin:?}");
            reference.id = 0;
            true
        }
        JsValue::Array { values } => values.iter_mut().all(|value| canonicalize(value, objects)),
        JsValue::Record { entries, .. } => entries
            .iter_mut()
            .all(|(_, value)| canonicalize(value, objects)),
        JsValue::Regexp { last_index, .. } => canonicalize(last_index, objects),
        _ => true,
    }
}

fn canonical_outcome(call: &Call, objects: &BTreeMap<ObjectRef, Origin>) -> Option<CallOutcome> {
    let mut outcome = call.outcome.clone();
    let value = match &mut outcome {
        CallOutcome::Returned { value } | CallOutcome::Threw { error: value } => value,
        CallOutcome::Pending | CallOutcome::Untransportable { .. } => return None,
    };
    canonicalize(value, objects).then_some(outcome)
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

fn value_kind(value: &JsValue) -> &str {
    match value {
        JsValue::Object { reference } => &reference.family,
        JsValue::Undefined => "undefined",
        JsValue::Null => "null",
        JsValue::Hole => "array hole",
        JsValue::NegativeZero => "negative zero",
        JsValue::Nan => "NaN",
        JsValue::Infinity => "infinity",
        JsValue::NegativeInfinity => "negative infinity",
        JsValue::Boolean { .. } => "boolean",
        JsValue::Number { .. } => "number",
        JsValue::String { .. } => "string",
        JsValue::Bigint { .. } => "bigint",
        JsValue::Array { .. } => "array",
        JsValue::Record { .. } => "record",
        JsValue::Regexp { .. } => "regexp",
    }
}

fn outcome_kind(outcome: &CallOutcome) -> String {
    match outcome {
        CallOutcome::Returned { value } => format!("returned {}", value_kind(value)),
        CallOutcome::Threw { .. } => "threw a public error".into(),
        CallOutcome::Pending => "pending".into(),
        CallOutcome::Untransportable { .. } => "untransportable".into(),
    }
}

fn property<'a>(value: &'a JsValue, name: &str) -> Option<&'a JsValue> {
    let JsValue::Record { entries, .. } = value else {
        return None;
    };
    entries
        .iter()
        .find_map(|(key, value)| key.iter().copied().eq(name.encode_utf16()).then_some(value))
}

fn error_message(error: &Value) -> Option<String> {
    let value: JsValue = serde_json::from_value(error.clone()).ok()?;
    let message = property(&value, "message")
        .or_else(|| property(&value, "value"))
        .unwrap_or(&value);
    let JsValue::String { units } = message else {
        return None;
    };
    // Diagnostic text escapes lone surrogates instead of replacing them. The retained
    // tagged error and its exact public comparison never pass through this rendering.
    Some(
        char::decode_utf16(units.iter().copied())
            .map(|unit| match unit {
                Ok(character) => character.to_string(),
                Err(error) => format!("\\u{:04x}", error.unpaired_surrogate()),
            })
            .collect(),
    )
}

fn is_complete(value: Option<&JsValue>) -> bool {
    matches!(value, Some(JsValue::String { units }) if units.iter().copied().eq("complete".encode_utf16()))
}

fn complete(observation: Option<&Observation>) -> bool {
    observation.is_some_and(|value| {
        value.status == "complete"
            && is_complete(property(&value.data, "status"))
            && matches!(property(&value.data, "incomplete"), Some(JsValue::Array { values }) if values.is_empty())
            && matches!(property(&value.data, "pages"), Some(JsValue::Array { values }) if !values.is_empty())
            && matches!(property(&value.data, "events"), Some(JsValue::Array { .. }))
            && property(&value.data, "activation").is_some_and(|activation| {
                is_complete(property(activation, "status"))
                    && matches!(property(activation, "tabs"), Some(JsValue::Array { .. }))
            })
    })
}

fn journal_facets<'a>(
    result: &mut ResultRecord,
    engine: &str,
    phase: &'a Phase,
) -> Option<Correlation<'a>> {
    let Some(journal) = &phase.journal else {
        result
            .evidence_incomplete
            .insert(format!("{engine}: engine journal is missing"));
        return None;
    };
    for (kind, gaps) in [
        ("census", &journal.census_gaps),
        ("correlation", &journal.correlation_gaps),
        ("transport", &journal.transport_errors),
    ] {
        for gap in gaps {
            result
                .evidence_incomplete
                .insert(format!("{engine}: {kind}: {gap}"));
        }
    }
    for limitation in &journal.harness_limitations {
        let limitation = format!("{engine}: {limitation}");
        result.harness_limitations.insert(limitation.clone());
        result
            .evidence_incomplete
            .insert(format!("harness limitation: {limitation}"));
    }
    if journal.calls.iter().any(|call| {
        matches!(
            call.outcome,
            CallOutcome::Pending | CallOutcome::Untransportable { .. }
        )
    }) {
        result
            .evidence_incomplete
            .insert(format!("{engine}: pending or untransportable call outcome"));
    }
    let correlation = Correlation::new(journal);
    for gap in &correlation.gaps {
        result
            .evidence_incomplete
            .insert(format!("{engine}: correlation: {gap}"));
    }
    Some(correlation)
}

fn compare_journals(result: &mut ResultRecord, reference: &Phase, neo: &Phase) {
    if let Some(journal) = &neo.journal {
        result.missing_observed.clone_from(&journal.missing);
        if let Some(reference) = &reference.journal {
            result.missing_reference = reference
                .requirements
                .iter()
                .filter(|method| absent(method, &journal.capabilities))
                .cloned()
                .collect();
        }
    }
    let expected = journal_facets(result, "reference", reference);
    let actual = journal_facets(result, "neo", neo);
    if let Some(correlation) = &expected {
        let journal = reference.journal.as_ref();
        result.reference_complete = result.reference_valid
            && journal.is_some_and(|journal| {
                journal.census_gaps.is_empty()
                    && journal.correlation_gaps.is_empty()
                    && journal.transport_errors.is_empty()
                    && journal.harness_limitations.is_empty()
                    && !journal.calls.is_empty()
                    && journal
                        .calls
                        .iter()
                        .all(|call| outcome_value(&call.outcome).is_some())
            })
            && correlation.gaps.is_empty();
    }
    let (Some(expected), Some(actual)) = (expected, actual) else {
        return;
    };
    for (key, expected_call) in &expected.calls {
        if let Some(actual_call) = actual.calls.get(key) {
            let left = canonical_outcome(expected_call, &expected.objects);
            let right = canonical_outcome(actual_call, &actual.objects);
            if expected_call.method != actual_call.method
                || matches!((&left, &right), (Some(left), Some(right)) if left != right)
            {
                result.differences.push(format!("{}:{}:{} occurrence {} {} differs in method, exact value or public error (reference {}; Neo {} {})", key.site.file, key.site.line, key.site.column, key.occurrence, expected_call.method, outcome_kind(&expected_call.outcome), actual_call.method, outcome_kind(&actual_call.outcome)));
            }
        } else if !actual.might_be_unresolved(key) {
            result.differences.push(format!(
                "{}:{}:{} occurrence {} {} has no Neo call on the same logical receiver",
                key.site.file, key.site.line, key.site.column, key.occurrence, expected_call.method
            ));
        }
    }
    for (key, call) in &actual.calls {
        if !expected.calls.contains_key(key) && !expected.might_be_unresolved(key) {
            result.differences.push(format!(
                "{}:{}:{} occurrence {} {} has no reference call on the same logical receiver",
                key.site.file, key.site.line, key.site.column, key.occurrence, call.method
            ));
        }
    }
    for (name, expected_root) in &reference
        .journal
        .as_ref()
        .map_or_else(BTreeMap::new, |journal| journal.roots.clone())
    {
        let Some(actual_root) = neo
            .journal
            .as_ref()
            .and_then(|journal| journal.roots.get(name))
        else {
            continue;
        };
        if !matches!(
            (expected_root, actual_root),
            (JsValue::Object { .. }, JsValue::Object { .. })
        ) {
            continue;
        }
        let mut expected_root = expected_root.clone();
        let mut actual_root = actual_root.clone();
        if canonicalize(&mut expected_root, &expected.objects)
            && canonicalize(&mut actual_root, &actual.objects)
            && expected_root != actual_root
        {
            result
                .differences
                .push(format!("logical identity root {name} differs"));
        }
    }
}

fn native_witness(neo: &Phase) -> Result<(), String> {
    let native = neo
        .journal
        .as_ref()
        .and_then(|journal| journal.native_outcome.as_ref())
        .ok_or("No native RunOutcome proves target execution")?;
    if native["status"] != "ok"
        || native
            .get("callsDropped")
            .is_some_and(|dropped| !dropped.is_null())
    {
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

#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "Verdict precedence keeps independent evidence facets visible; only complete evidence can pass."
)]
pub fn classify(pair: &Pair) -> ResultRecord {
    let mut result = record(pair.entry.clone(), "Paired evidence is incomplete".into());
    let (Some(reference), Some(neo)) = (pair.phases.get("reference"), pair.phases.get("neo"))
    else {
        result
            .evidence_incomplete
            .insert("An engine phase is missing".into());
        return result;
    };
    let reference_test = raw(reference, &pair.entry.key);
    let neo_test = raw(neo, &pair.entry.key);
    let mut infrastructure = Vec::new();
    for (engine, phase, test) in [
        ("reference", reference, reference_test),
        ("neo", neo, neo_test),
    ] {
        for error in &phase.infrastructure_errors {
            infrastructure.push(format!("{engine}: {error}"));
        }
        if let Some(error) = exit_error(phase, test) {
            infrastructure.push(format!("{engine}: {error}"));
        }
        if test.is_none() {
            infrastructure.push(format!(
                "{engine}: runner did not finish exactly the requested upstream case"
            ));
        }
    }
    result.reference_valid = reference_test.is_some_and(passed)
        && reference.infrastructure_errors.is_empty()
        && exit_error(reference, reference_test).is_none()
        && reference
            .journal
            .as_ref()
            .is_none_or(|journal| journal.harness_limitations.is_empty());
    // A genuine upstream skip does not execute the fixture or need browser observations.
    if infrastructure.is_empty()
        && reference_test.is_some_and(|test| test.raw_status == "skipped")
        && neo_test.is_some_and(|test| test.raw_status == "skipped")
    {
        result.verdict = Verdict::Skip;
        result.reason =
            "Original upstream skip/fixme under actual headless Chromium conditions".into();
        return result;
    }
    compare_journals(&mut result, reference, neo);
    for (engine, phase) in [("reference", reference), ("neo", neo)] {
        if !complete(phase.observation.as_ref()) {
            result.evidence_incomplete.insert(format!(
                "{engine}: read-only observation is incomplete or unreadable"
            ));
        }
    }
    if let Err(error) = native_witness(neo) {
        result.evidence_incomplete.insert(error);
    }
    for error in &infrastructure {
        result
            .evidence_incomplete
            .insert(format!("Infrastructure: {error}"));
    }
    if let Some(error) = infrastructure.first() {
        result.reason = format!("Infrastructure: {error}");
        return result;
    }
    let (Some(reference_test), Some(neo_test)) = (reference_test, neo_test) else {
        return result;
    };
    if let Some(limitation) = reference
        .journal
        .as_ref()
        .and_then(|journal| journal.harness_limitations.first())
    {
        result.reason = format!("Harness limitation: reference: {limitation}");
        return result;
    }
    if !passed(reference_test) {
        result.verdict = Verdict::ReferenceFailure;
        result.reason = format!(
            "Reference raw status {} (expected {})",
            reference_test.raw_status, reference_test.expected_status
        );
        if let Some(message) = reference_test.errors.iter().find_map(error_message) {
            result.reason.push_str(": ");
            result.reason.push_str(&message);
        }
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
    if let Some(limitation) = result.harness_limitations.first() {
        result.reason = format!("Harness limitation: {limitation}");
        return result;
    }
    if complete(reference.observation.as_ref())
        && complete(neo.observation.as_ref())
        && reference.observation != neo.observation
    {
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
    if reference
        .journal
        .as_ref()
        .is_some_and(|journal| journal.calls.is_empty())
        || neo
            .journal
            .as_ref()
            .is_some_and(|journal| journal.calls.is_empty())
    {
        result
            .evidence_incomplete
            .insert("No measured browser API calls in the executed body".into());
    }
    // A reliable unequal return or upstream assertion remains a real gap when a later
    // observation or witness is lost. Completeness is an independent facet and gates Pass.
    if let Some(difference) = result.differences.first() {
        result.verdict = Verdict::Fail;
        result.reason.clone_from(difference);
        result.comparable = result.evidence_incomplete.is_empty() && result.reference_complete;
    } else if let Some(gap) = result.evidence_incomplete.first() {
        result.reason.clone_from(gap);
    } else {
        result.verdict = Verdict::Pass;
        result.reason = "Upstream assertions, every return/error and observed effects agree".into();
        result.comparable = result.reference_complete;
    }
    result
}

/// Complete reference requirements and actual native missing use confirm blockers independently.
/// Partial reference-only requirements remain separate; all sets deduplicate full generated keys.
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

/// Integer ratios retain an explicit empty denominator instead of manufacturing a percentage.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Ratio {
    pub numerator: usize,
    pub denominator: usize,
}

/// Eligible cases have a valid passing reference. Fully comparable cases also have complete
/// evidence and a Pass or Fail verdict; reliable partial Fails remain in the compatibility ratio.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub completed_count: usize,
    pub counts: BTreeMap<Verdict, usize>,
    pub compatibility: Ratio,
    pub coverage: Ratio,
    pub partial_evidence_count: usize,
    pub partial_fail_count: usize,
    pub partial_reference_count: usize,
    pub harness_limitation_count: usize,
}

impl Default for Totals {
    fn default() -> Self {
        Self {
            completed_count: 0,
            counts: [
                Verdict::Pass,
                Verdict::Fail,
                Verdict::Unsupported,
                Verdict::Skip,
                Verdict::NotApplicable,
                Verdict::ReferenceFailure,
                Verdict::HarnessError,
            ]
            .into_iter()
            .map(|verdict| (verdict, 0))
            .collect(),
            compatibility: Ratio::default(),
            coverage: Ratio::default(),
            partial_evidence_count: 0,
            partial_fail_count: 0,
            partial_reference_count: 0,
            harness_limitation_count: 0,
        }
    }
}

impl Totals {
    fn add(&mut self, record: &ResultRecord) {
        self.completed_count += 1;
        *self.counts.entry(record.verdict).or_default() += 1;
        self.compatibility.numerator += usize::from(record.verdict == Verdict::Pass);
        self.compatibility.denominator +=
            usize::from(matches!(record.verdict, Verdict::Pass | Verdict::Fail));
        self.coverage.numerator += usize::from(record.comparable);
        self.coverage.denominator += usize::from(record.reference_valid);
        self.partial_evidence_count += usize::from(!record.evidence_incomplete.is_empty());
        self.partial_fail_count +=
            usize::from(record.verdict == Verdict::Fail && !record.comparable);
        self.partial_reference_count +=
            usize::from(!record.reference_complete && !record.missing_reference.is_empty());
        self.harness_limitation_count += usize::from(!record.harness_limitations.is_empty());
    }
}

/// The Cargo parent publishes these same counters overall, by feature and by suite. Harness
/// limitations retain distinct case sets without becoming missing native methods.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub totals: Totals,
    pub features: BTreeMap<String, Totals>,
    pub suites: BTreeMap<String, Totals>,
    pub harness_limitations: BTreeMap<String, BTreeSet<String>>,
}

#[must_use]
pub fn summarize(records: &[ResultRecord]) -> Summary {
    let mut summary = Summary::default();
    for record in records {
        summary.totals.add(record);
        summary
            .features
            .entry(record.case.feature.clone())
            .or_default()
            .add(record);
        summary
            .suites
            .entry(record.case.file.clone())
            .or_default()
            .add(record);
        for limitation in &record.harness_limitations {
            summary
                .harness_limitations
                .entry(limitation.clone())
                .or_default()
                .insert(record.case.key.clone());
        }
    }
    summary
}
