/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { createParser, type EventSourceMessage } from 'eventsource-parser'
import { OpenClawSessionNotFoundError } from './errors'
import type { OpenClawStreamEvent } from './openclaw-types'

export interface OpenClawChatHistoryMessage {
  role: 'user' | 'assistant'
  content: string
}

/**
 * OpenAI-compatible content parts for multimodal user messages. OpenClaw's
 * gateway accepts the standard `content: [{ type: 'text', ... }, { type:
 * 'image_url', image_url: { url } }]` shape on /v1/chat/completions and
 * routes it to whichever upstream provider the agent's model points at.
 */
export type OpenClawChatContentPart =
  | { type: 'text'; text: string }
  | {
      type: 'image_url'
      image_url: { url: string; detail?: 'auto' | 'low' | 'high' }
    }

export interface OpenClawChatRequest {
  agentId: string
  sessionKey: string
  message: string
  // When present, sent as the user message's `content` array verbatim. The
  // legacy string `message` is folded into a leading text part if no text
  // part is present in `messageParts`.
  messageParts?: OpenClawChatContentPart[]
  history?: OpenClawChatHistoryMessage[]
  signal?: AbortSignal
}

export interface OpenClawSessionHistoryMessage {
  role: 'user' | 'assistant' | 'system' | 'tool'
  content: string
  messageId?: string
  messageSeq?: number
  timestamp?: number
}

export interface OpenClawSessionHistory {
  sessionKey: string
  messages: OpenClawSessionHistoryMessage[]
  cursor?: string | null
  hasMore?: boolean
  truncated?: boolean
}

export interface OpenClawSessionHistoryInput {
  limit?: number
  cursor?: string
  signal?: AbortSignal
}

export type OpenClawSessionHistoryEvent =
  | { type: 'history'; data: OpenClawSessionHistory }
  | {
      type: 'message'
      data: {
        sessionKey: string
        message: OpenClawSessionHistoryMessage
        messageId?: string
        messageSeq: number
      }
    }
  | { type: 'error'; data: { message: string } }

export class OpenClawHttpClient {
  constructor(
    private readonly hostPort: number,
    private readonly getToken: () => Promise<string>,
  ) {}

  async getSessionHistory(
    sessionKey: string,
    input: OpenClawSessionHistoryInput = {},
  ): Promise<OpenClawSessionHistory> {
    const response = await this.fetchSessionHistory(sessionKey, input, {})
    return (await response.json()) as OpenClawSessionHistory
  }

  async streamSessionHistory(
    sessionKey: string,
    input: OpenClawSessionHistoryInput = {},
  ): Promise<ReadableStream<OpenClawSessionHistoryEvent>> {
    const response = await this.fetchSessionHistory(sessionKey, input, {
      Accept: 'text/event-stream',
    })
    const body = response.body
    if (!body) {
      throw new Error('OpenClaw session history stream had no body')
    }
    return createHistoryEventStream(body, input.signal)
  }

  async isAuthenticated(): Promise<boolean> {
    try {
      const token = await this.getToken()
      const response = await fetch(
        `http://127.0.0.1:${this.hostPort}/v1/models`,
        {
          method: 'GET',
          headers: {
            Authorization: `Bearer ${token}`,
          },
        },
      )
      return response.ok
    } catch {
      return false
    }
  }

  private async fetchSessionHistory(
    sessionKey: string,
    input: OpenClawSessionHistoryInput,
    extraHeaders: Record<string, string>,
  ): Promise<Response> {
    const token = await this.getToken()
    const response = await fetch(
      `http://127.0.0.1:${this.hostPort}${buildHistoryPath(sessionKey, input)}`,
      {
        method: 'GET',
        headers: {
          Authorization: `Bearer ${token}`,
          ...extraHeaders,
        },
        signal: input.signal,
      },
    )

    if (response.status === 404) {
      throw new OpenClawSessionNotFoundError(sessionKey)
    }
    if (!response.ok) {
      const detail = await response.text()
      throw new Error(
        detail ||
          `OpenClaw session history failed with status ${response.status}`,
      )
    }
    return response
  }
}

function buildHistoryPath(
  sessionKey: string,
  input: OpenClawSessionHistoryInput,
): string {
  const qs = new URLSearchParams()
  if (input.limit !== undefined) qs.set('limit', String(input.limit))
  if (input.cursor !== undefined) qs.set('cursor', input.cursor)
  const suffix = qs.toString()
  return `/sessions/${encodeURIComponent(sessionKey)}/history${
    suffix ? `?${suffix}` : ''
  }`
}

function _resolveAgentModel(agentId: string): string {
  return agentId === 'main' ? 'openclaw' : `openclaw/${agentId}`
}

/**
 * Build the OpenAI-compatible `content` payload for the trailing user
 * message. When the caller supplies multimodal parts via `messageParts`,
 * use them as-is, ensuring at least one text part is present (we fold the
 * legacy `message` string in as a leading text part if not). Otherwise,
 * fall back to a plain string `content` so simple text-only sends keep
 * the same wire shape we've always sent.
 */
function _buildUserContent(
  input: OpenClawChatRequest,
): string | OpenClawChatContentPart[] {
  if (!input.messageParts || input.messageParts.length === 0) {
    return input.message
  }

  const hasText = input.messageParts.some((p) => p.type === 'text')
  if (hasText) return input.messageParts

  const trimmed = input.message.trim()
  if (!trimmed) return input.messageParts

  return [{ type: 'text', text: input.message }, ...input.messageParts]
}

function _createEventStream(
  body: ReadableStream<Uint8Array>,
  signal?: AbortSignal,
): ReadableStream<OpenClawStreamEvent> {
  return new ReadableStream<OpenClawStreamEvent>({
    start(controller) {
      void pumpChatEvents(body, controller, signal)
    },
  })
}

async function pumpChatEvents(
  body: ReadableStream<Uint8Array>,
  controller: ReadableStreamDefaultController<OpenClawStreamEvent>,
  signal?: AbortSignal,
): Promise<void> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let text = ''
  let done = false
  const parser = createParser({
    onEvent(message) {
      if (done) return
      const nextText = updateAccumulatedText(message, text)
      done = handleMessage(message, controller, nextText, done)
      if (!done) {
        text = nextText
      }
    },
  })

  try {
    while (true) {
      if (signal?.aborted) {
        await reader.cancel()
        done = true
        controller.close()
        return
      }

      const { done: streamDone, value } = await reader.read()
      if (streamDone) break
      parser.feed(decoder.decode(value, { stream: true }))
    }
  } catch (error) {
    if (!done) {
      controller.enqueue({
        type: 'error',
        data: {
          message: error instanceof Error ? error.message : String(error),
        },
      })
      done = true
      controller.close()
    }
  } finally {
    if (!done) {
      controller.close()
    }
    reader.releaseLock()
  }
}

function handleMessage(
  message: EventSourceMessage,
  controller: ReadableStreamDefaultController<OpenClawStreamEvent>,
  text: string,
  done: boolean,
): boolean {
  if (message.data === '[DONE]') {
    return finishStream(controller, text, done)
  }

  const chunk = parseChunk(message.data)
  if (!chunk) {
    controller.enqueue({
      type: 'error',
      data: { message: 'Failed to parse OpenClaw chat stream chunk' },
    })
    controller.close()
    return true
  }

  for (const event of mapChunkToEvents(chunk)) {
    controller.enqueue(event)
  }

  return hasFinishReason(chunk) ? finishStream(controller, text, done) : false
}

function updateAccumulatedText(
  message: EventSourceMessage,
  text: string,
): string {
  const chunk = parseChunk(message.data)
  if (!chunk) return text

  let next = text
  for (const choice of readChoices(chunk)) {
    const delta = readDeltaText(choice)
    if (delta) {
      next += delta
    }
  }
  return next
}

function finishStream(
  controller: ReadableStreamDefaultController<OpenClawStreamEvent>,
  text: string,
  done: boolean,
): boolean {
  if (!done) {
    if (!text.trim()) {
      controller.enqueue({
        type: 'error',
        data: {
          message: "Agent couldn't generate a response. Please try again.",
        },
      })
      controller.close()
      return true
    }
    controller.enqueue({
      type: 'done',
      data: { text },
    })
    controller.close()
  }

  return true
}

function mapChunkToEvents(
  chunk: Record<string, unknown>,
): OpenClawStreamEvent[] {
  const events: OpenClawStreamEvent[] = []

  for (const choice of readChoices(chunk)) {
    const delta = readDeltaText(choice)
    if (delta) {
      events.push({
        type: 'text-delta',
        data: { text: delta },
      })
    }
  }

  return events
}

function hasFinishReason(chunk: Record<string, unknown>): boolean {
  return readChoices(chunk).some((choice) => !!readFinishReason(choice))
}

function readChoices(
  chunk: Record<string, unknown>,
): Array<Record<string, unknown>> {
  const choices = chunk.choices
  return Array.isArray(choices)
    ? choices.filter(
        (choice): choice is Record<string, unknown> =>
          !!choice && typeof choice === 'object',
      )
    : []
}

function readDeltaText(choice: Record<string, unknown>): string {
  const delta = choice.delta
  if (!delta || typeof delta !== 'object') return ''

  const content = (delta as Record<string, unknown>).content
  return typeof content === 'string' ? content : ''
}

function readFinishReason(choice: Record<string, unknown>): string | null {
  const reason = choice.finish_reason
  return typeof reason === 'string' && reason ? reason : null
}

function parseChunk(data: string): Record<string, unknown> | null {
  try {
    return JSON.parse(data) as Record<string, unknown>
  } catch {
    return null
  }
}

function createHistoryEventStream(
  body: ReadableStream<Uint8Array>,
  signal?: AbortSignal,
): ReadableStream<OpenClawSessionHistoryEvent> {
  return new ReadableStream<OpenClawSessionHistoryEvent>({
    start(controller) {
      void pumpHistoryEvents(body, controller, signal)
    },
  })
}

async function pumpHistoryEvents(
  body: ReadableStream<Uint8Array>,
  controller: ReadableStreamDefaultController<OpenClawSessionHistoryEvent>,
  signal?: AbortSignal,
): Promise<void> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let closed = false
  const close = () => {
    if (closed) return
    closed = true
    controller.close()
  }
  const parser = createParser({
    onEvent(message) {
      if (closed) return
      const event = toHistoryEvent(message)
      if (!event) return
      controller.enqueue(event)
      if (event.type === 'error') close()
    },
  })

  const onAbort = () => {
    void reader.cancel().catch(() => {})
    close()
  }
  signal?.addEventListener('abort', onAbort, { once: true })

  try {
    while (true) {
      if (signal?.aborted) {
        await reader.cancel()
        close()
        return
      }
      const { done, value } = await reader.read()
      if (done) break
      parser.feed(decoder.decode(value, { stream: true }))
    }
  } catch (error) {
    if (!closed) {
      controller.enqueue({
        type: 'error',
        data: {
          message: error instanceof Error ? error.message : String(error),
        },
      })
      close()
    }
  } finally {
    signal?.removeEventListener('abort', onAbort)
    close()
    reader.releaseLock()
  }
}

function toHistoryEvent(
  message: EventSourceMessage,
): OpenClawSessionHistoryEvent | null {
  if (!message.event) return null
  const payload = parseChunk(message.data)
  if (!payload) return null
  if (message.event === 'history') {
    return {
      type: 'history',
      data: payload as unknown as OpenClawSessionHistory,
    }
  }
  if (message.event === 'message') {
    return {
      type: 'message',
      data: payload as unknown as {
        sessionKey: string
        message: OpenClawSessionHistoryMessage
        messageId?: string
        messageSeq: number
      },
    }
  }
  if (message.event === 'error') {
    const errMessage =
      typeof payload.message === 'string'
        ? payload.message
        : 'OpenClaw session history stream error'
    return { type: 'error', data: { message: errMessage } }
  }
  return null
}
