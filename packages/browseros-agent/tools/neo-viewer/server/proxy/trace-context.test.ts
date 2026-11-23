import { describe, expect, test } from 'bun:test'
import { readRequestBody } from './request-body'
import { parseTraceparent, traceContextFor } from './trace-context'

const VALID = '00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01'

function runCall(id: number, meta?: Record<string, unknown>) {
  return {
    jsonrpc: '2.0',
    id,
    method: 'tools/call',
    params: {
      name: 'run',
      arguments: { code: 'return 1' },
      ...(meta ? { _meta: { 'com.browseros.neo/run': meta } } : {}),
    },
  }
}

function messages(body: unknown) {
  return readRequestBody(new TextEncoder().encode(JSON.stringify(body)))
    .messages
}

/** Counts up from 1, so a minted context is predictable. */
function counting() {
  let n = 0
  return (bytes: number) => Uint8Array.from({ length: bytes }, () => ++n)
}

describe('parseTraceparent reads what Neo reads, and nothing else', () => {
  test('a valid value', () => {
    expect(parseTraceparent(VALID)).toEqual({
      traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
      spanId: '00f067aa0ba902b7',
    })
    // A later version may append fields; its first four are read.
    expect(parseTraceparent(`01${VALID.slice(2)}-later`)?.spanId).toBe(
      '00f067aa0ba902b7',
    )
  })

  test.each([
    ['version ff', `ff${VALID.slice(2)}`],
    ['all-zero trace id', `00-${'0'.repeat(32)}-00f067aa0ba902b7-01`],
    [
      'all-zero span id',
      `00-4bf92f3577b34da6a3ce929d0e0e4736-${'0'.repeat(16)}-01`,
    ],
    ['uppercase hex', VALID.toUpperCase()],
    ['version 00 with more after its flags', `${VALID}-x`],
    ['short trace id', '00-4bf92f35-00f067aa0ba902b7-01'],
    ['garbage', 'not a traceparent'],
  ])('%s is ignored', (_, value) => {
    expect(parseTraceparent(value)).toBeNull()
  })
})

describe('traceContextFor', () => {
  test('one run call with no context: the viewer mints a sampled one and adds the header', () => {
    const decision = traceContextFor(
      new Headers(),
      messages(runCall(1)),
      counting(),
    )
    expect(decision).toEqual({
      context: {
        traceId: '0102030405060708090a0b0c0d0e0f10',
        spanId: '1112131415161718',
        injected: true,
      },
      inject: {
        traceparent: '00-0102030405060708090a0b0c0d0e0f10-1112131415161718-01',
      },
    })
  })

  test("the caller's header is the run's context, never replaced", () => {
    expect(
      traceContextFor(
        new Headers({ traceparent: VALID }),
        messages(runCall(1)),
      ),
    ).toEqual({
      context: {
        traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
        spanId: '00f067aa0ba902b7',
        injected: false,
      },
      inject: null,
    })
  })

  test("the caller's _meta traceparent is the run's context; a header would override it, so none is added", () => {
    const decision = traceContextFor(
      new Headers(),
      messages(runCall(1, { traceparent: VALID, taskId: 't-1' })),
    )
    expect(decision?.inject).toBeNull()
    expect(decision?.context).toMatchObject({
      traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
      injected: false,
    })
  })

  test('an invalid caller value, header or _meta, leaves the run without context and adds nothing', () => {
    expect(
      traceContextFor(
        new Headers({ traceparent: 'nonsense' }),
        messages(runCall(1)),
      ),
    ).toBeNull()
    expect(
      traceContextFor(
        new Headers(),
        messages(runCall(1, { traceparent: 'nonsense' })),
      ),
    ).toBeNull()
    expect(
      traceContextFor(new Headers(), messages(runCall(1, { traceparent: 7 }))),
    ).toBeNull()
  })

  test('no run call, or a batch of several: nothing is minted', () => {
    expect(
      traceContextFor(
        new Headers(),
        messages({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
      ),
    ).toBeNull()
    expect(
      traceContextFor(new Headers(), messages([runCall(1), runCall(2)])),
    ).toBeNull()
    // A call of another tool: Neo refuses it before any run, so no span would name it.
    expect(
      traceContextFor(
        new Headers(),
        messages({
          jsonrpc: '2.0',
          id: 1,
          method: 'tools/call',
          params: { name: 'other', arguments: {} },
        }),
      ),
    ).toBeNull()
    // A notification-shaped call (no id) is not a run.
    const { id: _, ...notification } = runCall(1)
    expect(traceContextFor(new Headers(), messages(notification))).toBeNull()
  })
})
