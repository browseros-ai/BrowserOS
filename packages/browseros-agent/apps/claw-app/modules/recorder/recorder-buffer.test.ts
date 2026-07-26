import { describe, expect, it } from 'bun:test'
import {
  createRecorderBuffer,
  installRecorderFlushListeners,
} from './recorder-buffer'

function rrwebEvent(timestamp: number) {
  return { timestamp, type: 3, data: { source: timestamp } }
}

function serialized(event: ReturnType<typeof rrwebEvent>): string {
  return JSON.stringify({
    ts: event.timestamp,
    type: event.type,
    data: event.data,
  })
}

describe('createRecorderBuffer', () => {
  it('flushes at 50 events with only the recorder event fields', () => {
    const batches: string[] = []
    const buffer = createRecorderBuffer({
      send: (ndjson) => batches.push(ndjson),
      queueMicrotask: (callback) => callback(),
      setTimeout: () => 1,
    })

    for (let timestamp = 1; timestamp <= 50; timestamp++) {
      buffer.emit(rrwebEvent(timestamp))
    }

    expect(batches).toHaveLength(1)
    const lines = batches[0].split('\n').map((line) => JSON.parse(line))
    expect(lines).toHaveLength(50)
    expect(lines[0]).toEqual({ ts: 1, type: 3, data: { source: 1 } })
    expect(Object.keys(lines[0])).toEqual(['ts', 'type', 'data'])
  })

  it('flushes a partial batch when the timer fires', () => {
    const batches: string[] = []
    let timerCallback: (() => void) | undefined
    let timerDelay: number | undefined
    const buffer = createRecorderBuffer({
      send: (ndjson) => batches.push(ndjson),
      queueMicrotask: (callback) => callback(),
      setTimeout: (callback, delay) => {
        timerCallback = callback
        timerDelay = delay
        return 1
      },
    })

    buffer.emit(rrwebEvent(1))
    expect(batches).toEqual([])
    expect(timerDelay).toBe(2_500)

    timerCallback?.()
    expect(batches).toHaveLength(1)
  })

  it('keeps an exact-byte-boundary batch together and flushes before the next line', () => {
    const batches: string[] = []
    const first = rrwebEvent(1)
    const second = rrwebEvent(2)
    const exactBytes = new TextEncoder().encode(
      `${serialized(first)}\n${serialized(second)}`,
    ).byteLength
    const buffer = createRecorderBuffer({
      send: (ndjson) => batches.push(ndjson),
      queueMicrotask: (callback) => callback(),
      setTimeout: () => 1,
      flushAtSize: 10,
      maxBatchBytes: exactBytes,
    })

    buffer.emit(first)
    buffer.emit(second)
    buffer.emit(rrwebEvent(3))
    buffer.flushNow()

    expect(batches).toEqual([
      `${serialized(first)}\n${serialized(second)}`,
      serialized(rrwebEvent(3)),
    ])
  })

  it('counts multibyte text by UTF-8 bytes', () => {
    const batches: string[] = []
    const event = { timestamp: 1, type: 3, data: { text: 'é' } }
    const line = JSON.stringify({ ts: 1, type: 3, data: { text: 'é' } })
    const characterBoundary = line.length * 2 + 1
    const buffer = createRecorderBuffer({
      send: (ndjson) => batches.push(ndjson),
      queueMicrotask: (callback) => callback(),
      setTimeout: () => 1,
      flushAtSize: 10,
      maxBatchBytes: characterBoundary,
    })

    buffer.emit(event)
    buffer.emit({ ...event, timestamp: 2 })
    buffer.flushNow()

    expect(
      new TextEncoder().encode(`${line}\n${line}`).byteLength,
    ).toBeGreaterThan(characterBoundary)
    expect(batches).toHaveLength(2)
  })

  it('isolates an oversized line and continues later events in order', () => {
    const batches: string[] = []
    const large = { timestamp: 1, type: 2, data: { html: 'x'.repeat(200) } }
    const later = rrwebEvent(2)
    const buffer = createRecorderBuffer({
      send: (ndjson) => batches.push(ndjson),
      queueMicrotask: (callback) => callback(),
      setTimeout: () => 1,
      flushAtSize: 10,
      maxBatchBytes: 100,
    })

    buffer.emit(large)
    buffer.emit(later)
    buffer.flushNow()

    expect(batches.map((batch) => JSON.parse(batch).ts)).toEqual([1, 2])
    expect(batches[0]?.split('\n')).toHaveLength(1)
  })

  it('bounds the raw callback backlog before serialization', () => {
    let drain: (() => void) | undefined
    let serializedCount = 0
    const warnings: number[] = []
    const batches: string[] = []
    const buffer = createRecorderBuffer({
      send: (ndjson) => batches.push(ndjson),
      warnDropped: (count) => warnings.push(count),
      queueMicrotask: (callback) => {
        drain = callback
      },
      setTimeout: () => 1,
      bufferCap: 2,
      flushAtSize: 10,
    })
    const event = (timestamp: number) => ({
      timestamp,
      type: 3,
      data: {
        toJSON() {
          serializedCount++
          return { source: timestamp }
        },
      },
    })

    buffer.emit(event(1))
    buffer.emit(event(2))
    buffer.emit(event(3))
    drain?.()
    buffer.flushNow()

    expect(serializedCount).toBe(2)
    expect(warnings).toEqual([1])
    expect(batches[0]?.split('\n').map((line) => JSON.parse(line).ts)).toEqual([
      2, 3,
    ])
  })

  it('drops the oldest events at the buffer cap and reports the count', () => {
    const batches: string[] = []
    const gaps: boolean[] = []
    const warnings: number[] = []
    const buffer = createRecorderBuffer({
      send: (ndjson, hasGap) => {
        batches.push(ndjson)
        gaps.push(hasGap)
      },
      warnDropped: (count) => warnings.push(count),
      queueMicrotask: (callback) => callback(),
      setTimeout: () => 1,
      bufferCap: 2,
      flushAtSize: 10,
    })

    buffer.emit(rrwebEvent(1))
    buffer.emit(rrwebEvent(2))
    buffer.emit(rrwebEvent(3))
    buffer.flushNow()

    expect(warnings).toEqual([1])
    expect(gaps).toEqual([true])
    expect(batches[0].split('\n').map((line) => JSON.parse(line).ts)).toEqual([
      2, 3,
    ])
  })
})

describe('installRecorderFlushListeners', () => {
  it('flushes pending events on pagehide', () => {
    const pageListeners = new Map<string, () => void>()
    const documentListeners = new Map<string, () => void>()
    const batches: string[] = []
    const buffer = createRecorderBuffer({
      send: (ndjson) => batches.push(ndjson),
      queueMicrotask: (callback) => callback(),
      setTimeout: () => 1,
    })

    installRecorderFlushListeners({
      page: {
        addEventListener: (type, listener) =>
          pageListeners.set(type, listener as () => void),
      },
      document: {
        visibilityState: 'visible',
        addEventListener: (type, listener) =>
          documentListeners.set(type, listener as () => void),
      },
      flush: buffer.flushNow,
    })

    buffer.emit(rrwebEvent(1))
    pageListeners.get('pagehide')?.()

    expect(batches).toHaveLength(1)
  })

  it('flushes only when visibility changes to hidden', () => {
    const documentListeners = new Map<string, () => void>()
    const batches: string[] = []
    let visibilityState = 'visible'
    const buffer = createRecorderBuffer({
      send: (ndjson) => batches.push(ndjson),
      queueMicrotask: (callback) => callback(),
      setTimeout: () => 1,
    })

    installRecorderFlushListeners({
      page: { addEventListener: () => {} },
      document: {
        get visibilityState() {
          return visibilityState
        },
        addEventListener: (type, listener) =>
          documentListeners.set(type, listener as () => void),
      },
      flush: buffer.flushNow,
    })

    buffer.emit(rrwebEvent(1))
    documentListeners.get('visibilitychange')?.()
    expect(batches).toEqual([])

    visibilityState = 'hidden'
    documentListeners.get('visibilitychange')?.()
    expect(batches).toHaveLength(1)
  })
})
