//! In-process coverage of the feedback invitation routes.
//!
//! These drive the real router over real app state, so the checks under test are the ones a
//! cockpit would actually hit: consent, cohort membership and the once-ever rule.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use claw_server_rust::{
    AppState, build_router, config::Config, services::feedback_cohort::CohortDocument,
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tower::ServiceExt;

const INVITATION: &str = "/api/v1/feedback/invitation";
const DEFAULT_BOOK_URL: &str = "https://cal.com/team/felafax/browseros";

#[tokio::test]
async fn an_installation_outside_the_cohort_is_never_invited() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    join_cohort(&app, &["someone-else"]).await?;

    let (status, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invitation, json!({ "eligible": false }));
    Ok(())
}

#[tokio::test]
async fn nobody_is_invited_before_a_cohort_is_published() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;

    let (status, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invitation, json!({ "eligible": false }));
    Ok(())
}

#[tokio::test]
async fn a_cohort_member_is_invited_and_reading_the_answer_does_not_spend_it() -> anyhow::Result<()>
{
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    let (status, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invitation["eligible"], true);
    assert_eq!(invitation["bookUrl"], DEFAULT_BOOK_URL);

    // A second page load before anything is recorded still sees it: the invitation is
    // spent by an outcome, not by asking.
    let (_, again) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(again["eligible"], true);
    Ok(())
}

#[tokio::test]
async fn the_published_cohort_can_override_the_booking_link() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    app.state
        .feedback_cohort
        .adopt(
            cohort_document(
                now_ms(),
                &[install_id.as_str()],
                Some("https://cal.test/book"),
            ),
            now_ms(),
        )
        .await;

    let (_, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(invitation["bookUrl"], "https://cal.test/book");
    Ok(())
}

#[tokio::test]
async fn an_impression_spends_the_invitation_and_survives_a_restart() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    let (status, recorded) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "shown" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recorded, json!({ "eligible": false }));

    let (_, after) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(after, json!({ "eligible": false }));

    // The rule has to outlive the process that recorded it, or a restart re-invites.
    drop(app);
    let restarted = test_app(dir.path()).await?;
    let same_install = install_id_of(&restarted).await;
    assert_eq!(
        same_install, install_id,
        "the install id outlives the process"
    );
    join_cohort(&restarted, &[same_install.as_str()]).await?;
    let (_, after_restart) = request(&restarted.router, "GET", INVITATION, None).await?;
    assert_eq!(after_restart, json!({ "eligible": false }));
    Ok(())
}

#[tokio::test]
async fn a_dismissal_is_final() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "shown" })),
    )
    .await?;
    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "dismissed" })),
    )
    .await?;

    let (_, after) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(after, json!({ "eligible": false }));
    Ok(())
}

/// Consent gates this exactly as it gates the failure reports. An installation that opted
/// out of analytics has not agreed to be contacted either.
#[tokio::test]
async fn an_installation_that_opted_out_is_not_invited() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    let (status, _) = request(
        &app.router,
        "PUT",
        "/api/v1/settings/telemetry",
        Some(json!({ "consent": false })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    let (_, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(invitation, json!({ "eligible": false }));

    // Nothing is written either, so consenting again later still leaves the invitation
    // available rather than silently spent while the reader was opted out.
    request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "shown" })),
    )
    .await?;
    request(
        &app.router,
        "PUT",
        "/api/v1/settings/telemetry",
        Some(json!({ "consent": true })),
    )
    .await?;
    let (_, restored) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(restored["eligible"], true);
    Ok(())
}

#[tokio::test]
async fn an_unknown_outcome_is_refused() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let app = test_app(dir.path()).await?;
    let install_id = install_id_of(&app).await;
    join_cohort(&app, &[install_id.as_str()]).await?;

    let (status, _) = request(
        &app.router,
        "POST",
        INVITATION,
        Some(json!({ "outcome": "interested" })),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A refused body records nothing, so the invitation is still there.
    let (_, invitation) = request(&app.router, "GET", INVITATION, None).await?;
    assert_eq!(invitation["eligible"], true);
    Ok(())
}

struct TestApp {
    router: Router,
    state: AppState,
}

async fn test_app(root: &Path) -> anyhow::Result<TestApp> {
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: root.join("resources"),
        browserclaw_dir: root.join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
    });
    let state = AppState::new_with_home(config, root.join("home")).await?;
    Ok(TestApp {
        router: build_router(state.clone()),
        state,
    })
}

async fn install_id_of(app: &TestApp) -> String {
    app.state.analytics.get_state().await.distinct_id
}

async fn join_cohort(app: &TestApp, installs: &[&str]) -> anyhow::Result<()> {
    let now = now_ms();
    anyhow::ensure!(
        app.state
            .feedback_cohort
            .adopt(cohort_document(now, installs, None), now)
            .await,
        "fixture cohort was refused"
    );
    Ok(())
}

fn cohort_document(
    generated_at_ms: i64,
    installs: &[&str],
    book_url: Option<&str>,
) -> CohortDocument {
    serde_json::from_value(json!({
        "generated_at_ms": generated_at_ms,
        "installs": installs,
        "copy": { "book_url": book_url },
    }))
    .unwrap_or_else(|error| panic!("fixture: {error}"))
}

fn now_ms() -> i64 {
    claw_server_rust::services::feedback_cohort::now_ms()
}

async fn request(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost");
    let request_body = if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = router.clone().oneshot(builder.body(request_body)?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}
