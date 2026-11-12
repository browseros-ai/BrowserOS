use super::{check_fixed_fixture_epoch, fake_ctx};
use crate::{framework::execute_tool, tools::tab_groups};
use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, NativeCapabilities, SessionId,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

struct Connection {
    events: broadcast::Sender<CdpEvent>,
    calls: Mutex<Vec<(String, Value)>>,
    delete_error: Option<&'static str>,
}

impl CdpConnection for Connection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((method.to_owned(), params));
            match method {
                "Browser.getTabGroups" => Ok(json!({"groups": []})),
                "Browser.getClosedTabGroups" => Ok(json!({"groups": [
                    {"savedGroupId":"saved-a", "browserProfileId":"profile-a", "title":"Work", "color":"blue", "tabCount":2, "shared":false},
                    {"savedGroupId":"saved-b", "browserProfileId":"profile-b", "title":"Work", "color":"blue", "tabCount":1, "shared":true}
                ]})),
                "Browser.deleteClosedTabGroup" => match self.delete_error {
                    Some(message) => Err(CdpError::Protocol {
                        code: -32000,
                        message: message.to_owned(),
                    }),
                    None => Ok(json!({})),
                },
                _ => Err(CdpError::Protocol {
                    code: -32601,
                    message: format!("unexpected {method}"),
                }),
            }
        })
    }

    fn send_at_epoch<'a>(
        &'a self,
        epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            check_fixed_fixture_epoch(epoch, self.connection_epoch())?;
            self.send(method, params, session).await
        })
    }

    fn send_raw_json<'a>(
        &'a self,
        _method: &'a str,
        _params: &'a str,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async {
            Err(CdpError::Protocol {
                code: -32601,
                message: "unexpected raw call".to_owned(),
            })
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        1
    }
}

fn context(delete_error: Option<&'static str>) -> (crate::framework::ToolCtx, Arc<Connection>) {
    context_with_native(
        delete_error,
        NativeCapabilities::from_browseros_version(Some("0.52.2")),
    )
}

fn context_with_native(
    delete_error: Option<&'static str>,
    native: NativeCapabilities,
) -> (crate::framework::ToolCtx, Arc<Connection>) {
    let connection = Arc::new(Connection {
        events: broadcast::channel(8).0,
        calls: Mutex::default(),
        delete_error,
    });
    let session =
        BrowserSession::new_with_native(connection.clone(), BrowserSessionHooks::default(), native);
    let mut ctx = fake_ctx();
    ctx.browser = BrowserView::unbound(session, None);
    (ctx, connection)
}

#[tokio::test]
async fn tab_groups_default_list_keeps_the_open_only_contract() -> anyhow::Result<()> {
    let (ctx, connection) = context(None);
    let result = execute_tool(&tab_groups::definition(), json!({}), &ctx).await?;
    assert!(!result.is_error);
    assert_eq!(
        result.structured_content,
        Some(json!({"groups":[], "count":0}))
    );
    assert_eq!(
        *connection.calls.lock().unwrap_or_else(|p| p.into_inner()),
        vec![("Browser.getTabGroups".to_owned(), json!({}))]
    );
    Ok(())
}

#[tokio::test]
async fn tab_groups_lists_closed_groups_with_profile_and_shared_state() -> anyhow::Result<()> {
    let (ctx, _) = context(None);
    let result = execute_tool(
        &tab_groups::definition(),
        json!({"includeClosed":true}),
        &ctx,
    )
    .await?;
    assert!(!result.is_error);
    let data = result
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("missing groups"))?;
    assert_eq!(data["closedGroups"].as_array().map(Vec::len), Some(2));
    assert_eq!(data["closedGroups"][0]["browserProfileId"], "profile-a");
    assert_eq!(data["closedGroups"][1]["shared"], true);
    assert_eq!(data["groups"], json!([]));
    Ok(())
}

#[tokio::test]
async fn tab_groups_delete_targets_exact_saved_and_profile_ids() -> anyhow::Result<()> {
    let (ctx, connection) = context(None);
    let result = execute_tool(
        &tab_groups::definition(),
        json!({"action":"delete", "savedGroupId":"saved-a", "browserProfileId":"profile-a"}),
        &ctx,
    )
    .await?;
    assert!(!result.is_error);
    assert_eq!(
        *connection.calls.lock().unwrap_or_else(|p| p.into_inner()),
        vec![(
            "Browser.deleteClosedTabGroup".to_owned(),
            json!({"savedGroupId":"saved-a", "browserProfileId":"profile-a"})
        )]
    );
    Ok(())
}

#[tokio::test]
async fn tab_groups_delete_rejects_missing_ids_and_live_group_ids_before_dispatch()
-> anyhow::Result<()> {
    let (ctx, connection) = context(None);
    for args in [
        json!({"action":"delete"}),
        json!({"action":"delete", "savedGroupId":"saved-a"}),
        json!({"action":"delete", "savedGroupId":"", "browserProfileId":"profile-a"}),
        json!({"action":"delete", "savedGroupId":"saved-a", "browserProfileId":"profile-a", "groupId":"live-a"}),
    ] {
        assert!(
            execute_tool(&tab_groups::definition(), args, &ctx)
                .await?
                .is_error
        );
    }
    assert!(
        connection
            .calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn tab_groups_delete_preserves_native_refusals() -> anyhow::Result<()> {
    for message in [
        "Tab group is open",
        "Shared tab groups cannot be deleted",
        "Saved tab group not found",
        "Method not found",
    ] {
        let (ctx, connection) = context(Some(message));
        let result = execute_tool(
            &tab_groups::definition(),
            json!({"action":"delete", "savedGroupId":"saved-a", "browserProfileId":"profile-a"}),
            &ctx,
        )
        .await?;
        assert!(result.is_error);
        assert!(super::result_text(&result).contains(message));
        assert_eq!(
            connection
                .calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .len(),
            1
        );
    }
    Ok(())
}

#[tokio::test]
async fn tab_groups_old_native_hides_new_guidance_and_refuses_before_dispatch() -> anyhow::Result<()>
{
    for native in [
        NativeCapabilities::default(),
        NativeCapabilities::from_browseros_version(Some("0.52.1.1")),
    ] {
        let (ctx, connection) = context_with_native(None, native);
        let definition = tab_groups::definition();
        assert!(!definition.description_for(native).contains("includeClosed"));
        for args in [
            json!({"includeClosed":true}),
            json!({"action":"delete", "savedGroupId":"saved-a", "browserProfileId":"profile-a"}),
        ] {
            let result = execute_tool(&definition, args, &ctx).await?;
            assert!(result.is_error);
            assert!(super::result_text(&result).contains("needs BrowserOS 0.52.2.0 or newer"));
        }
        assert!(
            connection
                .calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_empty()
        );
        assert!(!execute_tool(&definition, json!({}), &ctx).await?.is_error);
    }
    Ok(())
}
