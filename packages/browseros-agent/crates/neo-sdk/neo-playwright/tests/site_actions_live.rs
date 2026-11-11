//! Actions and `expect` masked with the element's whole site, on a real browser, through the
//! real prelude in QuickJS over `Playwright::invoke`. The page's script copies a password typed
//! in a frame of its site into the top document (`tests/data/site_copy/top.html`), so the top
//! frame's own sweep never sees the field. Every page string an action or a matcher reports
//! must then come back masked: a strict violation's previews, the element a covered click
//! names as in the way, a text matcher's `Received` and its `locator resolved to` lines, and a
//! value an `expect.poll` reads with `textContent`. The call-log lines reach the op's trace
//! record as they are logged (`OpEnv::note`), so the notes are checked too.
//!
//! The frame is embedded as `site_reads_live.rs` embeds it: in process, out of process under a
//! cross-site bridge, and out of process behind a closed shadow root.
//!
//! Playwright's engine also cuts page text in the page, before Rust can mask whole values: a
//! preview keeps 50 characters, a locator hint a few words. Two tests copy the secret into text
//! long enough that the cut falls inside it, and check that a strict click, a click whose
//! cover came in the way before the interceptor was armed or after it, and a `toHaveText`
//! failure hold no start of the secret three characters or more. A last test checks that
//! `selectOption` answers a sensitive select's selected values masked, all of them, where the
//! seal's sweep holds only the first.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005), for example
//! `PORT_BASE=9751 spikes/eval/browser.sh <profile> &`, then
//! `BROWSEROS_CDP_PORT=9751 cargo test -p neo-playwright --test site_actions_live --
//! --ignored`. Every test opens its own tab and closes it, one test at a time.

#[path = "support/fixture_server.rs"]
#[expect(dead_code, reason = "no nested fixture or scripts here")]
mod fixture_server;
#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the capture refusal and the trusted click serve the other live suites"
)]
mod live_run;
#[path = "support/locator.rs"]
#[expect(dead_code, reason = "these locators are CSS only")]
mod locator;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fixture_server::{FixtureServer, Page};
use js_runner::{OpHost, run_script};
use live_run::{EnvCall, LiveRun, TestPage, TestResult, brand};
use locator::{css, locator};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time;

/// One test at a time: each connects its own `Playwright`, whose frame registry attaches to
/// every frame target in the browser.
static ALONE: Semaphore = Semaphore::const_new(1);

/// The fixture, under `/site_copy/`.
fn site_copy_pages() -> HashMap<String, Page> {
    [
        ("top", include_str!("data/site_copy/top.html")),
        ("bridge", include_str!("data/site_copy/bridge.html")),
        ("field", include_str!("data/site_copy/field.html")),
    ]
    .into_iter()
    .map(|(name, body)| {
        let page = Page::html(body).with_header("cache-control", "no-store");
        (format!("/site_copy/{name}.html"), page)
    })
    .collect()
}

/// Opens a tab on `url`, runs `body` on it, and closes the tab whatever happened.
async fn on_page<F, Fut>(run: Arc<LiveRun>, url: &str, body: F) -> TestResult
where
    F: FnOnce(Arc<LiveRun>, TestPage) -> Fut,
    Fut: Future<Output = TestResult>,
{
    let page = run.new_page().await?;
    let opened = run
        .invoke(
            "page.goto",
            json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page), "url": url }),
        )
        .await;
    let result = match opened {
        Ok(_) => body(Arc::clone(&run), page.clone()).await,
        Err(error) => Err(error.into()),
    };
    run.close_page(&page).await;
    result
}

/// Types `secret` into the same-site frame's password field and runs the page's own script that
/// copies it into the top document.
async fn type_and_copy(run: &LiveRun, page: &TestPage, secret: &str) -> TestResult {
    let ready = "(() => { try { return !!field(); } catch (_) { return false; } })()";
    let started = Instant::now();
    while run.eval(page, ready).await? != json!(true) {
        if started.elapsed() > Duration::from_secs(10) {
            return Err("the same-site field frame never loaded".into());
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    run.eval(page, &format!("field().value = {secret:?}; copyField()"))
        .await?;
    Ok(())
}

/// [`type_and_copy`], then covers a new button with the copied heading.
async fn type_copy_and_cover(run: &LiveRun, page: &TestPage, secret: &str) -> TestResult {
    type_and_copy(run, page, secret).await?;
    // A stylesheet, not a style attribute, so the heading's preview stays its fixture form.
    run.eval(
        page,
        "(() => { const style = document.createElement('style'); \
         style.textContent = 'h1.who { position: fixed; inset: 0; margin: 0; z-index: 9; \
         background: white; }'; document.head.append(style); \
         document.body.insertAdjacentHTML('beforeend', '<button id=\"under\">Under</button>'); \
         return true; })()",
    )
    .await?;
    Ok(())
}

/// One script line, and what its failure's message must show once masked.
struct Case {
    label: &'static str,
    script: &'static str,
    shows: &'static [&'static str],
}

/// Each page string an action or a matcher reports, each failing on purpose.
const CASES: &[Case] = &[
    Case {
        label: "a strict violation's previews",
        script: "page.locator('.who').click({ timeout: 1000 })",
        shows: &[
            r#"1) <h1 class="who" data-who="[private]">Signed in with [private]</h1>"#,
            r#"2) <p class="who">[private]</p>"#,
        ],
    },
    Case {
        label: "the element a covered click names",
        script: "page.locator('#under').click({ timeout: 1000 })",
        shows: &["Signed in with [private]</h1> intercepts pointer events"],
    },
    Case {
        label: "toHaveText's Received and call log",
        script: "expect(page.locator('h1')).toHaveText('Signed out', { timeout: 500 })",
        shows: &[
            r#"Received: "Signed in with [private]""#,
            r#"locator resolved to <h1 class="who" data-who="[private]">Signed in with [private]</h1>"#,
        ],
    },
    Case {
        label: "toContainText's Received",
        script: "expect(page.locator('p.who')).toContainText('nobody', { timeout: 500 })",
        shows: &[r#"Received string:    "[private]""#],
    },
    Case {
        label: "a textContent an expect.poll compares",
        script: "expect.poll(() => page.locator('h1').textContent(), { timeout: 500 }).toBe('Signed out')",
        shows: &[r#"Received: "Signed in with [private]""#],
    },
];

/// Runs each case's script on `page`, one after the other, and answers what each failed with:
/// its message by label, or `null` for a case that did not fail.
async fn failures(run: &Arc<LiveRun>, page: &TestPage, cases: &[Case]) -> TestResult<Value> {
    let mut checks = String::new();
    for case in cases {
        let _infallible = writeln!(
            checks,
            "await check({}, async () => {{ await {} }})",
            json!(case.label),
            case.script
        );
    }
    let script = format!(
        "const results = {{}}\n\
         async function check(label, run) {{\n\
           try {{ await run(); results[label] = null }}\n\
           catch (e) {{ results[label] = String(e.message) }}\n\
         }}\n\
         {checks}\
         return results"
    );
    let init = json!({ "page": brand(page), "pages": [], "browserVersion": "" });
    let host: Arc<dyn OpHost> = Arc::clone(run) as Arc<dyn OpHost>;
    Ok(run_script(&host, &init, &script).await?)
}

/// Runs every case on `page` once it copied `secret`, and fails with each case whose failure
/// did not show its copy masked, or that showed `secret` raw, in its message or in a call-log
/// line the trace got.
async fn reports_mask(run: Arc<LiveRun>, page: &TestPage, secret: &str) -> TestResult {
    type_copy_and_cover(&run, page, secret).await?;
    let results = failures(&run, page, CASES).await?;
    let mut wrong = Vec::new();
    for case in CASES {
        let Some(message) = results.get(case.label).and_then(Value::as_str) else {
            wrong.push(format!("{}: did not fail: {results}", case.label));
            continue;
        };
        if message.contains(secret) {
            wrong.push(format!("{}: holds {secret} raw: {message}", case.label));
            continue;
        }
        let missing: Vec<&&str> = case
            .shows
            .iter()
            .filter(|part| !message.contains(**part))
            .collect();
        if !missing.is_empty() {
            wrong.push(format!(
                "{}: does not show {missing:?}: {message}",
                case.label
            ));
        }
    }
    let leaked: Vec<String> = run
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            EnvCall::Note(line) if line.contains(secret) => Some(line),
            EnvCall::Note(_) | EnvCall::Recheck | EnvCall::RecheckReach(_) | EnvCall::MarkSent => {
                None
            }
        })
        .collect();
    if !leaked.is_empty() {
        wrong.push(format!(
            "call-log lines the trace got hold {secret}: {leaked:#?}"
        ));
    }
    assert!(
        wrong.is_empty(),
        "the page copied {secret} from a frame of its site, so every page string an action or a \
         matcher reports holds it masked; these did not:\n{}",
        wrong.join("\n")
    );
    let swept = run.swept();
    assert!(
        swept.iter().any(|value| value == secret),
        "the host is told of {secret}, so its own mask knows it in later results: {swept:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_actions_and_expect_mask_a_value_copied_from_a_same_site_frame_in_process()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(site_copy_pages()).await?;
    let url = server.url("127.0.0.1", "/site_copy/top.html?inproc");
    let run = Arc::new(LiveRun::connect().await?);
    on_page(run, &url, |run, page| async move {
        reports_mask(run, &page, "inproc-secret-74").await
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_actions_and_expect_mask_a_value_copied_from_a_same_site_frame_out_of_process()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(site_copy_pages()).await?;
    let url = server.url("127.0.0.1", "/site_copy/top.html?oopif");
    let run = Arc::new(LiveRun::connect().await?);
    on_page(run, &url, |run, page| async move {
        reports_mask(run, &page, "oopif-secret-85").await
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_actions_and_expect_mask_a_value_copied_from_a_frame_under_a_shadow_root_oopif()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(site_copy_pages()).await?;
    let url = server.url("127.0.0.1", "/site_copy/top.html?shadow");
    let run = Arc::new(LiveRun::connect().await?);
    on_page(run, &url, |run, page| async move {
        reports_mask(run, &page, "shadow-secret-96").await
    })
    .await
}

/// Makes `#long` cover the whole page from the first pointer move over a new `#late` button, so
/// the click's own press lands on it after the interceptor was armed, and adds an `#under`
/// button for a click the cover is already in the way of.
const LATE_COVER: &str = "(() => {
  const style = document.createElement('style');
  style.textContent = '#long.cover { position: fixed; inset: 0; margin: 0; z-index: 9; background: white; }';
  document.head.append(style);
  document.body.insertAdjacentHTML('beforeend', '<button id=\"late\">Late</button><button id=\"under\">Under</button>');
  document.getElementById('late').addEventListener('mousemove', () => document.getElementById('long').classList.add('cover'));
  return true;
})()";

/// Each action and matcher whose page text Playwright's engine cuts in the page, each failing on
/// purpose, run in this order on the page `LATE_COVER` set up. The copy in `#long` is long
/// enough that a preview's 50-character cut falls inside the secret, so a mask of whole values
/// put on the cut preview afterwards would leave most of the secret in it.
const CUT_CASES: &[Case] = &[
    Case {
        label: "a strict violation's cut previews",
        script: "page.locator('h1, #long').click({ timeout: 1000 })",
        shows: &[r#"2) <div id="long">The name this account was opened under is [privat…</div>"#],
    },
    Case {
        label: "a cover that came in the way after the interceptor was armed",
        script: "page.locator('#late').click({ timeout: 1000 })",
        shows: &[
            r#"<div id="long" class="cover">The name this account was opened under is [privat…</div> intercepts pointer events"#,
        ],
    },
    Case {
        label: "a cover already in the way",
        script: "page.locator('#under').click({ timeout: 1000 })",
        shows: &[
            r#"<div id="long" class="cover">The name this account was opened under is [privat…</div> intercepts pointer events"#,
        ],
    },
    Case {
        label: "toHaveText's Received and its cut call log",
        script: "expect(page.locator('#long')).toHaveText('Signed out', { timeout: 500 })",
        shows: &[
            r#"Received: "The name this account was opened under is [private] tail""#,
            r#"locator resolved to <div id="long" class="cover">The name this account was opened under is [privat…</div>"#,
        ],
    },
];

/// The longest start of `secret`, three characters or more, that `shown` holds.
fn leaked_start<'a>(shown: &str, secret: &'a str) -> Option<&'a str> {
    (3..=secret.len())
        .rev()
        .filter_map(|end| secret.get(..end))
        .find(|start| shown.contains(start))
}

/// Runs every cut case on `page` once it copied `secret`, and fails with each case whose
/// failure did not show its copy masked, or that holds any start of `secret` three characters
/// or more, in its message or in a call-log line the trace got. A preview or a locator hint
/// cut short would hold one even where a mask of whole values found nothing to mask.
async fn reports_no_start(run: Arc<LiveRun>, page: &TestPage, secret: &str) -> TestResult {
    type_and_copy(&run, page, secret).await?;
    run.eval(page, LATE_COVER).await?;
    let results = failures(&run, page, CUT_CASES).await?;
    let mut wrong = Vec::new();
    for case in CUT_CASES {
        let Some(message) = results.get(case.label).and_then(Value::as_str) else {
            wrong.push(format!("{}: did not fail: {results}", case.label));
            continue;
        };
        if let Some(start) = leaked_start(message, secret) {
            wrong.push(format!(
                "{}: holds {start:?}, the start of {secret}: {message}",
                case.label
            ));
            continue;
        }
        let missing: Vec<&&str> = case
            .shows
            .iter()
            .filter(|part| !message.contains(**part))
            .collect();
        if !missing.is_empty() {
            wrong.push(format!(
                "{}: does not show {missing:?}: {message}",
                case.label
            ));
        }
    }
    let notes: Vec<String> = run
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            EnvCall::Note(line) => Some(line),
            EnvCall::Recheck | EnvCall::RecheckReach(_) | EnvCall::MarkSent => None,
        })
        .collect();
    let leaked: Vec<&String> = notes
        .iter()
        .filter(|line| leaked_start(line, secret).is_some())
        .collect();
    if !leaked.is_empty() {
        wrong.push(format!(
            "call-log lines the trace got hold a start of {secret}: {leaked:#?}"
        ));
    }
    // Only `#late`'s click presses: what follows is its verdict, which names what the press hit.
    let after_press = notes
        .iter()
        .skip_while(|line| line.as_str() != "  performing click action")
        .nth(1);
    let late_line = r#"  <div id="long" class="cover">The name this account was opened under is [privat…</div> intercepts pointer events"#;
    if after_press.map(String::as_str) != Some(late_line) {
        wrong.push(format!(
            "the verdict after #late's press does not name the cover masked: {after_press:?}"
        ));
    }
    assert!(
        wrong.is_empty(),
        "the page copied {secret} from a frame of its site into text Playwright's engine cuts, so \
         no preview, hint or call-log line holds any part of it; these did:\n{}",
        wrong.join("\n")
    );
    let swept = run.swept();
    assert!(
        swept.iter().any(|value| value == secret),
        "the host is told of {secret}, so its own mask knows it in later results: {swept:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_actions_and_expect_cut_no_part_of_a_same_site_secret_in_process() -> TestResult
{
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(site_copy_pages()).await?;
    let url = server.url("127.0.0.1", "/site_copy/top.html?inproc");
    let run = Arc::new(LiveRun::connect().await?);
    on_page(run, &url, |run, page| async move {
        reports_no_start(run, &page, "zq-act-secret-41").await
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_actions_and_expect_cut_no_part_of_a_same_site_secret_out_of_process()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(site_copy_pages()).await?;
    let url = server.url("127.0.0.1", "/site_copy/top.html?oopif");
    let run = Arc::new(LiveRun::connect().await?);
    on_page(run, &url, |run, page| async move {
        reports_no_start(run, &page, "zq-oop-secret-52").await
    })
    .await
}

/// `selectOption` by `labels` on `selector`, as the prelude sends it.
async fn select(
    run: &LiveRun,
    page: &TestPage,
    selector: &str,
    labels: &[&str],
) -> Result<Value, neo_playwright::PlaywrightError> {
    let options: Vec<Value> = labels
        .iter()
        .map(|label| json!({ "label": label }))
        .collect();
    let input = json!({
        "call": { "api": "locator.selectOption", "timeoutMs": 2_000 },
        "target": locator(page, &[css(selector)], &format!("locator('{selector}')")),
        "strict": "strict",
        "options": options,
        "elements": [],
    });
    run.invoke("locator.selectOption", input).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_sensitive_select_answers_every_selected_value_masked() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(site_copy_pages()).await?;
    let url = server.url("127.0.0.1", "/site_copy/top.html?inproc");
    let run = Arc::new(LiveRun::connect().await?);
    on_page(run, &url, |run, page| async move {
        // A card's expiry select, which the seal counts as a card field (`cc-exp-month`), and a
        // plain one beside it. Options are chosen by label, so the values are the page's own.
        run.eval(
            &page,
            "(() => { document.body.insertAdjacentHTML('beforeend', \
             '<select id=\"exp\" multiple autocomplete=\"cc-exp-month\">\
             <option value=\"exp-month-07\">July</option>\
             <option value=\"exp-month-09\">September</option></select>\
             <select id=\"plain\" multiple><option value=\"plain-a1\">A</option>\
             <option value=\"plain-b2\">B</option></select>'); return true; })()",
        )
        .await?;
        assert_eq!(
            select(&run, &page, "#plain", &["A", "B"]).await?,
            json!(["plain-a1", "plain-b2"]),
            "a select that is no sensitive field answers its values"
        );
        assert_eq!(
            select(&run, &page, "#exp", &["July", "September"]).await?,
            json!(["[private]", "[private]"]),
            "a sensitive select's selected values are its value, each masked"
        );
        assert_eq!(
            run.eval(
                &page,
                "[...document.getElementById('exp').selectedOptions].map((o) => o.value)"
            )
            .await?,
            json!(["exp-month-07", "exp-month-09"]),
            "both options are selected"
        );
        let swept = run.swept();
        assert!(
            ["exp-month-07", "exp-month-09"]
                .iter()
                .all(|value| swept.iter().any(|swept| swept == value)),
            "the host is told of both, so its own mask knows them: {swept:?}"
        );
        Ok(())
    })
    .await
}
