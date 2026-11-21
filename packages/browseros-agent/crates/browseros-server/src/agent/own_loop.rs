//! The server's own loop for an LLM provider. A turn is at most `MAX_STEPS` steps and
//! `MAX_TURN_TIME`; each step is one `Model::step`, then every tool call that response asked
//! for, one after another through `Browser::call`. The turn ends when a response asks for no
//! tool. Stop is checked at the model call, the browser call and between steps.
//!
//! ```text
//!   run: provider row ─▶ effort ─▶ Models::resolve ─▶ turn
//!   turn, per step:  StepStart ─▶ fit ─▶ Model::step ─▶ for each call: ToolInput ─▶ Browser::call ─▶ ToolDone ─▶ StepEnd
//! ```
//!
//! Nothing outlives the turn: the model's copy of the conversation is rebuilt from the stored
//! transcript every time (design rule 3).

use std::future::Future;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::context::Context;
use super::{MAX_STEPS, MAX_TURN_TIME, prompt};
use crate::browser::{Browser, Scope};
use crate::model::{Effort, Model, Models, StepRequest, ToolSpec};
use crate::store::{ProviderConfig, Store, StoreError};
use crate::turn::{
    ChatError, ChatErrorCode, EventSink, FinishReason, ToolOwner, TurnEnd, TurnEvent, TurnInput,
};

/// Runs one turn on a provider row (`None` = the selected row). A row that cannot be found or
/// read fails the turn before any event, so the panel shows only the error card.
///
/// One clock covers the whole turn: the hour starts before the row is read, and the read and
/// `resolve` (which can refresh a sign-in token over the network) are raced like every step.
pub(super) async fn run(
    models: &Models,
    browser: &Browser,
    store: &Store,
    provider: Option<&str>,
    input: &TurnInput,
    sink: &mut dyn EventSink,
    cancel: &CancellationToken,
) -> TurnEnd {
    let budget = Budget::from_now();
    let found = match race(store.providers().config(provider), cancel, budget.deadline).await {
        Ok(found) => found,
        Err(interrupted) => return interrupted.into(),
    };
    let config = match provider_config(provider, found) {
        Ok(config) => config,
        Err(error) => return TurnEnd::Failed(error),
    };
    // Read before `resolve` takes the row: `None` switches reasoning off for every step.
    let effort = effort(
        config.reasoning_effort.as_deref(),
        input.context.supports_reasoning,
    );
    let model = match race(
        models.resolve(config, input.conversation),
        cancel,
        budget.deadline,
    )
    .await
    {
        Ok(Ok(model)) => model,
        Ok(Err(error)) => return TurnEnd::Failed(error),
        Err(interrupted) => return interrupted.into(),
    };
    turn(&model, browser, input, effort, budget, sink, cancel).await
}

/// The turn's limits: at most `steps` model calls, and nothing awaited past `deadline`.
pub(super) struct Budget {
    pub steps: usize,
    pub deadline: Instant,
}

impl Budget {
    pub(super) fn from_now() -> Self {
        Self {
            steps: MAX_STEPS,
            deadline: Instant::now() + MAX_TURN_TIME,
        }
    }
}

/// Why an awaited call was abandoned.
enum Interrupted {
    Stopped,
    OutOfTime,
}

impl From<Interrupted> for TurnEnd {
    /// Running out of time ends the turn as a spent step budget does: a normal end, cut short.
    fn from(interrupted: Interrupted) -> Self {
        match interrupted {
            Interrupted::Stopped => TurnEnd::Stopped,
            Interrupted::OutOfTime => TurnEnd::Done(FinishReason::Length),
        }
    }
}

/// The one interruption rule for every await that can block (resolve, the model call, the
/// browser call). Stop wins over the deadline, and both over a result that is ready at the same
/// time. Dropping the losing `work` is what cancels it: the HTTPS stream to the provider closes,
/// and the engine cancels a script whose request was dropped.
async fn race<T>(
    work: impl Future<Output = T>,
    cancel: &CancellationToken,
    deadline: Instant,
) -> Result<T, Interrupted> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(Interrupted::Stopped),
        () = tokio::time::sleep_until(deadline) => Err(Interrupted::OutOfTime),
        output = work => Ok(output),
    }
}

/// Whether Stop or the hour has already passed: checked before anything new starts, so no step
/// begins and no call is marked as about to run once either has.
fn interruption(cancel: &CancellationToken, deadline: Instant) -> Option<Interrupted> {
    if cancel.is_cancelled() {
        Some(Interrupted::Stopped)
    } else if Instant::now() >= deadline {
        Some(Interrupted::OutOfTime)
    } else {
        None
    }
}

/// The step loop on a resolved model.
async fn turn(
    model: &Model,
    browser: &Browser,
    input: &TurnInput,
    effort: Option<Effort>,
    budget: Budget,
    sink: &mut dyn EventSink,
    cancel: &CancellationToken,
) -> TurnEnd {
    let deadline = budget.deadline;
    let tools: Vec<ToolSpec> = browser
        .tools()
        .into_iter()
        .map(|tool| ToolSpec {
            name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
        })
        .collect();
    let has_browser = !tools.is_empty();
    let system = prompt::system(&input.context, browser.guidance(), has_browser);
    // The permission every call of this turn runs under: the engine keys the turn's tabs by the
    // conversation id, and chat mode may read pages but never act on them.
    let scope = Scope {
        task: input.conversation.to_string(),
        access: input.context.mode.access(),
    };
    let overhead_chars = system.len()
        + tools
            .iter()
            .map(|tool| {
                tool.name.len() + tool.description.len() + tool.input_schema.to_string().len()
            })
            .sum::<usize>();
    let mut context = Context::from_history(&input.history, &input.context, overhead_chars);

    for _ in 0..budget.steps {
        if let Some(interrupted) = interruption(cancel, deadline) {
            return interrupted.into();
        }
        sink.emit(TurnEvent::StepStart).await;
        context.fit(model.window());

        let request = StepRequest {
            system: &system,
            messages: context.messages(),
            tools: &tools,
            effort,
        };
        let mut counting = Counting {
            sink: &mut *sink,
            chars: 0,
        };
        let assistant = match race(model.step(&request, &mut counting), cancel, deadline).await {
            Ok(Ok(assistant)) => assistant,
            Ok(Err(error)) => return TurnEnd::Failed(error),
            Err(interrupted) => return interrupted.into(),
        };
        let streamed_chars = counting.chars;
        let calls = assistant.tool_calls().to_vec();
        let finish = assistant.finish();
        context.push_assistant(assistant, streamed_chars);

        // In order, one at a time: browser calls have side effects, and the next often depends
        // on the last.
        for call in &calls {
            // The previous call's `ToolDone` write may have outlived the hour: a call that will
            // not run must not be marked as about to run.
            if let Some(interrupted) = interruption(cancel, deadline) {
                return interrupted.into();
            }
            // The checkpoint: the run's sink returns only once this row is on disk, so the intent
            // is recorded before the browser moves. Emitted for the call about to run, never for
            // one still queued behind it. The sink's writes are not raced: dropping one would
            // cancel the run's own checkpoint, and a hung write is the run task's to drop.
            sink.emit(TurnEvent::ToolInput {
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.input.clone(),
                owner: ToolOwner::Server,
            })
            .await;
            let called = browser.call(&scope, &call.name, call.input.clone(), cancel);
            // An interrupted call gets no `ToolDone`: the reducer seals the open call.
            let output = match race(called, cancel, deadline).await {
                Ok(output) => output,
                Err(interrupted) => return interrupted.into(),
            };
            sink.emit(TurnEvent::ToolDone {
                id: call.id.clone(),
                result: output.for_panel(),
            })
            .await;
            context.push_tool_result(call, output, model.supports_images());
            // Calls still queued stay `ToolStarted`; the reducer seals them "Not run".
            if cancel.is_cancelled() {
                return TurnEnd::Stopped;
            }
        }
        sink.emit(TurnEvent::StepEnd).await;

        if cancel.is_cancelled() {
            return TurnEnd::Stopped;
        }
        if calls.is_empty() {
            return TurnEnd::Done(finish);
        }
    }
    TurnEnd::Done(FinishReason::Length)
}

/// Forwards every event unchanged and counts the characters of streamed text and reasoning:
/// the size of a response whose text is opaque to the loop, for compaction's estimate.
struct Counting<'a> {
    sink: &'a mut dyn EventSink,
    chars: usize,
}

#[async_trait::async_trait]
impl EventSink for Counting<'_> {
    async fn emit(&mut self, event: TurnEvent) {
        if let TurnEvent::Text(delta) | TurnEvent::Reasoning(delta) = &event {
            self.chars += delta.len();
        }
        self.sink.emit(event).await;
    }
}

/// The provider row a turn runs on. A missing row is the user's to fix in settings; a store that
/// cannot be read is not, so it is not coded as a settings problem.
fn provider_config(
    provider: Option<&str>,
    found: Result<Option<ProviderConfig>, StoreError>,
) -> Result<ProviderConfig, ChatError> {
    match (found, provider) {
        (Ok(Some(config)), _) => Ok(config),
        (Ok(None), None) => Err(ChatError::new(
            ChatErrorCode::ProviderConfig,
            "No provider selected",
            "Choose an AI provider in settings, then try again.",
        )),
        (Ok(None), Some(_)) => Err(ChatError::new(
            ChatErrorCode::ProviderConfig,
            "Provider not found",
            "The selected AI provider no longer exists. Choose another one in settings.",
        )),
        (Err(error), _) => Err(ChatError::new(
            ChatErrorCode::Unknown,
            "Couldn't read your provider settings",
            error.to_string(),
        )),
    }
}

/// The per-call reasoning switch and its coarse fallback (CORRECTIONS-1, clarification 2).
/// `None` is off: the model does not reason, or the row says `none`. `Some` is on, and `model`
/// reads the row's exact value itself; this level is used only where its wire has no match.
fn effort(row: Option<&str>, supports_reasoning: bool) -> Option<Effort> {
    if !supports_reasoning {
        return None;
    }
    match row {
        Some("none") => None,
        Some("low" | "minimal") => Some(Effort::Low),
        Some("high" | "xhigh" | "max") => Some(Effort::High),
        _ => Some(Effort::Medium),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::browser::{Access, RecordedCall, ScriptedBrowser, ScriptedReply};
    use crate::model::{Script, ScriptedError, ScriptedStep};
    use crate::store::{Credentials, ProviderKind};
    use crate::turn::{
        ConversationId, MessageId, Mode, Role, ToolCallId, TurnContext, UiMessage, UiPart,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const CONVERSATION: ConversationId =
        uuid::Uuid::from_u128(0x5eed_0000_0000_4000_8000_0000_0000_0001);

    fn input(mode: Mode) -> TurnInput {
        let question = UiMessage {
            id: MessageId("u1".into()),
            role: Role::User,
            parts: vec![UiPart::Text {
                text: "What is the page title?".into(),
                state: None,
                extra: serde_json::Map::new(),
            }],
            metadata: None,
        };
        TurnInput {
            conversation: CONVERSATION,
            history: Arc::from(vec![question]),
            context: TurnContext {
                mode,
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

    fn model(steps: Vec<ScriptedStep>) -> Model {
        Model::scripted(Script { steps }, 0, true)
    }

    fn run_code(code: &str) -> serde_json::Value {
        json!({ "code": code })
    }

    fn call_id(step: usize, position: usize) -> ToolCallId {
        ToolCallId(format!("call_scripted_{step}_{position}"))
    }

    /// A step that asks for `count` calls of `run` and says nothing.
    fn calls(count: usize) -> ScriptedStep {
        ScriptedStep {
            tool_calls: (0..count)
                .map(|index| crate::model::ScriptedCall {
                    name: "run".into(),
                    input: run_code(&format!("return {index}")),
                    id: None,
                })
                .collect(),
            ..ScriptedStep::default()
        }
    }

    fn budget(steps: usize) -> Budget {
        Budget {
            steps,
            ..Budget::from_now()
        }
    }

    #[tokio::test]
    async fn a_two_step_turn_emits_the_events_in_order() {
        let model = model(vec![
            ScriptedStep {
                reasoning: Some("The title is on the page.".into()),
                text: Some("Reading the page.".into()),
                ..ScriptedStep::call("run", run_code("return document.title"))
            },
            ScriptedStep::answer("It is Example Domain."),
        ]);
        let (browser, handle) = Browser::scripted([ScriptedReply::text("Example Domain")]);
        let mut events = Vec::new();

        let end = turn(
            &model,
            &browser,
            &input(Mode::Agent),
            None,
            Budget::from_now(),
            &mut events,
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(end, TurnEnd::Done(FinishReason::Stop));
        assert_eq!(
            events,
            vec![
                TurnEvent::StepStart,
                TurnEvent::Reasoning("The title is on the page.".into()),
                TurnEvent::Text("Reading the page.".into()),
                TurnEvent::ToolStarted {
                    id: call_id(0, 0),
                    name: "run".into(),
                    owner: ToolOwner::Server,
                },
                TurnEvent::ToolInput {
                    id: call_id(0, 0),
                    name: "run".into(),
                    input: run_code("return document.title"),
                    owner: ToolOwner::Server,
                },
                TurnEvent::ToolDone {
                    id: call_id(0, 0),
                    result: Ok(json!({
                        "content": [{ "type": "text", "text": "Example Domain" }],
                        "isError": false
                    })),
                },
                TurnEvent::StepEnd,
                TurnEvent::StepStart,
                TurnEvent::Text("It is Example Domain.".into()),
                TurnEvent::StepEnd,
            ]
        );
        assert_eq!(
            handle.calls(),
            vec![RecordedCall {
                scope: Scope {
                    task: CONVERSATION.to_string(),
                    access: Access::Full,
                },
                tool: "run".into(),
                args: run_code("return document.title"),
            }]
        );
    }

    /// Cancels `cancel` 20 ms from now and returns when it did.
    fn cancel_soon(cancel: &CancellationToken) -> tokio::task::JoinHandle<std::time::Instant> {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let cancelled_at = std::time::Instant::now();
            cancel.cancel();
            cancelled_at
        })
    }

    #[tokio::test]
    async fn stop_during_the_model_call_ends_the_turn_at_once() -> TestResult {
        let model = model(vec![ScriptedStep {
            delay_ms: 10_000,
            ..ScriptedStep::answer("too late")
        }]);
        let (browser, _) = Browser::scripted([]);
        let cancel = CancellationToken::new();
        let canceller = cancel_soon(&cancel);
        let mut events = Vec::new();

        let end = turn(
            &model,
            &browser,
            &input(Mode::Agent),
            None,
            Budget::from_now(),
            &mut events,
            &cancel,
        )
        .await;
        let returned_at = std::time::Instant::now();

        assert_eq!(end, TurnEnd::Stopped);
        let latency = returned_at.duration_since(canceller.await?);
        assert!(
            latency < Duration::from_millis(100),
            "stopped after {latency:?}"
        );
        assert_eq!(events, vec![TurnEvent::StepStart]);
        Ok(())
    }

    #[tokio::test]
    async fn stop_during_the_tool_call_ends_the_turn_at_once() -> TestResult {
        let model = model(vec![ScriptedStep::call("run", run_code("await forever()"))]);
        let (browser, _) = Browser::scripted([ScriptedReply::UntilCancelled]);
        let cancel = CancellationToken::new();
        let canceller = cancel_soon(&cancel);
        let mut events = Vec::new();

        let end = turn(
            &model,
            &browser,
            &input(Mode::Agent),
            None,
            Budget::from_now(),
            &mut events,
            &cancel,
        )
        .await;
        let returned_at = std::time::Instant::now();

        assert_eq!(end, TurnEnd::Stopped);
        let latency = returned_at.duration_since(canceller.await?);
        assert!(
            latency < Duration::from_millis(100),
            "stopped after {latency:?}"
        );
        assert!(
            matches!(events.last(), Some(TurnEvent::ToolInput { .. })),
            "an interrupted call gets no ToolDone: {events:?}"
        );
        Ok(())
    }

    /// Records, at each `ToolInput` and `ToolDone`, how many calls the browser had received.
    struct CallCounter {
        browser: ScriptedBrowser,
        seen: Vec<(&'static str, ToolCallId, usize)>,
    }

    #[async_trait::async_trait]
    impl EventSink for CallCounter {
        async fn emit(&mut self, event: TurnEvent) {
            let received = self.browser.calls().len();
            match event {
                TurnEvent::ToolInput { id, .. } => self.seen.push(("input", id, received)),
                TurnEvent::ToolDone { id, .. } => self.seen.push(("done", id, received)),
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn tool_input_is_emitted_right_before_its_own_call_runs() {
        let model = model(vec![calls(2), ScriptedStep::answer("Both ran.")]);
        let (browser, handle) =
            Browser::scripted([ScriptedReply::text("0"), ScriptedReply::text("1")]);
        let mut counter = CallCounter {
            browser: handle,
            seen: Vec::new(),
        };

        turn(
            &model,
            &browser,
            &input(Mode::Agent),
            None,
            Budget::from_now(),
            &mut counter,
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(
            counter.seen,
            vec![
                ("input", call_id(0, 0), 0),
                ("done", call_id(0, 0), 1),
                ("input", call_id(0, 1), 1),
                ("done", call_id(0, 1), 2),
            ]
        );
    }

    /// Keeps every event, and presses Stop when the first tool returns.
    struct StopAfterFirstTool {
        cancel: CancellationToken,
        events: Vec<TurnEvent>,
    }

    #[async_trait::async_trait]
    impl EventSink for StopAfterFirstTool {
        async fn emit(&mut self, event: TurnEvent) {
            if matches!(event, TurnEvent::ToolDone { .. }) {
                self.cancel.cancel();
            }
            self.events.push(event);
        }
    }

    #[tokio::test]
    async fn a_stop_after_the_first_of_two_calls_never_marks_the_second() {
        let model = model(vec![calls(2), ScriptedStep::answer("unreachable")]);
        let (browser, handle) =
            Browser::scripted([ScriptedReply::text("0"), ScriptedReply::text("1")]);
        let cancel = CancellationToken::new();
        let mut sink = StopAfterFirstTool {
            cancel: cancel.clone(),
            events: Vec::new(),
        };

        let end = turn(
            &model,
            &browser,
            &input(Mode::Agent),
            None,
            Budget::from_now(),
            &mut sink,
            &cancel,
        )
        .await;

        assert_eq!(end, TurnEnd::Stopped);
        let started = |position| TurnEvent::ToolStarted {
            id: call_id(0, position),
            name: "run".into(),
            owner: ToolOwner::Server,
        };
        assert_eq!(
            sink.events,
            vec![
                TurnEvent::StepStart,
                started(0),
                started(1),
                TurnEvent::ToolInput {
                    id: call_id(0, 0),
                    name: "run".into(),
                    input: run_code("return 0"),
                    owner: ToolOwner::Server,
                },
                TurnEvent::ToolDone {
                    id: call_id(0, 0),
                    result: Ok(json!({
                        "content": [{ "type": "text", "text": "0" }],
                        "isError": false
                    })),
                },
            ]
        );
        assert_eq!(handle.calls().len(), 1);
    }

    #[tokio::test]
    async fn a_spent_step_budget_ends_the_turn_as_length() {
        let model = model((0..5).map(|_| calls(1)).collect());
        let (browser, _) = Browser::scripted((0..5).map(|_| ScriptedReply::text("again")));
        let mut events = Vec::new();

        let end = turn(
            &model,
            &browser,
            &input(Mode::Agent),
            None,
            budget(3),
            &mut events,
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(end, TurnEnd::Done(FinishReason::Length));
        let steps = events
            .iter()
            .filter(|event| **event == TurnEvent::StepStart)
            .count();
        assert_eq!(steps, 3);
        assert_eq!(MAX_STEPS, 100);
        assert_eq!(MAX_TURN_TIME, Duration::from_secs(3_600));
    }

    #[tokio::test(start_paused = true)]
    async fn a_turn_that_runs_out_of_time_ends_as_length() {
        let model = model(vec![ScriptedStep {
            delay_ms: 2 * 60 * 60 * 1_000,
            ..ScriptedStep::answer("two hours later")
        }]);
        let (browser, _) = Browser::scripted([]);

        let end = turn(
            &model,
            &browser,
            &input(Mode::Agent),
            None,
            Budget::from_now(),
            &mut Vec::new(),
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(end, TurnEnd::Done(FinishReason::Length));
    }

    /// Spends the turn's hour inside the first `ToolDone` write, as a slow checkpoint would.
    struct SlowFirstDone {
        events: Vec<TurnEvent>,
        slowed: bool,
    }

    #[async_trait::async_trait]
    impl EventSink for SlowFirstDone {
        async fn emit(&mut self, event: TurnEvent) {
            let done = matches!(event, TurnEvent::ToolDone { .. });
            self.events.push(event);
            if done && !self.slowed {
                self.slowed = true;
                tokio::time::sleep(Duration::from_secs(2 * 60 * 60)).await;
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_hour_spent_in_a_checkpoint_never_marks_the_next_call() {
        let model = model(vec![calls(2)]);
        let (browser, handle) =
            Browser::scripted([ScriptedReply::text("0"), ScriptedReply::text("1")]);
        let mut sink = SlowFirstDone {
            events: Vec::new(),
            slowed: false,
        };

        let end = turn(
            &model,
            &browser,
            &input(Mode::Agent),
            None,
            Budget::from_now(),
            &mut sink,
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(end, TurnEnd::Done(FinishReason::Length));
        let marked = sink
            .events
            .iter()
            .filter(|event| matches!(event, TurnEvent::ToolInput { .. }))
            .count();
        assert_eq!(marked, 1, "{:?}", sink.events);
        assert_eq!(handle.calls().len(), 1);
    }

    #[tokio::test]
    async fn a_model_error_fails_the_turn_with_its_code() {
        let model = model(vec![ScriptedStep {
            error: Some(ScriptedError {
                code: "rate_limited".into(),
                message: "Slow down.".into(),
            }),
            ..ScriptedStep::default()
        }]);
        let (browser, _) = Browser::scripted([]);

        let end = turn(
            &model,
            &browser,
            &input(Mode::Agent),
            None,
            Budget::from_now(),
            &mut Vec::new(),
            &CancellationToken::new(),
        )
        .await;

        assert!(
            matches!(&end, TurnEnd::Failed(error) if error.code == ChatErrorCode::RateLimited),
            "{end:?}"
        );
    }

    #[tokio::test]
    async fn chat_mode_calls_the_browser_read_only() {
        let model = model(vec![calls(1), ScriptedStep::answer("Read it.")]);
        let (browser, handle) = Browser::scripted([ScriptedReply::text("a page")]);

        turn(
            &model,
            &browser,
            &input(Mode::Chat),
            None,
            Budget::from_now(),
            &mut Vec::new(),
            &CancellationToken::new(),
        )
        .await;

        let access: Vec<Access> = handle
            .calls()
            .into_iter()
            .map(|call| call.scope.access)
            .collect();
        assert_eq!(access, vec![Access::Read]);
    }

    fn row(reasoning_effort: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            id: "p1".into(),
            kind: ProviderKind::Openai,
            model: "gpt-5".into(),
            base_url: None,
            headers: Vec::new(),
            context_window: 128_000,
            supports_images: true,
            reasoning_effort: reasoning_effort.map(str::to_owned),
            reasoning_summary: None,
            resource_name: None,
            credentials: Credentials::None,
        }
    }

    fn code(result: Result<ProviderConfig, ChatError>) -> Option<ChatErrorCode> {
        result.err().map(|error| error.code)
    }

    #[test]
    fn a_found_row_is_the_turns_provider() {
        let found = provider_config(Some("p1"), Ok(Some(row(None))));
        assert!(matches!(found, Ok(config) if config.id == "p1"));
    }

    #[test]
    fn no_provider_selected_is_a_settings_problem() {
        assert_eq!(
            code(provider_config(None, Ok(None))),
            Some(ChatErrorCode::ProviderConfig)
        );
    }

    #[test]
    fn an_unknown_provider_is_a_settings_problem() {
        assert_eq!(
            code(provider_config(Some("gone"), Ok(None))),
            Some(ChatErrorCode::ProviderConfig)
        );
    }

    #[test]
    fn a_store_that_cannot_be_read_is_not_a_settings_problem() {
        let failed = provider_config(None, Err(StoreError::Unreadable("providers".into())));
        assert_eq!(code(failed), Some(ChatErrorCode::Unknown));
    }

    #[test]
    fn the_row_sets_the_effort_unless_the_model_does_not_reason() {
        let table = [
            (Some("none"), true, None),
            (Some("minimal"), true, Some(Effort::Low)),
            (Some("low"), true, Some(Effort::Low)),
            (Some("medium"), true, Some(Effort::Medium)),
            (Some("high"), true, Some(Effort::High)),
            (Some("xhigh"), true, Some(Effort::High)),
            (Some("max"), true, Some(Effort::High)),
            (Some("turbo"), true, Some(Effort::Medium)),
            (None, true, Some(Effort::Medium)),
            (Some("high"), false, None),
            (Some("none"), false, None),
            (None, false, None),
        ];
        for (row, supports_reasoning, expected) in table {
            assert_eq!(
                effort(row, supports_reasoning),
                expected,
                "row {row:?}, supports_reasoning {supports_reasoning}"
            );
        }
    }
}
