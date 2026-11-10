//! What the control journal logs, alone in its own test binary. tracing caches a callsite's
//! interest from the first thread that reaches it while one subscriber is registered, so another
//! test that reached the journal's warnings first, on its own thread, would hide them from this
//! capture.

use browseros_core::sensitive_fields::JOURNAL_BINDING;
use browseros_core::testing::{TestConnection, cdp_error};
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserSessionHooks, TabId};
use neo_takeover::testing::add_owner_presses;
use neo_takeover::{BrowserTabs, Journal};
use neo_trace::{Registration, SecretSet, SecretSetConfig, SecretSource, SecretValue};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Everything logged on the test's thread, which runs the journal's listener too.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Log {
    fn text(&self) -> Result<String, BoxError> {
        let bytes = self.0.lock().map_err(|_| "the log lock was poisoned")?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

impl std::io::Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("the log lock was poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Log {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Tab 3 with one frame, which arms but then takes the journal down every path that logs: its
/// catch-up sweep fails and its new-document script cannot be removed.
fn browser() -> Result<Arc<TestConnection>, BoxError> {
    let connection = TestConnection::new([
        (
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "identifier": "page-script" }),
        ),
        (
            "Runtime.evaluate",
            json!({ "result": { "type": "number", "value": 1 } }),
        ),
    ]);
    connection.always(
        "Browser.getTabs",
        json!({ "tabs": [{
            "tabId": 3, "targetId": "target-3", "windowId": 1, "url": "https://example.test/3",
            "title": "Tab 3", "isActive": true, "isLoading": false, "loadProgress": 1.0,
            "isPinned": false, "isHidden": false,
        }] }),
    )?;
    for method in [
        "Page.enable",
        "DOM.enable",
        "Runtime.enable",
        "Accessibility.enable",
        "Runtime.runIfWaitingForDebugger",
        "Target.setAutoAttach",
        "Runtime.addBinding",
    ] {
        connection.always(method, json!({}))?;
    }
    connection.always(
        "Page.getFrameTree",
        json!({ "frameTree": { "frame": { "id": "main" } } }),
    )?;
    connection.always(
        "DOM.getDocument",
        json!({ "root": { "nodeName": "#document" } }),
    )?;
    connection.always(
        "Page.createIsolatedWorld",
        json!({ "executionContextId": 11 }),
    )?;
    connection.always("Runtime.evaluate", cdp_error("the page went away"))?;
    connection.always(
        "Page.removeScriptToEvaluateOnNewDocument",
        cdp_error("the target went away"),
    )?;
    Ok(connection)
}

#[tokio::test(start_paused = true)]
async fn no_value_reaches_the_log() -> Result<(), BoxError> {
    let log = Log::default();
    let _logging = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(log.clone())
            .finish(),
    );
    let connection = browser()?;
    // Full of a value nothing may push out, so the page's value below is refused, and logged.
    let secrets = Arc::new(SecretSet::new(SecretSetConfig {
        max_entries: 1,
        ..SecretSetConfig::default()
    }));
    assert_eq!(
        secrets.register_from(
            SecretValue::new("HOSTHELD_6c1d".into()),
            None,
            SecretSource::Host
        ),
        Registration::Registered
    );
    let tabs = BrowserTabs::new(
        BrowserSession::new(connection.clone(), BrowserSessionHooks::default()),
        Arc::new(Viewports::default()),
    );
    let journal = Journal::new(tabs, Arc::clone(&secrets));
    let armed = journal.arm(&[TabId(3)]).await;
    let page = connection
        .calls()?
        .into_iter()
        .find(|call| call.method == "Page.getFrameTree")
        .and_then(|call| call.session)
        .ok_or("the page was never walked")?;
    let enter = |value: &str, trusted: bool| {
        let payload = json!({ "kind": "password", "value": value, "trusted": trusted });
        connection.emit(
            "Runtime.bindingCalled",
            json!({ "name": JOURNAL_BINDING, "payload": payload.to_string() }),
            Some(page.clone()),
        );
    };
    // The owner's keys on a control display: each one a press, then the entry it makes.
    let owner_types = |value: &str| {
        for (start, key) in value.char_indices() {
            add_owner_presses(&journal, 1);
            enter(&value[..start + key.len_utf8()], true);
        }
    };

    let values = ["QUIETVAL_4e2a", "LAGGEDVAL_8b3f", "DISARMVAL_1f9c"];
    owner_types(values[0]);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let refused = "SYNTHVAL_5d7e";
    enter(refused, false);
    tokio::time::sleep(Duration::from_secs(2)).await;
    owner_types(values[1]);
    tokio::time::sleep(Duration::from_millis(10)).await;
    for _ in 0..80 {
        connection.emit("Test.filler", json!({}), None);
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    owner_types(values[2]);
    armed.disarm().await;

    let text = log.text()?;
    for line in [
        "fell behind",
        "catch-up sweep failed",
        "was not removed",
        "masked nowhere refused=1",
    ] {
        assert!(text.contains(line), "the journal logged \"{line}\": {text}");
    }
    for value in values {
        let mut masked = value.to_owned();
        assert!(secrets.mask_str(&mut masked), "{value} was registered");
    }
    for value in values.into_iter().chain([refused]) {
        for end in 4..=value.len() {
            assert!(
                !text.contains(&value[..end]),
                "the log holds {}: {text}",
                &value[..end]
            );
        }
    }
    assert!(
        !text.contains("\"kind\""),
        "the log holds a payload: {text}"
    );
    Ok(())
}
