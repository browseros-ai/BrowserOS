//! The decisions worker client: what leaves this server for the worker, and with which licence.

#[path = "support/fake_decisions.rs"]
mod fake_decisions;
#[path = "support/fake_gateway.rs"]
mod fake_gateway;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use browseros_decide::{
    client::{JevError, Question},
    pursue::Oracle,
    writer::{Field, Writer},
};
use claw_server::services::{
    billing::{Billing, Secret},
    decisions::Decisions,
};
use fake_decisions::{FakeDecisions, JEV_ROUTE, WRITER_ROUTE};
use fake_gateway::{FakeGateway, KEY};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// Billing on an ultrafast plan, through the fake gateway, as a computer that pasted its key.
async fn ultrafast_billing(dir: &std::path::Path) -> anyhow::Result<(FakeGateway, Arc<Billing>)> {
    let gateway = FakeGateway::start().await?;
    gateway.set_plan(Some(FakeGateway::active_plan()));
    let billing = Arc::new(
        Billing::open(
            Some(gateway.config()?),
            dir.to_path_buf(),
            CancellationToken::new(),
        )
        .await,
    );
    billing
        .activate(Secret::new(KEY.to_string()), None)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    Ok((gateway, billing))
}

fn field() -> Field {
    Field {
        role: "searchbox".to_string(),
        name: "Search the shop".to_string(),
        value: None,
        site: "https://shop.test".to_string(),
        title: "Shop".to_string(),
    }
}

#[tokio::test]
async fn the_licence_travels_as_its_three_signed_parts() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (_gateway, billing) = ultrafast_billing(dir.path()).await?;
    let signed = billing
        .signed_licence()
        .await
        .ok_or_else(|| anyhow::anyhow!("no licence after activating"))?;
    assert_eq!(
        signed.bearer(),
        format!("{}.{}.{}", signed.licence, signed.signature, signed.kid)
    );
    assert!(billing.has_feature("claw.ultrafast").await);
    Ok(())
}

#[tokio::test]
async fn jev_is_asked_through_the_worker_with_the_stored_licence() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (_gateway, billing) = ultrafast_billing(dir.path()).await?;
    let worker = FakeDecisions::start().await?;
    let decisions = Decisions::new(Some(worker.url()?));
    let oracle = decisions
        .oracle(billing.clone())
        .await
        .ok_or_else(|| anyhow::anyhow!("no oracle with a licence and a worker"))?;

    let mut asked = BTreeMap::new();
    asked.insert(
        "operation".to_string(),
        Question::Noul {
            instructions: json!("is this a page?"),
            criteria: None,
        },
    );
    oracle
        .ask_within(&json!({ "page": "state" }), &asked, Duration::from_secs(5))
        .await?;

    let requests = worker.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, JEV_ROUTE);
    let expected = billing.signed_licence().await.map(|signed| signed.bearer());
    assert_eq!(requests[0].bearer, expected);
    assert_eq!(requests[0].body["state"], json!({ "page": "state" }));
    Ok(())
}

#[tokio::test]
async fn the_writer_gets_only_the_goal_and_the_field() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (_gateway, billing) = ultrafast_billing(dir.path()).await?;
    let worker = FakeDecisions::start().await?;
    let writer = Decisions::new(Some(worker.url()?))
        .writer(billing.clone())
        .ok_or_else(|| anyhow::anyhow!("no writer with a worker"))?;

    let values = writer.candidates("find sensodyne", &field()).await?;

    assert_eq!(values, vec!["sensodyne", "sensodyne fresh mint"]);
    let requests = worker.requests();
    assert_eq!(requests[0].path, WRITER_ROUTE);
    assert_eq!(
        requests[0].body,
        json!({
            "goal": "find sensodyne",
            "field": {
                "role": "searchbox",
                "name": "Search the shop",
                "value": null,
                "site": "https://shop.test",
                "title": "Shop",
            },
        })
    );
    Ok(())
}

#[tokio::test]
async fn a_refused_licence_is_refreshed_once_and_sent_again() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (gateway, billing) = ultrafast_billing(dir.path()).await?;
    let worker = FakeDecisions::start().await?;
    worker.script(WRITER_ROUTE, 401, json!({ "code": "licence_expired" }));
    let writer = Decisions::new(Some(worker.url()?))
        .writer(billing.clone())
        .ok_or_else(|| anyhow::anyhow!("no writer with a worker"))?;
    let refreshes = gateway.count("POST", "/v1/licence/refresh");

    let values = writer.candidates("find sensodyne", &field()).await?;

    assert_eq!(values.len(), 2);
    let requests = worker.requests();
    assert_eq!(requests.len(), 2);
    assert_ne!(
        requests[0].bearer, requests[1].bearer,
        "the retry carries the new licence"
    );
    assert_eq!(
        requests[1].bearer,
        billing.signed_licence().await.map(|signed| signed.bearer())
    );
    assert_eq!(gateway.count("POST", "/v1/licence/refresh"), refreshes + 1);
    Ok(())
}

#[tokio::test]
async fn a_licence_refused_twice_is_not_tried_a_third_time() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (_gateway, billing) = ultrafast_billing(dir.path()).await?;
    let worker = FakeDecisions::start().await?;
    worker.script(WRITER_ROUTE, 401, json!({ "code": "unauthorized" }));
    worker.script(WRITER_ROUTE, 401, json!({ "code": "unauthorized" }));
    let writer = Decisions::new(Some(worker.url()?))
        .writer(billing)
        .ok_or_else(|| anyhow::anyhow!("no writer with a worker"))?;

    assert!(writer.candidates("find sensodyne", &field()).await.is_err());
    assert_eq!(worker.requests().len(), 2);
    Ok(())
}

#[tokio::test]
async fn without_a_licence_nothing_is_sent() -> anyhow::Result<()> {
    let worker = FakeDecisions::start().await?;
    let decisions = Decisions::new(Some(worker.url()?));
    let billing = Arc::new(
        Billing::open(
            None,
            tempfile::tempdir()?.path().to_path_buf(),
            CancellationToken::new(),
        )
        .await,
    );

    assert!(decisions.oracle(billing.clone()).await.is_none());
    let writer = decisions
        .writer(billing)
        .ok_or_else(|| anyhow::anyhow!("no writer with a worker"))?;
    assert!(writer.candidates("find sensodyne", &field()).await.is_err());
    assert!(worker.requests().is_empty());
    Ok(())
}

#[test]
fn hosted_servers_have_no_worker_and_release_builds_wait_for_launch() {
    assert!(Decisions::for_build(true).url().is_none());
    if std::env::var_os("CLAW_DECISIONS_URL").is_none() && !cfg!(feature = "billing-staging") {
        assert!(Decisions::for_build(false).url().is_none());
    }
}

#[cfg(feature = "billing-staging")]
#[test]
fn a_staging_build_uses_the_staging_worker() {
    if std::env::var_os("CLAW_DECISIONS_URL").is_none() {
        assert_eq!(
            Decisions::for_build(false).url().map(url::Url::as_str),
            Some("https://decisions-api-worker-staging.eng-6b4.workers.dev/")
        );
    }
}

fn one_question() -> BTreeMap<String, Question> {
    let mut asked = BTreeMap::new();
    asked.insert(
        "operation".to_string(),
        Question::Noul {
            instructions: json!("is this a page?"),
            criteria: None,
        },
    );
    asked
}

#[tokio::test]
async fn a_paused_worker_comes_back_as_a_server_error() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (_gateway, billing) = ultrafast_billing(dir.path()).await?;
    let worker = FakeDecisions::start().await?;
    worker.script(JEV_ROUTE, 503, json!({ "code": "paused" }));
    let oracle = Decisions::new(Some(worker.url()?))
        .oracle(billing)
        .await
        .ok_or_else(|| anyhow::anyhow!("no oracle"))?;

    let refused = oracle
        .ask_within(&json!({}), &one_question(), Duration::from_secs(10))
        .await;

    assert!(
        matches!(refused, Err(JevError::ServerError { status: 503 })),
        "{refused:?}"
    );
    assert_eq!(worker.requests().len(), 1);
    Ok(())
}

#[tokio::test]
async fn a_plan_without_ultrafast_is_refused_once_and_not_retried() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (_gateway, billing) = ultrafast_billing(dir.path()).await?;
    let worker = FakeDecisions::start().await?;
    worker.script(JEV_ROUTE, 403, json!({ "code": "not_entitled" }));
    let oracle = Decisions::new(Some(worker.url()?))
        .oracle(billing)
        .await
        .ok_or_else(|| anyhow::anyhow!("no oracle"))?;

    let refused = oracle
        .ask_within(&json!({}), &one_question(), Duration::from_secs(10))
        .await;

    assert!(refused.is_err());
    assert_eq!(worker.requests().len(), 1);
    Ok(())
}
