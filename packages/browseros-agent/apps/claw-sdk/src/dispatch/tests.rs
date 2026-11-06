//! The door end to end: the real dispatcher, the fake browser, and a real trace store read back
//! from a temp dir. One test per promise of skeleton D7 and the grafts on it.

mod harness;
mod test_methods;

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use browseros_trace::{
    Delivery, Effect, ErrorOwner, OperationId, Outcome, RunEffects, RunStatus, SecretSource,
    SecretValue,
};
use futures_util::future;
use serde_json::{Value, json};
use tokio::time::Instant;

use self::harness::{GATE_PARTIES, Harness, SeenCheck, within};
use crate::browser::PageRef;
use crate::dispatch::{
    Call, Caller, ErrorCode, OpFailure, OpReply, OpsReport, PendingReply, RunEnd, StopReason,
    TEXT_CHARS, WITHHELD,
};

/// The page brand a script sends for `page`.
fn handle(page: &PageRef) -> Value {
    json!({ "$neo": "page", "id": page.id.0, "tab": page.tab.0 })
}

fn failure(reply: OpReply) -> Result<OpFailure, Box<dyn Error>> {
    match reply.outcome {
        Ok(value) => Err(format!("expected a failure, got {value}").into()),
        Err(failure) => Ok(failure),
    }
}

fn owner_and_code(failure: &OpFailure) -> (ErrorOwner, ErrorCode) {
    (failure.error.owner(), failure.error.code())
}

/// The refusal brand `__neo.refuse(api, 'method_unavailable', reason)` sends (FR/RC C1).
fn refused(reason: &str) -> Value {
    json!({ "$neo": "refused", "code": "method_unavailable", "reason": reason })
}

#[tokio::test]
async fn a_refusal_is_recorded_as_not_sent() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;
    harness.lock.writes.store(true, Ordering::SeqCst);

    let (_, reply) = harness
        .submit(&run, "test.write", json!({ "page": handle(&page) }))
        .await;
    let failure = failure(reply)?;
    assert_eq!(
        owner_and_code(&failure),
        (ErrorOwner::Host, ErrorCode::OWNER_HAS_CONTROL)
    );
    assert_eq!(
        failure.delivery,
        Delivery::NotSent,
        "a refusal comes before the run step"
    );
    assert!(failure.retryable, "nothing was sent, so repeating is safe");

    let row = harness.row(failure.operation).await?;
    assert_eq!(
        row.delivery,
        Some(Delivery::NotSent),
        "the trace records refusals too"
    );
    assert_eq!(row.outcome, Some(Outcome::Error));
    assert_eq!(row.tab, Some(page.tab), "recorded on the tab the rule saw");
    assert_eq!(
        row.error.map(|error| (error.owner, error.code)),
        Some((ErrorOwner::Host, "owner_has_control".to_owned()))
    );
    Ok(())
}

#[tokio::test]
async fn a_childs_send_marks_its_parent_sent() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;

    let (_, reply) = harness
        .submit(&run, "test.parent", json!({ "page": handle(&page) }))
        .await;
    let failure = failure(reply)?;
    assert_eq!(
        failure.delivery,
        Delivery::Unknown,
        "the child wrote, and the parent's failure did not say whether it landed"
    );
    assert!(
        !failure.retryable,
        "a write that may have landed is not repeated blindly"
    );

    let parent = harness.row(failure.operation).await?;
    let child = harness
        .row(OperationId::new(run.id(), failure.operation.seq + 1))
        .await?;
    assert_eq!(
        child.parent,
        Some(failure.operation),
        "the child is recorded under its parent"
    );
    assert_eq!(child.delivery, Some(Delivery::Done));
    assert_eq!(
        parent.sent_at, child.sent_at,
        "the parent was sent when its child was"
    );
    Ok(())
}

#[tokio::test]
async fn a_refused_recheck_is_the_ops_error_however_the_method_then_fails()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;

    let input = json!({ "page": handle(&page), "recover": false });
    let (_, reply) = harness.submit(&run, "test.sticky", input).await;
    let failure = failure(reply)?;
    assert_eq!(
        owner_and_code(&failure),
        (ErrorOwner::Host, ErrorCode::OWNER_HAS_CONTROL),
        "the refusal outlives the method's own error for it"
    );
    assert_eq!(failure.delivery, Delivery::NotSent);
    Ok(())
}

#[tokio::test]
async fn a_recheck_that_passes_again_lifts_the_refusal() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;

    let input = json!({ "page": handle(&page), "recover": true });
    let (_, reply) = harness.submit(&run, "test.sticky", input).await;
    let failure = failure(reply)?;
    assert_eq!(
        owner_and_code(&failure),
        (ErrorOwner::Website, ErrorCode::CDP_ERROR),
        "once the lock lifted, the page's own answer is the op's error"
    );
    assert_eq!(
        failure.delivery,
        Delivery::Done,
        "sent, and the page answered"
    );
    Ok(())
}

#[tokio::test]
async fn a_panicking_method_still_answers_and_is_recorded() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    let (_, reply) = harness.submit(&run, "test.panic", json!({})).await;
    let failure = failure(reply)?;
    assert_eq!(
        owner_and_code(&failure),
        (ErrorOwner::Neo, ErrorCode::INTERNAL_ERROR)
    );
    assert_eq!(
        failure.delivery,
        Delivery::NotSent,
        "it panicked before sending anything"
    );

    let row = harness.row(failure.operation).await?;
    assert_eq!(
        row.outcome,
        Some(Outcome::Error),
        "op_ended was recorded during the unwind"
    );
    assert_eq!(
        row.error.map(|error| error.code),
        Some("internal_error".to_owned())
    );

    let (_, after) = harness
        .submit(&run, "test.typed", json!({ "outer": { "inner": 1 } }))
        .await;
    assert!(
        after.outcome.is_ok(),
        "one method's panic does not take the run down"
    );
    Ok(())
}

#[tokio::test]
async fn a_stopped_op_is_owned_by_whoever_stopped_the_run() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let input = json!({ "page": handle(&page) });

    // The door's own race cuts the method.
    let (run, _guard) = harness.run()?;
    let pending = harness.dispatcher.submit(
        &run,
        Call::script("test.hang".to_owned(), input.clone(), None),
    );
    harness.hung.notified().await;
    run.stop(StopReason::CallerLeft);
    let cut = failure(pending.wait().await)?;
    assert_eq!(
        owner_and_code(&cut),
        (ErrorOwner::Host, ErrorCode::CANCELLED),
        "the caller left"
    );
    assert_eq!(
        cut.delivery,
        Delivery::Unknown,
        "it had sent, and never heard back"
    );

    // Teardown aborts the op's task before it sees the stop: its guard settles it on drop.
    let (run, _guard) = harness.run()?;
    let pump = harness
        .dispatcher
        .submit(&run, Call::script("test.pump".to_owned(), json!({}), None));
    let pending = harness.dispatcher.submit(
        &run,
        Call::script("test.hang".to_owned(), input.clone(), None),
    );
    harness.hung.notified().await;
    run.stop(StopReason::ScriptReturned);
    drop(run.close_ops());
    let _ = pump.wait().await;
    let aborted = failure(pending.wait().await)?;
    assert_eq!(
        owner_and_code(&aborted),
        (ErrorOwner::Script, ErrorCode::NOT_AWAITED),
        "the script returned without awaiting it"
    );
    assert_eq!(aborted.delivery, Delivery::Unknown);
    let OpsReport {
        effects,
        unknown,
        not_awaited,
    } = run.report();
    assert_eq!(effects, RunEffects::Unknown, "a sent write never confirmed");
    assert_eq!(unknown.len(), 1, "listed as unknown");
    let not_awaited: Vec<&str> = not_awaited.iter().map(|line| line.name).collect();
    assert_eq!(
        not_awaited,
        ["test.hang"],
        "listed as not awaited; the background pump is cut without a line"
    );
    let row = harness.row(aborted.operation).await?;
    assert_eq!(row.outcome, Some(Outcome::Cancelled));

    // Dropped with no stop at all: Neo lost it.
    let (run, _guard) = harness.run()?;
    let pending = harness
        .dispatcher
        .submit(&run, Call::script("test.hang".to_owned(), input, None));
    harness.hung.notified().await;
    drop(run.close_ops());
    let lost = failure(pending.wait().await)?;
    assert_eq!(
        owner_and_code(&lost),
        (ErrorOwner::Neo, ErrorCode::INTERNAL_ERROR)
    );
    Ok(())
}

#[tokio::test]
async fn unknown_and_internal_names_answer_alike_and_are_recorded() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    let unknown = failure(harness.submit(&run, "test.nope", json!({})).await.1)?;
    let hidden = failure(harness.submit(&run, "test.hidden", json!({})).await.1)?;
    for failure in [&unknown, &hidden] {
        assert_eq!(
            owner_and_code(failure),
            (ErrorOwner::Script, ErrorCode::METHOD_UNAVAILABLE)
        );
        assert_eq!(failure.delivery, Delivery::NotSent);
    }
    assert_eq!(
        unknown.error.message().replace("test.nope", "NAME"),
        hidden.error.message().replace("test.hidden", "NAME"),
        "a script cannot tell an internal method from a missing one"
    );
    for (failure, name) in [(&unknown, "test.nope"), (&hidden, "test.hidden")] {
        let row = harness.row(failure.operation).await?;
        assert_eq!(row.name, name, "recorded under the name the script used");
        assert_eq!(row.delivery, Some(Delivery::NotSent));
    }

    let from_neo = harness
        .dispatcher
        .invoke_child(&run, None, "test.hidden", json!({}))
        .await;
    assert_eq!(
        from_neo,
        Ok(json!({ "hidden": true })),
        "Neo's own Rust may call it"
    );
    Ok(())
}

#[tokio::test]
async fn invalid_input_names_the_field() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    let wrong_type = failure(
        harness
            .submit(&run, "test.typed", json!({ "outer": { "inner": "seven" } }))
            .await
            .1,
    )?;
    assert_eq!(
        owner_and_code(&wrong_type),
        (ErrorOwner::Script, ErrorCode::INVALID_INPUT)
    );
    assert!(
        wrong_type.error.message().contains("outer.inner"),
        "the path a model needs to fix its call: {}",
        wrong_type.error.message()
    );
    assert_eq!(wrong_type.delivery, Delivery::NotSent);

    let extra = failure(
        harness
            .submit(
                &run,
                "test.typed",
                json!({ "outer": { "inner": 7 }, "extra": true }),
            )
            .await
            .1,
    )?;
    assert!(
        extra.error.message().contains("extra"),
        "an unknown field is refused by name: {}",
        extra.error.message()
    );

    let (_, reply) = harness
        .submit(&run, "test.typed", json!({ "outer": { "inner": 7 } }))
        .await;
    assert_eq!(reply.outcome, Ok(json!({ "inner": 7 })));
    Ok(())
}

#[tokio::test]
async fn overlapping_calls_never_wait_on_each_other() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    // Each call returns only once all of them are inside their method at the same time, so
    // any serialization in the door would hang here.
    let pending: Vec<PendingReply> = (0..GATE_PARTIES)
        .map(|_| {
            harness
                .dispatcher
                .submit(&run, Call::script("test.gate".to_owned(), json!({}), None))
        })
        .collect();
    let replies = within(
        Duration::from_secs(10),
        future::join_all(pending.into_iter().map(PendingReply::wait)),
    )
    .await?;
    assert!(
        replies.iter().all(|reply| reply.outcome.is_ok()),
        "every call answered"
    );
    Ok(())
}

#[tokio::test]
async fn a_read_is_checked_again_before_its_result_is_released() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;

    let (_, reply) = harness
        .submit(&run, "test.takeover_read", json!({ "page": handle(&page) }))
        .await;
    assert!(
        harness.lock.reads.load(Ordering::SeqCst),
        "the read ran: entry let it in"
    );
    let failure = failure(reply)?;
    assert_eq!(
        owner_and_code(&failure),
        (ErrorOwner::Host, ErrorCode::OWNER_HAS_CONTROL),
        "the takeover began while it read, so its result is withheld"
    );
    assert_eq!(failure.delivery, Delivery::NotSent, "a read never sends");
    Ok(())
}

#[tokio::test]
async fn stale_and_closed_handles_name_their_owner() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;

    let stale = json!({ "page": { "$neo": "page", "id": page.id.0, "tab": page.tab.0 + 1000 } });
    let stale = failure(harness.submit(&run, "test.read", stale).await.1)?;
    assert_eq!(
        owner_and_code(&stale),
        (ErrorOwner::Script, ErrorCode::STALE_PAGE),
        "the page id now belongs to another tab, as after a Neo restart"
    );
    assert_eq!(stale.delivery, Delivery::NotSent);

    let (_, closed_it) = harness
        .submit(&run, "test.close", json!({ "page": handle(&page) }))
        .await;
    let fact = closed_it
        .pages
        .first()
        .ok_or("the closed page is reported")?;
    assert!(fact.closed, "the reply already knows the page is gone");
    let reused = failure(
        harness
            .submit(&run, "test.read", json!({ "page": handle(&page) }))
            .await
            .1,
    )?;
    assert_eq!(
        owner_and_code(&reused),
        (ErrorOwner::Script, ErrorCode::PAGE_CLOSED),
        "this script closed it, so using it again is the script's mistake"
    );

    let other = harness.page("https://example.com/other").await?;
    assert!(harness.fake.close_tab(other.tab), "the owner closes a tab");
    harness.until_forgotten(&other).await?;
    let gone = failure(
        harness
            .submit(&run, "test.read", json!({ "page": handle(&other) }))
            .await
            .1,
    )?;
    assert_eq!(
        owner_and_code(&gone),
        (ErrorOwner::Neo, ErrorCode::PAGE_CLOSED),
        "closed by someone else: not the script's doing"
    );
    Ok(())
}

#[tokio::test]
async fn a_navigated_page_is_refreshed_once() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;
    let before = harness.fake.count("Browser.getTabInfo");

    let input = json!({ "page": handle(&page), "url": "https://example.com/next" });
    let (operation, reply) = harness.submit(&run, "test.navigate", input).await;
    assert!(reply.outcome.is_ok());
    assert_eq!(
        harness.fake.count("Browser.getTabInfo") - before,
        1,
        "reported twice, refreshed once, and nothing else in the door asks the browser"
    );
    let fact = reply.pages.first().ok_or("the target's facts")?;
    assert_eq!(
        fact.url.as_deref(),
        Some("https://example.com/next"),
        "facts read after the refresh"
    );
    let row = harness.row(operation).await?;
    assert_eq!(
        row.page.url.as_deref(),
        Some("https://example.com/next"),
        "the trace's page_after is fresh too"
    );
    Ok(())
}

#[tokio::test]
async fn a_child_inherits_its_parents_viewport_lease() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;

    let submitted = harness.submit(&run, "test.lease", json!({ "page": handle(&page) }));
    let (_, reply) = within(Duration::from_secs(10), submitted).await?;
    assert_eq!(
        reply.outcome,
        Ok(json!({ "leased": true })),
        "the child ran under its parent's lease instead of queueing behind it"
    );
    Ok(())
}

#[tokio::test]
async fn concurrent_ops_on_the_lazy_page_open_one_tab() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, guard) = harness.run()?;
    let before = harness.fake.count("Browser.createTab");

    let pending: Vec<PendingReply> = (0..5)
        .map(|_| {
            let call = Call::script(
                "test.read".to_owned(),
                json!({ "page": { "$neo": "page" } }),
                None,
            );
            harness.dispatcher.submit(&run, call)
        })
        .collect();
    let operations: Vec<OperationId> = pending.iter().map(PendingReply::operation).collect();
    let replies = future::join_all(pending.into_iter().map(PendingReply::wait)).await;
    assert_eq!(
        harness.fake.count("Browser.createTab") - before,
        1,
        "single-flight"
    );
    let opened = replies
        .first()
        .and_then(|reply| reply.default_page.clone())
        .ok_or("the script learns its page")?;
    for reply in &replies {
        assert_eq!(
            reply.default_page.as_ref(),
            Some(&opened),
            "every op got the same page"
        );
        assert_eq!(reply.outcome, Ok(json!({ "tab": opened.tab.0 })));
    }
    assert!(
        harness
            .tabs
            .idle_agent_tabs(Duration::ZERO, Instant::now())
            .contains(&opened),
        "Neo opened it for the agent, so the janitor may close it later"
    );

    guard.finish(
        RunEnd {
            status: RunStatus::Ok,
            error: None,
            result: None,
            replay: None,
        },
        None,
    );
    for operation in operations {
        let row = harness.row(operation).await?;
        assert_eq!(
            row.delivery,
            Some(Delivery::NotSent),
            "the page opener is the run's own op, so its send is not the read's"
        );
    }
    let detail = harness
        .store
        .run(run.id())
        .await?
        .ok_or("the run is recorded")?;
    let opener = detail
        .operations
        .iter()
        .find(|row| row.name == "context.newPage")
        .ok_or("the opener is on the record")?;
    assert_eq!(opener.parent, None, "Neo's own op, under no script op");
    assert_eq!(opener.delivery, Some(Delivery::Done));
    Ok(())
}

#[tokio::test]
async fn the_rules_see_the_tab_the_door_checked() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;

    let (operation, reply) = harness
        .submit(&run, "test.write", json!({ "page": handle(&page) }))
        .await;
    assert!(reply.outcome.is_ok());
    let seen: Vec<SeenCheck> = harness
        .seen
        .all()
        .into_iter()
        .filter(|check| check.operation == operation)
        .collect();
    let expected = SeenCheck {
        method: "test.write",
        caller: Caller::Script,
        tab: Some(page.tab),
        operation,
    };
    assert_eq!(
        seen,
        vec![expected; 2],
        "at entry and at the recheck, on the tab the trace records (sdk-traces keys locks by it)"
    );

    let stale = json!({ "page": { "$neo": "page", "id": page.id.0, "tab": page.tab.0 + 1 } });
    let (refused, _) = harness.submit(&run, "test.write", stale).await;
    assert!(
        harness
            .seen
            .all()
            .iter()
            .all(|check| check.operation != refused),
        "a handle that fails its check never reaches a rule with the wrong tab"
    );
    Ok(())
}

#[tokio::test]
async fn known_secrets_are_masked_in_everything_a_script_sees() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;
    let _registration = harness.secrets.register_from(
        SecretValue::new("hunter2-token".to_owned()),
        None,
        SecretSource::Host,
    );

    let echo = json!({ "text": "the token is hunter2-token", "fail": false });
    let (_, value) = harness.submit(&run, "test.echo", echo).await;
    assert_eq!(
        value.outcome,
        Ok(json!({ "text": "the token is [private]" })),
        "in values"
    );

    let echo = json!({ "text": "hunter2-token", "fail": true });
    let failure = failure(harness.submit(&run, "test.echo", echo).await.1)?;
    assert_eq!(
        failure.error.message(),
        "the page said [private]",
        "in error messages"
    );

    let navigate = json!({ "page": handle(&page), "url": "https://example.com/?t=hunter2-token" });
    let (_, navigated) = harness.submit(&run, "test.navigate", navigate).await;
    let fact = navigated.pages.first().ok_or("the target's facts")?;
    assert_eq!(
        fact.url.as_deref(),
        Some("https://example.com/?t=[private]"),
        "in page facts"
    );
    Ok(())
}

/// Decision 35b. A value typed or swept a moment ago is held, and no secret yet: its field may
/// still be typed, and the process's set learns it once the field goes quiet. The script's own
/// `goto` could carry it out of Neo meanwhile, so a reply into the realm is masked with what the
/// run holds too, wherever the value is in it.
#[tokio::test]
async fn a_reply_into_the_realm_is_masked_with_what_the_run_holds() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;
    harness.held.hold("typed-pw-7Qx");

    let echo = json!({ "text": "the field holds typed-pw-7Qx", "fail": false });
    let (_, value) = harness.submit(&run, "test.echo", echo).await;
    assert_eq!(
        value.outcome,
        Ok(json!({ "text": "the field holds [private]" })),
        "in values"
    );

    let echo = json!({ "text": "typed-pw-7Qx", "fail": true });
    let failure = failure(harness.submit(&run, "test.echo", echo).await.1)?;
    assert_eq!(
        failure.error.message(),
        "the page said [private]",
        "in error messages"
    );

    let navigate = json!({ "page": handle(&page), "url": "https://example.com/?pw=typed-pw-7Qx" });
    let (_, navigated) = harness.submit(&run, "test.navigate", navigate).await;
    let fact = navigated.pages.first().ok_or("the target's facts")?;
    assert_eq!(
        fact.url.as_deref(),
        Some("https://example.com/?pw=[private]"),
        "in page facts"
    );
    Ok(())
}

/// Decision 35z. A number cannot hold `[private]`, and the realm's mask and the trace writer read
/// only strings, so a number whose text is a value the run holds leaves a main-world script's op
/// as that masked text, in its reply and in its record alike. The number next to the value stays
/// a number. A sealed op's numbers are Neo's own facts, such as a page brand's tab, and stay
/// numbers whatever they spell, or the brand would stop naming its tab.
#[tokio::test]
async fn a_number_that_spells_a_held_value_leaves_a_main_world_answer_as_its_masked_text()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    harness.held.hold("734921");

    let answer = json!({ "code": 734_921, "next": 734_922 });
    let (operation, reply) = harness.submit(&run, "test.script", answer).await;
    let masked = json!({ "code": "[private]", "next": 734_922 });
    assert_eq!(reply.outcome, Ok(masked.clone()), "into the realm");
    assert_eq!(
        harness.row(operation).await?.result,
        Some(masked),
        "in the op's record"
    );

    let opened = json!({ "page": { "$neo": "page", "id": 2, "tab": 1_734_921 } });
    let (operation, reply) = harness.submit(&run, "test.any", opened.clone()).await;
    assert_eq!(
        reply.outcome,
        Ok(opened.clone()),
        "a sealed op's tab reaches the realm as the number it is"
    );
    assert_eq!(
        harness.row(operation).await?.result,
        Some(opened),
        "and its record"
    );
    Ok(())
}

/// Decision 35b, for a run that lost a held value past the cap: nothing can mask that value out
/// of what the run's realm receives any more, so each reply from then on is `neo /
/// output_withheld`, and its page facts keep only Neo's own facts. The script still learns
/// whether its write landed, and the op's record keeps its real outcome.
#[tokio::test]
async fn a_lost_runs_replies_into_its_realm_are_withheld() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;
    harness.held.lose();

    let navigate = json!({ "page": handle(&page), "url": "https://example.com/next" });
    let (operation, navigated) = harness.submit(&run, "test.navigate", navigate).await;
    let fact = navigated.pages.first().ok_or("the target's facts")?;
    assert_eq!(
        (fact.tab, fact.url.as_deref(), fact.title.as_deref()),
        (page.tab, Some(WITHHELD), Some(WITHHELD)),
        "the page fact names the tab, and nothing the page showed"
    );
    let withheld = failure(navigated)?;
    assert_eq!(
        owner_and_code(&withheld),
        (ErrorOwner::Neo, ErrorCode::OUTPUT_WITHHELD),
        "the navigation went through, and its result is withheld"
    );
    assert_eq!(
        (withheld.operation, withheld.delivery, withheld.retryable),
        (operation, Delivery::Done, false),
        "with the op's id and real delivery, so the script knows the write landed"
    );
    let row = harness.row(operation).await?;
    assert_eq!(
        (row.outcome, row.delivery),
        (Some(Outcome::Ok), Some(Delivery::Done)),
        "the op's record keeps its real outcome"
    );

    let echo = json!({ "text": "typed-pw-7Qx", "fail": true });
    let withheld = failure(harness.submit(&run, "test.echo", echo).await.1)?;
    assert_eq!(
        (withheld.error.code(), withheld.delivery),
        (ErrorCode::OUTPUT_WITHHELD, Delivery::NotSent),
        "a failure is withheld too, as not sent when nothing was"
    );
    Ok(())
}

/// The trace writer masks an op's name with the process's set and what its run holds. A name the
/// script made up is its own text, though, and once its run lost a held value nothing masks that
/// value out of it, so the op's record withholds the name, as it withholds the start's input.
#[tokio::test]
async fn a_lost_runs_own_method_name_is_withheld_from_its_record() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    harness.held.lose();

    let (operation, reply) = harness.submit(&run, "typed-pw-7Qx", json!({})).await;
    assert_eq!(
        failure(reply)?.error.code(),
        ErrorCode::OUTPUT_WITHHELD,
        "the refusal reaches the realm withheld"
    );
    assert_eq!(
        harness.row(operation).await?.name,
        WITHHELD,
        "the record keeps none of the name"
    );
    Ok(())
}

/// A run can lose a held value while the door admits its op, between any two of the times the
/// door asks what the run holds. Wherever the loss falls, neither the op's record nor the refusal
/// the realm receives keeps a name the script made up raw.
#[tokio::test]
async fn a_run_that_turns_lost_as_its_own_method_name_is_admitted_records_none_of_it()
-> Result<(), Box<dyn Error>> {
    const NAME: &str = "typed-pw-7Qx";
    // How many times admitting and refusing such a name asks what the run holds.
    let asks = {
        let harness = Harness::new().await?;
        let (run, _guard) = harness.run()?;
        let before = harness.held.asks();
        harness.submit(&run, NAME, json!({})).await;
        harness.held.asks() - before
    };
    // Each of the op's asks, and one past them, where nothing is lost while the op goes.
    for ask in 1..=asks + 1 {
        let harness = Harness::new().await?;
        let (run, _guard) = harness.run()?;
        harness.held.hold(NAME);
        harness.held.lose_at_ask(ask);
        let (operation, reply) = harness.submit(&run, NAME, json!({})).await;
        let refusal = failure(reply)?;
        assert!(
            !refusal.error.message().contains(NAME),
            "lost at ask {ask} of {asks}: the realm receives the name masked or withheld: {}",
            refusal.error.message()
        );
        let row = harness.row(operation).await?;
        assert!(
            !format!("{row:?}").contains(NAME),
            "lost at ask {ask} of {asks}: the record keeps the name masked or withheld: {:?}",
            row.name
        );
    }
    Ok(())
}

/// FR/RC C4. A script's refusal brand is refused under the name it was sent with, whatever that
/// name is: a method the script may call never runs on it, and an unknown or Internal name answers
/// exactly as a registered one does, so the brand cannot probe for hidden methods. No rule runs
/// for it, and it is recorded as every admission refusal is.
#[tokio::test]
async fn a_refusal_brand_is_refused_under_its_name_and_never_runs() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let brand = refused("not served here");

    // A registered method, an Internal one and no method at all, with the spec each record keeps.
    for (name, effect) in [
        ("test.any", Effect::Read),
        ("test.hidden", Effect::Read),
        ("page.route", Effect::Write),
    ] {
        let (operation, reply) = harness.submit(&run, name, brand.clone()).await;
        let refusal = failure(reply)?;
        assert_eq!(
            (
                owner_and_code(&refusal),
                refusal.error.message(),
                refusal.delivery,
                refusal.retryable
            ),
            (
                (ErrorOwner::Script, ErrorCode::METHOD_UNAVAILABLE),
                "not served here",
                Delivery::NotSent,
                true
            ),
            "{name}: the brand's own code and reason, the same for every name"
        );
        assert!(
            harness
                .seen
                .all()
                .iter()
                .all(|check| check.operation != operation),
            "{name}: no rule ran for it"
        );
        let row = harness.row(operation).await?;
        assert_eq!(
            (row.name.as_str(), row.effect, row.tab, row.args.as_ref()),
            (name, effect, None, Some(&brand)),
            "recorded as sent, on no tab, under the spec the name has (unknown names are writes)"
        );
        assert_eq!(
            (row.outcome, row.delivery),
            (Some(Outcome::Error), Some(Delivery::NotSent))
        );
    }
    Ok(())
}

/// FR/RC C4, the spoof rule. A script can send the refusal brand itself through `__neo_invoke`, so
/// a brand claims only the two codes a script's own mistake carries, and its owner is always the
/// script, never Neo, the host or the website. Its other fields are recorded and otherwise ignored.
#[tokio::test]
async fn a_refusal_brand_cannot_claim_another_owner_or_code() -> Result<(), Box<dyn Error>> {
    const MALFORMED: &str =
        "`page.route`: a refusal may name only method_unavailable or invalid_input";
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    for brand in [
        json!({ "$neo": "refused", "code": "browser_disconnected", "reason": "page.route: x" }),
        json!({ "$neo": "refused", "code": 7, "reason": "page.route: x" }),
        json!({ "$neo": "refused", "code": "method_unavailable" }),
        json!({ "$neo": "refused", "code": "method_unavailable", "reason": ["page.route: x"] }),
    ] {
        let refusal = failure(harness.submit(&run, "page.route", brand.clone()).await.1)?;
        assert_eq!(
            (
                owner_and_code(&refusal),
                refusal.error.message(),
                refusal.delivery
            ),
            (
                (ErrorOwner::Script, ErrorCode::INVALID_INPUT),
                MALFORMED,
                Delivery::NotSent
            ),
            "{brand}: the script's own bad input"
        );
    }

    let spoofed = json!({
        "$neo": "refused", "code": "method_unavailable", "reason": "page.route: x",
        "owner": "neo", "delivery": "done", "retryable": false,
    });
    let (operation, reply) = harness.submit(&run, "page.route", spoofed.clone()).await;
    let refusal = failure(reply)?;
    assert_eq!(
        (
            owner_and_code(&refusal),
            refusal.delivery,
            refusal.retryable
        ),
        (
            (ErrorOwner::Script, ErrorCode::METHOD_UNAVAILABLE),
            Delivery::NotSent,
            true
        ),
        "the door decides the owner and the delivery, whatever the brand says"
    );
    let row = harness.row(operation).await?;
    assert_eq!(
        row.args,
        Some(spoofed),
        "every field the brand carried is on the record"
    );
    assert_eq!(
        row.error.map(|error| (error.owner, error.code)),
        Some((ErrorOwner::Script, "method_unavailable".to_owned()))
    );

    let option = json!({
        "$neo": "refused", "code": "invalid_input",
        "reason": "page.screenshot: path is not available in Neo",
    });
    let refusal = failure(harness.submit(&run, "page.screenshot", option).await.1)?;
    assert_eq!(
        (owner_and_code(&refusal), refusal.error.message()),
        (
            (ErrorOwner::Script, ErrorCode::INVALID_INPUT),
            "page.screenshot: path is not available in Neo"
        ),
        "the other code a refusal may name, with its reason"
    );
    Ok(())
}

/// FR/RC C4 and graft G3. A refusal's reason is the script's own text, and can hold a value Neo
/// knows: a process secret, or one its run holds, such as a password the script typed itself. The
/// door masks it with both before cutting it to `TEXT_CHARS`: a cut through such a value would
/// leave a prefix that no later mask recognizes, in the run's error and the reply the host reads.
/// A run that lost a held value withholds the refusal, as it withholds every reply.
#[tokio::test]
async fn a_refusal_reason_is_masked_with_what_its_run_holds_then_clipped()
-> Result<(), Box<dyn Error>> {
    const SECRET: &str = "hunter2-token-0123";
    const HELD: &str = "typed-pw-7Qx-0123";
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let _registration = harness.secrets.register_from(
        SecretValue::new(SECRET.to_owned()),
        None,
        SecretSource::Host,
    );
    harness.held.hold(HELD);

    for value in [SECRET, HELD] {
        let whole = refused(&format!("page.route: {value}"));
        let refusal = failure(harness.submit(&run, "page.route", whole).await.1)?;
        assert_eq!(refusal.error.message(), "page.route: [private]");

        // The cut keeps TEXT_CHARS characters, so it falls inside the value.
        let reason = format!("{}{value}", "x".repeat(TEXT_CHARS - 6));
        let refusal = failure(harness.submit(&run, "page.route", refused(&reason)).await.1)?;
        let message = refusal.error.message();
        assert!(
            !message.contains(value.get(..4).unwrap_or(value)),
            "no part of {value} survives the cut: …{}",
            message
                .get(message.len().saturating_sub(40)..)
                .unwrap_or_default()
        );
        assert!(
            message.chars().count() == TEXT_CHARS + 1 && message.ends_with('…'),
            "cut to TEXT_CHARS, and the cut marked"
        );
    }

    harness.held.lose();
    let (operation, reply) = harness
        .submit(&run, "page.route", refused(&format!("page.route: {HELD}")))
        .await;
    let withheld = failure(reply)?;
    assert_eq!(
        (owner_and_code(&withheld), withheld.delivery),
        (
            (ErrorOwner::Neo, ErrorCode::OUTPUT_WITHHELD),
            Delivery::NotSent
        ),
        "nothing masks a lost value any more, so the realm gets no reason"
    );
    let row = harness.row(operation).await?;
    assert!(
        !format!("{row:?}").contains(HELD),
        "nor does the record: {row:?}"
    );
    Ok(())
}

/// FR/RC C4 is honoured for the script only. Neo's own Rust never sends a refusal brand, so one in
/// a Neo call's input, such as script data a method passes on to a child op, is input like any
/// other: it reaches the method, where the same brand from the script is refused.
#[tokio::test]
async fn a_neo_callers_refusal_brand_is_ordinary_input() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let brand = refused("not served here");

    let from_neo = harness
        .dispatcher
        .invoke_child(&run, None, "test.any", brand.clone())
        .await;
    assert_eq!(
        from_neo,
        Ok(brand.clone()),
        "the method ran, with the brand as its input"
    );
    let from_script = failure(harness.submit(&run, "test.any", brand).await.1)?;
    assert_eq!(
        owner_and_code(&from_script),
        (ErrorOwner::Script, ErrorCode::METHOD_UNAVAILABLE),
        "the same brand from the script is refused"
    );
    Ok(())
}

#[tokio::test]
async fn the_trace_redacts_secret_fields_and_sensitive_ones_when_marked()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;

    let fill = |value: &str, sensitive: bool| json!({ "page": handle(&page), "value": value, "secret": "s3cret", "sensitive": sensitive });
    let (ordinary, _) = harness
        .submit(&run, "test.fill", fill("alice", false))
        .await;
    let (sensitive, _) = harness
        .submit(&run, "test.fill", fill("hunter2", true))
        .await;

    let row = harness.row(ordinary).await?;
    assert_eq!(
        row.args,
        Some(
            json!({ "page": handle(&page), "value": "alice", "secret": "[redacted]", "sensitive": false })
        ),
        "a secret field is always redacted; an ordinary field's value is kept"
    );
    let row = harness.row(sensitive).await?;
    assert_eq!(
        row.args,
        Some(
            json!({ "page": handle(&page), "value": "[redacted]", "secret": "[redacted]", "sensitive": true })
        ),
        "a sensitive target redacts the method's sensitive fields too"
    );
    Ok(())
}

#[tokio::test]
async fn a_tab_slot_belongs_to_its_tab() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let first = harness.page("https://example.com/one").await?;
    let second = harness.page("https://example.com/two").await?;
    let (run, _guard) = harness.run()?;

    let (operation, reply) = harness
        .submit(&run, "test.count", json!({ "page": handle(&first) }))
        .await;
    assert_eq!(
        reply.outcome,
        Ok(json!({ "count": 1, "operation": operation.to_string() })),
        "a method's op id is the one its caller and the trace see"
    );
    let (_, again) = harness
        .submit(&run, "test.count", json!({ "page": handle(&first) }))
        .await;
    let (_, other) = harness
        .submit(&run, "test.count", json!({ "page": handle(&second) }))
        .await;
    let count = |reply: &OpReply| {
        reply
            .outcome
            .as_ref()
            .ok()
            .and_then(|value| value.get("count").cloned())
    };
    assert_eq!(count(&again), Some(json!(2)), "the slot outlives one op");
    assert_eq!(count(&other), Some(json!(1)), "each tab has its own");
    Ok(())
}

#[tokio::test]
async fn a_method_sees_its_runs_budget() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    let (_, reply) = harness.submit(&run, "test.budget", json!({})).await;
    let budget = reply.outcome.map_err(|failure| failure.error.to_string())?;
    let left = budget
        .get("secondsLeft")
        .and_then(Value::as_u64)
        .ok_or("seconds left")?;
    assert!(
        (290..=300).contains(&left),
        "the run's 300 s deadline, less a moment: {left}"
    );
    assert_eq!(
        budget.get("cancelled"),
        Some(&json!(false)),
        "a running op's token is live"
    );
    Ok(())
}

#[tokio::test]
async fn a_runs_slot_is_shared_by_its_ops_and_goes_when_its_record_ends()
-> Result<(), Box<dyn Error>> {
    #[derive(Default)]
    struct Feature;

    let harness = Harness::new().await?;
    let (run, guard) = harness.run()?;
    let first = run.slot::<Feature>();
    assert!(
        Arc::ptr_eq(&first, &run.slot::<Feature>()),
        "one value per run and type"
    );
    let (other, other_guard) = harness.run()?;
    assert!(
        !Arc::ptr_eq(&first, &other.slot::<Feature>()),
        "every run has its own"
    );
    let kept = Arc::downgrade(&first);
    drop(first);
    guard.finish(
        RunEnd {
            status: RunStatus::Ok,
            error: None,
            result: None,
            replay: None,
        },
        None,
    );
    assert!(
        kept.upgrade().is_none(),
        "the record's end drops it, while the run itself is still held"
    );
    let late = Arc::downgrade(&run.slot::<Feature>());
    assert!(
        late.upgrade().is_none(),
        "after the end, nothing keeps a new one"
    );

    let crashed = Arc::downgrade(&other.slot::<Feature>());
    drop(other_guard);
    assert!(
        crashed.upgrade().is_none(),
        "a run that ended without finishing drops it too"
    );
    Ok(())
}
