import { describe, expect, test } from 'bun:test'
import type {
  AcpRuntimeEvent,
  AcpRuntimeHandle,
  AcpRuntimeTurnResult,
} from 'acpx/runtime'
import { acpEvent, acpResult } from './acp-event-builders.ts'
import { MockAcpRuntime } from './mock-acp-runtime.ts'

async function collectEvents(
  iterable: AsyncIterable<AcpRuntimeEvent>,
): Promise<AcpRuntimeEvent[]> {
  const out: AcpRuntimeEvent[] = []
  for await (const ev of iterable) out.push(ev)
  return out
}

const handle = (
  overrides: Partial<AcpRuntimeHandle> = {},
): AcpRuntimeHandle => ({
  sessionKey: 'k',
  backend: 'mock-acpx',
  runtimeSessionName: 'k::claude',
  cwd: '/tmp',
  acpxRecordId: 'rec',
  backendSessionId: 'sid',
  ...overrides,
})

describe('ensureSession', () => {
  test('records every call', async () => {
    const rt = new MockAcpRuntime()
    await rt.ensureSession({
      sessionKey: 'a',
      agent: 'claude',
      mode: 'persistent',
    })
    await rt.ensureSession({ sessionKey: 'b', agent: 'codex', mode: 'oneshot' })

    expect(rt.ensureSessionCalls).toHaveLength(2)
    expect(rt.ensureSessionCalls[0]?.sessionKey).toBe('a')
    expect(rt.ensureSessionCalls[1]?.agent).toBe('codex')
  })

  test('returns a deterministic default handle', async () => {
    const rt = new MockAcpRuntime()
    const h = await rt.ensureSession({
      sessionKey: 'k',
      agent: 'claude',
      mode: 'persistent',
      cwd: '/tmp',
    })
    expect(h).toMatchObject({
      sessionKey: 'k',
      backend: 'mock-acpx',
      cwd: '/tmp',
    })
  })

  test('rejects with the configured error', async () => {
    const err = new Error('no session')
    const rt = new MockAcpRuntime({ ensureSessionError: err })
    await expect(
      rt.ensureSession({
        sessionKey: 'k',
        agent: 'claude',
        mode: 'persistent',
      }),
    ).rejects.toBe(err)
  })

  test('honors a custom handle factory', async () => {
    const rt = new MockAcpRuntime({
      ensureSessionHandle: (input) =>
        handle({ sessionKey: `wrapped:${input.sessionKey}` }),
    })
    const h = await rt.ensureSession({
      sessionKey: 'k',
      agent: 'claude',
      mode: 'persistent',
    })
    expect(h.sessionKey).toBe('wrapped:k')
  })
})

describe('startTurn', () => {
  test('yields scripted events in order', async () => {
    const rt = new MockAcpRuntime({
      turnScripts: [{ events: [acpEvent.text('hi'), acpEvent.text(' there')] }],
    })
    const turn = rt.startTurn({
      handle: handle(),
      text: 'hello',
      mode: 'prompt',
      requestId: 'r',
    })
    expect(await collectEvents(turn.events)).toEqual([
      acpEvent.text('hi'),
      acpEvent.text(' there'),
    ])
  })

  test('records the input on every call and increments requestId', () => {
    const rt = new MockAcpRuntime({
      turnScripts: [{ events: [] }, { events: [] }],
    })
    const t1 = rt.startTurn({
      handle: handle(),
      text: 'one',
      mode: 'prompt',
      requestId: 'a',
    })
    const t2 = rt.startTurn({
      handle: handle(),
      text: 'two',
      mode: 'prompt',
      requestId: 'b',
    })
    expect(rt.startTurnCalls.map((c) => c.text)).toEqual(['one', 'two'])
    expect(t1.requestId).not.toBe(t2.requestId)
  })

  test('result resolves with the configured AcpRuntimeTurnResult', async () => {
    const result: AcpRuntimeTurnResult = {
      status: 'completed',
      stopReason: 'tool_calls',
    }
    const rt = new MockAcpRuntime({
      turnScripts: [{ events: [], result }],
    })
    const turn = rt.startTurn({
      handle: handle(),
      text: 'x',
      mode: 'prompt',
      requestId: 'r',
    })
    expect(await turn.result).toBe(result)
  })

  test('result rejects when configured', async () => {
    const err = new Error('turn blew up')
    const rt = new MockAcpRuntime({
      turnScripts: [{ events: [], resultError: err }],
    })
    const turn = rt.startTurn({
      handle: handle(),
      text: 'x',
      mode: 'prompt',
      requestId: 'r',
    })
    await expect(turn.result).rejects.toBe(err)
  })

  test('throws synchronously when startTurnError is configured', () => {
    const err = new Error('cant start')
    const rt = new MockAcpRuntime({
      turnScripts: [{ events: [], startTurnError: err }],
    })
    expect(() =>
      rt.startTurn({
        handle: handle(),
        text: 'x',
        mode: 'prompt',
        requestId: 'r',
      }),
    ).toThrow(err)
  })

  test('defaults to an end_turn completion with no events when no script remains', async () => {
    const rt = new MockAcpRuntime()
    const turn = rt.startTurn({
      handle: handle(),
      text: 'x',
      mode: 'prompt',
      requestId: 'r',
    })
    expect(await collectEvents(turn.events)).toEqual([])
    expect(await turn.result).toEqual({
      status: 'completed',
      stopReason: 'end_turn',
    })
  })

  test('cancel records the call and stops the iterator', async () => {
    const rt = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('a'), acpEvent.text('b')],
          delayMsBetweenEvents: 5,
        },
      ],
    })
    const turn = rt.startTurn({
      handle: handle(),
      text: 'x',
      mode: 'prompt',
      requestId: 'r',
    })
    const collected: AcpRuntimeEvent[] = []
    const reader = (async () => {
      for await (const ev of turn.events) {
        collected.push(ev)
        if (collected.length === 1) await turn.cancel({ reason: 'user' })
      }
    })()
    await reader
    expect(rt.turns[0]?.cancelCalls).toEqual([{ reason: 'user' }])
    expect(collected.length).toBeLessThan(2)
  })

  test('closeStream records the call and stops the iterator', async () => {
    const rt = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('a'), acpEvent.text('b')],
          delayMsBetweenEvents: 5,
        },
      ],
    })
    const turn = rt.startTurn({
      handle: handle(),
      text: 'x',
      mode: 'prompt',
      requestId: 'r',
    })
    await turn.closeStream({ reason: 'done' })
    expect(rt.turns[0]?.closeStreamCalls).toEqual([{ reason: 'done' }])
  })
})

describe('runTurn (compat)', () => {
  test('appends a done event from a completed result', async () => {
    const rt = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('hi')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const events = await collectEvents(
      rt.runTurn({
        handle: handle(),
        text: 'x',
        mode: 'prompt',
        requestId: 'r',
      }),
    )
    expect(events.at(-1)).toEqual({ type: 'done', stopReason: 'end_turn' })
  })

  test('appends an error event from a failed result', async () => {
    const rt = new MockAcpRuntime({
      turnScripts: [
        {
          events: [],
          result: acpResult.failed({ message: 'boom', code: 'rate' }),
        },
      ],
    })
    const events = await collectEvents(
      rt.runTurn({
        handle: handle(),
        text: 'x',
        mode: 'prompt',
        requestId: 'r',
      }),
    )
    expect(events.at(-1)).toEqual({
      type: 'error',
      message: 'boom',
      code: 'rate',
      retryable: undefined,
    })
  })

  test('appends an error event when result rejects', async () => {
    const rt = new MockAcpRuntime({
      turnScripts: [{ events: [], resultError: new Error('crashed') }],
    })
    const events = await collectEvents(
      rt.runTurn({
        handle: handle(),
        text: 'x',
        mode: 'prompt',
        requestId: 'r',
      }),
    )
    expect(events.at(-1)).toEqual({ type: 'error', message: 'crashed' })
  })
})

describe('lifecycle methods', () => {
  test('cancel and close are recorded with their inputs', async () => {
    const rt = new MockAcpRuntime()
    const h = handle()
    await rt.cancel({ handle: h, reason: 'a' })
    await rt.close({ handle: h, reason: 'b', discardPersistentState: true })
    expect(rt.cancelCalls).toEqual([{ handle: h, reason: 'a' }])
    expect(rt.closeCalls).toEqual([
      { handle: h, reason: 'b', discardPersistentState: true },
    ])
  })

  test('setMode and setConfigOption are recorded', async () => {
    const rt = new MockAcpRuntime()
    const h = handle()
    await rt.setMode({ handle: h, mode: 'plan' })
    await rt.setConfigOption({ handle: h, key: 'model', value: 'opus' })
    expect(rt.setModeCalls).toEqual([{ handle: h, mode: 'plan' }])
    expect(rt.setConfigOptionCalls).toEqual([
      { handle: h, key: 'model', value: 'opus' },
    ])
  })

  test('getStatus, getCapabilities, doctor return injected values when provided', async () => {
    const rt = new MockAcpRuntime({
      capabilities: { controls: ['session/status'] },
      status: { summary: 'busy' },
      doctorReport: { ok: false, message: 'no agent', code: 'AGENT_MISSING' },
    })
    expect(rt.getCapabilities({})).toEqual({ controls: ['session/status'] })
    expect(await rt.getStatus({ handle: handle() })).toEqual({
      summary: 'busy',
    })
    expect(await rt.doctor()).toMatchObject({
      ok: false,
      code: 'AGENT_MISSING',
    })
    expect(rt.getCapabilitiesCalls).toHaveLength(1)
    expect(rt.getStatusCalls).toHaveLength(1)
    expect(rt.doctorCalls).toHaveLength(1)
  })
})

describe('event builders', () => {
  test('text and thought produce text_delta with the right stream', () => {
    expect(acpEvent.text('a')).toEqual({
      type: 'text_delta',
      text: 'a',
      stream: 'output',
    })
    expect(acpEvent.thought('b')).toEqual({
      type: 'text_delta',
      text: 'b',
      stream: 'thought',
    })
  })

  test('toolCall passes status and title through', () => {
    const ev = acpEvent.toolCall({
      toolCallId: 't1',
      title: 'greet',
      text: '{}',
      status: 'completed',
    })
    expect(ev).toMatchObject({
      type: 'tool_call',
      toolCallId: 't1',
      title: 'greet',
      text: '{}',
      status: 'completed',
    })
  })

  test('usage is a status with usage_update tag', () => {
    expect(acpEvent.usage(100, 4096)).toEqual({
      type: 'status',
      text: '',
      tag: 'usage_update',
      used: 100,
      size: 4096,
    })
  })

  test('done with no stopReason yields undefined stopReason', () => {
    expect(acpEvent.done()).toEqual({ type: 'done', stopReason: undefined })
  })

  test('error preserves code and retryable', () => {
    expect(acpEvent.error('boom', { code: 'rate', retryable: true })).toEqual({
      type: 'error',
      message: 'boom',
      code: 'rate',
      retryable: true,
    })
  })

  test('acpResult builders match the AcpRuntimeTurnResult union', () => {
    expect(acpResult.completed()).toEqual({
      status: 'completed',
      stopReason: 'end_turn',
    })
    expect(acpResult.cancelled('user')).toEqual({
      status: 'cancelled',
      stopReason: 'user',
    })
    expect(acpResult.failed({ message: 'x' })).toEqual({
      status: 'failed',
      error: { message: 'x' },
    })
  })
})
