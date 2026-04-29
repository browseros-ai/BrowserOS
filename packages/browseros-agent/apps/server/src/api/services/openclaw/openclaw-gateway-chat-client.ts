/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Minimal OpenAI-compatible chat client against the OpenClaw gateway.
 * Used exclusively by the harness's image carve-out: when the user
 * attaches images to an OpenClaw agent, the harness diverts the turn
 * here instead of through the ACP bridge (which silently drops image
 * content blocks). The gateway's `/v1/chat/completions` endpoint
 * accepts OpenAI-style multimodal `image_url` parts.
 *
 * Output is normalized to `AgentStreamEvent` so the rest of the harness
 * pipeline (UI streaming, history persistence) doesn't care that the
 * transport is HTTP rather than ACP for this turn.
 */

import type { AgentStreamEvent } from '../../../lib/agents/types'
import { logger } from '../../../lib/logger'

export type OpenAIContentPart =
  | { type: 'text'; text: string }
  | { type: 'image_url'; image_url: { url: string } }

export interface OpenAIChatMessage {
  role: 'system' | 'user' | 'assistant'
  content: string | OpenAIContentPart[]
}

export interface GatewayChatTurnInput {
  /** Gateway-side agent name. Equal to the harness id post Step 9 backfill. */
  agentId: string
  sessionKey: string
  messages: OpenAIChatMessage[]
  signal?: AbortSignal
}

export class OpenClawGatewayChatClient {
  constructor(
    private readonly hostPort: number,
    private readonly getToken: () => Promise<string>,
  ) {}

  async streamTurn(
    input: GatewayChatTurnInput,
  ): Promise<ReadableStream<AgentStreamEvent>> {
    const token = await this.getToken()
    const response = await fetch(
      `http://127.0.0.1:${this.hostPort}/v1/chat/completions`,
      {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${token}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          model: resolveAgentModel(input.agentId),
          stream: true,
          messages: input.messages,
          user: `browseros:${input.agentId}:${input.sessionKey}`,
        }),
        signal: input.signal,
      },
    )

    if (!response.ok) {
      const detail = await response.text().catch(() => '')
      throw new Error(
        detail || `OpenClaw gateway chat failed with status ${response.status}`,
      )
    }
    const body = response.body
    if (!body) {
      throw new Error('OpenClaw gateway chat response had no body')
    }

    return new ReadableStream<AgentStreamEvent>({
      start(controller) {
        void pumpOpenAIChunks(body, controller, input.signal)
      },
    })
  }
}

function resolveAgentModel(agentId: string): string {
  // The gateway routes `openclaw` → its default `main` provider config,
  // and `openclaw/<agentId>` → the per-agent provider config. Backfilled
  // legacy agents (`main`, orphans) can use the unprefixed form.
  return agentId === 'main' ? 'openclaw' : `openclaw/${agentId}`
}

async function pumpOpenAIChunks(
  body: ReadableStream<Uint8Array>,
  controller: ReadableStreamDefaultController<AgentStreamEvent>,
  signal?: AbortSignal,
): Promise<void> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  let closed = false
  let aborted = false
  let stopReason: string | undefined
  // Re-emit explicit signal aborts as a clean cancel rather than letting
  // the underlying `reader.read()` reject — keeps the controller in a
  // sensible state if the caller bails (e.g. tab close).
  const onAbort = () => {
    aborted = true
    void reader.cancel().catch(() => {})
  }
  signal?.addEventListener('abort', onAbort, { once: true })

  const flushLine = (line: string) => {
    if (closed || !line.startsWith('data:')) return
    const payload = line.slice(5).trim()
    if (!payload || payload === '[DONE]') {
      finish()
      return
    }
    let parsed: unknown
    try {
      parsed = JSON.parse(payload)
    } catch {
      controller.enqueue({
        type: 'error',
        message: 'Failed to parse OpenClaw gateway chunk',
      })
      finish()
      return
    }
    const text = extractDeltaText(parsed)
    if (text) {
      controller.enqueue({
        type: 'text_delta',
        text,
        stream: 'output',
        rawType: 'agent_message_chunk',
      })
    }
    const finishReason = extractFinishReason(parsed)
    if (finishReason) {
      stopReason = finishReason === 'stop' ? 'end_turn' : finishReason
      finish()
    }
  }

  const finish = () => {
    if (closed) return
    closed = true
    controller.enqueue({ type: 'done', stopReason: stopReason ?? 'end_turn' })
    controller.close()
  }

  try {
    while (true) {
      if (aborted) {
        if (!closed) {
          closed = true
          controller.close()
        }
        return
      }
      const { done, value } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      let idx = buffer.indexOf('\n\n')
      while (idx >= 0) {
        const event = buffer.slice(0, idx)
        buffer = buffer.slice(idx + 2)
        for (const line of event.split('\n')) flushLine(line)
        if (closed) return
        idx = buffer.indexOf('\n\n')
      }
    }
    if (!closed) {
      // Stream ended without an explicit [DONE]. Treat as natural end.
      finish()
    }
  } catch (err) {
    if (closed || aborted) return
    logger.warn('OpenClaw gateway chat stream errored', {
      error: err instanceof Error ? err.message : String(err),
    })
    controller.enqueue({
      type: 'error',
      message: err instanceof Error ? err.message : String(err),
    })
    closed = true
    controller.close()
  } finally {
    signal?.removeEventListener('abort', onAbort)
    reader.releaseLock()
  }
}

interface OpenAIStreamChunk {
  choices?: Array<{
    delta?: { content?: unknown }
    finish_reason?: string | null
  }>
}

function extractDeltaText(value: unknown): string {
  const chunk = value as OpenAIStreamChunk
  const content = chunk?.choices?.[0]?.delta?.content
  return typeof content === 'string' ? content : ''
}

function extractFinishReason(value: unknown): string | null {
  const chunk = value as OpenAIStreamChunk
  return chunk?.choices?.[0]?.finish_reason ?? null
}
