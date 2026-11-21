//! The model's copy of the conversation for one turn. Built from the stored transcript at the
//! start of the turn (earlier turns as text, each tool part as one line, reasoning dropped, the
//! current user message wrapped), extended in memory with this turn's responses and tool
//! results, compacted to fit the provider's context window, and dropped when the turn ends.
//!
//! Earlier turns never carry real tool calls: only the current turn sends tool calls, results
//! and the provider's own record of each response (signed reasoning) back to the model. That is
//! what keeps tool parts of tools that no longer exist, replays missing their signed reasoning,
//! and a provider changed between turns from ever reaching a wire (design section 3.3).
//!
//! The stored transcript is only borrowed: nothing here can change what is stored or shown.

use serde_json::Value;

use super::prompt;
use crate::browser::ToolOutput;
use crate::model::{AssistantTurn, Image, ModelMessage, ToolCall};
use crate::turn::{Role, ToolOutcome, ToolPart, TurnContext, UiMessage, UiPart};

/// What a compacted tool result or picture message says instead of its content.
const PLACEHOLDER: &str = "[removed to fit the context window]";
/// The window assumed for a provider row that names none (the old server's default).
const DEFAULT_WINDOW: usize = 200_000;
/// Tokens kept free for the response: this much, or half the window when that is smaller.
const RESERVE: usize = 20_000;
/// Tool results of this many of the latest calls survive the first compaction stage.
const KEEP_RECENT_CALLS: usize = 3;
const CHARS_PER_TOKEN: usize = 3;
const TOKENS_PER_PICTURE: usize = 1_000;
/// How much of an earlier turn's tool input and result its one line keeps, in characters.
const INPUT_CHARS: usize = 300;
const RESULT_CHARS: usize = 1_500;

/// The model's messages for one turn, plus what compaction needs to know about each.
///
/// `ModelMessage` is not `Clone` and a request borrows `&[ModelMessage]`, so the messages
/// themselves are the primary storage and `meta[i]` describes `messages[i]`. Only this type's
/// methods push or remove, always both together, so the two never drift.
pub(super) struct Context {
    messages: Vec<ModelMessage>,
    meta: Vec<Meta>,
    /// The system prompt and tool definitions, in characters, for the character estimate.
    overhead_chars: usize,
    /// The last response's usage, while the copy is unchanged since it was reported.
    reported: Option<Reported>,
    /// Tool results pushed so far this turn: the ordinal of the next one.
    calls: usize,
    /// The id of the next group.
    groups: usize,
}

/// A provider's own count of the copy as it stood after one response.
struct Reported {
    /// Input plus output tokens of that response.
    tokens: u64,
    /// `messages.len()` right after that response was pushed: what came later is estimated.
    at: usize,
}

/// One message as compaction sees it.
///
/// A group is what compaction keeps or drops whole: a user message alone, or an assistant turn
/// with its tool results and the picture message after a result.
#[derive(Clone, Copy)]
struct Meta {
    group: usize,
    kind: Kind,
    /// The message's size in bytes: a slight over-estimate for non-ASCII text, which errs safe.
    chars: usize,
    images: usize,
}

#[derive(Clone, Copy)]
enum Kind {
    /// `pinned` for the first user message and the current one: compaction never drops them.
    User {
        pinned: bool,
    },
    Assistant,
    /// `call` is the ordinal of the tool result this turn, which decides its age.
    ToolResult {
        call: usize,
    },
    /// The user message carrying a tool result's pictures, right after that result.
    Pictures {
        call: usize,
    },
}

impl Meta {
    fn pinned(&self) -> bool {
        matches!(self.kind, Kind::User { pinned: true })
    }

    /// The ordinal of the tool call this message holds the result or pictures of.
    fn result_of(&self) -> Option<usize> {
        match self.kind {
            Kind::ToolResult { call } | Kind::Pictures { call } => Some(call),
            Kind::User { .. } | Kind::Assistant => None,
        }
    }
}

impl Context {
    /// The model's copy of `history`, which ends with the user message this turn answers.
    pub(super) fn from_history(
        history: &[UiMessage],
        context: &TurnContext,
        overhead_chars: usize,
    ) -> Self {
        let (projected, current) = project(history, context);
        let mut this = Self {
            messages: Vec::with_capacity(projected.len()),
            meta: Vec::with_capacity(projected.len()),
            overhead_chars,
            reported: None,
            calls: 0,
            groups: 0,
        };
        let mut seen_user = false;
        for (index, message) in projected.into_iter().enumerate() {
            let group = this.new_group();
            match message {
                Projected::User(text) => {
                    let pinned = !seen_user || current == Some(index);
                    seen_user = true;
                    let meta = Meta {
                        group,
                        kind: Kind::User { pinned },
                        chars: text.len(),
                        images: 0,
                    };
                    this.push(
                        ModelMessage::User {
                            text,
                            images: Vec::new(),
                        },
                        meta,
                    );
                }
                Projected::Assistant(text) => {
                    let meta = Meta {
                        group,
                        kind: Kind::Assistant,
                        chars: text.len(),
                        images: 0,
                    };
                    this.push(ModelMessage::Assistant(AssistantTurn::text(text)), meta);
                }
            }
        }
        this
    }

    pub(super) fn messages(&self) -> &[ModelMessage] {
        &self.messages
    }

    /// A response of this turn, kept whole (with the provider's own record) for the next step.
    /// `streamed_chars` is the text and reasoning it streamed: the turn's text is opaque here.
    pub(super) fn push_assistant(&mut self, turn: AssistantTurn, streamed_chars: usize) {
        let call_chars: usize = turn
            .tool_calls()
            .iter()
            .map(|call| call.name.len() + call.input.to_string().len())
            .sum();
        let usage = turn.usage();
        let meta = Meta {
            group: self.new_group(),
            kind: Kind::Assistant,
            chars: streamed_chars + call_chars,
            images: 0,
        };
        self.push(ModelMessage::Assistant(turn), meta);
        self.reported = usage.map(|usage| Reported {
            tokens: usage.input_tokens + usage.output_tokens,
            at: self.messages.len(),
        });
    }

    /// The result of `call`, in the group of the response that asked for it. Every wire takes a
    /// tool result as text in the first version, so a failed script is marked in the text, and
    /// pictures follow as a user message when the model takes images (or a note says they
    /// were left out).
    pub(super) fn push_tool_result(
        &mut self,
        call: &ToolCall,
        output: ToolOutput,
        supports_images: bool,
    ) {
        let ordinal = self.calls;
        self.calls += 1;
        // The loop pushes a result only after the response that asked for it, so the newest
        // group is that response's: the result joins it and compaction cannot separate them.
        let group = self.groups.saturating_sub(1);
        let pictures = output.pictures.len();
        let mut text = if output.is_error {
            format!("Error: {}", output.text)
        } else {
            output.text
        };
        if pictures > 0 && !supports_images {
            text.push_str(&format!(
                "\n[The tool result had {pictures} {}; this model cannot see images.]",
                if pictures == 1 { "picture" } else { "pictures" }
            ));
        }
        let meta = Meta {
            group,
            kind: Kind::ToolResult { call: ordinal },
            chars: text.len(),
            images: 0,
        };
        self.push(
            ModelMessage::ToolResult {
                id: call.id.clone(),
                text,
            },
            meta,
        );
        if pictures > 0 && supports_images {
            let text = if pictures == 1 {
                "Attached image from the tool result:"
            } else {
                "Attached images from the tool result:"
            };
            let images = output
                .pictures
                .into_iter()
                .map(|picture| Image {
                    media_type: picture.media_type,
                    bytes: picture.bytes,
                })
                .collect();
            let meta = Meta {
                group,
                kind: Kind::Pictures { call: ordinal },
                chars: text.len(),
                images: pictures,
            };
            self.push(
                ModelMessage::User {
                    text: text.to_owned(),
                    images,
                },
                meta,
            );
        }
    }

    /// Shrinks the copy until its estimate fits `window` (0 = unknown) less a reserve for the
    /// response, in three stages, stopping as soon as it fits: tool results and pictures older
    /// than the last three calls become a placeholder, then all of them do, then whole groups
    /// are dropped, oldest first.
    ///
    /// Invariant: an assistant turn, its tool results and its picture messages are one group,
    /// kept or dropped together, so a result never loses its call and the provider's own record
    /// of a response (signed reasoning) is never split from its calls. A wire would refuse
    /// either. No model is asked to summarise anything.
    pub(super) fn fit(&mut self, window: usize) {
        let window = if window == 0 { DEFAULT_WINDOW } else { window };
        let limit = window - RESERVE.min(window / 2);
        if self.estimate() <= limit {
            return;
        }
        let recent = self.calls.saturating_sub(KEEP_RECENT_CALLS);
        if self.replace_results(recent) && self.estimate() <= limit {
            return;
        }
        if self.replace_results(usize::MAX) && self.estimate() <= limit {
            return;
        }
        self.drop_groups(limit);
    }

    fn new_group(&mut self) -> usize {
        self.groups += 1;
        self.groups - 1
    }

    fn push(&mut self, message: ModelMessage, meta: Meta) {
        self.messages.push(message);
        self.meta.push(meta);
    }

    /// Tokens the copy is estimated to take. The provider's own count is the better input, so
    /// it is used while it still describes the copy; a change to the copy clears it.
    fn estimate(&self) -> usize {
        match &self.reported {
            Some(reported) => {
                let since = self.meta.get(reported.at..).unwrap_or_default();
                let (chars, images) = size(since.iter());
                usize::try_from(reported.tokens)
                    .unwrap_or(usize::MAX)
                    .saturating_add(tokens(chars, images))
            }
            None => {
                let (chars, images) = size(self.meta.iter());
                tokens(self.overhead_chars + chars, images)
            }
        }
    }

    /// Replaces each tool result and picture message of a call older than `older_than` with the
    /// placeholder. One already replaced is left alone, so a second `fit` changes nothing.
    /// Returns whether anything changed.
    fn replace_results(&mut self, older_than: usize) -> bool {
        let mut changed = false;
        for (message, meta) in self.messages.iter_mut().zip(self.meta.iter_mut()) {
            if !meta.result_of().is_some_and(|call| call < older_than) {
                continue;
            }
            match message {
                ModelMessage::ToolResult { text, .. } if text.as_str() != PLACEHOLDER => {
                    *text = PLACEHOLDER.to_owned();
                }
                ModelMessage::User { text, images }
                    if text.as_str() != PLACEHOLDER || !images.is_empty() =>
                {
                    *text = PLACEHOLDER.to_owned();
                    images.clear();
                }
                _ => continue,
            }
            meta.chars = PLACEHOLDER.len();
            meta.images = 0;
            changed = true;
        }
        if changed {
            self.reported = None;
        }
        changed
    }

    /// Keeps every group with a pinned user message and the newest group, then the longest run
    /// of the newest other groups whose character estimate still fits, and drops the rest.
    fn drop_groups(&mut self, limit: usize) {
        struct Group {
            id: usize,
            chars: usize,
            images: usize,
            keep: bool,
        }
        let mut groups: Vec<Group> = Vec::new();
        for meta in &self.meta {
            match groups.last_mut() {
                Some(group) if group.id == meta.group => {
                    group.chars += meta.chars;
                    group.images += meta.images;
                    group.keep |= meta.pinned();
                }
                _ => groups.push(Group {
                    id: meta.group,
                    chars: meta.chars,
                    images: meta.images,
                    keep: meta.pinned(),
                }),
            }
        }
        if let Some(newest) = groups.last_mut() {
            newest.keep = true;
        }
        let (mut chars, mut images) = groups
            .iter()
            .filter(|group| group.keep)
            .fold((self.overhead_chars, 0), |(chars, images), group| {
                (chars + group.chars, images + group.images)
            });
        for group in groups.iter_mut().rev().filter(|group| !group.keep) {
            if tokens(chars + group.chars, images + group.images) > limit {
                break;
            }
            chars += group.chars;
            images += group.images;
            group.keep = true;
        }
        if groups.iter().all(|group| group.keep) {
            return;
        }
        let kept: Vec<usize> = groups
            .iter()
            .filter(|group| group.keep)
            .map(|group| group.id)
            .collect();
        let messages = std::mem::take(&mut self.messages);
        let meta = std::mem::take(&mut self.meta);
        (self.messages, self.meta) = messages
            .into_iter()
            .zip(meta)
            .filter(|(_, meta)| kept.binary_search(&meta.group).is_ok())
            .unzip();
        self.reported = None;
    }
}

/// Characters and pictures of some messages.
fn size<'a>(meta: impl Iterator<Item = &'a Meta>) -> (usize, usize) {
    meta.fold((0, 0), |(chars, images), meta| {
        (chars + meta.chars, images + meta.images)
    })
}

/// The character estimate: a third of a token per character, a thousand per picture.
fn tokens(chars: usize, images: usize) -> usize {
    chars.div_ceil(CHARS_PER_TOKEN) + images * TOKENS_PER_PICTURE
}

/// One message of the stored transcript as the model reads it, before it becomes a
/// `ModelMessage` (whose text cannot be read back, so tests check this form).
#[derive(Debug, PartialEq)]
enum Projected {
    User(String),
    Assistant(String),
}

/// The transcript projected for the model, and the index of the current user message in it.
/// The last message, when it is the user's, is the one this turn answers: it is wrapped with the
/// browser context and kept even when empty. Earlier user messages stay raw, and messages with
/// nothing to say are left out.
fn project(history: &[UiMessage], context: &TurnContext) -> (Vec<Projected>, Option<usize>) {
    let (current, earlier) = match history.split_last() {
        Some((last, earlier)) if last.role == Role::User => (Some(last), earlier),
        _ => (None, history),
    };
    let mut projected: Vec<Projected> = earlier.iter().filter_map(project_earlier).collect();
    let current = current.map(|message| {
        let wrapped = prompt::wrap_user_message(&user_text(message), context);
        projected.push(Projected::User(wrapped));
        projected.len() - 1
    });
    (projected, current)
}

fn project_earlier(message: &UiMessage) -> Option<Projected> {
    let projected = match message.role {
        Role::User => Projected::User(user_text(message)),
        Role::Assistant => Projected::Assistant(assistant_text(message)),
        Role::System => return None,
    };
    match &projected {
        Projected::User(text) | Projected::Assistant(text) if text.trim().is_empty() => None,
        _ => Some(projected),
    }
}

fn user_text(message: &UiMessage) -> String {
    let texts: Vec<&str> = message
        .parts
        .iter()
        .filter_map(|part| match part {
            UiPart::Text { text, .. } if !text.is_empty() => Some(text.as_str()),
            _ => None,
        })
        .collect();
    texts.join("\n")
}

/// Text parts verbatim and each tool part as one line, in part order. Reasoning is never
/// replayed (as in the old server), and pictures were never stored.
fn assistant_text(message: &UiMessage) -> String {
    let lines: Vec<String> = message
        .parts
        .iter()
        .filter_map(|part| match part {
            UiPart::Text { text, .. } if !text.is_empty() => Some(text.clone()),
            UiPart::Tool(tool) => Some(tool_line(tool)),
            UiPart::Text { .. }
            | UiPart::Reasoning { .. }
            | UiPart::StepStart
            | UiPart::Other(_) => None,
        })
        .collect();
    lines.join("\n")
}

/// `[<name>: <input> → <result>]`: the script itself when the input is `run`'s `{ code }`.
fn tool_line(tool: &ToolPart) -> String {
    let input = match &tool.input {
        None => String::new(),
        Some(input) => match input.get("code") {
            Some(Value::String(code)) => code.clone(),
            _ => input.to_string(),
        },
    };
    let result = match &tool.outcome {
        ToolOutcome::Output(Value::String(text)) => text.clone(),
        ToolOutcome::Output(value) => value.to_string(),
        ToolOutcome::Error(error) => format!("error: {error}"),
        ToolOutcome::InputAvailable | ToolOutcome::InputStreaming => "no result".to_owned(),
    };
    format!(
        "[{}: {} → {}]",
        tool.name,
        cut(&input, INPUT_CHARS),
        cut(&result, RESULT_CHARS)
    )
}

/// The first `limit` characters of `text`, with `…` when anything was cut.
fn cut(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::Picture;
    use crate::model::{Model, Script, ScriptedCall, ScriptedStep, ScriptedUsage, StepRequest};
    use crate::turn::{MessageId, Mode, TabRef};
    use serde_json::json;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn turn_context() -> TurnContext {
        TurnContext {
            mode: Mode::Agent,
            window_id: None,
            active_tab: None,
            selected_tabs: Vec::new(),
            selected_text: None,
            user_system_prompt: None,
            working_dir: None,
            supports_reasoning: true,
        }
    }

    fn message(role: Role, text: &str) -> UiMessage {
        UiMessage {
            id: MessageId(format!("m-{text:.8}")),
            role,
            parts: vec![UiPart::Text {
                text: text.to_owned(),
                state: None,
                extra: serde_json::Map::new(),
            }],
            metadata: None,
        }
    }

    /// One response of this turn, from the scripted model: the only public way to get an
    /// `AssistantTurn` with tool calls or usage.
    async fn response(
        call_ids: &[&str],
        usage: Option<ScriptedUsage>,
    ) -> Result<AssistantTurn, crate::turn::ChatError> {
        let step = ScriptedStep {
            tool_calls: call_ids
                .iter()
                .map(|id| ScriptedCall {
                    name: "run".into(),
                    input: json!({ "code": "return 1" }),
                    id: Some((*id).to_owned()),
                })
                .collect(),
            usage,
            ..ScriptedStep::default()
        };
        let request = StepRequest {
            system: "",
            messages: &[],
            tools: &[],
            effort: None,
        };
        Model::scripted(Script { steps: vec![step] }, 0, true)
            .step(&request, &mut Vec::new())
            .await
    }

    fn output(text: &str) -> ToolOutput {
        ToolOutput {
            text: text.to_owned(),
            pictures: Vec::new(),
            is_error: false,
        }
    }

    /// Pushes one step: a response asking for `call_id`, then that call's result.
    async fn push_step(
        context: &mut Context,
        call_id: &str,
        streamed_chars: usize,
        result: ToolOutput,
        usage: Option<ScriptedUsage>,
    ) -> TestResult {
        let turn = response(&[call_id], usage).await?;
        let call = turn.tool_calls().first().cloned().ok_or("no call")?;
        context.push_assistant(turn, streamed_chars);
        context.push_tool_result(&call, result, true);
        Ok(())
    }

    /// Each message as a line a test can compare: `ModelMessage` derives nothing.
    fn shape(context: &Context) -> Vec<String> {
        context
            .messages()
            .iter()
            .map(|message| match message {
                ModelMessage::User { text, images } => {
                    format!("user ({} images): {text}", images.len())
                }
                ModelMessage::Assistant(turn) => {
                    let calls: Vec<&str> = turn
                        .tool_calls()
                        .iter()
                        .map(|call| call.id.0.as_str())
                        .collect();
                    format!("assistant calling {calls:?}")
                }
                ModelMessage::ToolResult { id, text } => format!("result {}: {text}", id.0),
            })
            .collect()
    }

    fn result_text<'a>(context: &'a Context, id: &str) -> Option<&'a str> {
        context.messages().iter().find_map(|message| match message {
            ModelMessage::ToolResult { id: result, text } if result.0 == id => Some(text.as_str()),
            _ => None,
        })
    }

    #[test]
    fn a_transcript_the_old_server_wrote_reaches_the_model_as_text() -> TestResult {
        let long_code = format!("{}{}", "é".repeat(INPUT_CHARS), "x".repeat(50));
        let long_result = format!("{}{}", "r".repeat(1_400), "s".repeat(200));
        let history: Vec<UiMessage> = serde_json::from_value(json!([
            { "id": "u1", "role": "user", "parts": [{ "type": "text", "text": "Where is Paris?" }] },
            { "id": "a1", "role": "assistant", "parts": [
                { "type": "step-start" },
                { "type": "reasoning", "id": "thought-1", "text": "I should look up Paris.", "state": "done" },
                { "type": "text", "text": "Checking…", "state": "done" },
                { "type": "tool-lookup_city", "toolCallId": "call-1", "state": "output-available",
                  "input": { "city": "Paris" }, "output": { "city": "Paris", "country": "France" } },
                { "type": "step-start" },
                { "type": "text", "text": "Paris is in France.\nLookup complete.", "state": "done" }
            ]},
            { "id": "u2", "role": "user", "parts": [{ "type": "text", "text": "Open its page" }] },
            { "id": "a2", "role": "assistant", "parts": [
                { "type": "step-start" },
                { "type": "tool-navigate", "toolCallId": "n1", "state": "output-available",
                  "input": { "url": "https://paris.example/" }, "output": "Navigated to https://paris.example/" },
                { "type": "tool-take_snapshot", "toolCallId": "s1", "state": "output-available",
                  "input": { "page": 1 }, "output": "[e1] heading \"Paris\"" },
                { "type": "tool-click", "toolCallId": "c1", "state": "output-error",
                  "input": { "ref": "e9" }, "errorText": "Element not found" },
                { "type": "tool-run", "toolCallId": "r1", "state": "output-available",
                  "input": { "code": long_code }, "output": long_result },
                { "type": "reasoning", "text": "Done looking.", "state": "done" }
            ]},
            { "id": "u3", "role": "user", "parts": [{ "type": "text", "text": "Thanks" }] }
        ]))?;

        let (projected, current) = project(&history, &turn_context());
        assert_eq!(
            projected,
            vec![
                Projected::User("Where is Paris?".into()),
                Projected::Assistant(
                    "Checking…\n\
                     [lookup_city: {\"city\":\"Paris\"} → {\"city\":\"Paris\",\"country\":\"France\"}]\n\
                     Paris is in France.\nLookup complete."
                        .into()
                ),
                Projected::User("Open its page".into()),
                Projected::Assistant(format!(
                    "[navigate: {{\"url\":\"https://paris.example/\"}} → Navigated to https://paris.example/]\n\
                     [take_snapshot: {{\"page\":1}} → [e1] heading \"Paris\"]\n\
                     [click: {{\"ref\":\"e9\"}} → error: Element not found]\n\
                     [run: {}… → {}{}…]",
                    "é".repeat(INPUT_CHARS),
                    "r".repeat(1_400),
                    "s".repeat(100),
                )),
                Projected::User("<USER_QUERY>\nThanks\n</USER_QUERY>".into()),
            ]
        );
        assert_eq!(current, Some(4));

        let context = Context::from_history(&history, &turn_context(), 0);
        assert_eq!(context.messages().len(), 5);
        for message in context.messages() {
            match message {
                ModelMessage::ToolResult { .. } => panic!("an earlier turn replayed a tool result"),
                ModelMessage::Assistant(turn) => assert!(turn.tool_calls().is_empty()),
                ModelMessage::User { .. } => {}
            }
        }
        Ok(())
    }

    #[test]
    fn only_the_current_user_message_is_wrapped() {
        let history = vec![
            message(Role::User, "first"),
            message(Role::Assistant, "an answer"),
            message(Role::Assistant, "  "),
            message(Role::User, "second"),
        ];
        let context = TurnContext {
            active_tab: Some(TabRef {
                tab_id: 5,
                url: Some("https://docs.example/".into()),
                title: Some("Docs".into()),
            }),
            ..turn_context()
        };
        let (projected, current) = project(&history, &context);
        assert_eq!(
            projected,
            vec![
                Projected::User("first".into()),
                Projected::Assistant("an answer".into()),
                Projected::User(
                    "## Browser Context\n**Active Tab:** Tab 5 - \"Docs\" (https://docs.example/)\
                     \n\n---\n\n<USER_QUERY>\nsecond\n</USER_QUERY>"
                        .into()
                ),
            ]
        );
        assert_eq!(current, Some(2));
    }

    #[tokio::test]
    async fn the_first_stage_replaces_results_older_than_the_last_three_calls() -> TestResult {
        let mut context = Context::from_history(&[message(Role::User, "go")], &turn_context(), 0);
        let big = "p".repeat(3_000);
        for id in ["c0", "c1", "c2", "c3", "c4"] {
            push_step(&mut context, id, 0, output(&big), None).await?;
        }
        // 15,000 characters of results is 5,000 tokens; an 8,000 window leaves 4,000.
        context.fit(8_000);
        assert_eq!(result_text(&context, "c0"), Some(PLACEHOLDER));
        assert_eq!(result_text(&context, "c1"), Some(PLACEHOLDER));
        for id in ["c2", "c3", "c4"] {
            assert_eq!(result_text(&context, id), Some(big.as_str()));
        }
        let fitted = shape(&context);
        context.fit(8_000);
        assert_eq!(shape(&context), fitted, "a second fit changes nothing");
        Ok(())
    }

    #[tokio::test]
    async fn the_second_stage_replaces_every_result() -> TestResult {
        let mut context = Context::from_history(&[message(Role::User, "go")], &turn_context(), 0);
        for id in ["c0", "c1", "c2", "c3", "c4"] {
            push_step(&mut context, id, 0, output(&"p".repeat(3_000)), None).await?;
        }
        // The last three results alone are 3,000 tokens; a 4,000 window leaves 2,000.
        context.fit(4_000);
        for id in ["c0", "c1", "c2", "c3", "c4"] {
            assert_eq!(result_text(&context, id), Some(PLACEHOLDER));
        }
        assert_eq!(context.messages().len(), 11, "no message was dropped");
        Ok(())
    }

    #[tokio::test]
    async fn the_third_stage_drops_whole_groups_oldest_first() -> TestResult {
        let history = vec![
            message(Role::User, "first question"),
            // Small enough to fit, but older than a group that does not: the kept tail is
            // contiguous, so it goes.
            message(Role::Assistant, "a short answer"),
            message(Role::User, &"y".repeat(3_000)),
            message(Role::Assistant, &"z".repeat(3_000)),
            message(Role::User, "now"),
        ];
        let stored = history.clone();
        let mut context = Context::from_history(&history, &turn_context(), 0);
        // The first step streamed a long answer: too big to keep beside the pinned messages.
        push_step(&mut context, "c1", 6_000, output("one"), None).await?;
        push_step(&mut context, "c2", 0, output("two"), None).await?;
        push_step(&mut context, "c3", 0, output("three"), None).await?;

        // A 2,000 window leaves 1,000 tokens: 3,000 characters.
        context.fit(2_000);

        assert_eq!(
            shape(&context),
            vec![
                "user (0 images): first question".to_owned(),
                "user (0 images): <USER_QUERY>\nnow\n</USER_QUERY>".to_owned(),
                "assistant calling [\"c2\"]".to_owned(),
                format!("result c2: {PLACEHOLDER}"),
                "assistant calling [\"c3\"]".to_owned(),
                format!("result c3: {PLACEHOLDER}"),
            ]
        );
        let mut asked: Vec<&str> = Vec::new();
        for message in context.messages() {
            match message {
                ModelMessage::Assistant(turn) => {
                    asked.extend(turn.tool_calls().iter().map(|call| call.id.0.as_str()));
                }
                ModelMessage::ToolResult { id, .. } => assert!(
                    asked.contains(&id.0.as_str()),
                    "the result of {} kept without its call",
                    id.0
                ),
                ModelMessage::User { .. } => {}
            }
        }
        assert_eq!(history, stored, "the stored transcript is never touched");
        Ok(())
    }

    #[tokio::test]
    async fn a_reported_token_count_over_the_limit_compacts_a_small_copy() -> TestResult {
        let over = ScriptedUsage {
            input_tokens: 9_000,
            output_tokens: 1_000,
        };
        for (usage, compacted) in [(Some(over), true), (None, false)] {
            let mut context =
                Context::from_history(&[message(Role::User, "go")], &turn_context(), 0);
            for id in ["c0", "c1", "c2"] {
                push_step(&mut context, id, 0, output("small"), None).await?;
            }
            push_step(&mut context, "c3", 0, output("small"), usage).await?;
            context.fit(8_000);
            let expected = if compacted { PLACEHOLDER } else { "small" };
            assert_eq!(result_text(&context, "c0"), Some(expected));
            assert_eq!(result_text(&context, "c1"), Some("small"));
        }
        Ok(())
    }

    #[tokio::test]
    async fn pictures_follow_a_result_when_the_model_takes_images() -> TestResult {
        let picture = |byte| Picture {
            media_type: "image/png".into(),
            bytes: vec![byte; 4],
        };
        let screenshot = ToolOutput {
            text: "took a screenshot".into(),
            pictures: vec![picture(1), picture(2)],
            is_error: false,
        };
        let turn = response(&["c1"], None).await?;
        let call = turn.tool_calls().first().cloned().ok_or("no call")?;

        let mut seeing = Context::from_history(&[message(Role::User, "go")], &turn_context(), 0);
        seeing.push_assistant(turn, 0);
        seeing.push_tool_result(&call, screenshot.clone(), true);
        match seeing.messages() {
            [
                _,
                _,
                ModelMessage::ToolResult { text, .. },
                ModelMessage::User {
                    text: label,
                    images,
                },
            ] => {
                assert_eq!(text, "took a screenshot");
                assert_eq!(label, "Attached images from the tool result:");
                let expected: Vec<Vec<u8>> = vec![vec![1; 4], vec![2; 4]];
                let bytes: Vec<Vec<u8>> = images.iter().map(|image| image.bytes.clone()).collect();
                assert_eq!(bytes, expected);
            }
            _ => panic!(
                "expected a result then a picture message: {:?}",
                shape(&seeing)
            ),
        }

        let blind_turn = response(&["c1"], None).await?;
        let mut blind = Context::from_history(&[message(Role::User, "go")], &turn_context(), 0);
        blind.push_assistant(blind_turn, 0);
        blind.push_tool_result(&call, screenshot, false);
        assert_eq!(
            shape(&blind),
            vec![
                "user (0 images): <USER_QUERY>\ngo\n</USER_QUERY>".to_owned(),
                "assistant calling [\"c1\"]".to_owned(),
                "result c1: took a screenshot\n\
                 [The tool result had 2 pictures; this model cannot see images.]"
                    .to_owned(),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_script_reaches_the_model_marked_as_an_error() -> TestResult {
        let mut context = Context::from_history(&[message(Role::User, "go")], &turn_context(), 0);
        let failed = ToolOutput {
            is_error: true,
            ..output("ReferenceError: x is not defined")
        };
        push_step(&mut context, "c1", 0, failed, None).await?;
        assert_eq!(
            result_text(&context, "c1"),
            Some("Error: ReferenceError: x is not defined")
        );
        Ok(())
    }
}
