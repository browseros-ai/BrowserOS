//! Hydration facts a scripted tab cannot show, against a real browser: an element the page moves
//! into a same-origin iframe still resolves in the main frame's isolated world and still takes
//! the probe's root pin, so only the in-document check refuses it (design 6.2).
//!
//! Needs BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --lib tree::live_tests -- --ignored`. Each test opens and
//! closes its own tab and serves its page on 127.0.0.1.

use std::collections::HashMap;

use neo_snapshot_apc::tree::DomNodeId;
use serde_json::{Value, json};

use super::hydrate::{self, ObjectPin};
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeReply, ProbeRequest, ProbeTarget, Want};
use crate::target::{CallHeader, Strictness};
use crate::test_support::fixture_server::{FixtureServer, Page};
use crate::test_support::{LiveBrowser, TestResult, count_op};
use crate::world::{BackendNodeId, Place};

const MOVING_PAGE: &str = r#"<!doctype html><title>moving</title>
<button id="pay">Pay</button>
<iframe id="frame" src="/child.html"></iframe>"#;

/// Moves `#pay` into the iframe's document, and whether it is now connected there.
const MOVE_INTO_FRAME: &str = "(() => { const pay = document.getElementById('pay'); \
     document.getElementById('frame').contentDocument.body.appendChild(pay); \
     return pay.isConnected && pay.ownerDocument !== document; })()";

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn an_element_moved_into_a_same_origin_iframe_no_longer_pins_or_keys() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(HashMap::from([
        ("/moving.html".to_owned(), Page::html(MOVING_PAGE)),
        (
            "/child.html".to_owned(),
            Page::html("<!doctype html><title>child</title><p>child frame</p>"),
        ),
    ]))
    .await?;
    let tab = live.open(&server.url("127.0.0.1", "/moving.html")).await?;
    let env = live.env();
    let scope = OpScope::new(&env, count_op()?, CallHeader::default());
    let page = OpPage::admit(&live.playwright, &scope, tab.id).await?;
    let world = page.world(&page.main_endpoint(), Place::Main).await?;

    let button: Value = tab
        .session
        .send(
            "Runtime.evaluate",
            json!({ "expression": "document.getElementById('pay')" }),
        )
        .await?;
    let button = button["result"]["objectId"]
        .as_str()
        .ok_or("the page's own world hands the button over by handle")?;
    let described: Value = tab
        .session
        .send("DOM.describeNode", json!({ "objectId": button }))
        .await?;
    let backend = described["node"]["backendNodeId"]
        .as_i64()
        .ok_or("the button has a backend node id")?;
    let dom = DomNodeId::new(i32::try_from(backend)?);
    let held = Some(vec![BackendNodeId(backend)]);

    let object = hydrate::resolve_node(&scope, &world, dom)
        .await?
        .ok_or("the button resolves in the main frame's world")?;
    let ObjectPin::Pinned { pin, .. } = hydrate::pin_object(&scope, &page, &world, &object).await?
    else {
        return Err("the button, in the world's own document, pins".into());
    };
    assert_eq!(
        probe::pinned_nodes_in_document(&scope, &page, &world, vec![pin]).await?,
        held,
        "the in-document batch answers the pinned button while it is in the world's document"
    );

    assert_eq!(
        tab.main_world(MOVE_INTO_FRAME).await?,
        json!(true),
        "the page moved the button into the iframe's document, where it is connected"
    );
    let moved = hydrate::resolve_node(&scope, &world, dom)
        .await?
        .ok_or("DOM.resolveNode still resolves the moved button in the main frame's world")?;
    let root_pin = ProbeRequest::Resolve {
        target: ProbeTarget::Root,
        strictness: Strictness::Strict,
        want: Want::Element,
    };
    let probed = probe::call_on(&scope, &page, &world, &root_pin, &moved).await?;
    assert!(
        matches!(probed, ProbeReply::Element { .. }),
        "the probe's root pin checks only isConnected, so it still pins the moved button: `{}`",
        probed.kind()
    );
    assert!(
        matches!(
            hydrate::pin_object(&scope, &page, &world, &moved).await?,
            ObjectPin::Detached
        ),
        "pin_object refuses an element of another document"
    );
    assert_eq!(
        probe::pinned_nodes_in_document(&scope, &page, &world, vec![pin]).await?,
        None,
        "the in-document batch answers no element for the first pin once the button moved"
    );
    assert_eq!(
        probe::pinned_nodes(&scope, &page, &world, vec![pin]).await?,
        held,
        "while the held batch still answers it, as its callers rely on"
    );
    live.close(&tab).await;
    Ok(())
}
