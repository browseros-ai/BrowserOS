//! Compiles the claw-sdk adapter (`adapter/playwright.rs`) against a mock of sdk-skeleton's
//! documented rev-3 types, and checks what it registers and how it maps the crate's errors.

#[path = "support/mock_skeleton.rs"]
mod mock_skeleton;
#[path = "support/offline_browser.rs"]
mod offline_browser;

// The adapter names `crate::dispatch`, as it will inside apps/claw-sdk.
use mock_skeleton::{dispatch, run};

#[path = "../adapter/playwright.rs"]
mod adapter;

use std::error::Error;
use std::ptr;
use std::sync::Arc;

use browseros_core::PageId;
use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
use browseros_playwright::{
    ErrorCode, Op, OpEnv, OpLifetime, OpTarget, Playwright, PlaywrightError, PlaywrightHooks,
    Refusal, RunState, TypedBy,
};
use browseros_trace::ErrorOwner;
use dispatch::{
    Answer, Lifetime, Listing, OpCtx, OperationError, Registry, RegistryBuilder, Target, TimedOut,
    Visibility,
};
use offline_browser::{offline_session, offline_view};
use run::RunCtx;
use serde_json::{Value, json};

struct NoHooks;

impl PlaywrightHooks for NoHooks {
    fn field_entered(&self, _page: PageId, _entry: JournalEntry, _by: TypedBy) {}
    fn values_swept(&self, _page: PageId, _values: &FieldValues) {}
    fn capture_allowed(&self, _page: PageId) -> Result<(), Refusal> {
        Ok(())
    }
}

/// The registry claw-sdk would build at startup, with every Playwright op in it.
fn registry() -> Result<Registry, Box<dyn Error>> {
    let playwright = Playwright::new(offline_session(), Arc::new(NoHooks));
    let mut builder = RegistryBuilder::default();
    adapter::register(&mut builder, &playwright)?;
    Ok(builder.build())
}

#[tokio::test]
async fn tests_that_every_op_registers_once_with_its_declared_spec() -> Result<(), Box<dyn Error>> {
    let registry = registry()?;
    assert_eq!(
        registry.script_methods().count(),
        Op::all().count(),
        "one script method per op, and no other"
    );
    for op in Op::all() {
        let spec = op.spec();
        let method = registry
            .method(spec.name)
            .ok_or(format!("{} is not registered", spec.name))?;
        let target = match spec.target {
            OpTarget::None => Target::None,
            OpTarget::Field(field) => Target::Field(field),
        };
        let lifetime = match spec.lifetime {
            OpLifetime::Operation => Lifetime::Operation,
            OpLifetime::Background => Lifetime::Background,
        };
        assert_eq!(method.doc, spec.doc, "{} doc", spec.name);
        assert_eq!(method.effect, spec.effect, "{} effect", spec.name);
        assert_eq!(method.target, target, "{} target field", spec.name);
        assert_eq!(method.lifetime, lifetime, "{} lifetime", spec.name);
        assert_eq!(
            method.visibility,
            Visibility::Script,
            "{} is callable from scripts",
            spec.name
        );
        assert_eq!(
            method.listing,
            Listing::Summarized,
            "{} is listed as Playwright",
            spec.name
        );
        assert_eq!(
            (method.input.json(), method.output.json()),
            (&json!({ "type": "object" }), &json!({})),
            "the crate's typed decode is the one input gate"
        );
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_a_second_registration_is_refused() -> Result<(), Box<dyn Error>> {
    let playwright = Playwright::new(offline_session(), Arc::new(NoHooks));
    let mut builder = RegistryBuilder::default();
    adapter::register(&mut builder, &playwright)?;
    let again = adapter::register(&mut builder, &playwright);
    assert!(
        again.is_err(),
        "the registry reports a duplicate instead of keeping two methods"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_fill_redacts_its_value_for_a_sensitive_target() -> Result<(), Box<dyn Error>> {
    let registry = registry()?;
    let fill = registry.method("locator.fill").ok_or("locator.fill")?;
    assert!(
        fill.redaction.secret.is_empty(),
        "nothing is redacted unless the target is sensitive"
    );
    let mut input = json!({ "target": { "$neo": "locator", "page": 7 }, "value": "hunter2" });
    for pointer in &fill.redaction.sensitive {
        pointer.replace_in(&mut input, &Value::from("[redacted]"));
    }
    assert_eq!(
        input,
        json!({ "target": { "$neo": "locator", "page": 7 }, "value": "[redacted]" }),
        "a fill into a password field never stores the typed value"
    );
    let click = registry.method("locator.click").ok_or("locator.click")?;
    assert!(
        click.redaction.sensitive.is_empty(),
        "a click types nothing"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_the_event_pump_is_the_only_background_method() -> Result<(), Box<dyn Error>> {
    let registry = registry()?;
    let pump = registry.method("events.next").ok_or("events.next")?;
    assert_eq!(
        pump.lifetime,
        Lifetime::Background,
        "the host cancels it quietly at run end"
    );
    let background: Vec<&str> = Op::all()
        .map(|op| op.spec().name)
        .filter(|name| {
            registry
                .method(name)
                .is_some_and(|method| method.lifetime == Lifetime::Background)
        })
        .collect();
    assert_eq!(background, ["events.next"]);
    Ok(())
}

#[test]
fn tests_that_a_timeout_keeps_its_owner_and_reads_as_timed_out() {
    let cases = [
        (
            ErrorCode::NotActionable,
            ErrorOwner::Website,
            "not_actionable",
        ),
        (ErrorCode::NoMatch, ErrorOwner::Script, "no_match"),
        (ErrorCode::WaitTimeout, ErrorOwner::Website, "wait_timeout"),
    ];
    for (code, owner, name) in cases {
        let message = "locator.click: Timeout 5000ms exceeded.\nCall log:\n  - waiting for x\n";
        let leaf = adapter::leaf_error(&PlaywrightError::new(code, message.to_owned()));
        assert_eq!(leaf.error.owner, owner, "{name} keeps its owner");
        assert_eq!(leaf.error.code.as_str(), name);
        assert_eq!(
            leaf.error.timed_out,
            TimedOut::Yes,
            "{name}: the bootstrap throws TimeoutError"
        );
        assert_eq!(
            leaf.answer,
            Answer::Ambiguous,
            "{name}: a timed-out effect may have landed"
        );
        assert_eq!(
            leaf.error.message, message,
            "the call log reaches the script"
        );
        assert_eq!(leaf.error.host_code, None);
    }
}

#[test]
fn tests_that_a_definite_failure_maps_one_to_one() {
    let leaf = adapter::leaf_error(&PlaywrightError::new(
        ErrorCode::StrictModeViolation,
        "locator.click: Error: strict mode violation".to_owned(),
    ));
    assert_eq!(
        (
            leaf.error.owner,
            leaf.error.code.as_str(),
            leaf.error.timed_out,
            leaf.answer
        ),
        (
            ErrorOwner::Script,
            "strict_mode_violation",
            TimedOut::No,
            Answer::Definite
        )
    );
}

#[test]
fn tests_that_a_refusal_keeps_its_owner_code_and_host_code() {
    let leaf = adapter::leaf_error(&PlaywrightError::from(Refusal {
        owner: ErrorOwner::Host,
        code: "host_error".to_owned(),
        message: "the host refused".to_owned(),
        host_code: Some("takeover".to_owned()),
    }));
    assert_eq!(leaf.error.owner, ErrorOwner::Host);
    assert_eq!(leaf.error.code.as_str(), "host_error");
    assert_eq!(leaf.error.host_code.as_deref(), Some("takeover"));
    assert_eq!(leaf.error.message, "the host refused");
    assert_eq!(leaf.error.timed_out, TimedOut::No);
    assert_eq!(
        leaf.answer,
        Answer::Definite,
        "nothing was sent before a refusal"
    );
}

#[tokio::test]
async fn tests_that_the_env_reaches_the_op_context() -> Result<(), Box<dyn Error>> {
    let run = RunCtx::new(offline_view());
    let ctx = OpCtx::refusing(
        Arc::clone(&run),
        OperationError {
            owner: ErrorOwner::Host,
            code: dispatch::ErrorCode::from("owner_has_control".to_owned()),
            message: "The owner is using this tab.".to_owned(),
            host_code: None,
            timed_out: TimedOut::No,
        },
    );
    let env = adapter::OpCtxEnv::new(&ctx);
    assert_eq!(
        env.recheck(),
        Err(Refusal {
            owner: ErrorOwner::Host,
            code: "owner_has_control".to_owned(),
            message: "The owner is using this tab.".to_owned(),
            host_code: None,
        }),
        "a policy refusal reaches the crate unchanged"
    );
    assert!(!ctx.was_sent(), "a refused recheck marks nothing");
    env.mark_sent();
    env.mark_sensitive_target();
    env.note("waiting for getByRole('button')".to_owned());
    assert!(ctx.was_sent() && ctx.target_was_sensitive());
    assert_eq!(ctx.notes(), ["waiting for getByRole('button')"]);
    assert!(
        Arc::ptr_eq(env.run_state(), &run.slot::<RunState>()),
        "every op of a run shares the run's one RunState"
    );
    assert!(
        ptr::eq(env.view(), run.view()),
        "ops admit pages through the run's view"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_registered_handler_runs_its_op() -> Result<(), Box<dyn Error>> {
    let registry = registry()?;
    let click = registry.method("locator.click").ok_or("locator.click")?;
    let ctx = OpCtx::new(RunCtx::new(offline_view()));

    let not_built = click
        .handler
        .call(
            &ctx,
            json!({ "call": { "api": "page.click", "timeoutMs": 5000 } }),
        )
        .await
        .err()
        .ok_or("locator.click is not built yet")?;
    assert_eq!(
        (
            not_built.error.owner,
            not_built.error.code.as_str(),
            not_built.answer
        ),
        (ErrorOwner::Neo, "internal", Answer::Definite)
    );
    assert_eq!(not_built.error.message, "page.click: not built yet");

    let bad = click
        .handler
        .call(&ctx, json!({ "call": { "timeout": 5000 } }))
        .await
        .err()
        .ok_or("an unknown header field must fail")?;
    assert_eq!(
        (
            bad.error.owner,
            bad.error.code.as_str(),
            bad.error.timed_out
        ),
        (ErrorOwner::Script, "invalid_input", TimedOut::No)
    );
    assert!(!ctx.was_sent(), "neither call reached the page");
    Ok(())
}
