//! Couples reported close records to the page facts released after the cache observation.
//! The producer's event policy runs earlier. A later refusal must withhold the record too,
//! because removing its replacement fact also removes the cutoff that disarms an old close.

use std::collections::{HashMap, HashSet};

use browseros_core::TabId;
use serde_json::Value;

use super::operation_guard::Ending;
use super::{Claim, ErrorCode, OperationError, PageFact};

pub(super) fn close_release(
    ending: &mut Ending,
    claims: &HashMap<TabId, Vec<Claim>>,
    facts: &[PageFact],
    refusals: &HashMap<TabId, OperationError>,
) {
    let blocked: HashSet<_> = claims
        .iter()
        .filter(|(tab, _)| !facts.iter().any(|fact| fact.tab == **tab))
        .flat_map(|(tab, claims)| claims.iter().map(move |claim| (*tab, claim.seq)))
        .collect();
    if blocked.is_empty() {
        return;
    }
    let Ending::Returned(value) = ending else {
        return;
    };
    if let Some(events) = value.get_mut("events").and_then(Value::as_array_mut) {
        let before = events.len();
        events.retain(|event| record(event).is_none_or(|key| !blocked.contains(&key)));
        let removed = before - events.len();
        if removed > 0 {
            let withheld = value
                .get("withheld")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            value["withheld"] = withheld
                .saturating_add(u64::try_from(removed).unwrap_or(u64::MAX))
                .into();
        }
    } else if let Some(key) = record(value)
        && blocked.contains(&key)
    {
        // A single-event pull cannot return a value after withholding that event. Preserve
        // the final policy's actual refusal when it is the reason the fact was withheld.
        let error = refusals.get(&key.0).cloned().unwrap_or_else(|| {
            OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                "the close record has no released page observation",
            )
        });
        *ending = Ending::Refused(error);
    }
}

/// Only native close records with matching producer reports are considered by [`close_release`].
fn record(value: &Value) -> Option<(TabId, u64)> {
    if value.get("event")?.as_str()? != "close" {
        return None;
    }
    let page = value.get("page")?;
    if page.get("$neo")?.as_str()? != "page" {
        return None;
    }
    Some((
        TabId(page.get("tab")?.as_i64()?),
        value.get("seq")?.as_u64()?,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use browseros_core::{TabId, TargetId};
    use serde_json::json;

    use super::close_release;
    use crate::dispatch::operation_guard::Ending;
    use crate::dispatch::{Claim, ErrorCode, OperationError, PageFact};

    #[test]
    fn late_withholding_removes_only_reported_closes_and_keeps_batch_order() {
        let claims = HashMap::from([(
            TabId(51),
            vec![Claim {
                target: TargetId("old".into()),
                seq: 7,
                released: true,
            }],
        )]);
        let keep = json!({"event":"console","seq":6,"page":{"$neo":"page","tab":52}});
        let close = json!({"event":"close","seq":7,"page":{"$neo":"page","tab":51}});
        let after = json!({"event":"console","seq":8,"page":{"$neo":"page","tab":52}});
        let mut ending =
            Ending::Returned(json!({"events":[keep,close,after],"lost":0,"withheld":2}));
        close_release(&mut ending, &claims, &[], &HashMap::new());
        let Ending::Returned(value) = ending else {
            panic!("batch must still return")
        };
        assert_eq!(value, json!({"events":[keep,after],"lost":0,"withheld":3}));
    }

    #[test]
    fn a_single_close_uses_the_final_policy_refusal_and_allowed_facts_preserve_it() {
        let tab = TabId(51);
        let claims = HashMap::from([(
            tab,
            vec![Claim {
                target: TargetId("old".into()),
                seq: 7,
                released: true,
            }],
        )]);
        let value = json!({"event":"close","seq":7,"page":{"$neo":"page","tab":51}});
        let mut allowed = Ending::Returned(value.clone());
        close_release(
            &mut allowed,
            &claims,
            &[PageFact {
                viewport_size: None,
                tab,
                url: None,
                title: None,
                closed: true,
            }],
            &HashMap::new(),
        );
        assert!(matches!(allowed, Ending::Returned(ref same) if same == &value));
        let refusal = OperationError::host(ErrorCode::OWNER_HAS_CONTROL, "owner has control");
        let refusals = HashMap::from([(tab, refusal)]);
        let mut ending = Ending::Returned(value);
        close_release(&mut ending, &claims, &[], &refusals);
        let Ending::Refused(error) = ending else {
            panic!("single close must be refused")
        };
        assert_eq!(error.code(), ErrorCode::OWNER_HAS_CONTROL);
    }
}
