// Runner N's stand-in for claw-sdk's bootstrap (sdk-skeleton D8), evaluated first in each run's
// fresh QuickJS realm, before assets/prelude.js. It gives the prelude exactly what bridge.ts
// reads from globalThis, over the two natives js_runner.rs installs:
//
//   __neo_invoke(name, inputJson, site) -> Promise<string>   one op through Playwright::invoke,
//                                                            always resolving to an envelope:
//                                                            {"ok": value, "pages": [...]} or
//                                                            {"err": {...}, "pages": [...]}
//   __neo_sleep(ms) -> Promise<undefined>                    the clock behind setTimeout
//
// Like the skeleton's codec it sends JSON only, throws its own NeoError or TimeoutError from an
// envelope's err, hands every envelope's page facts to the prelude, and revives the brands the
// prelude registers and the skeleton's bytes brand (a Uint8Array). NeoError and TimeoutError are globals here, so test scripts can name them.
;(() => {
  const nativeInvoke = globalThis.__neo_invoke
  const nativeSleep = globalThis.__neo_sleep

  class NeoError extends Error {
    constructor(err) {
      super(String(err.message))
      this.name = 'NeoError'
      for (const key of [
        'code',
        'owner',
        'delivery',
        'retryable',
        'operationId',
        'hostCode',
        'timeout',
      ])
        if (err[key] !== undefined) this[key] = err[key]
    }
  }

  class TimeoutError extends NeoError {
    constructor(err) {
      super(err)
      this.name = 'TimeoutError'
    }
  }

  const brands = new Map()
  const factListeners = []

  const BASE64 =
    'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

  // The skeleton's own `{"$neo":"bytes","base64":".."}` brand (D8), for screenshots and PDFs.
  // QuickJS has no atob.
  function bytesFrom(base64) {
    const digits = String(base64).replace(/[^A-Za-z0-9+/]/g, '')
    const bytes = new Uint8Array(Math.floor((digits.length * 3) / 4))
    let bits = 0
    let held = 0
    let at = 0
    for (const digit of digits) {
      bits = ((bits << 6) | BASE64.indexOf(digit)) & 0xffff
      held += 6
      if (held >= 8) {
        held -= 8
        bytes[at++] = (bits >> held) & 0xff
      }
    }
    return bytes
  }

  // In place: every object here came from JSON.parse, so a "__proto__" key is an own property.
  function revive(value) {
    if (value === null || typeof value !== 'object') return value
    if (value.$neo === 'bytes') return bytesFrom(value.base64)
    if (typeof value.$neo === 'string' && brands.has(value.$neo))
      return brands.get(value.$neo)(value)
    for (const key of Object.keys(value)) value[key] = revive(value[key])
    return value
  }

  async function invoke(name, input) {
    const envelope = JSON.parse(
      await nativeInvoke(String(name), JSON.stringify(input ?? {}), ''),
    )
    for (const listener of factListeners) listener(envelope.pages ?? [])
    if (envelope.err !== undefined)
      throw envelope.err.timeout
        ? new TimeoutError(envelope.err)
        : new NeoError(envelope.err)
    return revive(envelope.ok)
  }

  let nextTimer = 1
  const liveTimers = new Set()

  function setTimeout(callback, ms, ...args) {
    const id = nextTimer++
    liveTimers.add(id)
    nativeSleep(Number(ms) || 0).then(() => {
      if (liveTimers.delete(id) && typeof callback === 'function')
        callback(...args)
    })
    return id
  }

  function clearTimeout(id) {
    liveTimers.delete(id)
  }

  Object.assign(globalThis, {
    NeoError,
    TimeoutError,
    setTimeout,
    clearTimeout,
    __neo: Object.freeze({
      invoke,
      handles: Object.freeze({
        register(brand, fromJson) {
          brands.set(String(brand), fromJson)
        },
      }),
      onPageFacts(listener) {
        factListeners.push(listener)
      },
    }),
  })
})()
