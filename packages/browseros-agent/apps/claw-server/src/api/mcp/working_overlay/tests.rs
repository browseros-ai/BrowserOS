use crate::{
    AppState,
    api::mcp::{
        guards::person_control,
        test_support::{tool_call, tool_call_with_behavior},
    },
    config::{Behavior, ServerMode},
    ids::{DispatchId, SessionId},
    services::{
        help::{HelpOpenParams, HelpOrigin},
        page_hold::{HoldPhase, PageHoldService},
    },
};
use browseros_core::{
    PageId,
    overlay::{OverlayEvent, Via},
};
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn behavior_with_overlay() -> Behavior {
    Behavior {
        working_overlay: true,
        ..Behavior::defaults(ServerMode::Desktop)
    }
}

async fn state_with_overlay() -> anyhow::Result<AppState> {
    let call = tool_call_with_behavior(
        "act",
        json!({ "page": 1, "kind": "click", "ref": "e1" }),
        ServerMode::Desktop,
        behavior_with_overlay(),
    )
    .await?;
    Ok(call.state)
}

fn owner() -> SessionId {
    SessionId::new("s1")
}

fn page_hold(state: &AppState) -> &Arc<PageHoldService> {
    &state.page_hold
}

#[tokio::test]
async fn the_rollout_switch_and_the_setting_both_gate_the_overlay() -> anyhow::Result<()> {
    let off = tool_call_with_behavior(
        "act",
        json!({ "page": 1, "kind": "click", "ref": "e1" }),
        ServerMode::Desktop,
        Behavior {
            working_overlay: false,
            ..Behavior::defaults(ServerMode::Desktop)
        },
    )
    .await?;
    assert!(!off.state.page_hold.enabled());
    let hosted = tool_call_with_behavior(
        "act",
        json!({ "page": 1, "kind": "click", "ref": "e1" }),
        ServerMode::Hosted,
        Behavior::defaults(ServerMode::Hosted),
    )
    .await?;
    assert!(!hosted.state.page_hold.enabled());

    let state = state_with_overlay().await?;
    assert!(page_hold(&state).enabled());
    state
        .agent_settings
        .update(crate::services::agent_settings::AgentSettingsUpdate {
            working_overlay_enabled: Some(false),
            ..Default::default()
        })
        .await?;
    assert!(!page_hold(&state).enabled());
    Ok(())
}

#[tokio::test]
async fn taking_over_pauses_the_agent_on_that_page_without_telling_the_cockpit()
-> anyhow::Result<()> {
    let state = state_with_overlay().await?;
    let session = state
        .sessions
        .lookup(&owner())
        .await
        .ok_or_else(|| anyhow::anyhow!("session missing"))?;
    let acting = DispatchId::new();
    let token = CancellationToken::new();
    assert!(
        session
            .try_register_dispatch(acting.clone(), token.clone())
            .await
    );
    let page = PageId(1);
    let mut changes = state.help.subscribe();
    changes.borrow_and_update();
    page_hold(&state)
        .seed(
            page.clone(),
            &owner(),
            HoldPhase::Working,
            std::slice::from_ref(&acting),
        )
        .await;

    page_hold(&state)
        .handle_event(&page, OverlayEvent::TakeOver { via: Via::Esc })
        .await;

    assert_eq!(
        page_hold(&state).phase(&page).await,
        Some(HoldPhase::Person)
    );
    assert!(token.is_cancelled(), "the call acting on the page stops");
    assert!(page_hold(&state).take_interrupted(&acting).await);
    assert!(!page_hold(&state).take_interrupted(&acting).await);
    assert!(
        session
            .try_register_dispatch(DispatchId::new(), CancellationToken::new())
            .await,
        "the session keeps taking calls"
    );
    let entry = state
        .help
        .get(&owner())
        .await
        .ok_or_else(|| anyhow::anyhow!("no help entry"))?;
    assert_eq!(entry.origin(), HelpOrigin::Person);
    assert!(entry.is_open());
    assert_eq!(
        state.help.snapshot(&owner(), Some(7), None, None).await,
        None
    );
    assert!(
        !changes.has_changed()?,
        "Home is not told about a take over"
    );
    assert!(
        page_hold(&state)
            .person_message(&page)
            .await
            .is_some_and(|message| message.contains("await_human_help"))
    );
    Ok(())
}

#[tokio::test]
async fn handing_back_ends_the_wait_and_lets_neo_act_again() -> anyhow::Result<()> {
    let state = state_with_overlay().await?;
    let page = PageId(2);
    page_hold(&state)
        .seed(page.clone(), &owner(), HoldPhase::Between, &[])
        .await;
    page_hold(&state)
        .handle_event(&page, OverlayEvent::TakeOver { via: Via::Button })
        .await;
    let entry = state
        .help
        .get(&owner())
        .await
        .ok_or_else(|| anyhow::anyhow!("no help entry"))?;

    page_hold(&state)
        .handle_event(&page, OverlayEvent::HandBack)
        .await;

    assert!(!entry.is_open());
    assert_eq!(
        page_hold(&state).phase(&page).await,
        Some(HoldPhase::Between)
    );
    assert_eq!(page_hold(&state).person_message(&page).await, None);
    Ok(())
}

#[tokio::test]
async fn a_take_over_keeps_an_open_agent_request() -> anyhow::Result<()> {
    let state = state_with_overlay().await?;
    state
        .help
        .open(
            &owner(),
            HelpOpenParams {
                request_id: "help-1".to_string(),
                reason: "Sign in".to_string(),
                details: None,
                resume_hint: None,
                kind: None,
                browser_tab_id: Some(3),
                url: None,
                title: None,
                origin: HelpOrigin::Agent,
            },
        )
        .await;
    let page = PageId(3);
    page_hold(&state)
        .seed(page.clone(), &owner(), HoldPhase::Working, &[])
        .await;
    page_hold(&state)
        .handle_event(&page, OverlayEvent::TakeOver { via: Via::Button })
        .await;
    let entry = state
        .help
        .get(&owner())
        .await
        .ok_or_else(|| anyhow::anyhow!("no help entry"))?;
    assert_eq!(entry.request_id(), "help-1");
    assert_eq!(entry.origin(), HelpOrigin::Agent);
    Ok(())
}

#[tokio::test]
async fn a_taken_over_page_refuses_changes_but_not_reads() -> anyhow::Result<()> {
    let act = tool_call_with_behavior(
        "act",
        json!({ "page": 4, "kind": "click", "ref": "e1" }),
        ServerMode::Desktop,
        behavior_with_overlay(),
    )
    .await?;
    assert!(person_control::guard(&act).await.is_none());
    act.state
        .page_hold
        .seed(PageId(4), &owner(), HoldPhase::Person, &[])
        .await;
    let refusal = person_control::guard(&act)
        .await
        .ok_or_else(|| anyhow::anyhow!("act was not refused"))?;
    assert!(refusal.is_error);

    let mut snapshot = tool_call("snapshot", json!({ "page": 4 })).await?;
    snapshot.state = act.state.clone();
    assert!(person_control::guard(&snapshot).await.is_none());
    Ok(())
}

#[tokio::test]
async fn a_finished_session_lets_go_of_only_its_own_pages() -> anyhow::Result<()> {
    let state = state_with_overlay().await?;
    let other = SessionId::new("s2");
    page_hold(&state)
        .seed(PageId(5), &owner(), HoldPhase::Working, &[])
        .await;
    page_hold(&state)
        .seed(PageId(6), &owner(), HoldPhase::Person, &[])
        .await;
    page_hold(&state)
        .seed(PageId(7), &other, HoldPhase::Between, &[])
        .await;

    page_hold(&state).finish_session(&owner()).await;

    assert_eq!(page_hold(&state).phase(&PageId(5)).await, None);
    assert_eq!(page_hold(&state).phase(&PageId(6)).await, None);
    assert_eq!(
        page_hold(&state).phase(&PageId(7)).await,
        Some(HoldPhase::Between)
    );
    Ok(())
}
