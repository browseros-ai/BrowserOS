//! The host side's CDP surface in one marked place: every command it sends on its own socket,
//! every event it reads there, and every constant it ports, each with its source. The modules
//! behind it see domain types only.
//!
//! A command is typed through browseros-cdp's surface where the surface has its method; the
//! design's "CDP methods and events" table lists the untyped ones for browseros-cdp's
//! `protocol/surface.json`. Commands on an attached session take its [`Attached`], whose `send`
//! writes at the epoch the session was attached in; browser-level commands that name a session
//! take that epoch explicitly. Either way a command racing a reconnect is refused before it is
//! written, so a session id never reaches a socket it does not exist on.

use std::time::Duration;

use browseros_cdp::{
    CdpClient, CdpError, CdpEvent, ConnectOptions, ReconnectPolicy, SessionId, browser, target,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::host::directory::{DirectoryInput, PageTarget, TabEntry};
use crate::host::ids::TabId;
use crate::host::link::Attached;
use crate::run::PageId;

// ----- the tab directory (`directory.rs`) ----------------------------------------------------

/// How long a closed tab's last facts stay readable, for a call whose tab closed before its page
/// facts were read. claw-server keeps destroyed tab mappings as long, for late attribution
/// (`apps/claw-server/src/services/browser/tab_registry.rs:20-21`).
pub(crate) const TAB_TOMBSTONE: Duration = Duration::from_mins(5);

/// One event on the host socket as the tab directory reads it: the `Target.targetCreated`,
/// `Target.targetInfoChanged` and `Target.targetDestroyed` that `Target.setDiscoverTargets`
/// delivers. `None` for every other event, for params that do not parse, and for targets that
/// are not tabs.
pub(crate) fn directory_input(event: &CdpEvent) -> Option<DirectoryInput> {
    match event.method.as_str() {
        "Target.targetCreated" => target::TargetCreatedEvent::deserialize(&event.params)
            .ok()
            .and_then(|created| page_target(created.target_info))
            .map(DirectoryInput::Created),
        "Target.targetInfoChanged" => target::TargetInfoChangedEvent::deserialize(&event.params)
            .ok()
            .and_then(|changed| page_target(changed.target_info))
            .map(DirectoryInput::Changed),
        "Target.targetDestroyed" => target::TargetDestroyedEvent::deserialize(&event.params)
            .ok()
            .map(|destroyed| DirectoryInput::Destroyed(PageId::new(destroyed.target_id))),
        _ => None,
    }
}

/// The `Browser.getTabs` reply as the directory's seed. A tab without a positive tab id is left
/// out, as it would be from discovery.
pub(crate) fn seed(reply: browser::GetTabsResult) -> DirectoryInput {
    let tabs = reply
        .tabs
        .into_iter()
        .filter_map(|info| {
            Some(PageTarget {
                target: PageId::new(info.target_id),
                entry: TabEntry {
                    tab: TabId::parse(info.tab_id)?,
                    url: info.url,
                    title: info.title,
                },
            })
        })
        .collect();
    DirectoryInput::Seeded(tabs)
}

/// A discovered target, if it is a tab: a `page` target carrying BrowserOS's patched `tabId`
/// (probe 6 in the neo-host-events design), positive. Workers, iframes and the browser target
/// are not tabs.
fn page_target(info: target::TargetInfo) -> Option<PageTarget> {
    if info.r#type != "page" {
        return None;
    }
    Some(PageTarget {
        target: PageId::new(info.target_id),
        entry: TabEntry {
            tab: TabId::parse(info.tab_id?)?,
            url: info.url,
            title: info.title,
        },
    })
}

// ----- the host link (`link.rs`): claw-server `services/capture`, unchanged values -----------

/// How often the link checks that its socket is still the incarnation its sessions were
/// attached on. `apps/claw-server/src/services/capture/link.rs:19-20` (origin/main).
pub(crate) const WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);

/// The host side's own client: connect within 2 s with one retry, and reconnect every second
/// for as long as it takes. The default policy exits the process when reconnects run out.
/// `apps/claw-server/src/services/capture/ports.rs:122-131` (origin/main).
pub(crate) fn connect_options(port: u16) -> ConnectOptions {
    ConnectOptions {
        connect_timeout: Duration::from_secs(2),
        connect_max_retries: 1,
        reconnect_policy: ReconnectPolicy::KeepTrying,
        reconnect_delay: Duration::from_secs(1),
        reconnect_max_retries: usize::MAX,
        ..ConnectOptions::new(port)
    }
}

/// The one event the link takes off the shared ring with `events_targeted`: frames are large
/// and frequent, and the ring would copy them once per subscriber and hold thousands
/// (`apps/claw-server/src/services/capture/link.rs:46-48`, origin/main).
pub(crate) const SCREENCAST_FRAME: &str = "Page.screencastFrame";

/// `Target.attachToTarget{targetId, flatten: true}` (typed) on the socket incarnation `epoch`.
pub(crate) async fn attach_to_target(
    client: &CdpClient,
    epoch: u64,
    target: &PageId,
) -> Result<SessionId, CdpError> {
    let attached: target::AttachToTargetResult = send_at(
        client,
        epoch,
        "Target.attachToTarget",
        target::AttachToTargetParams {
            target_id: target.as_str().to_string(),
            flatten: Some(true),
        },
        None,
    )
    .await?;
    Ok(SessionId::from(attached.session_id))
}

/// `Target.detachFromTarget{sessionId}` (typed), on the incarnation `epoch` the session belongs
/// to.
pub(crate) async fn detach_from_target(
    client: &CdpClient,
    epoch: u64,
    session: &SessionId,
) -> Result<(), CdpError> {
    let _: target::DetachFromTargetResult = send_at(
        client,
        epoch,
        "Target.detachFromTarget",
        target::DetachFromTargetParams {
            session_id: Some(session.as_str().to_string()),
            target_id: None,
        },
        None,
    )
    .await?;
    Ok(())
}

/// The session a `Target.detachedFromTarget` (typed) names: the browser closed it, which is how
/// a closed tab shows up on the sessions attached to it.
pub(crate) fn detached_session(event: &CdpEvent) -> Option<SessionId> {
    if event.method != "Target.detachedFromTarget" {
        return None;
    }
    let detached: target::DetachedFromTargetEvent =
        serde_json::from_value(event.params.clone()).ok()?;
    Some(SessionId::from(detached.session_id))
}

async fn send_at<P: Serialize, R: DeserializeOwned>(
    client: &CdpClient,
    epoch: u64,
    method: &str,
    params: P,
    session: Option<&SessionId>,
) -> Result<R, CdpError> {
    let reply = client
        .send_at_epoch(epoch, method, serde_json::to_value(params)?, session)
        .await?;
    Ok(serde_json::from_value(reply)?)
}

// ----- presences (`presence.rs`) -------------------------------------------------------------

/// `Emulation.setFocusEmulationEnabled{enabled: true}` (untyped: browseros-cdp's surface has no
/// Emulation domain), on a tab's presence alone. Never `false`: see `presence.rs`.
pub(crate) async fn emulate_focus(session: &Attached) -> Result<(), CdpError> {
    session
        .send(
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": true }),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use serde_json::json;

    use super::*;

    // Recorded from BrowserOS neo 155.0.8318.26 (headless): what `Target.setDiscoverTargets` and
    // `Browser.getTabs` delivered around a tab made with `Target.createTarget` and then closed.
    const CREATED: &str = r#"{"targetInfo":{"targetId":"7EC663DA0290AB032344B3FF32267FE0","type":"page","title":"","url":"data:text/html,<title>Probe</title>hi","attached":false,"canAccessOpener":false,"browserContextId":"27C255E9D02F509F66CFFEB8A45DAA16","tabId":196019726}}"#;
    const CHANGED: &str = r#"{"targetInfo":{"targetId":"7EC663DA0290AB032344B3FF32267FE0","type":"page","title":"data:text/html,<title>Probe</title>hi","url":"data:text/html,<title>Probe</title>hi","attached":false,"canAccessOpener":false,"browserContextId":"27C255E9D02F509F66CFFEB8A45DAA16","tabId":196019726,"windowId":196019724}}"#;
    const DESTROYED: &str = r#"{"targetId":"7EC663DA0290AB032344B3FF32267FE0"}"#;
    const OMNIBOX: &str = r#"{"targetInfo":{"targetId":"52AB4B1B326CFE1C51BA6619E281A86A","type":"browser_ui","title":"Omnibox Popup","url":"chrome://omnibox-popup.top-chrome/","attached":false,"canAccessOpener":false,"browserContextId":"27C255E9D02F509F66CFFEB8A45DAA16"}}"#;
    const WORKER: &str = r#"{"targetInfo":{"targetId":"5CCF81BD9E22C9F4AF3E8B1CB1276785","type":"service_worker","title":"Service Worker chrome-extension://adlpneommgkgeanpaekgoaolcpncohkf/background.js","url":"chrome-extension://adlpneommgkgeanpaekgoaolcpncohkf/background.js","attached":false,"canAccessOpener":false,"browserContextId":"27C255E9D02F509F66CFFEB8A45DAA16"}}"#;
    const TABS: &str = r#"{"tabs":[{"tabId":196019725,"targetId":"7A20C3F0317E37E3F8E43ED16F70A196","url":"about:blank","title":"about:blank","isActive":false,"isLoading":false,"loadProgress":0,"isPinned":false,"isHidden":false,"windowId":196019724,"index":0,"browserContextId":"Default","browserProfileId":"BF7F0FC3C4674C40D1A19F8E41A061F4","liveBrowserContextId":"27C255E9D02F509F66CFFEB8A45DAA16"},{"tabId":196019726,"targetId":"7EC663DA0290AB032344B3FF32267FE0","url":"data:text/html,<title>Probe</title>hi","title":"Probe","isActive":true,"isLoading":false,"loadProgress":0,"isPinned":false,"isHidden":false,"windowId":196019724,"index":1,"browserContextId":"Default","browserProfileId":"BF7F0FC3C4674C40D1A19F8E41A061F4","liveBrowserContextId":"27C255E9D02F509F66CFFEB8A45DAA16"}]}"#;
    const PROBE_URL: &str = "data:text/html,<title>Probe</title>hi";

    fn recorded(params: &str) -> Value {
        serde_json::from_str(params).expect("recorded params are JSON")
    }

    fn event(method: &str, params: Value) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params,
            session_id: None,
        }
    }

    fn tab(target_id: &str, tab: i64, url: &str, title: &str) -> PageTarget {
        PageTarget {
            target: PageId::new(target_id.to_string()),
            entry: TabEntry {
                tab: TabId::parse(tab).expect("a positive tab id"),
                url: url.to_string(),
                title: title.to_string(),
            },
        }
    }

    fn probe_tab(title: &str) -> PageTarget {
        tab(
            "7EC663DA0290AB032344B3FF32267FE0",
            196_019_726,
            PROBE_URL,
            title,
        )
    }

    #[test]
    fn discovery_events_become_directory_inputs() {
        assert_eq!(
            directory_input(&event("Target.targetCreated", recorded(CREATED))),
            Some(DirectoryInput::Created(probe_tab(""))),
            "a new tab as discovery first reports it: a tab id, no window or title yet"
        );
        assert_eq!(
            directory_input(&event("Target.targetInfoChanged", recorded(CHANGED))),
            Some(DirectoryInput::Changed(probe_tab(PROBE_URL))),
            "a tab whose title changed"
        );
        assert_eq!(
            directory_input(&event("Target.targetDestroyed", recorded(DESTROYED))),
            Some(DirectoryInput::Destroyed(PageId::new(
                "7EC663DA0290AB032344B3FF32267FE0".to_string()
            ))),
            "a destroyed target, whatever it was"
        );
    }

    #[test]
    fn only_tabs_enter_the_directory() {
        let with = |field: &str, value: Value| {
            let mut params = recorded(CREATED);
            params["targetInfo"][field] = value;
            params
        };
        let mut untabbed = recorded(CREATED);
        untabbed["targetInfo"]
            .as_object_mut()
            .expect("an object")
            .remove("tabId");
        let not_tabs = [
            (recorded(OMNIBOX), "browser UI"),
            (recorded(WORKER), "an extension's service worker"),
            (
                with("type", json!("iframe")),
                "an iframe, even with a tab id",
            ),
            (untabbed, "a page without BrowserOS's tab id"),
            (with("tabId", json!(0)), "a page with tab id 0"),
            (with("tabId", json!(-1)), "a page with a negative tab id"),
            (
                json!({ "targetInfo": { "type": "page" } }),
                "a TargetInfo that does not parse",
            ),
        ];
        for (params, what) in not_tabs {
            assert_eq!(
                directory_input(&event("Target.targetCreated", params)),
                None,
                "{what} is not a tab"
            );
        }
        assert_eq!(
            directory_input(&event("Target.attachedToTarget", recorded(CREATED))),
            None,
            "the directory reads only discovery events"
        );
    }

    #[test]
    fn the_tab_list_seeds_the_directory() {
        let mut reply = recorded(TABS);
        let mut untabbed = reply["tabs"][0].clone();
        untabbed["tabId"] = json!(0);
        reply["tabs"]
            .as_array_mut()
            .expect("an array")
            .push(untabbed);
        let reply: browser::GetTabsResult =
            serde_json::from_value(reply).expect("a Browser.getTabs reply");
        assert_eq!(
            seed(reply),
            DirectoryInput::Seeded(vec![
                tab(
                    "7A20C3F0317E37E3F8E43ED16F70A196",
                    196_019_725,
                    "about:blank",
                    "about:blank"
                ),
                probe_tab("Probe"),
            ]),
            "every tab with a positive tab id seeds the directory"
        );
    }
}
