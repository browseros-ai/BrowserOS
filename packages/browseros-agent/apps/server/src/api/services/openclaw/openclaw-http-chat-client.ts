/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { createParser, type EventSourceMessage } from 'eventsource-parser'
import type { OpenClawStreamEvent } from './openclaw-types'

export interface OpenClawChatHistoryMessage {
  role: 'user' | 'assistant'
  content: string
}

export interface OpenClawChatRequest {
  agentId: string
  sessionKey: string
  message: string
  history?: OpenClawChatHistoryMessage[]
  signal?: AbortSignal
}

export class OpenClawHttpChatClient {
  constructor(
    private readonly port: number,
    private readonly getToken: () => Promise<string>,
  ) {}

  async streamChat(
    input: OpenClawChatRequest,
  ): Promise<ReadableStream<OpenClawStreamEvent>> {
    const response = await this.fetchChat(input)
    const body = response.body

    if (!body) {
      throw new Error('OpenClaw chat response had no body')
    }

    return createEventStream(body, input.signal)
  }

  private async fetchChat(input: OpenClawChatRequest): Promise<Response> {
    const token = await this.getToken()
    const response = await fetch(
      `http://127.0.0.1:${this.port}/v1/chat/completions`,
      {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${token}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          model: resolveAgentModel(input.agentId),
          stream: true,
          messages: [
            ...(input.history ?? []),
            { role: 'user', content: input.message },
          ],
          user: `browseros:${input.agentId}:${input.sessionKey}`,
        }),
        signal: input.signal,
      },
    )

    if (response.ok) {
      return response
    }

    const detail = await response.text()
    throw new Error(
      detail || `OpenClaw chat failed with status ${response.status}`,
    )
  }
}

function resolveAgentModel(agentId: string): string {
  return agentId === 'main' ? 'openclaw' : `openclaw/${agentId}`
}

function createEventStream(
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
