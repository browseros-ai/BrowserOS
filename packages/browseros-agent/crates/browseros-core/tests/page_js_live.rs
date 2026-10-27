//! `PageJs` against a real browser, which the unit tests cannot stand in for: whether Chromium
//! reuses the isolated world by name, keeps its globals from the page, lets its MutationObserver
//! see the page's changes, and hides page-assigned handlers from it.
//!
//! Needs a BrowserOS (or Chromium) instance with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p browseros-core --test page_js_live -- --ignored`. The test opens and closes
//! its own tab. Its own binary, so turning the process-wide switch on cannot leak into others.

use browseros_cdp::{CdpClient, ConnectOptions};
use browseros_core::{
    FrameId, PageJs, ProtocolSession, SessionId, page_js::set_isolated_world_enabled, settle,
};
use serde_json::{Value, json};
use std::sync::Arc;

const PAGE: &str =
    "data:text/html,<button id=b>go</button><iframe id=f srcdoc='<p id=p>child</p>'></iframe>";

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_isolated_scripts_share_the_dom_but_not_javascript_with_the_page()
-> Result<(), Box<dyn std::error::Error>> {
    set_isolated_world_enabled(true);
    let port = std::env::var("BROWSEROS_CDP_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(9005);
    let client = Arc::new(CdpClient::connect(ConnectOptions::new(port)).await?);
    let root = ProtocolSession::root(client.clone());
    let target = root
        .send_value("Target.createTarget", json!({ "url": PAGE }))
        .await?;
    let target_id = target["targetId"]
        .as_str()
        .ok_or("no target id")?
        .to_string();
    let result = exercise(&client, &root, &target_id).await;
    let _ = root
        .send_value("Target.closeTarget", json!({ "targetId": target_id }))
        .await;
    result
}

async fn exercise(
    client: &Arc<CdpClient>,
    root: &ProtocolSession,
    target_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let attached = root
        .send_value(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
        )
        .await?;
    let session_id = attached["sessionId"].as_str().ok_or("no session id")?;
    let page = ProtocolSession::for_session(client.clone(), SessionId::from(session_id));
    for method in ["Page.enable", "DOM.enable", "Runtime.enable"] {
        page.send_value(method, json!({})).await?;
    }
    wait_for_child_frame(&page).await?;

    // Globals stay on their own side, and the isolated world persists between calls.
    let _: Value = PageJs::isolated(&page)
        .eval("window.__isolatedOnly = 1")
        .await?;
    let _: Value = PageJs::page_world(&page)
        .eval("window.__pageOnly = 1")
        .await?;
    let seen_by_page: String = PageJs::page_world(&page)
        .eval("typeof window.__isolatedOnly")
        .await?;
    let seen_by_isolated: String = PageJs::isolated(&page)
        .eval("typeof window.__pageOnly")
        .await?;
    let kept: i64 = PageJs::isolated(&page)
        .eval("window.__isolatedOnly")
        .await?;
    assert_eq!(seen_by_page, "undefined");
    assert_eq!(seen_by_isolated, "undefined");
    assert_eq!(kept, 1);

    // The readiness probe's observer lives in the isolated world and still counts page changes.
    let before = settle::read_readiness(&page).await?;
    let _: Value = PageJs::page_world(&page)
        .eval("document.body.append(document.createElement('div'))")
        .await?;
    let after = settle::read_readiness(&page).await?;
    assert!(after.count > before.count, "{before:?} -> {after:?}");
    let probe_seen_by_page: String = PageJs::page_world(&page)
        .eval("typeof globalThis.__browseros_ready")
        .await?;
    assert_eq!(probe_seen_by_page, "undefined");

    // Element calls see the DOM, but not a handler the page assigned from script.
    let button = backend_node(&page, "#b").await?;
    let _: Value = PageJs::page_world(&page)
        .eval("document.getElementById('b').onclick = () => {}")
        .await?;
    let tag: String = PageJs::isolated(&page)
        .call(button, "function(){return this.tagName}", &[])
        .await?;
    let isolated_handler: bool = PageJs::isolated(&page)
        .call(button, "function(){return this.onclick !== null}", &[])
        .await?;
    let page_handler: bool = PageJs::page_world(&page)
        .call(button, "function(){return this.onclick !== null}", &[])
        .await?;
    assert_eq!(tag, "BUTTON");
    assert!(!isolated_handler);
    assert!(page_handler);

    // A node in a same-process child frame runs where its own `document` is.
    let paragraph = backend_node(&page, "#p").await?;
    let own_document: bool = PageJs::isolated(&page)
        .call(
            paragraph,
            "function(){return this.ownerDocument === document}",
            &[],
        )
        .await?;
    assert!(own_document);
    let child = child_frame(&page).await?;
    let child_url: String = PageJs::isolated(&page)
        .in_frame(&child)
        .eval("location.href")
        .await?;
    let child_url_main: String = PageJs::page_world(&page)
        .in_frame(&child)
        .eval("location.href")
        .await?;
    assert_eq!(child_url, "about:srcdoc");
    assert_eq!(child_url_main, "about:srcdoc");

    // A navigation replaces the document; the next isolated call gets the new one's world.
    page.send_value(
        "Page.navigate",
        json!({ "url": "data:text/html,<p>second</p>" }),
    )
    .await?;
    let mut text = String::new();
    for _ in 0..50 {
        text = PageJs::isolated(&page)
            .eval::<Option<String>>("document.body && document.body.innerText")
            .await?
            .unwrap_or_default();
        if text == "second" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(text, "second");
    let fresh: String = PageJs::isolated(&page)
        .eval("typeof window.__isolatedOnly")
        .await?;
    assert_eq!(fresh, "undefined");
    Ok(())
}

async fn backend_node(
    page: &ProtocolSession,
    selector: &str,
) -> Result<i64, Box<dyn std::error::Error>> {
    let document = page
        .send_value("DOM.getDocument", json!({ "depth": -1, "pierce": true }))
        .await?;
    let found = find_by_id(&document["root"], selector.trim_start_matches('#'))
        .ok_or_else(|| format!("{selector} not in the document"))?;
    Ok(found)
}

fn find_by_id(node: &Value, id: &str) -> Option<i64> {
    let attributes = node["attributes"].as_array();
    let matches = attributes.is_some_and(|attributes| {
        attributes
            .chunks(2)
            .any(|pair| pair[0] == "id" && pair.get(1).is_some_and(|value| value == id))
    });
    if matches {
        return node["backendNodeId"].as_i64();
    }
    for key in ["children", "shadowRoots"] {
        for child in node[key].as_array().into_iter().flatten() {
            if let Some(found) = find_by_id(child, id) {
                return Some(found);
            }
        }
    }
    node.get("contentDocument")
        .and_then(|document| find_by_id(document, id))
}

async fn child_frame(page: &ProtocolSession) -> Result<FrameId, Box<dyn std::error::Error>> {
    let tree = page.send_value("Page.getFrameTree", json!({})).await?;
    let id = tree["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .ok_or("no child frame")?;
    Ok(FrameId(id.to_string()))
}

async fn wait_for_child_frame(page: &ProtocolSession) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..50 {
        let ready: bool = PageJs::page_world(page)
            .eval("!!document.getElementById('f')?.contentDocument?.getElementById('p')")
            .await?;
        if ready {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Err("the child frame never loaded".into())
}
