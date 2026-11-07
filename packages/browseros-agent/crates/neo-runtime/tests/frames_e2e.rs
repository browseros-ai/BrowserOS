//! Frame routing on a real browser: visible effects distinguish each document and each run.

#[expect(
    dead_code,
    reason = "shared browser fixture includes helpers used by other suites"
)]
#[path = "support/browser.rs"]
mod browser;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::json;

const CONTROLS: &str = r#"
<title>Frame form</title>
<style>body{margin:8px}iframe{display:block;width:600px;height:480px;border:7px solid #888;padding:5px}button{width:120px;height:36px}input{display:block}</style>
<label>Email<input id=email value=seed></label>
<button id=go onclick="document.querySelector('#result').textContent='clicked'">Go</button>
<p id=result>idle</p><input type=checkbox id=accept>
<select id=choice><option value=one>One</option><option value=two>Two</option></select>
<ul><li>Alpha</li><li>Beta</li></ul>
"#;
const HOP: &str = " >> internal:control=enter-frame >> ";

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let top = format!(
        r"{CONTROLS}<iframe id=same name=same src=/same.html></iframe><iframe id=sibling src=/leaf.html></iframe><iframe id=cross name=cross></iframe>
<script>cross.src = 'http://localhost:' + location.port + '/cross.html'</script>"
    );
    let same = format!("{CONTROLS}<iframe id=nested src=/leaf.html></iframe>");
    let cross = format!(
        r"{CONTROLS}<iframe id=nested name=nested></iframe><iframe id=local src=/leaf.html></iframe>
<script>nested.src = 'http://127.0.0.1:' + location.port + '/leaf.html'</script>"
    );
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![
                    ("/top.html".into(), top),
                    ("/same.html".into(), same),
                    ("/cross.html".into(), cross),
                    ("/leaf.html".into(), CONTROLS.into()),
                ],
                ..Fixture::default()
            },
            "/top.html",
        )
        .await,
    )
}

async fn run(tab: &Tab, source: &str) -> RunOutcome {
    tab.start_run(source.into())
        .await
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"))
}

fn assert_ok(outcome: &RunOutcome) {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
}

async fn exercise(tab: &Tab, scope: &str) {
    let prefix = serde_json::to_string(&format!("{scope}{HOP}"))
        .unwrap_or_else(|error| panic!("selector: {error}"));
    let outcome = run(tab, &format!(r#"
        const inFrame = selector => page.locator({prefix} + selector);
        const email = inFrame('internal:label="Email"i');
        await email.fill('alice@example.com');
        await email.press('End');
        await email.press('!');
        const value = await email.inputValue();
        await inFrame('#go').click();
        await inFrame('#accept').check();
        const checked = await inFrame('#accept').isChecked();
        await inFrame('#accept').uncheck();
        await inFrame('#accept').setChecked(true);
        const options = await inFrame('#choice').selectOption('two');
        await inFrame('#email').waitFor({{state:'visible'}});
        return {{ value, text: await inFrame('#result').textContent(),
            inner: await inFrame('#result').innerText(), attr: await inFrame('#email').getAttribute('id'),
            checked, options, visible: await email.isVisible(), count: await inFrame('li').count(),
            texts: await inFrame('li').allTextContents() }};
    "#)).await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(
            json!({"value":"alice@example.com!", "text":"clicked", "inner":"clicked", "attr":"email", "checked":true, "options":["two"], "visible":true,"count":2,"texts":["Alpha","Beta"]})
        ),
        "{outcome:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_origin_and_nested_local_frames_use_every_existing_operation() {
    let Some(tab) = open().await else { return };
    exercise(&tab, "#same").await;
    exercise(&tab, &format!("#same{HOP}#nested")).await;
    let untouched = tab.evaluate("[document.querySelector('#email').value, document.querySelector('#sibling').contentDocument.querySelector('#email').value]").await;
    tab.close().await;
    assert_eq!(untouched, json!(["seed", "seed"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_iframe_owners_make_their_descendants_hidden() {
    let Some(tab) = open().await else { return };
    tab.evaluate("document.querySelector('#same').style.display='none'")
        .await;
    let outcome = run(&tab, r"
        const input = page.locator('#same >> internal:control=enter-frame >> #email');
        const nested = page.locator('#same >> internal:control=enter-frame >> #nested >> internal:control=enter-frame >> #email');
        await input.waitFor({state:'hidden'});
        let failed;
        try { await input.waitFor({state:'visible',timeout:150}); } catch (error) { failed = error.name; }
        return {visible: await input.isVisible(), nestedVisible: await nested.isVisible(), count: await input.count(), failed};
    ").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!({"visible":false,"nestedVisible":false,"count":1,"failed":"TimeoutError"}))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hop_errors_are_errors_and_missing_iframes_wait() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, r"
        const errors = [];
        for (const selector of ['iframe >> internal:control=enter-frame >> #email', '#email >> internal:control=enter-frame >> #email', '#absent >> internal:control=enter-frame >> #email']) {
            try { await page.locator(selector).fill('wrong', {timeout:200}); } catch (error) { errors.push(error.name); }
        }
        return errors;
    ").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!(["Error", "Error", "TimeoutError"]))
    );
}
