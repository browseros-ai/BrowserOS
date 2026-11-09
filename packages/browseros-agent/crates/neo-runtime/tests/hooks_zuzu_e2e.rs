//! Neo: execute the documented hook file against a real login page; verify effects over CDP.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the shared browser harness serves other suites too"
)]
mod browser;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{
    AfterHookResult, BeforeHookResult, CallOrigin, HostCallback, PageId, RunHooks, RunOutcome,
    RunRequest, RunStatus,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// Neo: the shipped example itself, not a test-only policy copy.
const HOOKS: &str = include_str!("../examples/hooks/zuzu-auth.js");
/// Neo: a fixture secret distinctive enough to find anywhere in the serialized outcome.
const PASSWORD: &str = "vault-zuzu-password-7e29";
/// Neo: leaves time for browser startup and hook host exchanges on a shared machine.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Neo: deliberately mirrors the secret into accessible text and an attribute to test read redaction.
const LOGIN: &str = r"<!doctype html><title>Login</title>
<form id=login><label>Username<input id=user></label>
<label>Password<input id=pw type=password></label>
<button id=submit type=submit><span>Sign in</span></button></form>
<button id=pay type=button>Pay</button><p id=mirror></p>
<script>
window.submits = 0;
document.querySelector('#login').addEventListener('submit', e => {
  e.preventDefault(); window.submits += 1;
});
document.querySelector('#pay').onclick = () => { window.submits += 1; };
document.querySelector('#pw').addEventListener('input', e => {
  e.target.setAttribute('value', e.target.value);
  document.querySelector('#mirror').textContent = e.target.value;
});
</script>";

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                slow_response: Duration::ZERO,
                pages: vec![("/login".to_string(), LOGIN.to_string())],
            },
            "/login",
        )
        .await,
    )
}

async fn run(tab: &Tab, code: &str, approved: bool, wrong_origin: bool) -> RunOutcome {
    let origin = tab.evaluate("location.origin").await;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let host: HostCallback = {
        let requests = Arc::clone(&requests);
        Arc::new(move |request| {
            requests
                .lock()
                .unwrap_or_else(|error| panic!("fixture request ledger poisoned: {error}"))
                .push(request);
            Box::pin(async { Ok(json!({"username": "alice", "password": PASSWORD})) })
        })
    };
    let expected_origin = if wrong_origin {
        json!("https://wrong.example")
    } else {
        origin.clone()
    };
    let outcome = tab
        .runtime()
        .await
        .run(RunRequest {
            code: code.to_string(),
            page: PageId::new(tab.target_id.clone()),
            timeout: TIMEOUT,
            cancel: CancellationToken::new(),
            artifacts: None,
            hooks: Some(
                RunHooks::new(HOOKS)
                    .with_meta(json!({"origin": expected_origin, "ownerApproved": approved}))
                    .with_host(host),
            ),
        })
        .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert!(
        !serde_json::to_string(&outcome)
            .unwrap_or_else(|error| panic!("outcome must serialize: {error}"))
            .contains(PASSWORD),
        "secret in outcome"
    );
    for request in requests
        .lock()
        .unwrap_or_else(|error| panic!("fixture request ledger poisoned: {error}"))
        .iter()
    {
        assert_eq!(
            request.name, "vault.login",
            "only the fixture vault is exposed"
        );
        assert_eq!(
            request.args,
            json!({"origin": origin}),
            "the vault receives the verified origin"
        );
    }
    outcome
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_example_swaps_the_password_and_redacts_real_reads() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, r"
        await page.locator('#user').fill('{{username}}');
        await page.locator('#pw').fill('{{password}}');
        return {
          input: await page.locator('#pw').inputValue(),
          snapshot: await page.locator('body').ariaSnapshot(),
          text: await page.locator('#mirror').textContent(),
          inner: await page.locator('#mirror').innerText(),
          attribute: await page.locator('#pw').getAttribute('value'),
          evaluated: await page.evaluate(() => ({ nested: [document.querySelector('#pw').value, 7, null] })),
        };
    ", false, false).await;
    let value = outcome.value.as_ref().unwrap();
    for key in ["input", "text", "inner", "attribute"] {
        assert_eq!(value[key], "•••", "{key}");
    }
    assert!(value["snapshot"].as_str().unwrap().contains("•••"));
    assert_eq!(value["evaluated"], json!({"nested": ["•••", 7, null]}));
    assert_eq!(
        tab.evaluate(
            "[document.querySelector('#user').value, document.querySelector('#pw').value]"
        )
        .await,
        json!(["alice", PASSWORD])
    );
    let fills: Vec<_> = outcome
        .calls
        .iter()
        .filter(|call| call.origin == CallOrigin::Agent && call.method == "fill")
        .collect();
    assert_eq!(fills.len(), 2);
    assert_eq!(fills[1].params["value"], "{{password}}");
    assert!(fills.iter().all(|call| call.hook.unwrap().before
        == Some(BeforeHookResult::ArgsReplaced)
        && call.log.is_empty()));
    assert!(outcome.calls.iter().any(|call| {
        call.hook
            .is_some_and(|hook| hook.after == Some(AfterHookResult::Replaced))
    }));
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_example_blocks_a_password_in_a_text_box_or_on_another_site() {
    let Some(tab) = open().await else { return };
    for (selector, wrong_origin) in [("#user", false), ("#pw", true)] {
        let code = format!(
            "try {{ await page.locator('{selector}').fill('{{{{password}}}}'); return 'ran'; }} catch {{ return 'blocked'; }}"
        );
        let outcome = run(&tab, &code, false, wrong_origin).await;
        assert_eq!(outcome.value, Some(json!("blocked")));
        assert!(outcome.calls.iter().any(|call| {
            call.hook
                .is_some_and(|hook| hook.before == Some(BeforeHookResult::Blocked))
        }));
        assert_eq!(
            tab.evaluate(
                "[document.querySelector('#user').value, document.querySelector('#pw').value]"
            )
            .await,
            json!(["", ""])
        );
    }
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_example_requires_owner_approval_for_submit_pay_and_enter() {
    let Some(tab) = open().await else { return };
    for approved in [false, true] {
        let outcome = run(
            &tab,
            r"
          const results = [];
          for (const act of [
            () => page.locator('#submit span').click(),
            () => page.locator('#pay').click(),
            () => page.locator('#pw').press('Enter'),
          ]) {
            try { await act(); results.push('ran'); }
            catch (error) {
              if (!error.message.includes('Owner approval required')) throw error;
              results.push('blocked');
            }
          }
          return results;
        ",
            approved,
            false,
        )
        .await;
        assert_eq!(
            outcome.value,
            Some(json!(vec![if approved { "ran" } else { "blocked" }; 3]))
        );
        assert_eq!(
            tab.evaluate("window.submits").await,
            json!(if approved { 3 } else { 0 })
        );
    }
    tab.close().await;
}
