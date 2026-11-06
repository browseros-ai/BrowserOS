//! The sweep over core's scripted connection, as Chromium sees it: the sealed world goes into
//! every frame in an order that keeps the journal working, closed shadow roots are watched in
//! their own frame's world, an out-of-process frame is sealed on its own session, a frame that
//! fails or stalls is skipped, and the secret set gets each distinct value long enough to mask.

use browseros_core::sensitive_fields::{JOURNAL_BINDING, SEALED_WORLD, seal_source};
use browseros_core::testing::{TestCall, TestConnection, cdp_error, cdp_pending};
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserSessionHooks, SessionId, TabId};
use browseros_takeover::{BrowserTabs, PartError, Swept, sweep};
use browseros_trace::{SecretSet, SecretSetConfig};
use serde_json::{Value, json};
use std::sync::Arc;

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type TestResult = Result<(), BoxError>;

const READ: &str = "__clawSeal.sensitiveValues()";
const WATCH: &str = "function () { __clawSeal.watch(this); }";

fn tab(id: i64) -> Value {
    json!({
        "tabId": id,
        "targetId": format!("target-{id}"),
        "windowId": 1,
        "url": format!("https://example.test/{id}"),
        "title": format!("Tab {id}"),
        "isActive": true,
        "isLoading": false,
        "loadProgress": 1.0,
        "isPinned": false,
        "isHidden": false,
    })
}

/// A browser with tab 3 open, answering `queued` in order per method, and whatever core's page
/// attach, its frame walk, the binding and the closed-root watch need.
fn browser(
    queued: impl IntoIterator<Item = (&'static str, Value)>,
) -> Result<Arc<TestConnection>, BoxError> {
    let connection = TestConnection::new(queued);
    connection.always("Browser.getTabs", json!({ "tabs": [tab(3)] }))?;
    connection.always("Browser.getTabInfo", json!({ "tab": tab(3) }))?;
    for method in [
        "Page.enable",
        "DOM.enable",
        "Runtime.enable",
        "Accessibility.enable",
        "Runtime.runIfWaitingForDebugger",
        "Target.setAutoAttach",
        "Runtime.addBinding",
        "Runtime.releaseObject",
    ] {
        connection.always(method, json!({}))?;
    }
    connection.always(
        "DOM.resolveNode",
        json!({ "object": { "objectId": "closed-root" } }),
    )?;
    connection.always(
        "Runtime.callFunctionOn",
        json!({ "result": { "type": "undefined" } }),
    )?;
    Ok(connection)
}

fn tabs(connection: &Arc<TestConnection>) -> BrowserTabs {
    BrowserTabs::new(
        BrowserSession::new(connection.clone(), BrowserSessionHooks::default()),
        Arc::new(Viewports::default()),
    )
}

fn secrets() -> SecretSet {
    SecretSet::new(SecretSetConfig::default())
}

fn world(context: i64) -> (&'static str, Value) {
    (
        "Page.createIsolatedWorld",
        json!({ "executionContextId": context }),
    )
}

/// The worlds one sweep of `two_frames` asks for, in order, when every frame and root takes the
/// seal: main's is 11 and the child's 12. Each frame's world is made before the binding, then
/// asked for again by every script run there: the seals, the closed roots' watches, the reads.
fn two_frame_worlds() -> [(&'static str, Value); 8] {
    [11, 12, 11, 12, 11, 12, 11, 12].map(world)
}

/// What evaluating `seal_source()` answers: the seal's version.
fn sealed() -> (&'static str, Value) {
    (
        "Runtime.evaluate",
        json!({ "result": { "type": "number", "value": 1 } }),
    )
}

/// What `__clawSeal.sensitiveValues()` answers.
fn found(values: &[&str]) -> (&'static str, Value) {
    (
        "Runtime.evaluate",
        json!({ "result": { "type": "object", "value": values } }),
    )
}

fn frame_tree(frames: &[&str]) -> Value {
    let children = frames[1..]
        .iter()
        .map(|id| json!({ "frame": { "id": id } }))
        .collect::<Vec<_>>();
    json!({ "frameTree": { "frame": { "id": frames[0] }, "childFrames": children } })
}

/// A main frame with a same-process child frame: a closed shadow root in each document (the
/// child's under an open one), which only Rust can hand the seal.
fn two_frames(connection: &TestConnection) -> TestResult {
    connection.always("Page.getFrameTree", frame_tree(&["main", "child"]))?;
    connection.always(
        "DOM.getDocument",
        json!({ "root": { "nodeName": "#document", "children": [
            { "nodeName": "DIV", "shadowRoots": [
                { "nodeName": "#document-fragment", "shadowRootType": "closed", "backendNodeId": 41 },
            ] },
            { "nodeName": "IFRAME", "frameId": "child", "contentDocument": {
                "nodeName": "#document", "children": [
                    { "nodeName": "SPAN", "shadowRoots": [
                        { "shadowRootType": "open", "backendNodeId": 43, "children": [
                            { "nodeName": "B", "shadowRoots": [
                                { "shadowRootType": "closed", "backendNodeId": 42 },
                            ] },
                        ] },
                    ] },
                ],
            } },
        ] } }),
    )?;
    Ok(())
}

/// The sealed-world steps among `calls`, one line each, with the session each went to.
fn steps(calls: &[TestCall]) -> Vec<(String, Option<SessionId>)> {
    calls
        .iter()
        .filter_map(|call| {
            let p = &call.params;
            let text = |key: &str| p[key].as_str().unwrap_or_default().to_owned();
            let line = match call.method.as_str() {
                "Page.createIsolatedWorld" => {
                    format!("world {} in {}", text("frameId"), text("worldName"))
                }
                "Runtime.addBinding" => {
                    format!(
                        "binding {} for {}",
                        text("name"),
                        text("executionContextName")
                    )
                }
                "Runtime.evaluate" if p["expression"] == seal_source() => {
                    format!("seal context {}", p["contextId"])
                }
                "Runtime.evaluate" => format!("{} context {}", text("expression"), p["contextId"]),
                "DOM.resolveNode" => format!(
                    "resolve {} into context {}",
                    p["backendNodeId"], p["executionContextId"]
                ),
                "Runtime.callFunctionOn" => {
                    // Core's PageJs wraps the function in its other-document guard.
                    let function = text("functionDeclaration");
                    let function = if function.contains(WATCH) {
                        WATCH.to_owned()
                    } else {
                        function
                    };
                    format!("{function} on {}", text("objectId"))
                }
                "Runtime.releaseObject" => format!("release {}", text("objectId")),
                _ => return None,
            };
            Some((line, call.session.clone()))
        })
        .collect()
}

fn lines(steps: &[(String, Option<SessionId>)]) -> Vec<String> {
    steps.iter().map(|(line, _)| line.clone()).collect()
}

/// The session core attached to the page: the one the frame walk starts on.
fn page_session(calls: &[TestCall]) -> Option<SessionId> {
    calls
        .iter()
        .find(|call| call.method == "Page.getFrameTree")
        .and_then(|call| call.session.clone())
}

fn masks(secrets: &SecretSet, value: &str) -> bool {
    let mut text = format!("value: {value}.");
    secrets.mask_str(&mut text)
}

#[tokio::test]
async fn every_frame_is_sealed_after_the_binding_and_closed_roots_are_watched_in_their_own_frame()
-> TestResult {
    let connection = browser(two_frame_worlds().into_iter().chain([
        sealed(),
        sealed(),
        found(&[]),
        found(&[]),
    ]))?;
    two_frames(&connection)?;

    let swept = sweep(&tabs(&connection), &secrets(), TabId(3)).await?;
    assert_eq!(
        swept,
        Swept {
            registered: 0,
            refused: 0,
            frames: 2
        }
    );

    let calls = connection.calls()?;
    let page = page_session(&calls);
    assert!(page.is_some(), "the walk ran on a session of its own");
    let steps = steps(&calls);
    assert_eq!(
        lines(&steps),
        [
            "world main in browseros-sdk".to_owned(),
            "world child in browseros-sdk".to_owned(),
            format!("binding {JOURNAL_BINDING} for {SEALED_WORLD}"),
            "world main in browseros-sdk".to_owned(),
            "seal context 11".to_owned(),
            "world child in browseros-sdk".to_owned(),
            "seal context 12".to_owned(),
            "world main in browseros-sdk".to_owned(),
            "resolve 41 into context 11".to_owned(),
            format!("{WATCH} on closed-root"),
            "release closed-root".to_owned(),
            "world child in browseros-sdk".to_owned(),
            "resolve 42 into context 12".to_owned(),
            format!("{WATCH} on closed-root"),
            "release closed-root".to_owned(),
            "world main in browseros-sdk".to_owned(),
            format!("{READ} context 11"),
            "world child in browseros-sdk".to_owned(),
            format!("{READ} context 12"),
        ],
        "the worlds, then the binding, then the seal; each closed root in its own frame's world, \
         its handle released once it is watched; the open root is left to the seal"
    );
    assert!(
        steps.iter().all(|(_, session)| *session == page),
        "a same-process child frame is reached on its page's session"
    );
    Ok(())
}

#[tokio::test]
async fn the_sweep_registers_each_distinct_value_long_enough_to_mask_and_counts_it() -> TestResult {
    let connection = browser(
        two_frame_worlds()
            .into_iter()
            .chain(two_frame_worlds())
            .chain([
                sealed(),
                sealed(),
                found(&["PWTYPED_4f7h", "987", ""]),
                found(&["4111111111111111", "PWTYPED_4f7h"]),
                sealed(),
                sealed(),
                found(&["PWTYPED_4f7h"]),
                found(&[]),
            ]),
    )?;
    two_frames(&connection)?;
    let secrets = secrets();
    let tabs = tabs(&connection);

    let swept = sweep(&tabs, &secrets, TabId(3)).await?;
    assert_eq!(
        swept,
        Swept {
            registered: 2,
            refused: 0,
            frames: 2
        }
    );
    assert!(masks(&secrets, "PWTYPED_4f7h"));
    assert!(masks(&secrets, "4111111111111111"));
    assert!(
        !masks(&secrets, "987"),
        "a 3-character value would mask ordinary text"
    );
    assert_eq!(secrets.count(), 2);

    let again = sweep(&tabs, &secrets, TabId(3)).await?;
    assert_eq!(
        again,
        Swept {
            registered: 1,
            refused: 0,
            frames: 2
        },
        "a value already masked is refreshed, and counted"
    );
    assert_eq!(secrets.count(), 2);
    Ok(())
}

#[tokio::test]
async fn a_frame_that_fails_is_skipped_and_the_rest_are_read() -> TestResult {
    let connection = browser([
        world(11),
        (
            "Page.createIsolatedWorld",
            cdp_error("No frame for given id found"),
        ),
        // The main frame's seal, its closed root's watch and its read.
        world(11),
        world(11),
        world(11),
        sealed(),
        found(&["PWTYPED_4f7h"]),
    ])?;
    two_frames(&connection)?;
    let secrets = secrets();

    let swept = sweep(&tabs(&connection), &secrets, TabId(3)).await?;
    assert_eq!(
        swept,
        Swept {
            registered: 1,
            refused: 0,
            frames: 1
        }
    );
    assert!(masks(&secrets, "PWTYPED_4f7h"));
    let lines = lines(&steps(&connection.calls()?));
    assert!(
        lines.contains(&"seal context 11".to_owned())
            && lines.contains(&format!("{READ} context 11")),
        "{lines:#?}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("resolve 42")),
        "the failed frame's closed root is not resolved into another frame's world"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_frame_that_never_answers_is_skipped_after_its_budget() -> TestResult {
    let connection = browser([
        world(11),
        world(12),
        // The two seals, then the main frame's closed root's watch and its read.
        world(11),
        world(12),
        world(11),
        world(11),
        sealed(),
        ("Runtime.evaluate", cdp_pending()),
        found(&["PWTYPED_4f7h"]),
    ])?;
    two_frames(&connection)?;
    let secrets = secrets();

    let swept = sweep(&tabs(&connection), &secrets, TabId(3)).await?;
    assert_eq!(
        swept,
        Swept {
            registered: 1,
            refused: 0,
            frames: 1
        }
    );
    assert!(masks(&secrets, "PWTYPED_4f7h"));
    Ok(())
}

#[tokio::test]
async fn an_out_of_process_frame_is_sealed_and_read_on_its_own_session() -> TestResult {
    let page_tree = frame_tree(&["main"]);
    let page_document = json!({ "root": { "nodeName": "#document", "children": [
        { "nodeName": "IFRAME", "frameId": "remote", "backendNodeId": 50 },
    ] } });
    let connection = browser([
        // The walk on the page, then core's own frame walk from the page finding "remote".
        ("Page.getFrameTree", page_tree.clone()),
        ("Page.getFrameTree", page_tree),
        ("Page.getFrameTree", frame_tree(&["remote"])),
        ("DOM.getDocument", page_document.clone()),
        ("DOM.getDocument", page_document),
        (
            "DOM.getDocument",
            json!({ "root": { "nodeName": "#document", "children": [
                { "nodeName": "FORM", "shadowRoots": [
                    { "shadowRootType": "closed", "backendNodeId": 61 },
                ] },
            ] } }),
        ),
        // The page's world, then its seal; the frame's world, then its seal and its closed
        // root's watch; then each frame's read.
        world(11),
        world(11),
        world(21),
        world(21),
        world(21),
        world(11),
        world(21),
        sealed(),
        sealed(),
        found(&["PWTYPED_4f7h"]),
        found(&["PWIFRAME_7y7y"]),
    ])?;
    connection.always(
        "DOM.getFrameOwner",
        json!({ "backendNodeId": 50, "frameId": "remote" }),
    )?;
    connection.always(
        "DOM.describeNode",
        json!({ "node": { "nodeName": "IFRAME", "backendNodeId": 50 } }),
    )?;
    let secrets = secrets();

    let swept = sweep(&tabs(&connection), &secrets, TabId(3)).await?;
    assert_eq!(
        swept,
        Swept {
            registered: 2,
            refused: 0,
            frames: 2
        }
    );
    assert!(masks(&secrets, "PWIFRAME_7y7y"));

    let calls = connection.calls()?;
    let page = page_session(&calls);
    let steps = steps(&calls);
    let remote = steps
        .iter()
        .find(|(line, _)| line == "world remote in browseros-sdk")
        .and_then(|(_, session)| session.clone());
    assert!(
        remote.is_some() && remote != page,
        "the frame is sealed on its own target's session"
    );
    let on_remote = steps
        .iter()
        .filter(|(_, session)| *session == remote)
        .map(|(line, _)| line.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        on_remote,
        [
            "world remote in browseros-sdk".to_owned(),
            format!("binding {JOURNAL_BINDING} for {SEALED_WORLD}"),
            "world remote in browseros-sdk".to_owned(),
            "seal context 21".to_owned(),
            "world remote in browseros-sdk".to_owned(),
            "resolve 61 into context 21".to_owned(),
            format!("{WATCH} on closed-root"),
            "release closed-root".to_owned(),
            "world remote in browseros-sdk".to_owned(),
            format!("{READ} context 21"),
        ]
    );
    Ok(())
}

/// A resolved root is a remote object that lives as long as its document unless released, and
/// every sweep and every arming resolves the roots again.
#[tokio::test]
async fn each_closed_roots_handle_is_released_whether_or_not_its_watch_ran() -> TestResult {
    let connection = browser(two_frame_worlds().into_iter().chain([
        sealed(),
        sealed(),
        (
            "DOM.resolveNode",
            json!({ "object": { "objectId": "root-41" } }),
        ),
        (
            "DOM.resolveNode",
            json!({ "object": { "objectId": "root-42" } }),
        ),
        (
            "Runtime.callFunctionOn",
            json!({ "exceptionDetails": { "text": "__clawSeal is not defined" } }),
        ),
        ("Runtime.callFunctionOn", cdp_error("Cannot find context")),
        found(&[]),
        found(&[]),
    ]))?;
    two_frames(&connection)?;

    sweep(&tabs(&connection), &secrets(), TabId(3)).await?;
    let lines = lines(&steps(&connection.calls()?));
    let watched = |root: &str| {
        let watch = format!("{WATCH} on {root}");
        let release = format!("release {root}");
        let at = |line: &String| lines.iter().position(|seen| seen == line);
        at(&watch).is_some() && at(&watch) < at(&release)
    };
    assert!(
        watched("root-41") && watched("root-42"),
        "a watch that threw, and one that failed, each released afterwards: {lines:#?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("release"))
            .count(),
        2,
        "one release per resolved root"
    );
    Ok(())
}

#[tokio::test]
async fn a_tab_that_is_not_open_or_cannot_be_read_fails_the_sweep() -> TestResult {
    let connection = browser([])?;
    two_frames(&connection)?;
    assert!(matches!(
        sweep(&tabs(&connection), &secrets(), TabId(9)).await,
        Err(PartError::TabNotFound(TabId(9)))
    ));

    connection.always(
        "Page.createIsolatedWorld",
        cdp_error("No frame for given id found"),
    )?;
    assert!(
        matches!(
            sweep(&tabs(&connection), &secrets(), TabId(3)).await,
            Err(PartError::Browser(_))
        ),
        "a sweep that read no frame did not happen"
    );

    connection.always("Page.getFrameTree", cdp_error("Target closed"))?;
    assert!(matches!(
        sweep(&tabs(&connection), &secrets(), TabId(3)).await,
        Err(PartError::Browser(_))
    ));

    connection.always("Browser.getTabs", cdp_error("Browser closed"))?;
    assert!(matches!(
        sweep(&tabs(&connection), &secrets(), TabId(3)).await,
        Err(PartError::Browser(_))
    ));
    Ok(())
}
