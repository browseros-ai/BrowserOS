//! Frame routing on a real browser: visible effects distinguish each document and each run.

#[expect(
    dead_code,
    reason = "shared browser fixture includes helpers used by other suites"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use browseros_cdp::SessionId;
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;

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
<script>document.getElementById('cross').src = 'http://localhost:' + location.port + '/cross.html'</script>"
    );
    let same = format!("{CONTROLS}<iframe id=nested src=/leaf.html></iframe>");
    let cross = format!(
        r"{CONTROLS}<iframe id=nested name=nested></iframe><iframe id=local src=/leaf.html></iframe>
<script>document.getElementById('nested').src = 'http://127.0.0.1:' + location.port + '/leaf.html'</script>"
    );
    let tab = Tab::open(
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
    .await;
    time::timeout(Duration::from_secs(10), async {
        while tab.evaluate("document.readyState").await != "complete" {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("fixture did not finish loading: {error}"));
    Some(tab)
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
        await email.fill('alice@example.com', {{timeout:3000}});
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

async fn iframe_targets(tab: &Tab) -> Vec<Value> {
    let targets = tab
        .cdp
        .send("Target.getTargets", json!({"filter":[{}]}), None)
        .await
        .unwrap_or_else(|error| panic!("targets: {error}"));
    targets["targetInfos"]
        .as_array()
        .unwrap_or_else(|| panic!("targets: {targets}"))
        .iter()
        .filter(|target| target["type"] == "iframe")
        .cloned()
        .collect()
}

async fn cross_target(tab: &Tab) -> Value {
    time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(target) = iframe_targets(tab).await.into_iter().find(|target| {
                target["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/cross.html"))
            }) {
                return target;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("OOPIF target did not appear: {error}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn oopifs_and_nested_mixed_hosts_use_every_existing_operation() {
    let Some(tab) = open().await else { return };
    exercise(&tab, "#cross").await;
    exercise(&tab, &format!("#cross{HOP}#nested")).await;
    exercise(&tab, &format!("#cross{HOP}#local")).await;
    let untouched = tab.evaluate("[document.querySelector('#email').value, document.querySelector('#same').contentDocument.querySelector('#email').value, document.querySelector('#sibling').contentDocument.querySelector('#email').value]").await;
    // Cold regression: no child target enumeration or attach occurs until Neo has acted.
    let target = cross_target(&tab).await;
    assert!(
        target["url"]
            .as_str()
            .unwrap()
            .starts_with("http://localhost:")
    );
    let attached = tab
        .cdp
        .send(
            "Target.attachToTarget",
            json!({"targetId":target["targetId"],"flatten":true}),
            None,
        )
        .await
        .expect("driver child attach");
    let driver = SessionId::from(attached["sessionId"].as_str().unwrap());
    let edited = tab
        .cdp
        .send(
            "Runtime.evaluate",
            json!({"expression":"document.querySelector('#email').value","returnByValue":true}),
            Some(&driver),
        )
        .await
        .expect("read OOPIF edit");
    tab.close().await;
    assert_eq!(edited["result"]["value"], "alice@example.com!");
    assert_eq!(untouched, json!(["seed", "seed", "seed"]));
}

async fn child_driver(tab: &Tab, target: &Value) -> SessionId {
    let attached = tab
        .cdp
        .send(
            "Target.attachToTarget",
            json!({"targetId":target["targetId"],"flatten":true}),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("attach child driver: {error}"));
    SessionId::from(
        attached["sessionId"]
            .as_str()
            .unwrap_or_else(|| panic!("attach answered {attached}")),
    )
}

async fn evaluate_child(tab: &Tab, driver: &SessionId, expression: &str) -> Value {
    let result = tab
        .cdp
        .send(
            "Runtime.evaluate",
            json!({"expression":expression,"returnByValue":true}),
            Some(driver),
        )
        .await
        .unwrap_or_else(|error| panic!("child evaluation: {error}"));
    assert!(result.get("exceptionDetails").is_none(), "{result}");
    result["result"]["value"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_an_oopif_interrupts_its_pending_call_and_refinds_the_replacement() {
    let Some(tab) = open().await else { return };
    let target = cross_target(&tab).await;
    let driver = child_driver(&tab, &target).await;
    let utility = tab.cdp.send("Page.createIsolatedWorld",json!({"frameId":target["targetId"],"worldName":"__neo_utility_world__","grantUniveralAccess":true}),Some(&driver)).await.expect("utility world");
    // Hold the real checkElementStates call inside its animation-frame promise. The shared
    // DOM marker proves the call has started before removal; no CPU or renderer is frozen.
    tab.cdp.send("Runtime.evaluate",json!({"contextId":utility["executionContextId"],"expression":"globalThis.requestAnimationFrame = () => { document.documentElement.setAttribute('data-neo-frame-pending', 'yes'); return 1; }"}),Some(&driver)).await.expect("hold animation frame");
    let pending = tab.start_run(r"
        const button = page.locator('#cross >> internal:control=enter-frame >> #go');
        await button.click({timeout:5000});
        return await page.locator('#cross >> internal:control=enter-frame >> #result').textContent();
    ".into()).await;
    time::timeout(Duration::from_secs(10), async {
        while evaluate_child(
            &tab,
            &driver,
            "document.documentElement.getAttribute('data-neo-frame-pending')",
        )
        .await
            != "yes"
        {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Neo call entered the old OOPIF");
    tab.evaluate(
        "{ const old = document.querySelector('#cross'); old.replaceWith(old.cloneNode(false)); }",
    )
    .await;
    let outcome = pending.await.expect("pending run");
    let main = tab.evaluate("document.querySelector('#email').value").await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!("clicked")), "{outcome:#?}");
    assert_eq!(main, "seed");
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_frame_guids_serve_local_remote_and_nested_documents() {
    let Some(tab) = open().await else { return };
    let cross = cross_target(&tab).await;
    let tree = tab
        .cdp
        .send("Page.getFrameTree", json!({}), Some(&tab.driver))
        .await
        .expect("main tree");
    let local = tree["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("local id")
        .to_string();
    let driver = child_driver(&tab, &cross).await;
    let remote_tree = tab
        .cdp
        .send("Page.getFrameTree", json!({}), Some(&driver))
        .await
        .expect("remote tree");
    let remote_local = remote_tree["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("local child in OOPIF")
        .to_string();
    let targets = iframe_targets(&tab).await;
    let nested = targets
        .iter()
        .find(|target| target["parentFrameId"] == cross["targetId"])
        .expect("nested OOPIF")["targetId"]
        .as_str()
        .unwrap()
        .to_string();
    for (scope_name, id) in [
        ("same", local),
        ("cross", cross["targetId"].as_str().unwrap().to_string()),
        ("nested", nested),
        ("cross-local", remote_local),
    ] {
        let guid = serde_json::to_string(&format!("frame@{}/{id}", tab.target_id)).expect("guid");
        let outcome = run(&tab,&format!(r"
            let next = 0;
            const call = async (method, params={{}}) => {{
                const reply = JSON.parse(await __neo_runtime.call(JSON.stringify({{id:++next,guid:{guid},method,params,metadata:{{timeout:5000}}}})));
                if (reply.error) throw new Error(reply.error.error.message);
                return reply.result;
            }};
            await call('fill', {{selector:'#email',strict:true,value:'direct'}});
            await call('click', {{selector:'#go',strict:true}});
            return {{input:(await call('inputValue',{{selector:'#email',strict:true}})).value,
                text:(await call('textContent',{{selector:'#result',strict:true}})).value,
                title:(await call('title')).value,
                visible:(await call('isVisible',{{selector:'#email',strict:true}})).value}};
        ")).await;
        assert_ok(&outcome);
        assert_eq!(
            outcome.value,
            Some(json!({"input":"direct","text":"clicked","title":"Frame form","visible":true})),
            "scope={scope_name}: {outcome:#?}"
        );
    }
    let main = tab.evaluate("document.querySelector('#email').value").await;
    tab.close().await;
    assert_eq!(main, "seed");
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_child_navigation_is_explicitly_unserved_and_missing_scopes_are_detached() {
    let Some(tab) = open().await else { return };
    let cross = cross_target(&tab).await;
    let guid = serde_json::to_string(&format!(
        "frame@{}/{}",
        tab.target_id,
        cross["targetId"].as_str().unwrap()
    ))
    .unwrap();
    let missing = serde_json::to_string(&format!("frame@{}/removed", tab.target_id)).unwrap();
    let outcome = run(&tab,&format!(r"
        let id=0;
        const call = async (guid,method,params={{}}) => JSON.parse(await __neo_runtime.call(JSON.stringify({{id:++id,guid,method,params,metadata:{{timeout:500}}}})));
        return await Promise.all([
            call({guid},'goto',{{url:'https://example.test/'}}),
            call({guid},'neo:waitForURL',{{url:{{glob:'**'}}}}),
            call({guid},'neo:waitForLoadState',{{state:'load'}}),
            call({missing},'isVisible',{{selector:'#email'}}),
        ]).then(replies=>replies.map(reply=>reply.error.error.name));
    ")).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!(["Error", "Error", "Error", "Error"]))
    );
}
