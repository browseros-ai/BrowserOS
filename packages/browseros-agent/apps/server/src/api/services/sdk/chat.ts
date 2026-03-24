/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Chat Service - Executes actions via /chat endpoint
 */

import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import type { ChatSource } from '@browseros/shared/schemas/chat-source'
import type { LLMConfig } from '@browseros/shared/schemas/llm'
import type { UIMessageStreamEvent } from '@browseros/shared/schemas/ui-stream'
import { createParser, type EventSourceMessage } from 'eventsource-parser'
import { SdkError } from './types'

export interface ExecuteActionOptions {
  instruction: string
  context?: Record<string, unknown>
  browserContext?: BrowserContext
  source?: ChatSource
  llmConfig: LLMConfig
  signal?: AbortSignal
  onSSEEvent?: (event: UIMessageStreamEvent) => Promise<void>
  /** If provided, reuses existing session. Caller is responsible for cleanup. */
  sessionId?: string
}

export class ChatService {
  private readonly chatUrl: string

  constructor(port: number) {
    this.chatUrl = `http://127.0.0.1:${port}/chat`
  }

  async executeAction(options: ExecuteActionOptions): Promise<void> {
    const {
      instruction,
      context,
      browserContext,
      source,
      llmConfig,
      signal,
      onSSEEvent,
      sessionId: providedSessionId,
    } = options

    if (signal?.aborted) {
      throw new SdkError('Operation aborted', 400)
    }

    let message = instruction
    if (context) {
      message = `${instruction}\n\nContext:\n${JSON.stringify(context, null, 2)}`
    }

    // Use provided sessionId or generate ephemeral one
    const isEphemeral = !providedSessionId
    const conversationId = providedSessionId ?? crypto.randomUUID()

    const response = await fetch(this.chatUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        conversationId,
        message,
        provider: llmConfig.provider,
        model: llmConfig.model ?? 'default',
        apiKey: llmConfig.apiKey,
        baseUrl: llmConfig.baseUrl,
        resourceName: llmConfig.resourceName,
        region: llmConfig.region,
        accessKeyId: llmConfig.accessKeyId,
        secretAccessKey: llmConfig.secretAccessKey,
        sessionToken: llmConfig.sessionToken,
        browserContext,
        source,
      }),
      signal,
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new SdkError(
        errorText || 'Chat request failed',
        response.status >= 400 && response.status < 600 ? response.status : 500,
      )
    }

    const reader = response.body?.getReader()
    if (reader) {
      try {
        if (onSSEEvent) {
          await this.parseAndForwardSSE(reader, signal, onSSEEvent)
        } else {
          await this.drainStream(reader, signal)
        }
      } finally {
        reader.releaseLock()
      }
    }

    // Clean up ephemeral sessions only (persistent sessions are managed by caller)
    if (isEphemeral) {
      await fetch(`${this.chatUrl}/${conversationId}`, {
        method: 'DELETE',
      }).catch(() => {})
    }
  }

  private async parseAndForwardSSE(
    reader: ReadableStreamDefaultReader<Uint8Array>,
    signal: AbortSignal | undefined,
    onSSEEvent: (event: UIMessageStreamEvent) => Promise<void>,
  ): Promise<void> {
    const decoder = new TextDecoder()
    const pendingEvents: UIMessageStreamEvent[] = []

    const parser = createParser({
      onEvent: (msg: EventSourceMessage) => {
        if (msg.data === '[DONE]') return

        try {
          const event = JSON.parse(msg.data) as UIMessageStreamEvent
          pendingEvents.push(event)
        } catch {
          // Invalid JSON, skip
        }
      },
    })

    while (true) {
      if (signal?.aborted) {
        await reader.cancel()
        throw new SdkError('Operation aborted', 400)
      }

      const { done, value } = await reader.read()
      if (done) break

      const text = decoder.decode(value, { stream: true })
      parser.feed(text)

      // Process any events that were parsed
      let event = pendingEvents.shift()
      while (event) {
        await onSSEEvent(event)
        event = pendingEvents.shift()
      }
    }

    // Process any remaining events
    let remaining = pendingEvents.shift()
    while (remaining) {
      await onSSEEvent(remaining)
      remaining = pendingEvents.shift()
    }
  }

  private async drainStream(
    reader: ReadableStreamDefaultReader<Uint8Array>,
    signal: AbortSignal | undefined,
  ): Promise<void> {
    while (true) {
      if (signal?.aborted) {
        await reader.cancel()
        throw new SdkError('Operation aborted', 400)
      }
      const { done } = await reader.read()
      if (done) break
    }
  }
}
