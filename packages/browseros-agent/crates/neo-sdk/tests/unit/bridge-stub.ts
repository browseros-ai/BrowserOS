/**
 * Adapts a test's `__neo_runtime` stub whose `call` answers reply text, as the bridge did before
 * events, to the bridge's call contract (`runtime-bridge.ts`): `call` resolves
 * `{ reply, events }`, and `subscribe`, `drain` and `nextEvents` exist. A stub adapted here
 * queues no events, so every reply's watermark is 0.
 */

interface ReplyStub {
  get?: (request: string) => string
  call: (message: string) => Promise<string>
}

export function eventless<Stub extends ReplyStub>(stub: Stub) {
  return {
    ...stub,
    get: stub.get?.bind(stub),
    call: async (message: string) => ({
      reply: await stub.call(message),
      events: 0,
    }),
    subscribe: () => {},
    drain: () => '[]',
    nextEvents: () => new Promise<void>(() => {}),
  }
}
