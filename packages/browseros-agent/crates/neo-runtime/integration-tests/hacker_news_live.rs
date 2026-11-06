//! Submits the saved Hacker News script through one Rust controller. Independent CDP reads
//! verify the live listing and the five comment tabs; a later run resumes on a returned tab ID.
//! Requires live Hacker News and a temporary BrowserOS profile. From packages/browseros-agent:
//!
//! ```sh
//! NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo" \
//!   cargo test -p neo-runtime --test hacker_news_live -- --ignored --nocapture
//! ```

#[path = "../tests/support/browser.rs"]
#[expect(
    dead_code,
    reason = "the live scenario uses part of the shared browser support"
)]
mod browser;

use std::env;
use std::time::Duration;

use browser::{Fixture, Tab};
use browseros_cdp::SessionId;
use neo_runtime::{PageId, RunOutcome, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;

const SCRIPT: &str = include_str!("../../neo-sdk/tests/scripts/hacker-news-best.js");
const BEST: &str = "https://news.ycombinator.com/best";

// DOM reads deliberately use a different CDP session, without SDK selectors or return values.
const LISTING: &str = r"(() => ({
  url: location.href,
  stories: [...document.querySelectorAll('tr.athing')].slice(0, 5).map(row => {
    const title = row.querySelector('.titleline > a');
    const comments = [...row.nextElementSibling.querySelectorAll('a')]
      .find(link => /^(?:\d+\s+comments?|discuss)$/.test(link.textContent.trim()));
    return {
      rank: parseInt(row.querySelector('.rank').textContent, 10),
      id: row.id,
      title: title.textContent.trim(),
      url: title.href,
      comments: parseInt(comments.textContent, 10) || 0,
      commentsUrl: comments.href,
    };
  }),
}))()";

const COMMENT_PAGE: &str = r"(() => ({
  url: location.href,
  title: document.title,
  id: document.querySelector('table.fatitem tr.athing')?.id,
  storyTitle: document.querySelector('.fatitem .titleline > a')?.textContent.trim(),
  commentRows: document.querySelectorAll('tr.comtr').length,
}))()";

async fn run(runtime: &Runtime, page: &PageId, code: &str) -> RunOutcome {
    runtime
        .run(RunRequest {
            code: code.to_string(),
            page: page.clone(),
            timeout: Duration::from_secs(120),
            cancel: CancellationToken::new(),
        })
        .await
}

/// Keep observer failures as data until the browser closes, preserving the run's diagnostics.
async fn observe(tab: &Tab, target: &str, expression: &str) -> Result<Value, String> {
    let attached = tab
        .cdp
        .send(
            "Target.attachToTarget",
            json!({"targetId": target, "flatten": true}),
            None,
        )
        .await
        .map_err(|error| error.to_string())?;
    let session = SessionId::from(
        attached["sessionId"]
            .as_str()
            .ok_or_else(|| format!("missing observer session: {attached}"))?,
    );
    let reply = tab
        .cdp
        .send(
            "Runtime.evaluate",
            json!({"expression": expression, "returnByValue": true}),
            Some(&session),
        )
        .await
        .map_err(|error| error.to_string())?;
    if reply.get("exceptionDetails").is_some() {
        return Err(format!("observer threw: {reply}"));
    }
    Ok(reply["result"]["value"].clone())
}

#[expect(
    clippy::expect_used,
    reason = "this test helper asserts script and observer output"
)]
fn assert_story_tabs(value: &Value, listing: &Value, observed: Vec<Result<Value, String>>) {
    assert_eq!(value["best"]["url"], BEST, "{value:#}");
    assert_eq!(listing["url"], BEST, "{listing:#}");
    let stories = value["stories"].as_array().expect("stories is an array");
    let expected = listing["stories"]
        .as_array()
        .expect("the observer lists stories");
    let comments = value["commentTabs"]
        .as_array()
        .expect("commentTabs is an array");
    assert_eq!(stories.len(), 5, "{value:#}");
    assert_eq!(expected.len(), 5, "{listing:#}");
    assert_eq!(comments.len(), 5, "{value:#}");
    let mut ids = Vec::new();
    for (index, ((story, comment), actual)) in
        stories.iter().zip(comments).zip(observed).enumerate()
    {
        let actual = actual.expect("the independent observer can read each comment tab");
        let id = story["id"].as_str().expect("each story has an ID");
        assert!(
            !id.is_empty() && !ids.contains(&id),
            "distinct story IDs: {stories:#?}"
        );
        ids.push(id);
        assert_eq!(story["rank"], index + 1, "{story:#}");
        for field in ["rank", "id", "title", "comments", "commentsUrl"] {
            assert_eq!(
                story[field], expected[index][field],
                "listing read of {field}: {listing:#}"
            );
        }
        // DOM href properties normalize https://host to https://host/; compare URL values.
        assert_eq!(
            Url::parse(story["url"].as_str().expect("story URL is a string"))
                .expect("absolute story URL"),
            Url::parse(
                expected[index]["url"]
                    .as_str()
                    .expect("observed URL is a string")
            )
            .expect("absolute observed URL"),
            "extracted story link matches the listing"
        );
        let url = Url::parse(
            story["commentsUrl"]
                .as_str()
                .expect("comments URL is a string"),
        )
        .expect("comments URL is absolute");
        assert_eq!(url.host_str(), Some("news.ycombinator.com"), "{url}");
        assert_eq!(url.path(), "/item", "{url}");
        assert!(
            url.query_pairs().any(|(key, val)| key == "id" && val == id),
            "{url}"
        );
        assert_eq!(comment["id"], story["id"], "{comment:#}");
        assert_eq!(comment["url"], story["commentsUrl"], "{comment:#}");
        assert_eq!(comment["open"], true, "{comment:#}");
        for field in ["id", "url", "title", "storyTitle", "commentRows"] {
            assert_eq!(
                actual[field], comment[field],
                "independent tab read of {field}: {actual:#}"
            );
        }
        assert_eq!(actual["storyTitle"], story["title"], "{actual:#}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live Hacker News; opt in with NEO_E2E_BROWSER and --ignored"]
async fn tests_that_one_runtime_reads_best_and_keeps_five_comment_tabs_for_a_later_run() {
    let binary = env::var_os("NEO_E2E_BROWSER")
        .expect("set NEO_E2E_BROWSER to the BrowserOS neo binary to run this live test");
    let tab = Tab::open(binary, Fixture::default(), "/hacker-news-start").await;
    let page = PageId::new(tab.target_id.clone());
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .expect("connect the one Rust runtime controller");
    let before = run(
        &runtime,
        &page,
        "globalThis.__hnScenario = true; return page.isClosed();",
    )
    .await;
    let scenario = run(&runtime, &page, SCRIPT).await;
    let listing = observe(&tab, page.as_str(), LISTING).await;
    let created: Vec<_> = scenario
        .pages
        .iter()
        .filter(|p| p.created && !p.closed)
        .collect();
    let mut observed = Vec::new();
    for p in &created {
        observed.push(observe(&tab, &p.id, COMMENT_PAGE).await);
    }
    // Use a RunOutcome ID just as the customer does when continuing work on a chosen tab.
    let resume = created
        .first()
        .map_or_else(|| page.clone(), |p| PageId::new(p.id.clone()));
    let after = run(
        &runtime,
        &resume,
        r"
      return {
        url: page.url(),
        title: await page.title(),
        previousGlobal: typeof globalThis.__hnScenario,
        commentUrls: context.pages().map(p => p.url())
          .filter(url => url.startsWith('https://news.ycombinator.com/item?')),
      };
    ",
    )
    .await;
    tab.close().await;

    assert_eq!(before.status, RunStatus::Ok, "preflight: {before:#?}");
    assert_eq!(before.value, Some(json!(false)), "{before:#?}");
    assert_eq!(
        scenario.status,
        RunStatus::Ok,
        "hacker-news-best.js: {scenario:#?}\nListing: {listing:#?}\nComment tabs: {observed:#?}\nLater run: {after:#?}"
    );
    let value = scenario.value.as_ref().expect("the script returns JSON");
    let listing = listing.expect("the independent observer can read the listing");
    assert_eq!(created.len(), 5, "five new live tabs: {scenario:#?}");
    assert_story_tabs(value, &listing, observed);
    let stories = value["stories"].as_array().expect("stories is an array");
    let comments = value["commentTabs"]
        .as_array()
        .expect("commentTabs is an array");
    assert_eq!(
        after.status,
        RunStatus::Ok,
        "later run by tab ID: {after:#?}"
    );
    let read = after.value.as_ref().expect("later run returns JSON");
    assert_eq!(
        read["previousGlobal"], "undefined",
        "fresh JS realm: {read:#}"
    );
    assert_eq!(read["url"], stories[0]["commentsUrl"], "{read:#}");
    assert_eq!(read["title"], comments[0]["title"], "{read:#}");
    let urls = read["commentUrls"]
        .as_array()
        .expect("later run lists comment URLs");
    assert_eq!(urls.len(), 5, "{read:#}");
    for story in stories {
        assert!(
            urls.contains(&story["commentsUrl"]),
            "tab survives into later run: {story:#}\n{read:#}"
        );
    }
}
