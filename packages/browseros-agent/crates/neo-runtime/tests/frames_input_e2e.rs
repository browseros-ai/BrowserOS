//! Neo: Real frame input tests cover ancestor hit targets, geometry, focus, dialogs and tab turns.

#[expect(dead_code, reason = "shared browser fixture serves other e2e suites")]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{Delivery, RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;

const CONTROLS: &str = r#"<!doctype html>
<style>
body{margin:17px}iframe{display:block;width:360px;height:210px;border:7px solid #888;padding:5px;margin-top:12px}
#go{display:block;width:16px;height:16px;border:0;padding:0;margin:19px 53px}
</style>
<button id=go aria-label=Go onclick="document.querySelector('#result').textContent='clicked'">G</button>
<output id=result>idle</output>
<form onsubmit="event.preventDefault();document.querySelector('#submitted').textContent=document.querySelector('#email').value">
<label>Email<input id=email value=seed></label><button type=submit>Send</button>
</form><output id=submitted>idle</output>
<button id=dialog onclick="alert('frame dialog');document.querySelector('#result').textContent='dismissed'">Alert</button>
<script>
for(const type of ['mousedown','mouseup','click']) addEventListener(type,event=>{
  if(event.target.id==='go') top.postMessage({kind:'frame-input',who:window.name,type},'*');
},true);
addEventListener('message',event=>{
  if(event.data!=='cover-children') return;
  const overlay=document.createElement('div');overlay.id='inner-overlay';
  overlay.style='position:fixed;inset:0;z-index:1000;background:#ddd';
  overlay.onclick=()=>document.querySelector('#result').textContent='cover clicked';
  document.body.append(overlay);top.postMessage({kind:'frame-covered'},'*');
});
</script>"#;

async fn open(extra_style: &str) -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let top = format!(
        r"<!doctype html>
<style>
body{{margin:23px}}.frames{{display:flex;gap:23px;align-items:flex-start}}
iframe{{width:440px;height:760px;border:7px solid #888;padding:5px}}
{extra_style}
</style>
<label>Parent<input id=email value=parent></label><output id=result>idle</output>
<div class=frames><iframe id=same name=same src=/same.html></iframe><iframe id=cross name=cross></iframe></div>
<script>
window.events=[];
addEventListener('message',event=>{{
  if(event.data.kind==='frame-input') events.push(event.data);
  if(event.data.kind==='frame-covered') document.documentElement.dataset.covered='yes';
}});
document.querySelector('#cross').src='http://localhost:'+location.port+'/cross.html';
</script>"
    );
    let same = format!("{CONTROLS}<iframe id=local name=same-local src=/leaf.html></iframe>");
    let cross = format!(
        r"{CONTROLS}<iframe id=nested name=nested></iframe><iframe id=local name=cross-local src=/leaf.html></iframe>
<script>document.querySelector('#nested').src='http://127.0.0.1:'+location.port+'/leaf.html'</script>"
    );
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                ("/input.html".into(), top),
                ("/same.html".into(), same),
                ("/cross.html".into(), cross),
                ("/leaf.html".into(), CONTROLS.into()),
            ],
            ..Fixture::default()
        },
        "/input.html",
    )
    .await;
    tab.cdp
        .send(
            "Emulation.setDeviceMetricsOverride",
            json!({"width":1100,"height":1000,"deviceScaleFactor":1,"mobile":false}),
            Some(&tab.driver),
        )
        .await
        .unwrap_or_else(|error| panic!("fixture viewport: {error}"));
    time::timeout(Duration::from_secs(10), async {
        while tab.evaluate("document.readyState").await != "complete" {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("frame fixture load: {error}"));
    Some(tab)
}

async fn run(tab: &Tab, source: &str) -> RunOutcome {
    tab.start_run(source.into())
        .await
        .await
        .unwrap_or_else(|error| panic!("Neo run task: {error}"))
}

fn assert_ok(outcome: &RunOutcome) {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
}

async fn wait_for(tab: &Tab, expression: &str, expected: Value) {
    time::timeout(Duration::from_secs(10), async {
        while tab.evaluate(expression).await != expected {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("waiting for {expression} to be {expected}: {error}"));
}

/// Discover only after Neo has acted: a driver must not warm up Chromium's lazy OOPIF hosts.
async fn assert_oopifs(tab: &Tab) {
    let reply = tab
        .cdp
        .send(
            "Target.getTargets",
            json!({"filter":[{"type":"iframe"}]}),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("prove real OOPIFs: {error}"));
    let targets = reply["targetInfos"]
        .as_array()
        .unwrap_or_else(|| panic!("targetInfos: {reply}"));
    let cross = targets
        .iter()
        .find(|target| {
            target["type"] == "iframe"
                && target["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/cross.html"))
        })
        .unwrap_or_else(|| panic!("cross-site iframe target: {targets:?}"));
    assert!(
        targets.iter().any(|target| target["type"] == "iframe"
            && (target["parentFrameId"] == cross["targetId"]
                || target["parentId"] == cross["targetId"])
            && target["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/leaf.html"))),
        "nested cross-site iframe target: {targets:?}"
    );
}

async fn cover(tab: &Tab) {
    tab.evaluate(
        r"{
const overlay=document.createElement('div');overlay.id='overlay';
overlay.style='position:fixed;inset:0;z-index:1000;background:#ddd';
overlay.onclick=()=>document.querySelector('#result').textContent='cover clicked';
document.body.append(overlay);
}",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_parent_overlay_times_out_without_clicking_either_document() {
    let Some(tab) = open("").await else { return };
    cover(&tab).await;
    let outcome = run(&tab, r"
        const paths=['#same','#cross','#cross >> internal:control=enter-frame >> #nested'];
        const errors=[];
        for(const path of paths) {
            const button=page.locator(path+' >> internal:control=enter-frame >> #go');
            try { await button.click({timeout:1000}); errors.push('clicked'); }
            catch(error) { errors.push(error.name); }
        }
        return {errors, results:await Promise.all(paths.map(path=>page.locator(path+' >> internal:control=enter-frame >> #result').textContent()))};
    ").await;
    let parent = tab
        .evaluate("document.querySelector('#result').textContent")
        .await;
    let events = tab.evaluate("window.events").await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(
            json!({"errors":["TimeoutError","TimeoutError","TimeoutError"],"results":["idle","idle","idle"]})
        ),
        "{outcome:#?}"
    );
    assert_eq!(parent, "idle");
    assert_eq!(events, json!([]));
    assert!(
        outcome
            .calls
            .iter()
            .filter(|call| call.method == "click")
            .all(|call| call.delivery == Some(Delivery::NotSent)),
        "{outcome:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_removing_a_parent_overlay_lets_the_waiting_frame_click_land() {
    let Some(tab) = open("").await else { return };
    cover(&tab).await;
    tab.evaluate("setTimeout(()=>document.querySelector('#overlay').remove(),600)")
        .await;
    let outcome = run(&tab, r"
        const frame=selector=>page.locator('#cross >> internal:control=enter-frame >> #nested >> internal:control=enter-frame >> '+selector);
        await frame('#go').click({timeout:5000});
        return await frame('#result').textContent();
    ").await;
    let parent = tab
        .evaluate("document.querySelector('#result').textContent")
        .await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!("clicked")), "{outcome:#?}");
    assert_eq!(parent, "idle");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_clicks_reach_same_origin_remote_and_nested_bordered_padded_frames() {
    let Some(tab) = open("").await else { return };
    let outcome = run(&tab, r"
        const paths=[
            '#same', '#cross',
            '#same >> internal:control=enter-frame >> #local',
            '#cross >> internal:control=enter-frame >> #nested',
            '#cross >> internal:control=enter-frame >> #local'
        ];
        const results=[];
        for(const path of paths) {
            const frame=selector=>page.locator(path+' >> internal:control=enter-frame >> '+selector);
            await frame('#go').click({timeout:5000});
            results.push(await frame('#result').textContent());
        }
        return results;
    ").await;
    wait_for(
        &tab,
        "window.events.filter(event=>event.type==='click').length",
        json!(5),
    )
    .await;
    let clicked = tab
        .evaluate("window.events.filter(event=>event.type==='click').map(event=>event.who)")
        .await;
    let parent = tab.evaluate("[document.querySelector('#email').value,document.querySelector('#result').textContent]").await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!([
            "clicked", "clicked", "clicked", "clicked", "clicked"
        ])),
        "{outcome:#?}"
    );
    assert_eq!(
        clicked,
        json!(["same", "cross", "same-local", "nested", "cross-local"])
    );
    assert_eq!(parent, json!(["parent", "idle"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_overlay_inside_an_oopif_blocks_its_nested_frames() {
    let Some(tab) = open("").await else { return };
    tab.evaluate(
        "document.querySelector('#cross').contentWindow.postMessage('cover-children','*')",
    )
    .await;
    wait_for(
        &tab,
        "document.documentElement.dataset.covered",
        json!("yes"),
    )
    .await;
    let outcome = run(&tab,r"
        const errors=[];
        for(const owner of ['#nested','#local']) {
            const frame=selector=>page.locator('#cross >> internal:control=enter-frame >> '+owner+' >> internal:control=enter-frame >> '+selector);
            try {await frame('#go').click({timeout:1000});errors.push('clicked');}
            catch(error) {errors.push(error.name);}
        }
        return {errors,parent:await page.locator('#cross >> internal:control=enter-frame >> #result').textContent()};
    ").await;
    let events = tab.evaluate("window.events").await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!({"errors":["TimeoutError","TimeoutError"],"parent":"idle"})),
        "{outcome:#?}"
    );
    assert_eq!(events, json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_transformed_iframes_use_the_event_interceptor() {
    let Some(tab) = open("#same{width:300px;height:240px;transform:rotate(30deg)}").await else {
        return;
    };
    let outcome = run(
        &tab,
        r"
        const frame=selector=>page.locator('#same >> internal:control=enter-frame >> '+selector);
        await frame('#go').click({timeout:5000});
        return await frame('#result').textContent();
    ",
    )
    .await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!("clicked")), "{outcome:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_main_frame_button_in_the_scrollbar_gutter_times_out() {
    let Some(tab) = open(
        "html{overflow-x:hidden}body{min-height:3000px}::-webkit-scrollbar{width:15px;height:15px}",
    )
    .await
    else {
        return;
    };
    tab.evaluate(r"{
        const button=document.createElement('button');button.id='gutter';
        button.style='position:fixed;top:100px;left:calc(100vw - 20px);width:30px;height:40px;padding:0;border:0';
        button.textContent='G';
        button.onclick=()=>document.querySelector('#result').textContent='gutter clicked';
        document.body.append(button);
    }").await;
    assert_eq!(
        tab.evaluate("innerWidth>document.documentElement.clientWidth")
            .await,
        json!(true),
        "fixture has a scrollbar gutter"
    );
    let outcome = run(
        &tab,
        r"
        try {await page.locator('#gutter').click({timeout:1000});return 'clicked';}
        catch(error) {return error.name;}
    ",
    )
    .await;
    let parent = tab
        .evaluate("document.querySelector('#result').textContent")
        .await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!("TimeoutError")), "{outcome:#?}");
    assert_eq!(parent, "idle");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_scroll_none_does_not_scroll_an_offscreen_oopif_owner() {
    let Some(tab) = open("#cross{margin-top:3000px}").await else {
        return;
    };
    assert_eq!(tab.evaluate("[scrollX, scrollY]").await, json!([0, 0]));
    let outcome = run(
        &tab,
        r"
        try {
            await page.frameLocator('#cross').locator('#go').click({scroll:'none',timeout:1000});
            return 'clicked';
        } catch(error) { return error.name; }
    ",
    )
    .await;
    let offsets = tab.evaluate("[scrollX, scrollY]").await;
    let events = tab.evaluate("window.events").await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        offsets,
        json!([0, 0]),
        "scroll:none keeps the owners in place"
    );
    assert_eq!(outcome.value, Some(json!("TimeoutError")), "{outcome:#?}");
    assert_eq!(events, json!([]), "no input reaches the offscreen button");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_oopif_below_the_fold_scrolls_its_parent_and_receives_the_click() {
    let Some(tab) = open("#cross{margin-top:3000px}").await else {
        return;
    };
    assert_eq!(tab.evaluate("scrollY").await, json!(0));
    let outcome = run(
        &tab,
        r"
        const frame=selector=>page.locator('#cross >> internal:control=enter-frame >> '+selector);
        await frame('#go').click({timeout:5000});
        return await frame('#result').textContent();
    ",
    )
    .await;
    let scrolled = tab.evaluate("scrollY>0").await;
    let parent = tab
        .evaluate("document.querySelector('#result').textContent")
        .await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!("clicked")), "{outcome:#?}");
    assert_eq!(scrolled, json!(true));
    assert_eq!(parent, "idle");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_enter_submits_the_form_in_the_focused_oopif() {
    let Some(tab) = open("").await else { return };
    let outcome = run(
        &tab,
        r"
        const frame=selector=>page.locator('#cross >> internal:control=enter-frame >> '+selector);
        await frame('#email').fill('alice@example.com');
        await frame('#email').press('Enter');
        return await frame('#submitted').textContent();
    ",
    )
    .await;
    let parent = tab.evaluate("document.querySelector('#email').value").await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!("alice@example.com")),
        "{outcome:#?}"
    );
    assert_eq!(parent, "parent");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_background_fill_moves_focus_from_the_parent_into_the_oopif() {
    let Some(tab) = open("").await else { return };
    tab.evaluate("document.querySelector('#email').focus()")
        .await;
    assert_eq!(tab.evaluate("document.activeElement.id").await, "email");
    let foreground = tab
        .cdp
        .send("Target.createTarget", json!({"url":"about:blank"}), None)
        .await
        .unwrap_or_else(|error| panic!("foreground tab: {error}"));
    tab.cdp
        .send(
            "Target.activateTarget",
            json!({"targetId":foreground["targetId"]}),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("activate foreground tab: {error}"));
    wait_for(&tab, "document.visibilityState", json!("hidden")).await;
    assert_eq!(tab.evaluate("document.activeElement.id").await, "email");
    let outcome = run(
        &tab,
        r"
        const input=page.locator('#cross >> internal:control=enter-frame >> #email');
        await input.fill('background');
        await input.press('End');
        await input.press('!');
        return await input.inputValue();
    ",
    )
    .await;
    let parent = tab
        .evaluate("[document.querySelector('#email').value,document.activeElement.id]")
        .await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!("background!")), "{outcome:#?}");
    assert_eq!(parent, json!(["parent", "cross"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_oopif_alert_is_dismissed_and_the_click_finishes() {
    let Some(tab) = open("").await else { return };
    let outcome = run(
        &tab,
        r"
        const frame=selector=>page.locator('#cross >> internal:control=enter-frame >> '+selector);
        await frame('#dialog').click({timeout:5000});
        return await frame('#result').textContent();
    ",
    )
    .await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!("dismissed")), "{outcome:#?}");
    assert!(
        outcome
            .calls
            .iter()
            .filter(|call| call.method == "click")
            .all(|call| call.delivery == Some(Delivery::Done)),
        "{outcome:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_concurrent_clicks_in_different_frames_keep_each_mouse_sequence_whole() {
    let Some(tab) = open("").await else { return };
    let outcome = run(&tab,r"
        for(let i=0;i<3;i++) await Promise.all([
            page.locator('#same >> internal:control=enter-frame >> #go').click({delay:80}),
            page.locator('#cross >> internal:control=enter-frame >> #go').click({delay:80})
        ]);
        return await Promise.all(['#same','#cross'].map(path=>page.locator(path+' >> internal:control=enter-frame >> #result').textContent()));
    ").await;
    wait_for(&tab, "window.events.length", json!(18)).await;
    let events = tab.evaluate("window.events").await;
    assert_oopifs(&tab).await;
    tab.close().await;
    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!(["clicked", "clicked"])),
        "{outcome:#?}"
    );
    for sequence in events
        .as_array()
        .unwrap_or_else(|| panic!("mouse events: {events}"))
        .chunks(3)
    {
        assert_eq!(
            sequence
                .iter()
                .map(|event| event["type"]
                    .as_str()
                    .unwrap_or_else(|| panic!("event type: {event}")))
                .collect::<Vec<_>>(),
            ["mousedown", "mouseup", "click"],
            "{events}"
        );
        assert!(
            sequence
                .iter()
                .all(|event| event["who"] == sequence[0]["who"]),
            "{events}"
        );
    }
    assert!(
        outcome
            .calls
            .iter()
            .filter(|call| call.method == "click")
            .all(|call| call.delivery == Some(Delivery::Done)),
        "{outcome:#?}"
    );
}
