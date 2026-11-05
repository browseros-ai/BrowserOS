// Runs the built prelude asset the way claw-sdk does: a fresh realm per run, the bootstrap
// first, then the prelude, then the script. The bootstrap here follows sdk-skeleton D8: it lives
// in the script's realm, sends JSON over one host function, reads `{ ok | err, pages }`
// envelopes, throws its own NeoError / TimeoutError, revives registered brands, and refuses
// any input that is not plain data, as the real codec refuses unbranded class instances.
import { readFileSync } from 'node:fs'
import { createContext, runInContext } from 'node:vm'

export type Call = { name: string; input: Record<string, unknown> }

/** An op's answer: a value, or an envelope (`{ ok }`, `{ err }`, with `pages`). */
type Envelope = {
  ok?: unknown
  err?: Record<string, unknown>
  pages?: unknown[]
}
export type Answer = (call: Call) => unknown | Promise<unknown>

/** The committed prelude asset, read when a runtime is built without prelude text. */
const committedPrelude = (): string =>
  readFileSync(
    new URL(
      '../../../../crates/browseros-playwright/assets/prelude.js',
      import.meta.url,
    ),
    'utf8',
  )

const BOOTSTRAP = `(() => {
  class NeoError extends Error {
    constructor(err) {
      super(err.message)
      this.name = 'NeoError'
      for (const key of ['code', 'owner', 'delivery', 'retryable', 'operationId', 'hostCode', 'timeout'])
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
  const plain = (value, path) => {
    if (typeof value === 'function' || typeof value === 'symbol')
      throw new TypeError('codec: ' + path + ' is a ' + typeof value)
    if (typeof value !== 'object' || value === null) return
    const proto = Object.getPrototypeOf(value)
    if (proto !== Object.prototype && proto !== Array.prototype && proto !== null)
      throw new TypeError('codec: ' + path + ' is an unbranded class instance')
    for (const [key, item] of Object.entries(value)) plain(item, path + '.' + key)
  }
  const revive = (value) => {
    if (!globalThis.__testRevive || typeof value !== 'object' || value === null) return value
    if (Array.isArray(value)) return value.map(revive)
    if (typeof value.$neo === 'string' && brands.has(value.$neo)) return brands.get(value.$neo)(value)
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, revive(item)]))
  }
  globalThis.NeoError = NeoError
  globalThis.TimeoutError = TimeoutError
  globalThis.__neo = {
    invoke(name, input) {
      plain(input, 'input')
      return __hostInvoke(name, JSON.stringify(input)).then((text) => {
        const envelope = JSON.parse(text)
        for (const listener of factListeners) listener(envelope.pages ?? [])
        if (envelope.err) throw envelope.err.timeout ? new TimeoutError(envelope.err) : new NeoError(envelope.err)
        return revive(envelope.ok)
      })
    },
    handles: { register(brand, fromJson) { brands.set(brand, fromJson) } },
    onPageFacts(listener) { factListeners.push(listener) },
  }
})()`

export type Runtime = {
  /** Every op the prelude sent, decoded from the JSON the bridge carried. */
  calls: Call[]
  /** Runs a script body (`return await page.title()`) as claw-sdk runs `run(code)`. */
  run(code: string): Promise<unknown>
  /** Evaluates an expression in the run's realm. */
  evaluate(code: string): unknown
  /** The ops named `name`, in order. */
  sent(name: string): Call[]
}

export function loadPrelude(
  options: {
    answer?: Answer
    seed?: unknown
    revive?: boolean
    /** The prelude's source; the committed asset when absent (build.ts passes a fresh build). */
    prelude?: string
    /** A fixed `Date.now()` for the realm, so recorded `since` stamps are reproducible. */
    now?: number
  } = {},
): Runtime {
  const calls: Call[] = []
  const answer = options.answer ?? (() => null)
  const realm = createContext({
    __hostInvoke: async (name: string, inputJson: string): Promise<string> => {
      const call: Call = { name, input: JSON.parse(inputJson) }
      calls.push(call)
      let reply: unknown
      try {
        reply = await answer(call)
      } catch (error) {
        reply = { err: error }
      }
      const envelope: Envelope =
        typeof reply === 'object' &&
        reply !== null &&
        ('ok' in reply || 'err' in reply)
          ? (reply as Envelope)
          : { ok: reply }
      return JSON.stringify({ pages: [], ...envelope, ok: envelope.ok ?? null })
    },
    __testRevive: options.revive ?? true,
    setTimeout,
    clearTimeout,
  })
  if (options.seed !== undefined)
    Reflect.set(realm, '__neoPlaywrightSeed', options.seed)
  if (options.now !== undefined)
    runInContext(`Date.now = () => ${options.now}`, realm)
  runInContext(BOOTSTRAP, realm)
  runInContext(options.prelude ?? committedPrelude(), realm)
  return {
    calls,
    run: (code) => runInContext(`(async () => {\n${code}\n})()`, realm),
    evaluate: (code) => runInContext(code, realm),
    sent: (name) => calls.filter((call) => call.name === name),
  }
}

/** A seed with one open tab, page 7 on tab 1007, as the run's default page. */
export const SEED = {
  page: 7,
  pages: [
    {
      id: 7,
      tab: 1007,
      url: 'https://shop.example/cart',
      title: 'Cart',
      closed: false,
    },
  ],
  browserVersion: '146.0.7680.0',
}

/** JSON copy of a value from the run's realm, for assertions across realms. */
export function json(value: unknown): unknown {
  return value === undefined ? undefined : JSON.parse(JSON.stringify(value))
}
