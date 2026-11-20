use crate::{
    AppState,
    api::mcp::{
        guards::person_control,
        test_support::{tool_call, tool_call_with_behavior},
    },
    config::{Behavior, ServerMode},
    ids::{DispatchId, SessionId},
    services::page_hold::{HoldPhase, PageHoldService},
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
async fn taking_over_stops_the_call_and_ends_the_hold_without_a_wait() -> anyhow::Result<()> {
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

    assert_eq!(page_hold(&state).phase(&page).await, None);
    assert!(token.is_cancelled(), "the call acting on the page stops");
    assert!(page_hold(&state).take_interrupted(&acting).await);
    assert!(!page_hold(&state).take_interrupted(&acting).await);
    assert!(
        session
            .try_register_dispatch(DispatchId::new(), CancellationToken::new())
            .await,
        "the session keeps taking calls"
    );
    assert!(
        state.help.get(&owner()).await.is_none(),
        "nothing waits on the person"
    );
    assert_eq!(
        page_hold(&state).take_notice(&page, &owner()).await,
        None,
        "the interrupted call already told the agent"
    );
    Ok(())
}

#[tokio::test]
async fn a_take_over_between_actions_refuses_the_next_change_once() -> anyhow::Result<()> {
    let state = state_with_overlay().await?;
    let page = PageId(2);
    page_hold(&state)
        .seed(page.clone(), &owner(), HoldPhase::Between, &[])
        .await;

    page_hold(&state)
        .handle_event(&page, OverlayEvent::TakeOver { via: Via::Button })
        .await;

    assert_eq!(page_hold(&state).phase(&page).await, None);
    assert_eq!(
        page_hold(&state).take_notice(&PageId(3), &owner()).await,
        None
    );
    assert_eq!(
        page_hold(&state)
            .take_notice(&page, &SessionId::new("s2"))
            .await,
        None
    );
    let notice = page_hold(&state)
        .take_notice(&page, &owner())
        .await
        .ok_or_else(|| anyhow::anyhow!("the agent was not told"))?;
    assert!(notice.contains("check with the user"), "{notice}");
    assert_eq!(page_hold(&state).take_notice(&page, &owner()).await, None);
    Ok(())
}

#[tokio::test]
async fn a_taken_over_page_refuses_one_change_and_never_a_read() -> anyhow::Result<()> {
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
        .seed(PageId(4), &owner(), HoldPhase::Between, &[])
        .await;
    act.state
        .page_hold
        .handle_event(&PageId(4), OverlayEvent::TakeOver { via: Via::Esc })
        .await;

    let mut snapshot = tool_call("snapshot", json!({ "page": 4 })).await?;
    snapshot.state = act.state.clone();
    assert!(person_control::guard(&snapshot).await.is_none());
    let refusal = person_control::guard(&act)
        .await
        .ok_or_else(|| anyhow::anyhow!("act was not refused"))?;
    assert!(refusal.is_error);
    assert!(
        person_control::guard(&act).await.is_none(),
        "the agent may act once the user has had their say"
    );
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
        .seed(PageId(6), &owner(), HoldPhase::Between, &[])
        .await;
    page_hold(&state)
        .handle_event(&PageId(6), OverlayEvent::TakeOver { via: Via::Button })
        .await;
    page_hold(&state)
        .seed(PageId(7), &other, HoldPhase::Between, &[])
        .await;

    page_hold(&state).finish_session(&owner()).await;

    assert_eq!(page_hold(&state).phase(&PageId(5)).await, None);
    assert_eq!(
        page_hold(&state).take_notice(&PageId(6), &owner()).await,
        None
    );
    assert_eq!(
        page_hold(&state).phase(&PageId(7)).await,
        Some(HoldPhase::Between)
    );
    Ok(())
}

async fn second_session(state: &AppState) -> Arc<crate::services::sessions::Session> {
    let session = crate::services::sessions::Session::new(
        SessionId::new("s2"),
        crate::identity::ClientIdentity::Ephemeral {
            slug: "claude".to_string(),
            label: "Claude".to_string(),
        },
        crate::identity::ConversationIdentity::new("claude", "brave-bison".to_string()),
        "Claude".to_string(),
        tokio::time::Instant::now(),
    );
    state.sessions.insert_for_testing(session.clone()).await;
    session
}

#[tokio::test]
async fn taking_over_a_shared_page_stops_every_agent_on_it() -> anyhow::Result<()> {
    let state = state_with_overlay().await?;
    let first = state
        .sessions
        .lookup(&owner())
        .await
        .ok_or_else(|| anyhow::anyhow!("session missing"))?;
    let second = second_session(&state).await;
    let (first_call, second_call) = (DispatchId::new(), DispatchId::new());
    let (first_token, second_token) = (CancellationToken::new(), CancellationToken::new());
    first
        .try_register_dispatch(first_call.clone(), first_token.clone())
        .await;
    second
        .try_register_dispatch(second_call.clone(), second_token.clone())
        .await;
    let page = PageId(8);
    page_hold(&state)
        .seed(
            page.clone(),
            &owner(),
            HoldPhase::Working,
            std::slice::from_ref(&first_call),
        )
        .await;
    page_hold(&state)
        .seed_call(&page, &second_call, second.id())
        .await;

    page_hold(&state)
        .handle_event(&page, OverlayEvent::TakeOver { via: Via::Button })
        .await;

    assert!(first_token.is_cancelled());
    assert!(second_token.is_cancelled());
    assert!(page_hold(&state).take_interrupted(&first_call).await);
    assert!(page_hold(&state).take_interrupted(&second_call).await);
    assert_eq!(page_hold(&state).phase(&page).await, None);
    Ok(())
}

#[tokio::test]
async fn turning_the_overlay_off_drops_every_hold_and_notice() -> anyhow::Result<()> {
    let state = state_with_overlay().await?;
    page_hold(&state)
        .seed(PageId(10), &owner(), HoldPhase::Working, &[])
        .await;
    page_hold(&state)
        .seed(PageId(11), &owner(), HoldPhase::Between, &[])
        .await;
    page_hold(&state)
        .handle_event(&PageId(11), OverlayEvent::TakeOver { via: Via::Esc })
        .await;

    state
        .agent_settings
        .update(crate::services::agent_settings::AgentSettingsUpdate {
            working_overlay_enabled: Some(false),
            ..Default::default()
        })
        .await?;
    page_hold(&state).tick_for_test().await;

    assert_eq!(page_hold(&state).phase(&PageId(10)).await, None);
    assert_eq!(
        page_hold(&state).take_notice(&PageId(11), &owner()).await,
        None
    );
    Ok(())
}

#[tokio::test]
async fn a_finished_call_lets_go_of_every_page_it_held() -> anyhow::Result<()> {
    let state = state_with_overlay().await?;
    let script = DispatchId::new();
    let other = DispatchId::new();
    page_hold(&state)
        .seed(
            PageId(12),
            &owner(),
            HoldPhase::Working,
            std::slice::from_ref(&script),
        )
        .await;
    page_hold(&state)
        .seed(
            PageId(13),
            &owner(),
            HoldPhase::Working,
            &[script.clone(), other.clone()],
        )
        .await;

    let released = page_hold(&state).release_for_test(&script).await;

    assert_eq!(released, vec![PageId(12)]);
    assert_eq!(
        page_hold(&state).phase(&PageId(12)).await,
        Some(HoldPhase::Between)
    );
    assert_eq!(
        page_hold(&state).phase(&PageId(13)).await,
        Some(HoldPhase::Working),
        "another call still acts on it"
    );
    Ok(())
}

#[tokio::test]
async fn a_finishing_agent_leaves_a_page_another_agent_is_acting_on() -> anyhow::Result<()> {
    let state = state_with_overlay().await?;
    let second = second_session(&state).await;
    let page = PageId(14);
    page_hold(&state)
        .seed(page.clone(), &owner(), HoldPhase::Working, &[])
        .await;
    page_hold(&state)
        .seed_call(&page, &DispatchId::new(), second.id())
        .await;

    page_hold(&state).finish_session(&owner()).await;

    assert_eq!(
        page_hold(&state).phase(&page).await,
        Some(HoldPhase::Working)
    );
    Ok(())
}
