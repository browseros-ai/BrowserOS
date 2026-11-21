//! The own loop through `Agent::run_turn`, the seam `conversations` calls, over a real
//! `browseros.sqlite` in a temporary folder: the provider row is looked up in the store, the
//! scripted model plays the steps, and the browser double answers the tool call.

use std::sync::Arc;

use browseros_server::agent::{Agent, AgentRuntime};
use browseros_server::browser::{Access, Browser, RecordedCall, Scope, ScriptedReply};
use browseros_server::model::{Models, Script, ScriptedStep};
use browseros_server::store::{Edit, ProviderEdit, ProviderKind, Secret, Store};
use browseros_server::turn::{
    ChatErrorCode, ConversationId, EventSink, FinishReason, MessageId, Mode, Role, Target,
    ToolCallId, ToolOwner, TurnContext, TurnEnd, TurnEvent, TurnInput, TurnRunner, UiMessage,
    UiPart,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const CONVERSATION: ConversationId =
    uuid::Uuid::from_u128(0x5eed_0000_0000_4000_8000_0000_0000_0002);

/// An agent over a fresh store; `provider` is saved and selected when given.
async fn agent(
    state: &tempfile::TempDir,
    provider: Option<&str>,
    script: Script,
    replies: Vec<ScriptedReply>,
) -> anyhow::Result<(Agent, browseros_server::browser::ScriptedBrowser)> {
    let store = Store::open(state.path()).await?;
    if let Some(id) = provider {
        store.providers().save(id, openai_row()).await?;
        store.providers().set_default(id).await?;
    }
    let (browser, handle) = Browser::scripted(replies);
    let models = Models::scripted(store.clone(), None, script);
    let runtime = AgentRuntime {
        bun: None,
        state_dir: state.path().to_owned(),
    };
    Ok((Agent::new(models, browser, store, runtime), handle))
}

fn openai_row() -> ProviderEdit {
    ProviderEdit {
        kind: ProviderKind::Openai,
        name: "OpenAI".into(),
        model_id: "gpt-5".into(),
        context_window: 128_000,
        base_url: Edit::Keep,
        headers: Edit::Keep,
        supports_images: Some(true),
        temperature: None,
        api_key: Edit::Set(Secret::new("sk-test-not-a-key")),
        access_key_id: Edit::Keep,
        secret_access_key: Edit::Keep,
        session_token: Edit::Keep,
        resource_name: Edit::Keep,
        region: Edit::Keep,
        reasoning_effort: Edit::Keep,
        reasoning_summary: Edit::Keep,
        created_at: None,
    }
}

fn input() -> TurnInput {
    TurnInput {
        conversation: CONVERSATION,
        history: Arc::from(vec![UiMessage {
            id: MessageId("u1".into()),
            role: Role::User,
            parts: vec![UiPart::Text {
                text: "What is the page title?".into(),
                state: None,
                extra: serde_json::Map::new(),
            }],
            metadata: None,
        }]),
        context: TurnContext {
            mode: Mode::Agent,
            window_id: None,
            active_tab: None,
            selected_tabs: Vec::new(),
            selected_text: None,
            user_system_prompt: None,
            working_dir: None,
            supports_reasoning: true,
        },
        attachments: Vec::new(),
        ephemeral: false,
    }
}

async fn run(agent: &Agent, target: Target, events: &mut dyn EventSink) -> TurnEnd {
    agent
        .run_turn(&target, &input(), events, &CancellationToken::new())
        .await
}

#[tokio::test]
async fn a_turn_on_the_selected_provider_runs_the_tool_then_answers() -> anyhow::Result<()> {
    let state = tempfile::tempdir()?;
    let script = Script {
        steps: vec![
            ScriptedStep::call("run", json!({ "code": "return document.title" })),
            ScriptedStep::answer("It is Example Domain."),
        ],
    };
    let (agent, browser) = agent(
        &state,
        Some("openai-1"),
        script,
        vec![ScriptedReply::text("Example Domain")],
    )
    .await?;
    let mut events = Vec::new();

    let end = run(&agent, Target::Provider(None), &mut events).await;

    let call = ToolCallId("call_scripted_0_0".into());
    assert_eq!(end, TurnEnd::Done(FinishReason::Stop));
    assert_eq!(
        events,
        vec![
            TurnEvent::StepStart,
            TurnEvent::ToolStarted {
                id: call.clone(),
                name: "run".into(),
                owner: ToolOwner::Server,
            },
            TurnEvent::ToolInput {
                id: call.clone(),
                name: "run".into(),
                input: json!({ "code": "return document.title" }),
                owner: ToolOwner::Server,
            },
            TurnEvent::ToolDone {
                id: call,
                result: Ok(json!("Example Domain")),
            },
            TurnEvent::StepEnd,
            TurnEvent::StepStart,
            TurnEvent::Text("It is Example Domain.".into()),
            TurnEvent::StepEnd,
        ]
    );
    assert_eq!(
        browser.calls(),
        vec![RecordedCall {
            scope: Scope {
                task: CONVERSATION.to_string(),
                access: Access::Full,
            },
            tool: "run".into(),
            args: json!({ "code": "return document.title" }),
        }]
    );
    Ok(())
}

#[tokio::test]
async fn a_turn_names_its_provider_by_id() -> anyhow::Result<()> {
    let state = tempfile::tempdir()?;
    let (agent, _) = agent(&state, Some("openai-1"), Script::default_answer(), vec![]).await?;
    let mut events = Vec::new();

    let end = run(
        &agent,
        Target::Provider(Some("openai-1".into())),
        &mut events,
    )
    .await;

    assert_eq!(end, TurnEnd::Done(FinishReason::Stop));
    assert!(events.contains(&TurnEvent::Text("Mock BrowserOS test response.".into())));
    Ok(())
}

#[tokio::test]
async fn no_provider_selected_fails_as_a_settings_problem_before_any_event() -> anyhow::Result<()> {
    let state = tempfile::tempdir()?;
    let (agent, browser) = agent(&state, None, Script::default_answer(), vec![]).await?;
    let mut events = Vec::new();

    let end = run(&agent, Target::Provider(None), &mut events).await;

    assert!(
        matches!(&end, TurnEnd::Failed(error) if error.code == ChatErrorCode::ProviderConfig && error.title == "No provider selected"),
        "{end:?}"
    );
    assert_eq!(events, Vec::new());
    assert!(browser.calls().is_empty());
    Ok(())
}

#[tokio::test]
async fn an_unknown_provider_fails_as_a_settings_problem_before_any_event() -> anyhow::Result<()> {
    let state = tempfile::tempdir()?;
    let (agent, _) = agent(&state, Some("openai-1"), Script::default_answer(), vec![]).await?;
    let mut events = Vec::new();

    let end = run(
        &agent,
        Target::Provider(Some("deleted".into())),
        &mut events,
    )
    .await;

    assert!(
        matches!(&end, TurnEnd::Failed(error) if error.code == ChatErrorCode::ProviderConfig && error.title == "Provider not found"),
        "{end:?}"
    );
    assert_eq!(events, Vec::new());
    Ok(())
}
