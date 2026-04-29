/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import {
  RingBuffer,
  type TurnFrame,
  TurnRegistry,
} from '../../../src/lib/agents/active-turn-registry'

const registries: TurnRegistry[] = []
afterEach(() => {
  for (const r of registries.splice(0)) r.stopSweeper()
})

function makeRegistry(opts?: ConstructorParameters<typeof TurnRegistry>[0]) {
  const r = new TurnRegistry({
    retainAfterDoneMs: 1000,
    sweepIntervalMs: 60_000,
    ...(opts ?? {}),
  })
  registries.push(r)
  return r
}

async function collect(
  stream: ReadableStream<TurnFrame>,
): Promise<TurnFrame[]> {
  const reader = stream.getReader()
  const out: TurnFrame[] = []
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    out.push(value)
  }
  return out
}

describe('RingBuffer', () => {
  it('assigns monotonic seq starting at 0', () => {
    const buf = new RingBuffer(10)
    const a = buf.push({ type: 'text_delta', text: 'a', stream: 'output' })
    const b = buf.push({ type: 'text_delta', text: 'b', stream: 'output' })
    expect(a.seq).toBe(0)
    expect(b.seq).toBe(1)
    expect(buf.lastSeq).toBe(1)
  })

  it('drops oldest on overflow and sets truncated', () => {
    const buf = new RingBuffer(2)
    buf.push({ type: 'text_delta', text: 'a', stream: 'output' })
    buf.push({ type: 'text_delta', text: 'b', stream: 'output' })
    buf.push({ type: 'text_delta', text: 'c', stream: 'output' })
    expect(buf.length).toBe(2)
    expect(buf.truncated).toBe(true)
  })

  it('preserves the terminal frame even after eviction', () => {
    const buf = new RingBuffer(2)
    buf.push({ type: 'text_delta', text: 'a', stream: 'output' })
    buf.push({ type: 'done', stopReason: 'end_turn' })
    buf.push({ type: 'text_delta', text: 'late', stream: 'output' })
    // Done frame had seq=1; the slice from -1 must still expose it.
    const frames = buf.slice(-1)
    const seqs = frames.map((f) => f.seq)
    expect(seqs).toContain(1)
  })

  it('slice returns only frames newer than fromSeq', () => {
    const buf = new RingBuffer(10)
    for (let i = 0; i < 5; i++) {
      buf.push({ type: 'text_delta', text: String(i), stream: 'output' })
    }
    const after2 = buf.slice(2).map((f) => f.seq)
    expect(after2).toEqual([3, 4])
  })
})

describe('TurnRegistry', () => {
  it('replays buffered frames to late subscribers', async () => {
    const registry = makeRegistry()
    const turn = registry.register('agent-1')
    registry.pushEvent(turn.turnId, {
      type: 'text_delta',
      text: 'one',
      stream: 'output',
    })
    registry.pushEvent(turn.turnId, {
      type: 'text_delta',
      text: 'two',
      stream: 'output',
    })

    const stream = registry.subscribe(turn.turnId, { fromSeq: -1 })!
    // Push more after subscribe — both buffered and live frames should
    // arrive in order.
    registry.pushEvent(turn.turnId, {
      type: 'text_delta',
      text: 'three',
      stream: 'output',
    })
    registry.pushEvent(turn.turnId, { type: 'done', stopReason: 'end_turn' })

    const frames = await collect(stream)
    const texts = frames.map((f) =>
      f.event.type === 'text_delta' ? f.event.text : f.event.type,
    )
    expect(texts).toEqual(['one', 'two', 'three', 'done'])
    expect(frames.map((f) => f.seq)).toEqual([0, 1, 2, 3])
  })

  it('skips already-seen frames when subscriber resumes from lastSeq', async () => {
    const registry = makeRegistry()
    const turn = registry.register('agent-1')
    for (let i = 0; i < 4; i++) {
      registry.pushEvent(turn.turnId, {
        type: 'text_delta',
        text: String(i),
        stream: 'output',
      })
    }
    registry.pushEvent(turn.turnId, { type: 'done', stopReason: 'end_turn' })

    const stream = registry.subscribe(turn.turnId, { fromSeq: 2 })!
    const frames = await collect(stream)
    expect(frames.map((f) => f.seq)).toEqual([3, 4])
  })

  it('marks status `cancelled` and emits a synthetic terminal on cancel', async () => {
    const registry = makeRegistry()
    const turn = registry.register('agent-1')
    const stream = registry.subscribe(turn.turnId, { fromSeq: -1 })!

    registry.pushEvent(turn.turnId, {
      type: 'text_delta',
      text: 'partial',
      stream: 'output',
    })
    registry.cancel(turn.turnId, 'user pressed stop')

    const frames = await collect(stream)
    const last = frames.at(-1)
    expect(last?.event.type).toBe('done')
    expect(last?.event.type === 'done' ? last.event.stopReason : null).toBe(
      'cancelled',
    )

    expect(registry.describe(turn.turnId)?.status).toBe('cancelled')
    expect(turn.abortController.signal.aborted).toBe(true)
  })

  it('reports the active turn for an agent', () => {
    const registry = makeRegistry()
    expect(registry.getActiveFor('agent-1')).toBeUndefined()
    const turn = registry.register('agent-1')
    expect(registry.getActiveFor('agent-1')?.turnId).toBe(turn.turnId)
    registry.pushEvent(turn.turnId, { type: 'done', stopReason: 'end_turn' })
    expect(registry.getActiveFor('agent-1')).toBeUndefined()
  })

  it('evicts terminal turns past the retain window via sweep', () => {
    const registry = makeRegistry({ retainAfterDoneMs: 1 })
    const turn = registry.register('agent-1')
    registry.pushEvent(turn.turnId, { type: 'done', stopReason: 'end_turn' })
    expect(registry.size()).toBe(1)
    // Advance fake "now" past the retain window.
    registry.sweep(Date.now() + 10)
    expect(registry.size()).toBe(0)
  })

  it('returns null for unknown turns', () => {
    const registry = makeRegistry()
    expect(registry.subscribe('nope')).toBeNull()
    expect(registry.describe('nope')).toBeNull()
    expect(registry.cancel('nope')).toBe(false)
  })
})
