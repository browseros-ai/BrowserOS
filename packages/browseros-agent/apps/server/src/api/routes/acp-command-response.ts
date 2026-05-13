/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Create a synthetic ACP UI message stream response for slash commands
 * that are handled server-side (no LLM call needed).
 */

import type { AgentStreamEvent } from '../../lib/agents/types'

/**
 * Build a ReadableStream of AgentStreamEvent that yields a single
 * text_delta followed by a done event — mimicking a normal LLM
 * response for a pre-computed string.
 */
export function createSyntheticCommandStream(
  text: string,
): ReadableStream<AgentStreamEvent> {
  const events: AgentStreamEvent[] = [
    { type: 'text_delta', text, stream: 'output' },
    { type: 'done', stopReason: 'end_turn' },
  ]

  return new ReadableStream<AgentStreamEvent>({
    start(controller) {
      for (const event of events) {
        controller.enqueue(event)
      }
      controller.close()
    },
  })
}
