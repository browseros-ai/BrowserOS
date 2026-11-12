//! The run envelope as the realm receives it, one example per shape: the text of a run's init
//! and of op replies, rendered by the code that renders them for a real run.
//!
//! `tests/envelope_generated.rs` pins every example against `tests/data/envelope-examples.json`,
//! and the prelude's `tests/prelude-envelope.test.ts` reads the same file, so both halves of the
//! contract are checked against identical bytes: a change to a field's name, order, omission or
//! encoding fails one of the two.

use browseros_core::TabId;
use browseros_core::viewport::Size;
use neo_trace::{Delivery, IdError, OperationId, RunId};
use serde_json::{Value, json};

use crate::browser::PageRef;
use crate::dispatch::{
    ApprovedClose, ErrorCode, Observation, OpFailure, OpReply, OperationError, PageFact, WorkerFact,
};
use crate::runtime::{self, RunInit};

/// One rendered example: its name in the pinned file and the exact text the realm reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeExample {
    /// The key in `tests/data/envelope-examples.json` (`reply.ok`, `init.lazy`, …).
    pub name: &'static str,
    /// The JSON text, byte for byte as Neo hands it to the realm.
    pub text: String,
}

/// The run every example belongs to, fixed so its operation ids are stable.
const RUN: &str = "0196a0c4-1f2e-7a3b-8c4d-5e6f7a8b9c0d";

/// Every example, in the pinned file's order.
pub fn envelope_examples() -> Result<Vec<EnvelopeExample>, IdError> {
    let operation = OperationId::new(RUN.parse::<RunId>()?, 4);
    Ok(vec![
        example(
            "init.page",
            init(Some(&page(3)), &[live(3), closed(4)], Some(&observed())),
        ),
        example("init.lazy", init(None, &[], None)),
        example("reply.ok", reply_ok()),
        example(
            "reply.okBare",
            runtime::envelope(&bare(Ok(Value::Null)), &[]),
        ),
        example("reply.err", reply_err(operation)),
        example("reply.errBare", reply_err_bare(operation)),
    ])
}

fn example(name: &'static str, text: String) -> EnvelopeExample {
    EnvelopeExample { name, text }
}

/// The run's init, as `run_script` and the hosted run hand it to the realm.
fn init(page: Option<&PageRef>, pages: &[PageFact], observed: Option<&Observation>) -> String {
    RunInit::new(page, pages, observed).to_json()
}

/// A `page.title` answer carrying every optional field the envelope has.
fn reply_ok() -> String {
    let reply = OpReply {
        pages: vec![live(3), closed(4)],
        observed: Some(Observation {
            seq: 5,
            events: 9,
            payload_current: Some(true),
            closes: vec![ApprovedClose {
                tab: TabId(4),
                seq: 8,
            }],
        }),
        default_page: Some(page(3)),
        pumped: 2,
        ..bare(Ok(json!("Facts")))
    };
    runtime::envelope(&reply, &[7])
}

/// A failed `expect` carrying every optional field the envelope has, its matcher's facts
/// included.
fn reply_err(operation: OperationId) -> String {
    let error = OperationError::website(
        ErrorCode::ASSERTION_FAILED,
        "expect(locator).toHaveText(expected) failed",
    )
    .timed_out()
    .with_matcher(json!({ "name": "toHaveText", "pass": false, "received": "Draft" }));
    let reply = OpReply {
        pages: vec![live(3)],
        observed: Some(Observation {
            seq: 6,
            events: 9,
            payload_current: Some(false),
            closes: Vec::new(),
        }),
        default_page: Some(page(3)),
        pumped: 1,
        ..bare(Err(OpFailure {
            error,
            delivery: Delivery::Done,
            retryable: true,
            operation,
        }))
    };
    runtime::envelope(&reply, &[8, 9])
}

/// A refused write: no facts, no observation, nothing sent.
fn reply_err_bare(operation: OperationId) -> String {
    let error = OperationError::script(
        ErrorCode::INVALID_INPUT,
        "page.click: selector: expected string",
    );
    let reply = bare(Err(OpFailure {
        error,
        delivery: Delivery::NotSent,
        retryable: false,
        operation,
    }));
    runtime::envelope(&reply, &[])
}

fn bare(outcome: Result<Value, OpFailure>) -> OpReply {
    OpReply {
        outcome,
        pages: Vec::new(),
        observed: None,
        default_page: None,
        pumped: 0,
        completed_publications: Vec::new(),
    }
}

fn page(tab: i64) -> PageRef {
    PageRef { tab: TabId(tab) }
}

/// A live tab with every optional fact: its task, a script-set viewport and one dedicated worker.
fn live(tab: i64) -> PageFact {
    PageFact {
        tab: TabId(tab),
        task_id: Some("hn-top-5-3f9a".to_owned()),
        url: Some("https://facts.example/".to_owned()),
        title: Some("Facts".to_owned()),
        closed: false,
        viewport_size: Some(Size {
            width: 800,
            height: 600,
        }),
        workers: Some(vec![WorkerFact {
            id: "worker-1".to_owned(),
            url: "https://facts.example/worker.js".to_owned(),
        }]),
    }
}

/// A closed tab: its URL, title and task gone, no optional facts.
fn closed(tab: i64) -> PageFact {
    PageFact {
        tab: TabId(tab),
        task_id: None,
        url: None,
        title: None,
        closed: true,
        viewport_size: None,
        workers: None,
    }
}

fn observed() -> Observation {
    Observation {
        seq: 1,
        events: 0,
        payload_current: None,
        closes: Vec::new(),
    }
}
