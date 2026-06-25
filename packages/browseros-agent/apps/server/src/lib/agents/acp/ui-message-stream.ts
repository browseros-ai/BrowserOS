/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { FinishReason, UIMessageChunk } from 'ai'

import type { AgentStreamEvent } from '../types'
import { isNudgeToolName } from './nudge'

const TEXT_PART_ID = 'acp-text'
const REASONING_PART_ID = 'acp-reasoning'
const STATUS_PART_ID = 'acp-status'
const UI_MESSAGE_STREAM_HEADERS = {
  'content-type': 'text/event-stream',
  'cache-control': 'no-cache',
  connection: 'keep-alive',
  'x-vercel-ai-ui-message-stream': 'v1',
  'x-accel-buffering': 'no',
}

interface AcpUIMessageStreamState {
  cancelled: boolean
  closed: boolean
  finished: boolean
  textOpen: boolean
  reasoningOpen: boolean
  textPartCount: number
  reasoningPartCount: number
  textPartId: string
  reasoningPartId: string
  toolCallCount: number
  toolNames: Map<string, string>
  toolInputs: Set<string>
}

export function createAcpUIMessageStream(
  events: ReadableStream<AgentStreamEvent>,
): ReadableStream<UIMessageChunk> {
  const state: AcpUIMessageStreamState = {
    cancelled: false,
    closed: false,
    finished: false,
    textOpen: false,
    reasoningOpen: false,
    textPartCount: 0,
    reasoningPartCount: 0,
    textPartId: TEXT_PART_ID,
    reasoningPartId: REASONING_PART_ID,
    toolCallCount: 0,
    toolNames: new Map(),
    toolInputs: new Set(),
  }
  let reader: ReadableStreamDefaultReader<AgentStreamEvent> | undefined

  return new ReadableStream<UIMessageChunk>({
    start(controller) {
      reader = events.getReader()
      controller.enqueue({ type: 'start' })

      void pumpAcpEvents(reader, controller, state)
    },
    async cancel(reason) {
      state.cancelled = true
      await reader?.cancel(reason)
    },
  })
}

export function createAcpUIMessageStreamResponse(
  events: ReadableStream<AgentStreamEvent>,
  init?: ResponseInit,
): Response {
  const headers = new Headers(init?.headers)
  for (const [key, value] of Object.entries(UI_MESSAGE_STREAM_HEADERS)) {
    if (!headers.has(key)) headers.set(key, value)
  }

  return new Response(createSseStream(createAcpUIMessageStream(events)), {
    ...init,
    headers,
  })
}

export function mapAcpStopReasonToFinishReason(
  stopReason: string | undefined,
): FinishReason {
  switch (stopReason) {
    case undefined:
    case 'end_turn':
    case 'stop':
    case 'stop_sequence':
      return 'stop'
    case 'max_tokens':
    case 'max_output_tokens':
      return 'length'
    case 'tool_use':
      return 'tool-calls'
    case 'content_filter':
      return 'content-filter'
    case 'error':
      return 'error'
    case 'cancelled':
      return 'other'
    default:
      return 'stop'
  }
}

async function pumpAcpEvents(
  reader: ReadableStreamDefaultReader<AgentStreamEvent>,
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
): Promise<void> {
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (state.cancelled) return

      if (done) {
        finish(controller, state, 'stop')
        return
      }

      handleAcpEvent(value, controller, state)
      if (state.finished || state.closed) return
    }
  } catch (error) {
    if (!state.cancelled) {
      finishWithError(controller, state, errorToMessage(error))
    }
  } finally {
    if (!state.cancelled) reader.releaseLock()
  }
}

function handleAcpEvent(
  event: AgentStreamEvent,
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
): void {
  switch (event.type) {
    case 'text_delta':
      enqueueTextDelta(event, controller, state)
      return
    case 'tool_call':
      enqueueToolCall(event, controller, state)
      return
    case 'app_connection_request':
      enqueueAppConnectionRequest(event, controller, state)
      return
    case 'status':
      controller.enqueue({
        type: 'data-acp-status',
        id: STATUS_PART_ID,
        data: { text: event.text },
        transient: true,
      })
      return
    case 'done':
      finish(
        controller,
        state,
        mapAcpStopReasonToFinishReason(event.stopReason),
      )
      return
    case 'error':
      finishWithError(controller, state, event.message)
      return
  }
}

function enqueueTextDelta(
  event: Extract<AgentStreamEvent, { type: 'text_delta' }>,
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
): void {
  if (event.stream === 'thought') {
    if (state.textOpen) closeTextPart(controller, state)
    if (!state.reasoningOpen) startReasoningPart(controller, state)
    controller.enqueue({
      type: 'reasoning-delta',
      id: state.reasoningPartId,
      delta: event.text,
    })
    return
  }

  if (state.reasoningOpen) closeReasoningPart(controller, state)
  if (!state.textOpen) startTextPart(controller, state)
  controller.enqueue({
    type: 'text-delta',
    id: state.textPartId,
    delta: event.text,
  })
}

function enqueueToolCall(
  event: Extract<AgentStreamEvent, { type: 'tool_call' }>,
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
): void {
  // Match against the raw title BEFORE normalization so the
  // namespace-prefixed form `nudge/suggest_app_connection` still
  // matches via the endsWith check. normalizeToolName would rewrite
  // the slash and the suffix check would silently miss.
  const rawTitle = event.title || event.rawType || ''

  // Drop the upstream tool_call rendering for nudge tools. The real
  // connect card arrives via app_connection_request; without this
  // suppression the renderer would briefly show a generic tool block
  // next to the card. Mirrors agent-company's StreamTranslator drop.
  // Checked BEFORE nextToolCallId so suppressed events don't bump
  // state.toolCallCount and skew ids for later non-nudge tools.
  if (isNudgeToolName(rawTitle)) return

  const toolCallId = event.id ?? nextToolCallId(state)
  const toolName =
    state.toolNames.get(toolCallId) ?? normalizeToolName(rawTitle || 'acp_tool')

  state.toolNames.set(toolCallId, toolName)

  if (!state.toolInputs.has(toolCallId)) {
    state.toolInputs.add(toolCallId)
    controller.enqueue({
      type: 'tool-input-available',
      toolCallId,
      toolName,
      title: event.title,
      input: { description: event.text },
      dynamic: true,
    })
  }

  if (isCompletedToolStatus(event.status)) {
    controller.enqueue({
      type: 'tool-output-available',
      toolCallId,
      output: { content: event.text },
      dynamic: true,
    })
  } else if (isFailedToolStatus(event.status)) {
    controller.enqueue({
      type: 'tool-output-error',
      toolCallId,
      errorText: event.text || event.title || 'Tool failed',
      dynamic: true,
    })
  }
}

/**
 * Emit the byok-shaped tool-input + tool-output parts the sidepanel's
 * existing nudge parser already understands. The wire shape matches
 * the in-process byok tool sentinel verbatim so getMessageSegments
 * and ConnectAppCard work unchanged across both runtimes.
 */
function enqueueAppConnectionRequest(
  event: Extract<AgentStreamEvent, { type: 'app_connection_request' }>,
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  _state: AcpUIMessageStreamState,
): void {
  const toolName = 'suggest_app_connection'
  controller.enqueue({
    type: 'tool-input-available',
    toolCallId: event.toolCallId,
    toolName,
    input: { appName: event.appName, reason: event.reason },
    dynamic: true,
  })
  controller.enqueue({
    type: 'tool-output-available',
    toolCallId: event.toolCallId,
    output: {
      content: [
        {
          type: 'text',
          text: JSON.stringify({
            type: 'app_connection',
            appName: event.appName,
            reason: event.reason,
          }),
        },
      ],
      isError: false,
    },
    dynamic: true,
  })
}

function finish(
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
  finishReason: FinishReason,
): void {
  if (state.cancelled || state.finished || state.closed) return

  closeOpenParts(controller, state)
  controller.enqueue({ type: 'finish', finishReason })
  state.finished = true
  closeController(controller, state)
}

function finishWithError(
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
  errorText: string,
): void {
  if (state.cancelled || state.finished || state.closed) return

  closeOpenParts(controller, state)
  controller.enqueue({ type: 'error', errorText })
  controller.enqueue({ type: 'finish', finishReason: 'error' })
  state.finished = true
  closeController(controller, state)
}

function closeOpenParts(
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
): void {
  if (state.reasoningOpen) closeReasoningPart(controller, state)
  if (state.textOpen) closeTextPart(controller, state)
}

function startReasoningPart(
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
): void {
  state.reasoningPartCount += 1
  state.reasoningPartId =
    state.reasoningPartCount === 1
      ? REASONING_PART_ID
      : `${REASONING_PART_ID}-${state.reasoningPartCount}`
  state.reasoningOpen = true
  controller.enqueue({ type: 'reasoning-start', id: state.reasoningPartId })
}

function closeReasoningPart(
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
): void {
  controller.enqueue({ type: 'reasoning-end', id: state.reasoningPartId })
  state.reasoningOpen = false
}

function startTextPart(
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
): void {
  state.textPartCount += 1
  state.textPartId =
    state.textPartCount === 1
      ? TEXT_PART_ID
      : `${TEXT_PART_ID}-${state.textPartCount}`
  state.textOpen = true
  controller.enqueue({ type: 'text-start', id: state.textPartId })
}

function closeTextPart(
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
): void {
  controller.enqueue({ type: 'text-end', id: state.textPartId })
  state.textOpen = false
}

function closeController(
  controller: ReadableStreamDefaultController<UIMessageChunk>,
  state: AcpUIMessageStreamState,
): void {
  state.closed = true
  controller.close()
}

function nextToolCallId(state: AcpUIMessageStreamState): string {
  state.toolCallCount += 1
  return `acp-tool-${state.toolCallCount}`
}

function normalizeToolName(value: string): string {
  const normalized = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')

  return normalized || 'acp_tool'
}

function isCompletedToolStatus(status: string | undefined): boolean {
  return status === 'completed' || status === 'done' || status === 'success'
}

function isFailedToolStatus(status: string | undefined): boolean {
  return status === 'failed' || status === 'error' || status === 'failure'
}

function createSseStream(
  stream: ReadableStream<UIMessageChunk>,
): ReadableStream<Uint8Array> {
  return stream
    .pipeThrough(
      new TransformStream<UIMessageChunk, string>({
        transform(chunk, controller) {
          controller.enqueue(`data: ${JSON.stringify(chunk)}\n\n`)
        },
        flush(controller) {
          controller.enqueue('data: [DONE]\n\n')
        },
      }),
    )
    .pipeThrough(new TextEncoderStream())
}

function errorToMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
