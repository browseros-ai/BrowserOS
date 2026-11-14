/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { isToolUIPart, type UIMessage } from 'ai'

/**
 * Checks whether a UIMessage has meaningful content that can be sent
 * to the AI provider without causing validation errors.
 *
 * Two layers of validation can reject messages:
 *
 * 1. **AI SDK** (`validate-ui-messages.ts`):
 *    - `parts` array must be `.nonempty()` — rejects `parts: []`
 *
 * 2. **Provider API** (e.g. Gemini `generateContent`, Anthropic, OpenAI):
 *    - Assistant messages with only empty-string text are rejected
 *      as semantically empty, even though the SDK schema allows it
 *
 * This function guards against both layers so callers can filter
 * messages before passing them to `createAgentUIStream`.
 */
export function hasMessageContent(message: UIMessage): boolean {
  if (message.parts.length === 0) return false

  // A message that contains any non-text part (tool invocation, reasoning,
  // file, step-start, etc.) is always considered valid — those part types
  // carry meaning regardless of text content.
  const hasNonTextPart = message.parts.some((p) => p.type !== 'text')
  if (hasNonTextPart) return true

  // All parts are text — at least one must have non-whitespace content.
  return message.parts.some(
    (p) => p.type === 'text' && p.text.trim().length > 0,
  )
}

/**
 * Filters a UIMessage array, removing messages that would fail
 * SDK validation or provider-level content checks.
 */
export function filterValidMessages(messages: UIMessage[]): UIMessage[] {
  return messages.filter(hasMessageContent)
}

/**
 * Remove `reasoning` parts from a message list before it is sent to the model.
 *
 * Reasoning is ephemeral. It stays in the durable and client-displayed history,
 * but must not be replayed to the provider: convertToModelMessages can turn a
 * multi-step assistant message into a reasoning-only assistant message (no text,
 * no tool call), which serializes to `content: null` with no `tool_calls`. Strict
 * OpenAI-compatible providers reject that with "The content field is a required
 * field", so a conversation cannot continue past the first turn once a reasoning
 * model has answered. Stripping reasoning from the request copy keeps replay
 * provider-agnostic; the model still generates fresh reasoning for the new turn.
 */
export function stripReasoningParts(messages: UIMessage[]): UIMessage[] {
  return messages
    .map((message) => {
      const parts = message.parts.filter((part) => part.type !== 'reasoning')
      return parts.length === message.parts.length
        ? message
        : { ...message, parts }
    })
    .filter(hasMessageContent)
}

/**
 * Close out tool calls that can never produce a result.
 *
 * A turn can end while a tool call is still waiting on execution: the provider
 * stream errors after emitting the complete call, or the abort signal fires
 * while the call is pending. The part then sits in a non-terminal state and,
 * replayed on the next turn, converts to an assistant tool-call with no
 * matching tool result — the AI SDK rejects that prompt with
 * MissingToolResultsError, so every later message in the conversation fails
 * before it reaches the model. Marking the part `output-error` keeps the tool
 * card visible and pairs the call with a result the wire format accepts. A
 * denied approval already produces an `execution-denied` result, so it is left
 * alone.
 */
export function closeDanglingToolCalls(messages: UIMessage[]): UIMessage[] {
  return messages.map((message) => {
    let touched = false
    const parts = message.parts.map((part) => {
      if (!isToolUIPart(part)) return part
      if (
        part.state !== 'input-streaming' &&
        part.state !== 'input-available' &&
        part.state !== 'approval-requested' &&
        !(
          part.state === 'approval-responded' &&
          part.approval?.approved !== false
        )
      ) {
        return part
      }
      touched = true
      return {
        ...part,
        state: 'output-error' as const,
        input: part.input,
        errorText: 'Tool call interrupted before it could complete.',
        approval:
          part.state === 'approval-responded' &&
          part.approval?.approved === true
            ? { ...part.approval, approved: true as const }
            : undefined,
      }
    })
    return touched ? { ...message, parts } : message
  })
}

/**
 * Remove tool parts that reference tools not present in the given toolset.
 *
 * When a session is rebuilt with a different set of tools (e.g., workspace
 * removed mid-conversation or MCP server disconnected), the carried-over
 * message history may contain tool parts for tools that no longer exist.
 * The AI SDK validates messages against the current toolset and rejects
 * parts with no matching schema.
 *
 * Tool parts use the type format `tool-${toolName}` (static tools) or
 * `dynamic-tool` (dynamic tools). This function filters out static tool
 * parts whose tool name is not in the provided set.
 */
export function sanitizeMessagesForToolset(
  messages: UIMessage[],
  toolNames: Set<string>,
): UIMessage[] {
  return messages
    .map((msg) => {
      const filteredParts = msg.parts.filter((part) => {
        // Static tool parts have type `tool-${toolName}`
        if (typeof part.type === 'string' && part.type.startsWith('tool-')) {
          const toolName = part.type.slice(5)
          if (!toolNames.has(toolName)) return false
        }
        return true
      })

      if (filteredParts.length === msg.parts.length) return msg
      return { ...msg, parts: filteredParts }
    })
    .filter(hasMessageContent)
}
