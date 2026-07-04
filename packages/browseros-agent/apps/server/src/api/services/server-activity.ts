/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { TurnRegistry } from '../../lib/agents/turns/active-turn-registry'

export class ServerActivity {
  private activeChatStreams = 0
  private activeMcpToolExecutions = 0

  constructor(private readonly turnRegistry: TurnRegistry) {}

  beginChatStream(): void {
    this.activeChatStreams += 1
  }

  endChatStream(): void {
    this.activeChatStreams = Math.max(0, this.activeChatStreams - 1)
  }

  beginMcpToolExecution(): void {
    this.activeMcpToolExecutions += 1
  }

  endMcpToolExecution(): void {
    this.activeMcpToolExecutions = Math.max(0, this.activeMcpToolExecutions - 1)
  }

  trackChatResponse(response: Response, abortSignal?: AbortSignal): Response {
    this.beginChatStream()
    let ended = false
    const end = () => {
      if (ended) return
      ended = true
      abortSignal?.removeEventListener('abort', end)
      this.endChatStream()
    }

    if (abortSignal?.aborted) {
      end()
      return response
    }
    abortSignal?.addEventListener('abort', end, { once: true })

    if (!response.body) {
      end()
      return response
    }

    const reader = response.body.getReader()
    const body = new ReadableStream<Uint8Array>({
      async pull(controller) {
        try {
          const { done, value } = await reader.read()
          if (done) {
            end()
            controller.close()
            return
          }
          controller.enqueue(value)
        } catch (error) {
          end()
          controller.error(error)
        }
      },
      async cancel(reason) {
        end()
        await reader.cancel(reason)
      },
    })

    return new Response(body, {
      status: response.status,
      statusText: response.statusText,
      headers: response.headers,
    })
  }

  isBusy(): boolean {
    return (
      this.activeChatStreams > 0 ||
      this.activeMcpToolExecutions > 0 ||
      this.turnRegistry.hasRunning()
    )
  }
}
