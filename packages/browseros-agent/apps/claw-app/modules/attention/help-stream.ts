/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Listens to the server's help stream, so a new help request is noticed the
 * moment it opens instead of on the next periodic look. The stream only says
 * that something changed; the caller re-reads the live sessions.
 */

const HELP_EVENTS_PATH = '/api/v1/help/events'
const HELP_EVENT = 'help'
const MAX_BACKOFF_MS = 30_000

/** Splits buffered SSE text into complete events' names and what is left. */
export function takeSseEvents(buffer: string): {
  events: string[]
  rest: string
} {
  const normalized = buffer.replace(/\r\n/g, '\n')
  const blocks = normalized.split('\n\n')
  const rest = blocks.pop() ?? ''
  const events: string[] = []
  for (const block of blocks) {
    const lines = block.split('\n')
    // A block with only comments is a keep-alive and has no event.
    if (!lines.some((line) => line.startsWith('data:'))) continue
    const name = lines
      .find((line) => line.startsWith('event:'))
      ?.slice('event:'.length)
      .trim()
    events.push(name || 'message')
  }
  return { events, rest }
}

/** The wait before reconnect attempt `attempt`: 1 s, 2 s, 4 s and so on, capped. */
export function backoffMs(attempt: number): number {
  return Math.min(MAX_BACKOFF_MS, 1000 * 2 ** Math.max(0, attempt))
}

export interface HelpStreamOptions {
  resolveServerBaseUrl: () => Promise<string>
  /** Called for every help change, including the one sent on connect. */
  onChange: () => void
}

export interface HelpStream {
  /** Opens the stream unless it is already open or reconnecting. */
  ensure: () => void
}

export function createHelpStream(options: HelpStreamOptions): HelpStream {
  let running = false

  const readOnce = async (): Promise<void> => {
    const base = await options.resolveServerBaseUrl()
    const response = await fetch(`${base}${HELP_EVENTS_PATH}`, {
      headers: { Accept: 'text/event-stream' },
    })
    if (!response.ok || !response.body) {
      throw new Error(`help stream answered ${response.status}`)
    }
    const reader = response.body.getReader()
    const decoder = new TextDecoder()
    let buffer = ''
    for (;;) {
      const { done, value } = await reader.read()
      if (done) return
      buffer += decoder.decode(value, { stream: true })
      const { events, rest } = takeSseEvents(buffer)
      buffer = rest
      if (events.includes(HELP_EVENT)) options.onChange()
    }
  }

  const run = async () => {
    let attempt = 0
    while (running) {
      try {
        await readOnce()
        attempt = 0
      } catch {
        attempt += 1
      }
      // The server restarted or the stream dropped; come back after a pause.
      await new Promise((resolve) => setTimeout(resolve, backoffMs(attempt)))
    }
  }

  return {
    ensure() {
      if (running) return
      running = true
      void run().finally(() => {
        running = false
      })
    },
  }
}
