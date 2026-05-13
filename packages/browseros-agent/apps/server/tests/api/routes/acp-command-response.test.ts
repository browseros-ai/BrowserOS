/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { createSyntheticCommandStream } from '../../../src/api/routes/acp-command-response'

describe('createSyntheticCommandStream', () => {
  it('produces text_delta + done events', async () => {
    const stream = createSyntheticCommandStream('Hello from command')
    const reader = stream.getReader()
    const events: unknown[] = []

    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      events.push(value)
    }

    expect(events).toHaveLength(2)
    expect(events[0]).toEqual({
      type: 'text_delta',
      text: 'Hello from command',
      stream: 'output',
    })
    expect(events[1]).toEqual({ type: 'done', stopReason: 'end_turn' })
  })

  it('handles empty text', async () => {
    const stream = createSyntheticCommandStream('')
    const reader = stream.getReader()
    const events: unknown[] = []

    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      events.push(value)
    }

    expect(events).toHaveLength(2)
    expect(events[0]).toEqual({
      type: 'text_delta',
      text: '',
      stream: 'output',
    })
  })

  it('handles multiline text', async () => {
    const text = 'line 1\nline 2\nline 3'
    const stream = createSyntheticCommandStream(text)
    const reader = stream.getReader()
    const events: unknown[] = []

    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      events.push(value)
    }

    expect(events[0]).toMatchObject({ text })
  })
})
