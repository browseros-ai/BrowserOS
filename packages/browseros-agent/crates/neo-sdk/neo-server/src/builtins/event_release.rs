//! The read rule on the events a Playwright event pull hands back (sf-review finding 1).
//!
//! The door checks an op's rules on the one tab the op names. An event pull names no tab
//! (`events.next`, the prelude's pump) or only the page it waits on (`events.wait`), yet it
//! returns what pages did: the pump, every page the run listens to, and a popup or a new tab
//! names a second page. So before such a result leaves the door, every tab each event names goes
//! through the same rules here. An event that names a tab the owner holds with reads blocked
//! (takeover, `reads: block`) is withheld for good: it is what the page did while the owner was
//! in control.
//!
//! The rules run when the pull reads an event, not when the page made it. The pump is pending
//! for as long as a listener exists and reads each event as it is recorded, so the two differ
//! only while the prelude is between two pulls: an event recorded then, under a lock that ends
//! before the next pull, is released. Closing that gap takes asking whether a lock held the tab
//! at the event's time, and `Locks::check` answers only for now.
//!
//! ```text
//!  Playwright::invoke(events.next) ─ { events, lost } ─ each event ─ its tabs ─ OpCtx::check_tab
//!     ├─ every tab passes ─▶ released, in order
//!     └─ one is refused ───▶ withheld: dropped, counted in the reply's `withheld`, noted on the op
//! ```

use browseros_core::TabId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::dispatch::{ErrorCode, LeafError, OpCtx, OperationError};

/// Whether a Playwright op's result carries events, whose tabs the door's rules clear one by one
/// before it is released.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EventRelease {
    /// No events: the door's own checks on the op's target cover its result.
    None,
    /// `events.next`, the pump: events from every page the run listens to. One from a refused
    /// tab is withheld and the rest are released, so a takeover of one tab never stalls the run's
    /// other listeners, and the pump always answers.
    Batch,
    /// `events.wait`, behind `waitForEvent`: the one event it waited for. One that names a
    /// refused tab fails the wait with the rule's refusal, as any read of that tab does.
    One,
}

impl EventRelease {
    /// The crate's op names, as the prelude sends them; a test checks each is an op.
    pub(super) fn of(name: &str) -> Self {
        match name {
            "events.next" => Self::Batch,
            "events.wait" => Self::One,
            _ => Self::None,
        }
    }

    /// `result` as it may leave the door.
    pub(super) fn release<M>(self, op: &OpCtx<M>, result: Value) -> Result<Value, LeafError> {
        match self {
            Self::None => Ok(result),
            Self::One => match refusal(op, &result) {
                None => Ok(result),
                Some(refused) => Err(LeafError::definite(refused)),
            },
            Self::Batch => release_batch(op, result),
        }
    }
}

/// The pump's result, `{ events, lost }` as the crate builds it, and as the door releases it:
/// less the events it withheld, with how many those were. Fields the crate adds pass through.
#[derive(Deserialize, Serialize)]
struct Batch {
    events: Vec<Value>,
    /// Events the door withheld from this pull; absent when it withheld none.
    #[serde(default, skip_serializing_if = "is_zero")]
    withheld: u64,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

fn release_batch<M>(op: &OpCtx<M>, result: Value) -> Result<Value, LeafError> {
    // Fails closed: a pull whose events Neo cannot read is never released unchecked.
    let unreadable = |error: serde_json::Error| {
        LeafError::definite(OperationError::neo(
            ErrorCode::INTERNAL_ERROR,
            format_args!("the event pull's result is not `{{ events, lost }}`: {error}"),
        ))
    };
    let Batch {
        events,
        withheld: _,
        rest,
    } = serde_json::from_value(result).map_err(unreadable)?;
    let mut released = Vec::with_capacity(events.len());
    let mut withheld: u64 = 0;
    let mut first_refusal = None;
    for event in events {
        match refusal(op, &event) {
            None => released.push(event),
            Some(refused) => {
                withheld += 1;
                first_refusal.get_or_insert(refused);
            }
        }
    }
    if let Some(refused) = first_refusal {
        op.note(format!(
            "withheld {withheld} event(s) from a tab the rules refuse: {refused}"
        ));
    }
    let batch = Batch {
        events: released,
        withheld,
        rest,
    };
    serde_json::to_value(batch).map_err(unreadable)
}

/// The rules' refusal of the first tab `event` names that they refuse, if any. The tabs come
/// from every page brand in the event (its page, a popup's new tab, a new tab's opener), so an
/// event kind the crate adds later is covered as well.
fn refusal<M>(op: &OpCtx<M>, event: &Value) -> Option<OperationError> {
    let mut tabs = Vec::new();
    tabs_named(event, &mut tabs);
    tabs.into_iter().find_map(|tab| op.check_tab(tab).err())
}

/// Adds the tab of every page brand in `value`, `{"$neo":"page","id":7,"tab":1234}`, to `tabs`.
fn tabs_named(value: &Value, tabs: &mut Vec<TabId>) {
    match value {
        Value::Object(fields) => {
            let page = fields.get("$neo").and_then(Value::as_str) == Some("page");
            if let (true, Some(tab)) = (page, fields.get("tab").and_then(Value::as_i64))
                && !tabs.contains(&TabId(tab))
            {
                tabs.push(TabId(tab));
            }
            for field in fields.values() {
                tabs_named(field, tabs);
            }
        }
        Value::Array(items) => {
            for item in items {
                tabs_named(item, tabs);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if hands the field by reference"
)]
fn is_zero(count: &u64) -> bool {
    *count == 0
}

#[cfg(test)]
mod tests {
    use neo_playwright::Op;
    use serde_json::json;

    use super::{EventRelease, tabs_named};

    #[test]
    fn every_event_release_names_an_op() {
        for name in ["events.next", "events.wait"] {
            assert!(Op::from_name(name).is_some(), "{name} is a crate op");
            assert_ne!(EventRelease::of(name), EventRelease::None, "{name}");
        }
        assert_eq!(EventRelease::of("events.listen"), EventRelease::None);
    }

    #[test]
    fn an_event_names_every_tab_it_carries() {
        let page = |tab: i64| json!({ "$neo": "page", "id": tab - 1000, "tab": tab });
        let cases = [
            (
                json!({ "seq": 9, "event": "console", "page": page(1007),
                        "message": { "type": "log", "text": "hi" } }),
                vec![1007],
            ),
            // A popup is the opener's event about another tab.
            (
                json!({ "seq": 8, "event": "popup", "page": page(1007),
                        "popup": { "page": page(1030), "url": "https://login.test/" } }),
                vec![1007, 1030],
            ),
            // A new tab, on the context, with the tab that opened it.
            (
                json!({ "seq": 8, "event": "page", "page": page(1030),
                        "url": "https://login.test/", "opener": page(1007) }),
                vec![1030, 1007],
            ),
            (
                json!({ "seq": 8, "event": "page", "page": page(1030), "opener": null }),
                vec![1030],
            ),
        ];
        for (event, expected) in cases {
            let mut tabs = Vec::new();
            tabs_named(&event, &mut tabs);
            let mut tabs: Vec<i64> = tabs.into_iter().map(|tab| tab.0).collect();
            tabs.sort_unstable();
            let mut expected = expected;
            expected.sort_unstable();
            assert_eq!(tabs, expected, "each tab once, in any order: {event}");
        }
    }
}
