//! `GET`, `PUT` and `PATCH /v1/settings`: defaults, unknown names skipped and listed, wrong
//! values refused with nothing applied or saved, and the saved file loaded again key by key. And
//! `Service::tab_hold`, the locks and fits the idle-tab janitor counts as the host's use.

use crate::support::{BoxError, Harness, Reply, TestResult, harness};
use axum::body::Body;
use axum::http::{Method, StatusCode};
use browseros_core::TabId;
use neo_service::Settings;
use neo_takeover::TabHold;
use neo_trace::{RecordPolicy, Screenshots};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::Instant;

/// Every setting at its default, as `GET` answers on a new folder.
fn defaults() -> Value {
    json!({
        "tabs": { "idleCloseSeconds": 1800 },
        "trace": { "retentionSeconds": 604_800, "screenshots": "perOperation", "valueBytes": 0 },
    })
}

/// `defaults()` with the `trace` values in `trace` set on top.
fn with_trace(trace: &Value) -> Value {
    let mut settings = defaults();
    if let (Some(into), Some(from)) = (settings["trace"].as_object_mut(), trace.as_object()) {
        into.extend(from.clone());
    }
    settings
}

impl Harness {
    async fn put(&self, body: &str) -> Result<Reply, BoxError> {
        self.call(Method::PUT, "/v1/settings", Body::from(body.to_owned()))
            .await
    }

    async fn patch(&self, body: &str) -> Result<Reply, BoxError> {
        self.call(Method::PATCH, "/v1/settings", Body::from(body.to_owned()))
            .await
    }

    /// Where the harness's settings are saved.
    fn settings_file(&self) -> PathBuf {
        self.dir.path().join("settings.json")
    }
}

#[tokio::test]
async fn get_on_a_new_folder_answers_the_defaults() -> TestResult {
    let harness = harness().await?;
    let reply = harness.get("/v1/settings").await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["settings"], defaults(), "nothing set yet");
    assert_eq!(reply.body["defaults"], defaults());
    assert!(
        reply.body.get("unknown").is_none(),
        "`unknown` belongs to a change: {}",
        reply.body
    );
    assert!(!harness.settings_file().exists(), "a read saves nothing");
    Ok(())
}

#[tokio::test]
async fn put_replaces_everything_and_patch_changes_only_what_it_names() -> TestResult {
    let harness = harness().await?;
    let put = harness
        .put(r#"{"tabs":{"idleCloseSeconds":300},"trace":{"retentionSeconds":86400}}"#)
        .await?;
    let set = json!({
        "tabs": { "idleCloseSeconds": 300 },
        "trace": { "retentionSeconds": 86_400, "screenshots": "perOperation", "valueBytes": 0 },
    });
    assert_eq!(put.status, StatusCode::OK, "{}", put.body);
    assert_eq!(put.body["settings"], set);
    assert_eq!(put.body["defaults"], defaults(), "defaults never move");
    assert_eq!(put.body["unknown"], json!([]), "every name was known");
    assert_eq!(
        put.body["unknownValues"],
        json!([]),
        "both lists, always, on a change"
    );
    let read = harness.get("/v1/settings").await?;
    assert_eq!(read.body["settings"], set, "GET agrees with the PUT");

    let patched = harness
        .patch(r#"{"tabs":{"idleCloseSeconds":600}}"#)
        .await?;
    assert_eq!(
        patched.body["settings"],
        json!({
            "tabs": { "idleCloseSeconds": 600 },
            "trace": { "retentionSeconds": 86_400, "screenshots": "perOperation", "valueBytes": 0 },
        }),
        "PATCH keeps what it does not name"
    );

    let replaced = harness.put(r#"{"tabs":{"idleCloseSeconds":600}}"#).await?;
    assert_eq!(
        replaced.body["settings"]["trace"],
        defaults()["trace"],
        "PUT sends a setting it leaves out back to its default"
    );
    let reset = harness.put("{}").await?;
    assert_eq!(reset.status, StatusCode::OK, "{}", reset.body);
    assert_eq!(
        reset.body["settings"],
        defaults(),
        "PUT {{}} is every default"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_name_is_skipped_listed_and_never_stops_the_known_ones() -> TestResult {
    let harness = harness().await?;
    let reply = harness
        .put(
            r#"{"tabs":{"idleCloseSeconds":300,"idleCloseMinutes":5},
                "files":{"readDirs":["/data"]},
                "trace":{"retentionSeconds":7200}}"#,
        )
        .await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.body["unknown"],
        json!(["files", "tabs.idleCloseMinutes"]),
        "each skipped name, at the level where it stops being a setting"
    );
    assert_eq!(
        reply.body["settings"],
        json!({
            "tabs": { "idleCloseSeconds": 300 },
            "trace": { "retentionSeconds": 7200, "screenshots": "perOperation", "valueBytes": 0 },
        }),
        "the known settings in the same call still apply"
    );
    assert_eq!(reply.body["unknownValues"], json!([]));
    let saved: Value = serde_json::from_slice(&std::fs::read(harness.settings_file())?)?;
    assert_eq!(
        saved,
        json!({ "tabs": { "idleCloseSeconds": 300 }, "trace": { "retentionSeconds": 7200 } }),
        "the file keeps what the host set, as a PUT body, and nothing it skipped"
    );
    Ok(())
}

#[tokio::test]
async fn a_wrong_value_is_refused_by_name_and_nothing_is_applied_or_saved() -> TestResult {
    let harness = harness().await?;
    let first = harness
        .patch(r#"{"tabs":{"idleCloseSeconds":300}}"#)
        .await?;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let file = harness.settings_file();
    let saved = std::fs::read(&file)?;
    let before = harness.get("/v1/settings").await?.body;

    let cases = [
        (
            r#"{"tabs":{"idleCloseSeconds":5}}"#,
            "`tabs.idleCloseSeconds` must be a whole number from 60 to 86400",
        ),
        (
            r#"{"tabs":{"idleCloseSeconds":90000}}"#,
            "`tabs.idleCloseSeconds` must be a whole number from 60 to 86400",
        ),
        (
            r#"{"tabs":{"idleCloseSeconds":"300"}}"#,
            "`tabs.idleCloseSeconds` must be a whole number",
        ),
        (
            r#"{"tabs":{"idleCloseSeconds":300.5}}"#,
            "`tabs.idleCloseSeconds` must be a whole number",
        ),
        (
            r#"{"tabs":{"idleCloseSeconds":-300}}"#,
            "`tabs.idleCloseSeconds` must be a whole number",
        ),
        (
            r#"{"tabs":{"idleCloseSeconds":null}}"#,
            "`tabs.idleCloseSeconds` must be a whole number",
        ),
        (
            r#"{"trace":{"retentionSeconds":60}}"#,
            "`trace.retentionSeconds` must be a whole number from 3600 to 2592000",
        ),
        (
            r#"{"trace":{"valueBytes":262145}}"#,
            "`trace.valueBytes` must be a whole number from 0 to 262144",
        ),
        (
            r#"{"trace":{"screenshots":5}}"#,
            "`trace.screenshots` must be one of perOperation, off",
        ),
        (
            r#"{"trace":{"screenshots":null}}"#,
            "`trace.screenshots` must be one of perOperation, off",
        ),
        (r#"{"tabs":300}"#, "`tabs` must be an object of settings"),
        ("[1]", "the body must be JSON"),
        ("not json", "the body must be JSON"),
        ("", "the body must be JSON"),
    ];
    for (body, names) in cases {
        for method in [Method::PUT, Method::PATCH] {
            let reply = harness
                .call(method.clone(), "/v1/settings", Body::from(body))
                .await?;
            reply.assert_error(StatusCode::BAD_REQUEST, "invalid_request");
            let message = reply.body["message"].as_str().unwrap_or_default();
            assert!(
                message.contains(names),
                "{method} {body}: the message names the setting and its rule: {message}"
            );
            for value in ["90000", "300.5", "\"300\"", "-300"] {
                assert!(
                    !body.contains(value) || !message.contains(value),
                    "{method} {body}: the message never repeats the value: {message}"
                );
            }
        }
    }

    // Two at fault, one fine: both are named, and the fine one is not applied either.
    let both = harness
        .put(r#"{"tabs":{"idleCloseSeconds":5},"trace":{"retentionSeconds":1},"zzz":true}"#)
        .await?;
    both.assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    let message = both.body["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("`tabs.idleCloseSeconds`") && message.contains("`trace.retentionSeconds`"),
        "every setting at fault is named: {message}"
    );
    let half = harness
        .patch(r#"{"tabs":{"idleCloseSeconds":900},"trace":{"retentionSeconds":1}}"#)
        .await?;
    half.assert_error(StatusCode::BAD_REQUEST, "invalid_request");

    assert_eq!(
        harness.get("/v1/settings").await?.body,
        before,
        "a refused call applied nothing"
    );
    assert_eq!(std::fs::read(&file)?, saved, "a refused call saved nothing");
    Ok(())
}

/// A mode a newer server has (an open list): skipped as if left out, its setting named in
/// `unknownValues`, never the value. Only a string can be such a name.
#[tokio::test]
async fn an_unknown_value_of_an_open_list_is_skipped_and_named_never_quoted() -> TestResult {
    let harness = harness().await?;
    let off = harness.patch(r#"{"trace":{"screenshots":"off"}}"#).await?;
    assert_eq!(
        off.body["settings"],
        with_trace(&json!({ "screenshots": "off" }))
    );

    let patched = harness
        .patch(r#"{"trace":{"screenshots":"blurred","valueBytes":65536}}"#)
        .await?;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.body);
    assert_eq!(patched.body["unknown"], json!([]));
    assert_eq!(patched.body["unknownValues"], json!(["trace.screenshots"]));
    assert_eq!(
        patched.body["settings"],
        with_trace(&json!({ "screenshots": "off", "valueBytes": 65_536 })),
        "PATCH: the setting keeps its value; the rest of the call applies"
    );
    assert!(
        !patched.body.to_string().contains("blurred"),
        "the value sent is never repeated: {}",
        patched.body
    );

    let put = harness
        .put(r#"{"trace":{"screenshots":"blurred"}}"#)
        .await?;
    assert_eq!(put.body["unknownValues"], json!(["trace.screenshots"]));
    assert_eq!(
        put.body["settings"],
        defaults(),
        "PUT: as if left out, so the default"
    );
    let saved = std::fs::read_to_string(harness.settings_file())?;
    assert!(
        !saved.contains("blurred"),
        "nothing skipped is saved: {saved}"
    );
    Ok(())
}

#[tokio::test]
async fn the_saved_values_come_back_in_a_store_opened_again() -> TestResult {
    let harness = harness().await?;
    let set = harness
        .patch(r#"{"tabs":{"idleCloseSeconds":300},"trace":{"retentionSeconds":3600}}"#)
        .await?;
    assert_eq!(set.status, StatusCode::OK, "{}", set.body);
    let file = harness.settings_file();
    let mode = std::fs::metadata(&file)?.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "the host's file, readable by Neo's uid alone");

    let again = Settings::load(file).await;
    assert_eq!(again.tab_idle_close(), Duration::from_secs(300));
    assert_eq!(again.trace_retention(), Duration::from_secs(3600));
    Ok(())
}

#[tokio::test]
async fn a_save_that_fails_answers_500_and_changes_nothing() -> TestResult {
    let harness = harness().await?;
    // The temporary file's path is a directory, so the save cannot open it.
    let mut blocker = harness.settings_file().into_os_string();
    blocker.push(".tmp");
    std::fs::create_dir(&blocker)?;
    let reply = harness
        .patch(r#"{"tabs":{"idleCloseSeconds":300}}"#)
        .await?;
    reply.assert_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error");
    assert_eq!(
        harness.get("/v1/settings").await?.body["settings"],
        defaults(),
        "nothing switched on that was not saved"
    );
    assert!(!harness.settings_file().exists(), "and nothing was saved");
    Ok(())
}

#[tokio::test]
async fn a_change_that_sets_nothing_new_writes_nothing() -> TestResult {
    let harness = harness().await?;
    let reply = harness.put("{}").await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(
        !harness.settings_file().exists(),
        "the host set nothing, so there is nothing to save"
    );
    Ok(())
}

#[tokio::test]
async fn a_saved_file_loads_key_by_key_and_never_stops_a_start() -> TestResult {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("settings.json");

    let missing = Settings::load(file.clone()).await;
    assert_eq!(
        missing.tab_idle_close(),
        Duration::from_mins(30),
        "no file is every default"
    );

    std::fs::write(&file, "{ this is not json")?;
    let corrupt = Settings::load(file.clone()).await;
    assert_eq!(
        corrupt.trace_retention(),
        Duration::from_hours(168),
        "a file that does not parse is every default"
    );

    std::fs::write(&file, "[1, 2]")?;
    let not_an_object = Settings::load(file.clone()).await;
    assert_eq!(not_an_object.tab_idle_close(), Duration::from_mins(30));

    // A newer build's setting, a value this build refuses, and one good value.
    std::fs::write(
        &file,
        r#"{"tabs":{"idleCloseSeconds":5,"idleCloseMinutes":5},"trace":{"retentionSeconds":7200,"screenshots":"blurred"},"files":{}}"#,
    )?;
    let mixed = Settings::load(file.clone()).await;
    assert_eq!(
        mixed.tab_idle_close(),
        Duration::from_mins(30),
        "a value out of range falls back to its default"
    );
    assert_eq!(mixed.trace_policy(), RecordPolicy::default());
    assert_eq!(
        mixed.trace_retention(),
        Duration::from_secs(7200),
        "and the others still apply"
    );

    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000))?;
    let unreadable = Settings::load(file.clone()).await;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))?;
    // Root reads any file; then the good value applies, which is also no failure.
    assert!(
        [Duration::from_secs(7200), Duration::from_hours(168)]
            .contains(&unreadable.trace_retention()),
        "a file that cannot be read starts on the defaults"
    );
    Ok(())
}

#[tokio::test]
async fn the_routes_need_the_host_auth() -> TestResult {
    let harness = harness().await?;
    // `routes` is the router without the host auth in front: it refuses everything.
    let request = axum::http::Request::builder()
        .method(Method::PUT)
        .uri("/v1/settings")
        .body(Body::from(r#"{"tabs":{"idleCloseSeconds":300}}"#))?;
    let reply =
        Reply::read(tower::ServiceExt::oneshot(harness.routes.clone(), request).await?).await?;
    reply.assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    assert_eq!(
        harness.get("/v1/settings").await?.body["settings"],
        defaults(),
        "the refused call changed nothing"
    );
    Ok(())
}

/// The trace writer is handed its policy at once and after every change, before the change
/// answers, so the next operation follows it.
#[tokio::test]
async fn a_watcher_hears_the_trace_policy_before_the_change_answers() -> TestResult {
    let harness = harness().await?;
    let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    harness.settings.watch({
        let heard = std::sync::Arc::clone(&heard);
        move |settings| {
            if let Ok(mut heard) = heard.lock() {
                heard.push(settings.trace_policy());
            }
        }
    });
    let reply = harness
        .patch(r#"{"trace":{"screenshots":"off","valueBytes":262144}}"#)
        .await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let heard = heard.lock().map_err(|_| "poisoned")?.clone();
    assert_eq!(
        heard,
        vec![
            RecordPolicy::default(),
            RecordPolicy {
                screenshots: Screenshots::Off,
                value_bytes: 262_144,
            },
        ],
        "once when watched, once for the change, before its answer"
    );
    Ok(())
}

/// The idle-tab janitor asks `Service::tab_hold` about each agent tab: a lock or a fit on it is
/// the host using it, and the instant the last one let go restarts the tab's idle clock.
#[tokio::test]
async fn tab_hold_reports_a_lock_and_a_fit_now_and_until_they_let_go() -> TestResult {
    let harness = harness().await?;
    harness.browser.open_tab(12);
    harness.browser.open_tab(13);
    let (locked, fitted) = (TabId(12), TabId(13));
    assert_eq!(harness.service.tab_hold(locked), None, "nothing holds it");

    let lock = harness
        .post("/v1/locks", r#"{"tabId":12,"ttlSeconds":60}"#)
        .await?;
    assert_eq!(lock.status, StatusCode::OK, "{}", lock.body);
    assert_eq!(harness.service.tab_hold(locked), Some(TabHold::Now));
    let id = lock.body["id"].as_str().ok_or("a lock id")?;
    let before = Instant::now();
    harness.delete(&format!("/v1/locks/{id}")).await?;
    assert!(
        matches!(harness.service.tab_hold(locked), Some(TabHold::Until(at)) if at >= before),
        "a released lock reports when it let go: {:?}",
        harness.service.tab_hold(locked)
    );

    let fit = harness
        .post("/v1/tabs/13/fit", r#"{"width":390,"height":844}"#)
        .await?;
    assert_eq!(fit.status, StatusCode::OK, "{}", fit.body);
    assert_eq!(
        harness.service.tab_hold(fitted),
        Some(TabHold::Now),
        "a fit with no lock still holds the tab: zuzu's handoff"
    );
    let id = fit.body["id"].as_str().ok_or("a fit id")?;
    let before = Instant::now();
    harness.post(&format!("/v1/fits/{id}/restore"), "").await?;
    assert!(
        matches!(harness.service.tab_hold(fitted), Some(TabHold::Until(at)) if at >= before),
        "a restored fit reports when it let go: {:?}",
        harness.service.tab_hold(fitted)
    );
    Ok(())
}

/// On the development TCP listener the token is the only check, so a script that got it could
/// call the host API from a page of the browser Neo drives. Every page request carries `Origin`
/// or `Sec-Fetch-Site`, a same-origin one too; the host's own clients send neither.
#[tokio::test]
async fn a_request_from_a_web_page_is_refused_whatever_its_token() -> TestResult {
    let harness = harness().await?;
    for (name, value) in [
        ("origin", "http://127.0.0.1:9300"),
        ("sec-fetch-site", "same-origin"),
    ] {
        for method in [Method::GET, Method::PUT, Method::PATCH] {
            let request = axum::http::Request::builder()
                .method(method.clone())
                .uri("/v1/settings")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .header(name, value)
                .body(Body::from(r#"{"tabs":{"idleCloseSeconds":300}}"#))?;
            let reply =
                Reply::read(tower::ServiceExt::oneshot(harness.router.clone(), request).await?)
                    .await?;
            reply.assert_error(StatusCode::FORBIDDEN, "forbidden");
            assert!(
                !harness.settings_file().exists(),
                "{method} with {name}: nothing saved"
            );
        }
    }
    assert_eq!(
        harness.get("/v1/settings").await?.body["settings"],
        defaults(),
        "nothing applied"
    );
    Ok(())
}
