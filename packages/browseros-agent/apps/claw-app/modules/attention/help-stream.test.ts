import { describe, expect, it } from 'bun:test'
import { backoffMs, takeSseEvents } from './help-stream'

describe('takeSseEvents', () => {
  it('names complete events and keeps a partial one for later', () => {
    const { events, rest } = takeSseEvents(
      'event: help\ndata: {"version":1}\n\n: keep-alive\n\nevent: help\ndata: {"ver',
    )
    expect(events).toEqual(['help'])
    expect(rest).toBe('event: help\ndata: {"ver')
  })

  it('reads CRLF framing and unnamed events', () => {
    const { events, rest } = takeSseEvents(
      'event: help\r\ndata: {}\r\n\r\ndata: x\r\n\r\n',
    )
    expect(events).toEqual(['help', 'message'])
    expect(rest).toBe('')
  })
})

describe('backoffMs', () => {
  it('doubles from a second and stops at thirty', () => {
    expect([0, 1, 2, 5, 9].map(backoffMs)).toEqual([
      1000, 2000, 4000, 30_000, 30_000,
    ])
  })
})
