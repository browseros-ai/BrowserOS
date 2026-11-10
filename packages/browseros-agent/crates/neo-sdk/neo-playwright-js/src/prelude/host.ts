/**
 * claw-sdk's run bootstrap as the prelude reaches it: the frozen `globalThis.__neo`
 * (crates/neo-sdk/neo-server/src/runtime/js/bootstrap.js). bridge.ts runs ops and gives the agent's
 * function its parameters through it, and errors.ts records the prelude's refusals through it,
 * so it sits in a module of its own: errors.ts cannot import bridge.ts, which imports errors.ts.
 */

/** The codes a refusal the prelude decides may carry: both script-owned (FR/RC C4). */
export type RefusalCode = 'method_unavailable' | 'invalid_input'

export type Bootstrap = {
  invoke(name: string, input?: unknown): Promise<unknown>
  brands: {
    register(brand: string, fromJson: (json: object) => unknown): void
  }
  onReply(listener: (envelope: unknown) => void): void
  provide(name: string, build: (init: unknown) => unknown): void
  /**
   * A refusal as a door call (FR/RC C1): the bootstrap's own NeoError, at once, while the door
   * records the try under `api` and answers with the error's code, owner, delivery and op id,
   * which reach the error only after the script next yields to Rust. Absent from a bootstrap
   * older than that contract, which also builds every parameter synchronously (C5).
   */
  refuse?(api: string, code: RefusalCode, reason: string): Error
}

/** The bootstrap, or a plain error when the prelude runs without one. */
export function host(): Bootstrap {
  const neo: unknown = Reflect.get(globalThis, '__neo')
  if (typeof neo !== 'object' || neo === null)
    throw new Error(
      "The Playwright prelude needs claw-sdk's bootstrap (globalThis.__neo), which was not installed",
    )
  return neo as Bootstrap
}

/**
 * The bootstrap's `refuse`, or null for an older bootstrap, which gets the prelude's refusals as
 * the errors they were before FR/RC. Read on each refusal, never cached, so a realm without a
 * bootstrap (a unit test of one module) has none.
 */
export function refuser():
  | ((api: string, code: RefusalCode, reason: string) => Error)
  | null {
  const neo: unknown = Reflect.get(globalThis, '__neo')
  if (typeof neo !== 'object' || neo === null) return null
  const refuse: unknown = Reflect.get(neo, 'refuse')
  return typeof refuse === 'function'
    ? (api, code, reason) =>
        Reflect.apply(refuse, neo, [api, code, reason]) as Error
    : null
}
