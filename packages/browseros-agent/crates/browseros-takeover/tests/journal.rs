//! The control journal over core's scripted connection: what arming sends and disarming takes
//! back, which binding calls reach the secret set and when, and frames that attach during
//! control. The clock is paused, so a test moves past the journal's quiet period without waiting
//! for it.

use browseros_core::sensitive_fields::{JOURNAL_BINDING, SEALED_WORLD, seal_source};
use browseros_core::testing::{TestCall, TestConnection, cdp_pending};
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection, SessionId, TabId};
use browseros_takeover::{BrowserTabs, Journal, PartError};
use browseros_trace::{SecretSet, SecretSetConfig, SecretSource, SecretValue};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type TestResult = Result<(), BoxError>;

const ADD_SCRIPT: &str = "Page.addScriptToEvaluateOnNewDocument";
const REMOVE_SCRIPT: &str = "Page.removeScriptToEvaluateOnNewDocument";

/// Decision 32: what is held is registered once this long passes with no entry.
const QUIET: Duration = Duration::from_secs(1);

/// More events than the scripted connection's broadcast holds for a reader that is behind.
const FLOOD: usize = 80;

fn tab(id: i64) -> Value {
    json!({
        "tabId": id,
        "targetId": format!("target-{id}"),
        "windowId": 1,
        "url": format!("https://example.test/{id}"),
        "title": format!("Tab {id}"),
        "isActive": true,
        "isLoading": false,
        "loadProgress": 1.0,
        "isPinned": false,
        "isHidden": false,
    })
}

/// A browser with tab 3 open: one frame, nothing else in it. New-document scripts get the
/// identifiers in `scripts`, in order.
fn browser(scripts: &[&str]) -> Result<Arc<TestConnection>, BoxError> {
    let connection = TestConnection::new(
        scripts
            .iter()
            .map(|id| (ADD_SCRIPT, json!({ "identifier": id }))),
    );
    connection.always("Browser.getTabs", json!({ "tabs": [tab(3)] }))?;
    for method in [
        "Page.enable",
        "DOM.enable",
        "Runtime.enable",
        "Accessibility.enable",
        "Runtime.runIfWaitingForDebugger",
        "Target.setAutoAttach",
        "Runtime.addBinding",
        REMOVE_SCRIPT,
    ] {
        connection.always(method, json!({}))?;
    }
    connection.always(
        "Page.getFrameTree",
        json!({ "frameTree": { "frame": { "id": "main" } } }),
    )?;
    connection.always(
        "DOM.getDocument",
        json!({ "root": { "nodeName": "#document" } }),
    )?;
    connection.always(
        "Page.createIsolatedWorld",
        json!({ "executionContextId": 11 }),
    )?;
    connection.always(
        "Runtime.evaluate",
        json!({ "result": { "type": "number", "value": 1 } }),
    )?;
    Ok(connection)
}

struct Harness {
    connection: Arc<TestConnection>,
    journal: Journal,
    secrets: Arc<SecretSet>,
}

fn harness(scripts: &[&str]) -> Result<Harness, BoxError> {
    harness_with(scripts, SecretSetConfig::default())
}

/// [`harness`] over a secret set with `limits`.
fn harness_with(scripts: &[&str], limits: SecretSetConfig) -> Result<Harness, BoxError> {
    let connection = browser(scripts)?;
    let tabs = BrowserTabs::new(
        BrowserSession::new(connection.clone(), BrowserSessionHooks::default()),
        Arc::new(Viewports::default()),
    );
    let secrets = Arc::new(SecretSet::new(limits));
    Ok(Harness {
        connection,
        journal: Journal::new(tabs, Arc::clone(&secrets)),
        secrets,
    })
}

impl Harness {
    fn calls(&self, method: &str) -> Result<Vec<TestCall>, BoxError> {
        Ok(self
            .connection
            .calls()?
            .into_iter()
            .filter(|call| call.method == method)
            .collect())
    }

    /// The session the page's own frame tree was read on: core's page session.
    fn page_session(&self) -> Result<SessionId, BoxError> {
        self.calls("Page.getFrameTree")?
            .first()
            .and_then(|call| call.session.clone())
            .ok_or_else(|| "the page was never walked".into())
    }

    /// What the seal's binding sends when a sensitive field gets input: its whole value, and
    /// whether the owner's key, paste or drop entered it (`trusted`).
    fn input(&self, session: &SessionId, binding: &str, value: &str, trusted: bool) {
        let payload = json!({ "kind": "password", "value": value, "trusted": trusted }).to_string();
        self.connection.emit(
            "Runtime.bindingCalled",
            json!({ "name": binding, "payload": payload, "executionContextId": 11 }),
            Some(session.clone()),
        );
    }

    /// The owner's keys put `value` in a sensitive field: a trusted entry.
    fn type_into(&self, session: &SessionId, binding: &str, value: &str) {
        self.input(session, binding, value, true);
    }

    /// A page's script puts each of `values` in a sensitive field and dispatches `input` itself:
    /// an untrusted entry each.
    fn script_enters(&self, session: &SessionId, values: &[&str]) {
        for value in values {
            self.input(session, JOURNAL_BINDING, value, false);
        }
    }

    /// `values`, one input event each.
    fn send(&self, session: &SessionId, values: &[&str]) {
        for value in values {
            self.type_into(session, JOURNAL_BINDING, value);
        }
    }

    /// `value` typed key by key into an empty field.
    fn type_keys(&self, session: &SessionId, value: &str) {
        self.send(session, &keys(value));
    }

    /// Floods the connection with events the journal ignores, so its listener falls behind.
    fn flood(&self) {
        for _ in 0..FLOOD {
            self.connection.emit("Test.filler", json!({}), None);
        }
    }

    /// The browser attaches the frame target `child` under `parent`'s session.
    fn attach(&self, parent: &SessionId, child: &str, kind: &str) {
        self.connection.emit(
            "Target.attachedToTarget",
            json!({
                "sessionId": child,
                "targetInfo": { "targetId": format!("{child}-target"), "type": kind },
                "waitingForDebugger": false,
            }),
            Some(parent.clone()),
        );
    }

    /// `count` distinct values, typed into one field after another at once: more than the
    /// scripted broadcast holds, which binding calls never pass through.
    fn type_many_fields(&self, session: &SessionId, count: usize) -> Vec<String> {
        let values = (0..count)
            .map(|i| format!("MANYFIELDS_{i:03}"))
            .collect::<Vec<_>>();
        self.send(
            session,
            &values.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        values
    }

    /// The values among `typed` that the secret set does not mask.
    fn unregistered<'a>(&self, typed: &'a [String]) -> Vec<&'a str> {
        typed
            .iter()
            .map(String::as_str)
            .filter(|value| !self.masks(value))
            .collect()
    }

    fn masks(&self, value: &str) -> bool {
        let mut text = format!("value: {value}.");
        self.secrets.mask_str(&mut text)
    }

    /// `text` as the secret set masks it.
    fn masked(&self, text: &str) -> String {
        let mut text = text.to_owned();
        self.secrets.mask_str(&mut text);
        text
    }

    /// Waits up to 5 s for `value` to be masked.
    async fn registered(&self, value: &str) -> TestResult {
        eventually(|| Ok(self.masks(value))).await
    }
}

/// The field's value after each key, as typing `value` sends it.
fn keys(value: &str) -> Vec<&str> {
    value
        .char_indices()
        .map(|(start, key)| &value[..start + key.len_utf8()])
        .collect()
}

/// Lets the listener read everything sent so far, on a clock that stays inside the quiet period.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(10)).await;
}

async fn eventually(mut check: impl FnMut() -> Result<bool, BoxError>) -> TestResult {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !check()? {
        if tokio::time::Instant::now() > deadline {
            return Err("timed out waiting".into());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

fn on(calls: &[TestCall], session: &SessionId) -> Vec<String> {
    calls
        .iter()
        .filter(|call| call.session.as_ref() == Some(session))
        .map(|call| call.method.clone())
        .collect()
}

#[tokio::test(start_paused = true)]
async fn arming_adds_the_binding_and_a_new_document_script_and_disarm_removes_only_the_script()
-> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    let binding = h.calls("Runtime.addBinding")?;
    assert!(
        binding
            .iter()
            .any(|call| call.session.as_ref() == Some(&page)
                && call.params
                    == json!({ "name": JOURNAL_BINDING, "executionContextName": SEALED_WORLD }))
    );
    let scripts = h.calls(ADD_SCRIPT)?;
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].session.as_ref(), Some(&page));
    assert_eq!(
        scripts[0].params,
        json!({ "source": seal_source(), "worldName": SEALED_WORLD, "runImmediately": true })
    );
    let order = on(&h.connection.calls()?, &page);
    let position = |method: &str| order.iter().position(|called| called == method);
    assert!(
        position("Runtime.enable") < position("Runtime.addBinding")
            && position("Runtime.addBinding") < position(ADD_SCRIPT),
        "Runtime is on before the binding, and the binding is in before the seal runs: {order:?}"
    );
    assert!(h.calls(REMOVE_SCRIPT)?.is_empty());

    armed.disarm().await;
    let removed = h.calls(REMOVE_SCRIPT)?;
    assert_eq!(removed.len(), 1, "disarm returns once the script is gone");
    assert_eq!(removed[0].session.as_ref(), Some(&page));
    assert_eq!(removed[0].params, json!({ "identifier": "page-script" }));
    assert!(
        h.calls("Runtime.removeBinding")?.is_empty(),
        "the binding stays"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn dropping_the_armed_journal_registers_what_it_was_sent_and_removes_its_script() -> TestResult
{
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_keys(&page, "DROPPED_6d0e");
    drop(armed);
    settle().await;
    assert_eq!(
        h.masked("DROPPED_6d0e"),
        "[private]",
        "registered in the background, well inside the quiet period"
    );
    assert_eq!(h.secrets.count(), 1, "the final value, not its prefixes");
    let removed = h.calls(REMOVE_SCRIPT)?;
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].params, json!({ "identifier": "page-script" }));
    Ok(())
}

/// sdk-playwright takes `Runtime.bindingCalled` on the connection's targeted channel for as long
/// as the process runs, which keeps every binding call off the broadcast. What the owner types
/// still reaches the journal, and sdk-playwright still gets every call.
#[tokio::test(start_paused = true)]
async fn typing_reaches_the_journal_while_sdk_playwright_also_takes_binding_calls() -> TestResult {
    let h = harness(&["page-script"])?;
    let mut playwright = h.connection.events_targeted("Runtime.bindingCalled");
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    let typed = "OWNERTYPED_9d4e";
    h.type_keys(&page, typed);
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(h.masked(typed), "[private]");
    assert_eq!(h.secrets.count(), 1, "the final value, not its prefixes");
    assert_eq!(
        std::iter::from_fn(|| playwright.try_recv().ok()).count(),
        keys(typed).len(),
        "sdk-playwright still got every call"
    );
    armed.disarm().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_value_typed_on_an_armed_session_is_registered_and_a_repeat_refreshes_it() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_into(&page, JOURNAL_BINDING, "hunter2pass");
    h.registered("hunter2pass").await?;
    h.type_into(&page, JOURNAL_BINDING, "hunter2pass");
    h.type_into(&page, JOURNAL_BINDING, "482913");
    h.registered("482913").await?;
    assert_eq!(h.secrets.count(), 2, "the repeat refreshed its entry");
    armed.disarm().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn typing_registers_the_final_value_and_none_of_its_prefixes() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.send(&page, &["h", "ht", "htt", "http", "https1234"]);
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(
        h.masked("see https://example.com, typed https1234"),
        "see https://example.com, typed [private]",
        "http was a prefix on the way, not a secret"
    );
    assert_eq!(h.secrets.count(), 1);
    armed.disarm().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_backspace_chain_registers_only_the_final_value() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_keys(&page, "hunter2pass");
    h.send(&page, &["hunter2pas", "hunter2pa", "hunter2p", "hunter2"]);
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(
        h.masked("hunter2pass hunter2pa hunter2"),
        "[private]pass [private]pa [private]",
        "only what the field was left holding"
    );
    assert_eq!(h.secrets.count(), 1);
    armed.disarm().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn two_fields_typed_in_turn_both_register() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_keys(&page, "hunter2pass");
    h.type_keys(&page, "482913");
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(
        h.masked("hunter2pass then 482913"),
        "[private] then [private]"
    );
    assert_eq!(h.secrets.count(), 2);
    armed.disarm().await;
    Ok(())
}

/// The first keys of the next field are shorter than the value before them, as a backspace
/// would be; taking them for one would erase a value nothing had registered.
#[tokio::test(start_paused = true)]
async fn a_field_that_starts_like_the_one_before_it_does_not_replace_it() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_keys(&page, "pass2024");
    h.type_keys(&page, "pass2025");
    h.type_keys(&page, "4242424242424242");
    h.type_keys(&page, "4249");
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(
        h.masked("current pass2024, new pass2025, card 4242424242424242, code 4249"),
        "current [private], new [private], card [private], code [private]"
    );
    assert_eq!(h.secrets.count(), 4);
    armed.disarm().await;
    Ok(())
}

/// A page can raise `input` events on its own fields, so what is held is capped at 64 values.
#[tokio::test(start_paused = true)]
async fn past_64_held_values_the_oldest_is_registered_at_once() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    let values = (0..=64)
        .map(|i| format!("field-{i:02}"))
        .collect::<Vec<_>>();
    h.script_enters(
        &page,
        &values.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    settle().await;
    assert_eq!(h.masked("field-00 field-01"), "[private] field-01");
    assert_eq!(h.secrets.count(), 1);
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(h.secrets.count(), 65);
    armed.disarm().await;
    Ok(())
}

/// sdk-traces decision 53: what the owner typed stays masked however many values a page's
/// sweeps report after it, past the secret set's cap.
#[tokio::test(start_paused = true)]
async fn what_the_owner_typed_outlasts_a_flood_of_page_values() -> TestResult {
    let h = harness_with(
        &["page-script"],
        SecretSetConfig {
            max_entries: 4,
            ..SecretSetConfig::default()
        },
    )?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_keys(&page, "OWNERTYPED_9k2x");
    h.registered("OWNERTYPED_9k2x").await?;
    for i in 0..8 {
        let _registration = h
            .secrets
            .register(SecretValue::new(format!("page-flood-{i}")), None);
    }
    assert!(
        h.masks("OWNERTYPED_9k2x"),
        "the page's values pushed the owner's out"
    );
    armed.disarm().await;
    Ok(())
}

/// Decision 53 lets owner values pass the set's cap because people type at human speed. A
/// page's script can raise `input` events at any rate, so what it enters is a page value.
#[tokio::test(start_paused = true)]
async fn what_a_page_script_entered_is_a_page_value_and_what_the_owner_typed_is_the_owners()
-> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.script_enters(&page, &["SCRIPTSET_7q2w"]);
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(h.masked("SCRIPTSET_7q2w"), "[private]");
    assert_eq!(
        (
            h.secrets.count_from(SecretSource::Owner),
            h.secrets.count_from(SecretSource::Page)
        ),
        (0, 1),
        "the script's value is the page's (owner, page)"
    );

    h.type_keys(&page, "OWNERTYPED_3m8v");
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(h.masked("OWNERTYPED_3m8v"), "[private]");
    assert_eq!(
        (
            h.secrets.count_from(SecretSource::Owner),
            h.secrets.count_from(SecretSource::Page)
        ),
        (1, 1),
        "what the owner typed is the owner's (owner, page)"
    );
    armed.disarm().await;
    Ok(())
}

/// A page's script cannot push out what the owner typed. An untrusted entry never replaces a
/// trusted one the journal holds, neither as a longer value nor as one a character shorter,
/// which the journal would otherwise read as a backspace.
#[tokio::test(start_paused = true)]
async fn a_page_script_never_displaces_what_the_owner_typed() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_keys(&page, "OWNERLONG_4k");
    h.script_enters(&page, &["OWNERLONG_4kx"]);
    h.type_keys(&page, "OWNERDEL_7q");
    h.script_enters(&page, &["OWNERDEL_7"]);
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(
        h.masked("OWNERLONG_4k OWNERDEL_7q"),
        "[private] [private]",
        "what the owner typed is masked whole"
    );
    let counts = (
        h.secrets.count_from(SecretSource::Owner),
        h.secrets.count_from(SecretSource::Page),
    );
    assert_eq!(counts, (2, 2), "(owner, page)");
    armed.disarm().await;
    Ok(())
}

/// A page's script replaying the owner's longer value does not clear the owner's shorter one.
/// The replay is a repeat of a trusted value, so it is the owner's, but its own event is
/// untrusted, so it displaces nothing the owner typed.
#[tokio::test(start_paused = true)]
async fn a_page_scripts_replay_of_one_owner_value_keeps_the_others() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_into(&page, JOURNAL_BINDING, "OWNERPASS_EXT9");
    h.type_into(&page, JOURNAL_BINDING, "OWNERPASS");
    h.script_enters(&page, &["OWNERPASS_EXT9"]);
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(
        h.masked("OWNERPASS and OWNERPASS_EXT9"),
        "[private] and [private]",
        "both of the owner's values are masked"
    );
    assert_eq!(
        h.secrets.count_from(SecretSource::Owner),
        2,
        "both are the owner's"
    );
    armed.disarm().await;
    Ok(())
}

/// A value entered both ways, such as one the owner typed that a page's script then copies into
/// another field, is the owner's, whichever came first.
#[tokio::test(start_paused = true)]
async fn a_value_both_the_owner_and_a_page_script_entered_stays_the_owners() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_into(&page, JOURNAL_BINDING, "TYPEDFIRST_5k1p");
    h.script_enters(&page, &["TYPEDFIRST_5k1p", "SCRIPTFIRST_8d3r"]);
    h.type_into(&page, JOURNAL_BINDING, "SCRIPTFIRST_8d3r");
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(
        h.masked("TYPEDFIRST_5k1p SCRIPTFIRST_8d3r"),
        "[private] [private]"
    );
    assert_eq!(
        (
            h.secrets.count_from(SecretSource::Owner),
            h.secrets.count_from(SecretSource::Page)
        ),
        (2, 0),
        "(owner, page)"
    );
    armed.disarm().await;
    Ok(())
}

/// The gap sdk-traces decision 53 found. A page that floods `input` events during control fills
/// only the page tier, which never passes the set's cap.
#[tokio::test(start_paused = true)]
async fn a_page_flooding_input_events_grows_no_owner_value_and_the_set_no_further_than_its_cap()
-> TestResult {
    let h = harness_with(
        &["page-script"],
        SecretSetConfig {
            max_entries: 4,
            ..SecretSetConfig::default()
        },
    )?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_keys(&page, "OWNERTYPED_6w4n");
    h.registered("OWNERTYPED_6w4n").await?;
    let flood = (0..100)
        .map(|i| format!("PAGEFLOOD_{i:03}"))
        .collect::<Vec<_>>();
    h.script_enters(&page, &flood.iter().map(String::as_str).collect::<Vec<_>>());
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(
        h.secrets.count_from(SecretSource::Owner),
        1,
        "the flood registered owner values"
    );
    assert_eq!(h.secrets.count(), 4, "the set grew past its cap");
    assert_eq!(
        h.masked("OWNERTYPED_6w4n PAGEFLOOD_099 PAGEFLOOD_000"),
        "[private] [private] PAGEFLOOD_000",
        "the owner's value and the newest page value stay masked, and the oldest page value went"
    );
    armed.disarm().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn nothing_is_registered_until_a_second_passes_with_no_entry() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_keys(&page, "slowtyper1");
    tokio::time::sleep(QUIET - Duration::from_millis(100)).await;
    h.send(&page, &["slowtyper12"]);
    tokio::time::sleep(QUIET - Duration::from_millis(1)).await;
    assert_eq!(
        h.secrets.count(),
        0,
        "each entry starts the second again, so nearly two seconds of typing registered nothing"
    );
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert_eq!(h.masked("slowtyper12"), "[private]");
    assert_eq!(h.secrets.count(), 1);
    armed.disarm().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn disarm_registers_what_the_journal_holds_before_it_removes_its_scripts() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;
    // The removal never answers, so disarm is held there for the frame budget.
    h.connection.always(REMOVE_SCRIPT, cdp_pending())?;

    let typed = keys("LASTKEYSTROKE_5");
    let (read, queued) = typed.split_at(7);
    h.send(&page, read);
    settle().await;
    h.send(&page, queued);
    let disarmed = tokio::spawn(armed.disarm());
    settle().await;
    assert!(
        h.calls(REMOVE_SCRIPT)?.len() == 1 && !disarmed.is_finished(),
        "disarm is still removing its script"
    );
    assert_eq!(
        h.masked("LASTKEYSTROKE_5"),
        "[private]",
        "registered before the scripts go, and inside the quiet period"
    );
    assert_eq!(
        h.secrets.count(),
        1,
        "the keys the listener read and those still queued at disarm make one value"
    );
    disarmed.await?;
    Ok(())
}

/// A runtime that shuts down drops the listener's task instead of stopping it.
#[test]
fn a_runtime_shutting_down_still_registers_what_the_listener_read() -> TestResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()?;
    let (h, armed) = runtime.block_on(async {
        let h = harness(&["page-script"])?;
        let armed = h.journal.arm(&[TabId(3)]).await;
        h.type_keys(&h.page_session()?, "SHUTDOWN_2a7f");
        settle().await;
        Ok::<_, BoxError>((h, armed))
    })?;
    assert_eq!(h.secrets.count(), 0, "still inside the quiet period");

    drop(runtime);
    assert_eq!(h.masked("SHUTDOWN_2a7f"), "[private]");
    assert_eq!(h.secrets.count(), 1);
    drop(armed);
    Ok(())
}

/// A lag on the broadcast can lose an iframe's attach, so the tabs are swept. It loses no
/// entry, since binding calls never pass through the broadcast, so what the journal holds waits
/// out its quiet period: registering it at the lag could register a prefix.
#[tokio::test(start_paused = true)]
async fn a_lag_sweeps_the_tabs_and_leaves_what_is_held_to_its_quiet_period() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;
    // The catch-up sweep's seal never answers, so the sweep runs for its frame budget.
    h.connection.always("Runtime.evaluate", cdp_pending())?;

    h.type_keys(&page, "BEFORELAG_7c1d");
    settle().await;
    let walks = h.calls("Page.getFrameTree")?.len();
    h.flood();
    settle().await;
    assert!(
        h.calls("Page.getFrameTree")?.len() > walks,
        "the lag's sweep walked the tab"
    );
    assert_eq!(
        h.secrets.count(),
        0,
        "nothing registered at the lag, inside the quiet period"
    );
    tokio::time::sleep(QUIET).await;
    assert_eq!(h.masked("BEFORELAG_7c1d"), "[private]");
    assert_eq!(h.secrets.count(), 1, "the final value, not its prefixes");
    armed.disarm().await;
    Ok(())
}

/// Browser work that stalls must never keep the listener from reading: an entry would wait
/// behind it, and the broadcast drops the frame attaches a reader that is behind has not read.
#[tokio::test(start_paused = true)]
async fn a_frame_that_arms_slowly_holds_up_no_entry() -> TestResult {
    let h = harness(&["page-script", "frame-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;
    // The frame's first arming step never answers, so its arming runs for the frame budget.
    h.connection.always("Page.enable", cdp_pending())?;
    h.attach(&page, "oopif-7", "iframe");
    settle().await;

    let typed = h.type_many_fields(&page, FLOOD + 20);
    tokio::time::sleep(QUIET + Duration::from_millis(100)).await;
    assert_eq!(
        h.calls(ADD_SCRIPT)?.len(),
        1,
        "the frame is still being armed"
    );
    assert_eq!(h.unregistered(&typed), Vec::<&str>::new());
    armed.disarm().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_slow_catch_up_sweep_holds_up_no_entry() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;
    // The catch-up sweep's seal never answers, so the sweep runs for its frame budget.
    h.connection.always("Runtime.evaluate", cdp_pending())?;
    h.flood();
    settle().await;

    let typed = h.type_many_fields(&page, FLOOD + 20);
    tokio::time::sleep(QUIET + Duration::from_millis(100)).await;
    assert_eq!(h.unregistered(&typed), Vec::<&str>::new());
    armed.disarm().await;
    Ok(())
}

/// Decision 33: an iframe that attaches under an armed session is armed, and so is one nested
/// in it, even when both attach before either is armed.
#[tokio::test(start_paused = true)]
async fn a_frame_nested_in_a_frame_that_just_attached_is_armed_too() -> TestResult {
    let h = harness(&["page-script", "frame-script", "nested-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.attach(&page, "oopif-7", "iframe");
    h.attach(&SessionId::from("oopif-7"), "oopif-8", "iframe");
    eventually(|| Ok(h.calls(ADD_SCRIPT)?.len() == 3)).await?;
    let nested = SessionId::from("oopif-8");
    h.type_into(&nested, JOURNAL_BINDING, "NESTEDFRAME_5c2e");
    h.registered("NESTEDFRAME_5c2e").await?;
    armed.disarm().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn another_binding_or_a_session_it_did_not_arm_registers_nothing() -> TestResult {
    let h = harness(&["page-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;

    h.type_into(&page, "__someoneElse", "OTHERNAME_1");
    h.type_into(
        &SessionId::from("not-armed"),
        JOURNAL_BINDING,
        "OTHERSESSION_2",
    );
    h.type_into(&page, JOURNAL_BINDING, "ARMED_3");
    h.registered("ARMED_3").await?;
    assert!(!h.masks("OTHERNAME_1"));
    assert!(!h.masks("OTHERSESSION_2"));
    assert_eq!(h.secrets.count(), 1);

    armed.disarm().await;
    h.type_into(&page, JOURNAL_BINDING, "AFTERWARDS_4");
    tokio::time::sleep(QUIET * 2).await;
    assert!(
        !h.masks("AFTERWARDS_4"),
        "a disarmed journal registers nothing"
    );
    Ok(())
}

/// Binding calls and frame detaches reach the journal on different channels. A value the owner
/// typed into a frame just before it detached is still taken from an armed session.
#[tokio::test(start_paused = true)]
async fn a_value_typed_just_before_its_frame_detached_is_registered() -> TestResult {
    let h = harness(&["page-script", "frame-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;
    h.attach(&page, "oopif-7", "iframe");
    eventually(|| Ok(h.calls(ADD_SCRIPT)?.len() == 2)).await?;

    h.type_into(
        &SessionId::from("oopif-7"),
        JOURNAL_BINDING,
        "LASTINFRAME_3b8a",
    );
    h.connection.emit(
        "Target.detachedFromTarget",
        json!({ "sessionId": "oopif-7" }),
        Some(page),
    );
    tokio::time::sleep(QUIET * 2).await;
    assert_eq!(h.masked("LASTINFRAME_3b8a"), "[private]");
    armed.disarm().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_frame_that_attaches_under_an_armed_session_is_armed_and_journals() -> TestResult {
    let h = harness(&["page-script", "frame-script"])?;
    let armed = h.journal.arm(&[TabId(3)]).await;
    let page = h.page_session()?;
    let frame = SessionId::from("oopif-7");

    h.attach(&page, "worker-1", "worker");
    h.attach(&SessionId::from("not-armed"), "stranger-2", "iframe");
    h.attach(&page, "oopif-7", "iframe");
    eventually(|| Ok(h.calls(ADD_SCRIPT)?.len() == 2)).await?;
    assert_eq!(
        on(&h.connection.calls()?, &frame),
        [
            "Page.enable",
            "Runtime.enable",
            "Runtime.addBinding",
            ADD_SCRIPT
        ],
        "the frame's session needs both domains: core enables neither there"
    );
    for other in ["worker-1", "stranger-2"] {
        assert!(on(&h.connection.calls()?, &SessionId::from(other)).is_empty());
    }

    h.type_into(&frame, JOURNAL_BINDING, "CARDFRAME_4111");
    h.registered("CARDFRAME_4111").await?;

    armed.disarm().await;
    let removed = h
        .calls(REMOVE_SCRIPT)?
        .into_iter()
        .map(|call| (call.session, call.params["identifier"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(removed.len(), 2);
    assert!(removed.contains(&(Some(page), json!("page-script"))));
    assert!(removed.contains(&(Some(frame), json!("frame-script"))));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_tab_armed_again_gets_one_script_and_an_unknown_tab_is_skipped() -> TestResult {
    let h = harness(&["page-script", "second-script"])?;
    let armed = h.journal.arm(&[TabId(9), TabId(3), TabId(3)]).await;
    armed.arm_tab(TabId(3)).await?;
    assert_eq!(h.calls(ADD_SCRIPT)?.len(), 1, "each session is armed once");
    assert!(matches!(
        armed.arm_tab(TabId(9)).await,
        Err(PartError::TabNotFound(TabId(9)))
    ));

    armed.disarm().await;
    assert_eq!(h.calls(REMOVE_SCRIPT)?.len(), 1);
    Ok(())
}
