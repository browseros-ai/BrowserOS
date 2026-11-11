//! The door end to end: the real dispatcher, the fake browser, and a real trace store read back
//! from a temp dir. One test per promise of skeleton D7 and the grafts on it.

mod harness;
mod test_methods;

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::future;
use neo_trace::{Delivery, Effect, ErrorOwner, OperationId, Outcome, RunEffects, RunStatus};
use serde_json::{Value, json};
use tokio::time::Instant;

use self::harness::{GATE_PARTIES, Harness, SeenCheck, within};
use crate::browser::PageRef;
use crate::dispatch::{
    Call, Caller, ErrorCode, OpFailure, OpReply, OpsReport, PendingReply, RunEnd, StopReason,
    TEXT_CHARS,
};

/// The page brand a script sends for `page`.
fn handle(page: &PageRef) -> Value {
    json!({ "$neo": "page", "tab": page.tab.0 })
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
    assert!(
        reply.observed.is_none(),
        "unwinding has no completed observation"
    );
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

    let legacy = json!({ "page": { "$neo": "page", "id": 7, "tab": page.tab.0 } });
    let refused = failure(harness.submit(&run, "test.read", legacy).await.1)?;
    assert_eq!(
        owner_and_code(&refused),
        (ErrorOwner::Script, ErrorCode::INVALID_INPUT)
    );
    assert_eq!(refused.delivery, Delivery::NotSent);

    let stale = json!({ "page": { "$neo": "page", "tab": page.tab.0 + 1000 } });
    let stale = failure(harness.submit(&run, "test.read", stale).await.1)?;
    assert_eq!(
        owner_and_code(&stale),
        (ErrorOwner::Neo, ErrorCode::PAGE_CLOSED),
        "an unknown tab is a closed page, never a replacement target"
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
                json!({ "page": { "$neo": "page", "tab": null } }),
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

    guard.finish(RunEnd {
        status: RunStatus::Ok,
        error: None,
        result: None,
        replay: None,
    });
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
        vec![expected; 3],
        "at entry, recheck and fact release, on the tab the trace records"
    );

    let stale = json!({ "page": { "$neo": "page", "tab": page.tab.0 + 1 } });
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

/// FR/RC C4 and risk 1 of the privacy cut. A refusal's reason is the script's own text: the door
/// passes it through whole, and cuts a long one to `TEXT_CHARS`, so a script cannot make a
/// refusal as large as it likes.
#[tokio::test]
async fn a_refusal_reason_is_clipped_to_text_chars() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    let whole = refused("page.route: not served here");
    let refusal = failure(harness.submit(&run, "page.route", whole).await.1)?;
    assert_eq!(refusal.error.message(), "page.route: not served here");

    let reason = "x".repeat(TEXT_CHARS + 10);
    let refusal = failure(harness.submit(&run, "page.route", refused(&reason)).await.1)?;
    let message = refusal.error.message();
    assert_eq!(
        message,
        format!("{}…", "x".repeat(TEXT_CHARS)),
        "cut to TEXT_CHARS, and the cut marked"
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
    guard.finish(RunEnd {
        status: RunStatus::Ok,
        error: None,
        result: None,
        replay: None,
    });
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

#[tokio::test]
async fn a_tab_readmitted_after_an_excluded_url_gets_fresh_feature_state()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/one").await?;
    let (run, _guard) = harness.run()?;
    let (_, first) = harness
        .submit(&run, "test.count", json!({ "page": handle(&page) }))
        .await;
    assert_eq!(
        first
            .outcome
            .as_ref()
            .ok()
            .and_then(|value| value.get("count")),
        Some(&json!(1))
    );
    assert!(harness.fake.navigate_tab(
        page.tab,
        "chrome-extension://internal/page.html",
        "Internal"
    ));
    run.view().list_pages().await?;
    assert!(
        !harness.tabs.is_open(page.tab),
        "core's prune hook drops the tab's slots"
    );
    assert!(
        harness
            .fake
            .navigate_tab(page.tab, "https://example.com/returned", "Returned")
    );
    run.view().list_pages().await?;
    let (_, returned) = harness
        .submit(&run, "test.count", json!({ "page": handle(&page) }))
        .await;
    assert_eq!(
        returned
            .outcome
            .as_ref()
            .ok()
            .and_then(|value| value.get("count")),
        Some(&json!(1)),
        "fresh admission recreates slots for the same live tab"
    );
    assert!(harness.tabs.is_open(page.tab));
    Ok(())
}

#[tokio::test]
async fn fact_replies_carry_increasing_observations_and_empty_replies_carry_none()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.page("https://example.com/").await?;
    let (run, _guard) = harness.run()?;
    let input = json!({ "page": handle(&page) });
    let (_, first) = harness.submit(&run, "test.read", input.clone()).await;
    let (_, second) = harness.submit(&run, "test.read", input).await;
    assert_eq!(first.pages.len(), 1);
    assert_eq!(second.pages.len(), 1);
    let first = first.observed.ok_or("first facts are stamped")?;
    let second = second.observed.ok_or("second facts are stamped")?;
    assert!(second.seq > first.seq);
    let (_, empty) = harness
        .submit(&run, "test.typed", json!({ "outer": { "inner": 1 } }))
        .await;
    assert!(empty.pages.is_empty());
    assert!(empty.observed.is_none());
    Ok(())
}
