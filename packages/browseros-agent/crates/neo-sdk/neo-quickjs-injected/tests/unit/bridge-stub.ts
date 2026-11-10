/**
 * Adapts a test's `__neo_runtime` stub whose `call` answers reply text to the native the SDK
 * calls (`runtime-bridge.ts`): `callWithEvents` resolves `{ reply, events }`, and `abort`,
 * `subscribe`, `drain` and `nextEvents` exist. A stub adapted here
 * queues no events, so every reply's watermark is 0.
 */

interface ReplyStub {
  get?: (request: string) => string
  call: (message: string) => Promise<string>
  abort?: (id: number, reason: string) => void
}

export function eventless<Stub extends ReplyStub>(stub: Stub) {
  return {
    ...stub,
    get: stub.get?.bind(stub),
    abort: stub.abort?.bind(stub) ?? (() => {}),
    callWithEvents: async (message: string) => ({
      reply: await stub.call(message),
      events: 0,
    }),
    subscribe: () => {},
    drain: () => '[]',
    nextEvents: () => new Promise<void>(() => {}),
  }
}
