/**
 * A script location captured before the public call suspends. It is passed as a required
 * transport argument, separately from wire inputs, so internal awaits cannot lose or share it.
 * Public entry -> captureSite() -> async argument preparation -> invoke(op, site, ...) ->
 * bootstrap options.site -> the Rust bridge. INTERNAL_SITE deliberately carries no location.
 */
export class CallSite {
  private constructor(readonly value: string) {}

  static readonly internal = new CallSite('')

  static capture(): CallSite {
    const neo: unknown = Reflect.get(globalThis, '__neo')
    if (typeof neo !== 'object' || neo === null) return CallSite.internal
    const capture: unknown = Reflect.get(neo, 'callSite')
    if (typeof capture !== 'function') return CallSite.internal
    const value: unknown = Reflect.apply(capture, neo, [])
    return typeof value === 'string' ? new CallSite(value) : CallSite.internal
  }
}

/** Call synchronously at public method entry, before any argument preparation awaits. */
export function captureSite(): CallSite {
  return CallSite.capture()
}

/** Explicitly marks work owned by the prelude, such as listener maintenance and event pulls. */
export const INTERNAL_SITE = CallSite.internal
