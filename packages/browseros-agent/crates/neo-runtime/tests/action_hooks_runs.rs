//! Action hooks through `Runtime::run` with a real hook file, against the fake browser in
//! `support/fake_browser.rs`: a filter that names no action label fails the hooks, so the run ends
//! blocked; the run's hook sandbox decides the agent's actions. No browser needed;
//! `action_hooks_e2e.rs` covers the effects on a real one.

#[path = "support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "the page's interceptor and click readers serve click_lifecycle.rs"
)]
mod fake_browser;

use std::time::Duration;

use fake_browser::{FakeBrowser, Replies};
use neo_runtime::{BeforeHookResult, RunHooks, RunOutcome, RunRequest, RunStatus};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const TIMEOUT: Duration = Duration::from_secs(10);

async fn run(code: &str, hooks: &str) -> RunOutcome {
    let browser = FakeBrowser::start(Replies::default()).await;
    let request = RunRequest {
        hooks: Some(RunHooks::new(hooks)),
        ..fake_browser::run_request(code, TIMEOUT, CancellationToken::new())
    };
    browser.runtime().await.run(request).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_filter_naming_no_action_label_blocks_the_run_before_its_script() {
    let script = "console.log('ran'); return await page.title()";
    for filter in [
        "{ groups: ['pagescript'] }",
        "{ kinds: ['evaluat'] }",
        "{ kinds: ['goto'], groups: ['navigate', 'Read'] }",
    ] {
        let hooks = format!("neo.on('beforeAction', {filter}, () => ({{ block: 'no' }}))");
        let outcome = run(script, &hooks).await;
        assert_eq!(outcome.status, RunStatus::Blocked, "{filter}: {outcome:?}");
        assert!(outcome.logs.is_empty(), "{filter}: the script never ran");
        assert_eq!(outcome.value, None);
        let misspelled = filter
            .split('\'')
            .find(|name| ["pagescript", "evaluat", "Read"].contains(name))
            .unwrap();
        assert!(
            outcome
                .hook_trace
                .iter()
                .any(|line| line.contains(misspelled)),
            "{filter}: the host reads which name: {:?}",
            outcome.hook_trace
        );
    }
    let outcome = run(
        script,
        "neo.on('afterAction', { kinds: ['goto'], groups: ['pageScript', 'read'] }, () => {})",
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.hook_trace);
    assert_eq!(outcome.value, Some(json!("Fake page")));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_runs_sandbox_decides_the_agents_actions() {
    let outcome = run(
        "try { await page.title(); return 'ran' } catch (error) { return error.message }",
        "neo.on('beforeAction', { kinds: ['title'] }, () => ({ block: 'no titles here' }))\n\
         neo.on('afterAction', () => ({ note: 'saw an action' }))",
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.hook_trace);
    let message = outcome
        .value
        .as_ref()
        .and_then(|value| value.as_str())
        .unwrap();
    assert!(message.contains("no titles here"), "{message}");
    let title = outcome
        .calls
        .iter()
        .find(|call| call.method == "title")
        .unwrap();
    assert_eq!(
        title.hook.and_then(|hook| hook.before),
        Some(BeforeHookResult::Blocked)
    );
    assert!(
        outcome.notes.is_empty(),
        "a blocked action never ran: {:?}",
        outcome.notes
    );

    let outcome = run(
        "return await page.title()",
        "neo.on('afterAction', () => ({ note: 'saw an action' }))",
    )
    .await;
    assert_eq!(outcome.value, Some(json!("Fake page")));
    assert_eq!(outcome.notes, ["saw an action"]);
}
