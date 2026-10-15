//! Context-window policy shared by every built-in model step: the exact TS budget, UTF-16
//! estimator and pruning order. The loop replaces its working transcript with the result,
//! making compaction sticky for this turn while durable UI history stays untouched.
use crate::messages::{
    AssistantPart, ModelMessage, ToolMessagePart, UserPart, estimate_output, file_placeholder,
    js_stringify, strip_output,
};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;
const RESERVE: u64 = 20_000;
const OVERHEAD: u64 = 12_000;
const KEEP_RECENT: usize = 6;

/// The system prompt and tool schemas are outside the transcript; overhead accounts for
/// them, and reserve leaves response room without consuming a small model's whole window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub window: u64,
    pub reserve: u64,
    pub overhead: u64,
    pub threshold: u64,
}
impl Budget {
    pub fn for_context_window(context_window: Option<f64>) -> Self {
        let window = context_window
            .filter(|window| window.is_finite() && *window >= 1.0)
            .map_or(DEFAULT_CONTEXT_WINDOW, |window| window.floor() as u64);
        let reserve = RESERVE.min(window / 2);
        let overhead = OVERHEAD.min((window as f64 * 0.4).floor() as u64);
        Self {
            window,
            reserve,
            overhead,
            threshold: window - reserve,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StepUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionStage {
    Untouched,
    PrunedOld,
    PrunedAll,
    DroppedOldest { still_over_budget: bool },
}

/// Provider usage already includes invisible overhead; only messages after the newest
/// assistant message are additional. A missing/zero report falls back to the estimator.
fn current_tokens(steps: &[StepUsage], messages: &[ModelMessage], overhead: u64) -> u64 {
    let Some(step) = steps
        .last()
        .filter(|step| step.input_tokens.is_some_and(|tokens| tokens > 0))
    else {
        return estimate_tokens(messages).saturating_add(overhead);
    };
    let base = step
        .input_tokens
        .unwrap_or_default()
        .saturating_add(step.output_tokens.unwrap_or_default());
    let Some(last_assistant) = messages
        .iter()
        .rposition(|message| matches!(message, ModelMessage::Assistant { .. }))
    else {
        return base;
    };
    messages[last_assistant + 1..]
        .iter()
        .filter(|message| !matches!(message, ModelMessage::System { .. }))
        .fold(base, |total, message| {
            total.saturating_add(estimate_tokens(std::slice::from_ref(message)))
        })
}

/// Drop media and reasoning, prune old tool exchanges, then all exchanges, then whole
/// messages. A single oversized message remains intact and is surfaced in the stage.
pub fn prepare_step(
    messages: Vec<ModelMessage>,
    steps: &[StepUsage],
    budget: &Budget,
) -> (Vec<ModelMessage>, CompactionStage) {
    if current_tokens(steps, &messages, budget.overhead) <= budget.threshold {
        return (messages, CompactionStage::Untouched);
    }
    let fits = |messages: &[ModelMessage]| {
        estimate_tokens(messages).saturating_add(budget.overhead) <= budget.threshold
    };
    let compacted = prune(strip_binary(messages), Some(KEEP_RECENT));
    if fits(&compacted) {
        return (compacted, CompactionStage::PrunedOld);
    }
    let cleared = prune(compacted, None);
    if fits(&cleared) {
        return (cleared, CompactionStage::PrunedAll);
    }
    let floor = drop_oldest(cleared, budget);
    let stage = CompactionStage::DroppedOldest {
        still_over_budget: !fits(&floor),
    };
    (floor, stage)
}
fn utf16(text: &str) -> u64 {
    text.encode_utf16().count() as u64
}
/// Images and files cost a flat 1,000 tokens regardless of encoded bytes. Reasoning-file
/// and custom parts deliberately cost zero, matching the SDK-era TS estimator's quirks.
pub fn estimate_tokens(messages: &[ModelMessage]) -> u64 {
    let (mut chars, mut images): (u64, u64) = (0, 0);
    for message in messages {
        match message {
            ModelMessage::System { content, .. } => chars = chars.saturating_add(utf16(content)),
            ModelMessage::User { content } => {
                for part in content {
                    match part {
                        UserPart::Text { text, .. } => chars = chars.saturating_add(utf16(text)),
                        UserPart::Image { .. } | UserPart::File { .. } => images += 1,
                    }
                }
            }
            ModelMessage::Assistant { content } => {
                for part in content {
                    let (c, i) = match part {
                        AssistantPart::Text { text, .. }
                        | AssistantPart::Reasoning { text, .. } => (utf16(text), 0),
                        AssistantPart::ToolCall { input, .. } => (
                            input
                                .as_ref()
                                .map_or(0, |value| utf16(&js_stringify(value))),
                            0,
                        ),
                        AssistantPart::ToolResult(result) => estimate_output(&result.output),
                        AssistantPart::ToolApprovalRequest {
                            approval_id,
                            tool_call_id,
                            ..
                        } => (utf16(approval_id) + utf16(tool_call_id), 0),
                        AssistantPart::File { .. } => (0, 1),
                        AssistantPart::ReasoningFile { .. } | AssistantPart::Custom { .. } => {
                            (0, 0)
                        }
                    };
                    chars = chars.saturating_add(c);
                    images = images.saturating_add(i);
                }
            }
            ModelMessage::Tool { content } => {
                for part in content {
                    let (c, i) = match part {
                        ToolMessagePart::ToolResult(result) => estimate_output(&result.output),
                        ToolMessagePart::ToolApprovalResponse {
                            approval_id,
                            reason,
                            ..
                        } => (utf16(approval_id) + reason.as_deref().map_or(0, utf16), 0),
                    };
                    chars = chars.saturating_add(c);
                    images = images.saturating_add(i);
                }
            }
        }
    }
    chars
        .div_ceil(3)
        .saturating_add(images.saturating_mul(1000))
}
fn strip_binary(mut messages: Vec<ModelMessage>) -> Vec<ModelMessage> {
    for message in &mut messages {
        match message {
            ModelMessage::User { content } => {
                for part in content {
                    let placeholder = match part {
                        UserPart::Image { .. } => Some("[Image]".into()),
                        UserPart::File {
                            media_type,
                            filename,
                            ..
                        } => Some(file_placeholder(media_type, filename.as_deref())),
                        _ => None,
                    };
                    if let Some(text) = placeholder {
                        *part = UserPart::Text {
                            text,
                            provider_options: None,
                        };
                    }
                }
            }
            ModelMessage::Assistant { content } => {
                for part in content {
                    match part {
                        AssistantPart::File {
                            media_type,
                            filename,
                            ..
                        } => {
                            *part = AssistantPart::Text {
                                text: file_placeholder(media_type, filename.as_deref()),
                                provider_options: None,
                            }
                        }
                        AssistantPart::ToolResult(result) => strip_output(&mut result.output),
                        _ => {}
                    }
                }
            }
            ModelMessage::Tool { content } => {
                for part in content {
                    if let ToolMessagePart::ToolResult(result) = part {
                        strip_output(&mut result.output);
                    }
                }
            }
            _ => {}
        }
    }
    messages
}

/// IDs, not message positions, protect pairs straddling the retained window. A kept
/// approval response also protects its originating call via the global request map.
fn prune(mut messages: Vec<ModelMessage>, keep_recent: Option<usize>) -> Vec<ModelMessage> {
    for message in &mut messages {
        if let ModelMessage::Assistant { content } = message {
            content.retain(|part| {
                !matches!(
                    part,
                    AssistantPart::Reasoning { .. } | AssistantPart::ReasoningFile { .. }
                )
            });
        }
    }
    let cutoff = keep_recent.map_or(messages.len(), |count| messages.len().saturating_sub(count));
    let (mut kept_calls, mut kept_approvals) = (BTreeSet::new(), BTreeSet::new());
    let mut approvals = BTreeMap::new();
    for (index, message) in messages.iter().enumerate() {
        match message {
            ModelMessage::Assistant { content } => {
                for part in content {
                    match part {
                        AssistantPart::ToolApprovalRequest {
                            approval_id,
                            tool_call_id,
                            ..
                        } => {
                            approvals.insert(approval_id.clone(), tool_call_id.clone());
                            if index >= cutoff {
                                kept_approvals.insert(approval_id.clone());
                            }
                        }
                        AssistantPart::ToolCall { tool_call_id, .. } if index >= cutoff => {
                            kept_calls.insert(tool_call_id.clone());
                        }
                        AssistantPart::ToolResult(result) if index >= cutoff => {
                            kept_calls.insert(result.tool_call_id.clone());
                        }
                        _ => {}
                    }
                }
            }
            ModelMessage::Tool { content } if index >= cutoff => {
                for part in content {
                    match part {
                        ToolMessagePart::ToolResult(result) => {
                            kept_calls.insert(result.tool_call_id.clone());
                        }
                        ToolMessagePart::ToolApprovalResponse { approval_id, .. } => {
                            kept_approvals.insert(approval_id.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for approval in &kept_approvals {
        if let Some(call) = approvals.get(approval) {
            kept_calls.insert(call.clone());
        }
    }
    for message in &mut messages[..cutoff] {
        match message {
            ModelMessage::Assistant { content } => content.retain(|part| match part {
                AssistantPart::ToolCall { tool_call_id, .. } => kept_calls.contains(tool_call_id),
                AssistantPart::ToolResult(result) => kept_calls.contains(&result.tool_call_id),
                AssistantPart::ToolApprovalRequest { approval_id, .. } => {
                    kept_approvals.contains(approval_id)
                }
                _ => true,
            }),
            ModelMessage::Tool { content } => content.retain(|part| match part {
                ToolMessagePart::ToolResult(result) => kept_calls.contains(&result.tool_call_id),
                ToolMessagePart::ToolApprovalResponse { approval_id, .. } => {
                    kept_approvals.contains(approval_id)
                }
            }),
            _ => {}
        }
    }
    messages.retain(|message| match message {
        ModelMessage::System { content, .. } => !content.is_empty(),
        ModelMessage::User { content } => !content.is_empty(),
        ModelMessage::Assistant { content } => !content.is_empty(),
        ModelMessage::Tool { content } => !content.is_empty(),
    });
    messages
}
fn drop_oldest(mut messages: Vec<ModelMessage>, budget: &Budget) -> Vec<ModelMessage> {
    if messages.len() <= 2 {
        return messages;
    }
    let costs = messages
        .iter()
        .map(|message| estimate_tokens(std::slice::from_ref(message)))
        .collect::<Vec<_>>();
    // Signed arithmetic matters when the first message alone exceeds the budget: even a
    // zero-cost suffix then fails to fit, and TS keeps exactly the first and last messages.
    let mut remaining =
        i128::from(budget.threshold) - i128::from(budget.overhead) - i128::from(costs[0]);
    let mut start = messages.len();
    for index in (1..messages.len()).rev() {
        if i128::from(costs[index]) > remaining {
            break;
        }
        remaining -= i128::from(costs[index]);
        start = index;
    }
    if start == messages.len() {
        start -= 1;
    }
    messages.drain(1..start);
    messages
}
