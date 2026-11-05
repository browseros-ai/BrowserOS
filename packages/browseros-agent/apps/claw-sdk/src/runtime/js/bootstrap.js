// claw-sdk's bootstrap, evaluated first in every run's fresh QuickJS realm (run_script.rs).
//
// The bridge's three natives take and return strings only. This file turns them into the agent
// API's plumbing: NeoError and TimeoutError, the input codec, reply envelopes, call sites,
// handle brands, console, timers, and the run itself. After it and the preludes run, the realm's
// globals are QuickJS's own plus __neo_invoke, __neo_log, __neo_sleep and __neo (pinned by
// runtime/tests.rs): the agent's API reaches its code as the parameters of its function, never
// as globals.

;(() => {
  const nativeInvoke = globalThis.__neo_invoke // (name, inputJson, site) => Promise<string>, never rejects
  const nativeLog = globalThis.__neo_log // (level, text) => undefined
  const nativeSleep = globalThis.__neo_sleep // (ms) => Promise<undefined>, cut short by the run's stop

  // How much of a thrown error's stack crosses to Rust, which reads only the agent's first frame
  // from it. The name and message cross whole: Rust masks known secrets before it clips them,
  // since a clip through a secret would leave a prefix no mask recognizes.
  const STACK_CHARS = 4096

  // Every NeoError this file built, with the op it came from. An error the script made or edited
  // itself is not here, so it can never pass for an op's failure as the run's error: Rust looks
  // the op up and reports the failure the door decided.
  const genuine = new WeakMap()

  /** Every SDK failure. The fields are the trace's, so the agent and the record agree. */
  class NeoError extends Error {
    constructor(err) {
      super(String(err.message))
      this.name = 'NeoError'
      this.code = err.code // e.g. "page_closed"
      this.owner = err.owner // "neo" | "script" | "website" | "host"
      this.delivery = err.delivery // "not_sent" | "done" | "unknown"
      this.retryable = err.retryable
      this.operationId = err.operationId
    }
  }

  /** Thrown when the operation's error says it timed out, as Playwright does. */
  class TimeoutError extends NeoError {
    constructor(err) {
      super(err)
      this.name = 'TimeoutError'
    }
  }

  const BASE64 =
    'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

  function toBase64(bytes) {
    const parts = []
    for (let index = 0; index < bytes.length; index += 3) {
      const second = bytes[index + 1]
      const third = bytes[index + 2]
      const n = (bytes[index] << 16) | ((second ?? 0) << 8) | (third ?? 0)
      parts.push(
        BASE64[(n >> 18) & 63] +
          BASE64[(n >> 12) & 63] +
          (second === undefined ? '=' : BASE64[(n >> 6) & 63]) +
          (third === undefined ? '=' : BASE64[n & 63]),
      )
    }
    return parts.join('')
  }

  function fromBase64(text) {
    const digits = String(text).replace(/[^A-Za-z0-9+/]/g, '')
    const bytes = new Uint8Array(Math.floor((digits.length * 3) / 4))
    let bits = 0
    let buffer = 0
    let length = 0
    for (const char of digits) {
      buffer = ((buffer << 6) | BASE64.indexOf(char)) & 0xffffff
      bits += 6
      if (bits >= 8) {
        bits -= 8
        bytes[length++] = (buffer >> bits) & 255
      }
    }
    return bytes.subarray(0, length)
  }

  // "$neo" brand -> reviver, for handles inside results. Preludes add theirs ("page" from the
  // page prelude, "locator" from sdk-playwright's).
  const brands = new Map([['bytes', (json) => fromBase64(json.base64)]])

  // In place: every object here came from JSON.parse, so a "__proto__" key is an own data
  // property and assigning to it never reaches the prototype.
  function revive(value) {
    if (value === null || typeof value !== 'object') return value
    if (typeof value.$neo === 'string' && brands.has(value.$neo)) {
      return brands.get(value.$neo)(value)
    }
    for (const key of Object.keys(value)) value[key] = revive(value[key])
    return value
  }

  class CodecError extends Error {}

  function className(value) {
    try {
      return value.constructor?.name || 'class instance'
    } catch {
      return 'class instance'
    }
  }

  // Plain data only. A handle (or a Date) serializes through its toJSON, bytes become the bytes
  // brand, and a function, symbol, bigint, cycle or class instance is refused with its path.
  function plain(value, path, ancestors) {
    switch (typeof value) {
      case 'string':
      case 'boolean':
      case 'number':
      case 'undefined':
        return value
      case 'bigint':
      case 'function':
      case 'symbol':
        throw new CodecError(
          `${path} is a ${typeof value}; SDK calls take plain data`,
        )
    }
    if (value === null) return null
    if (ancestors.includes(value)) {
      throw new CodecError(
        `${path} refers back to itself; SDK calls take plain data`,
      )
    }
    if (value instanceof Uint8Array)
      return { $neo: 'bytes', base64: toBase64(value) }
    ancestors.push(value)
    try {
      if (typeof value.toJSON === 'function')
        return plain(value.toJSON(), path, ancestors)
      if (Array.isArray(value)) {
        return value.map((item, index) => {
          const copy = plain(item, `${path}[${index}]`, ancestors)
          return copy === undefined ? null : copy
        })
      }
      const prototype = Object.getPrototypeOf(value)
      if (prototype !== Object.prototype && prototype !== null) {
        throw new CodecError(
          `${path} is a ${className(value)}; SDK calls take plain data`,
        )
      }
      const copy = {}
      for (const key of Object.keys(value)) {
        const field = plain(value[key], `${path}.${key}`, ancestors)
        if (field !== undefined) {
          Object.defineProperty(copy, key, {
            value: field,
            enumerable: true,
            writable: true,
            configurable: true,
          })
        }
      }
      return copy
    } finally {
      ancestors.pop()
    }
  }

  // Always valid JSON. Input the codec refuses still reaches the door as the invalid brand, so
  // the call is recorded and refused there as script / invalid_input (dispatch/registry.rs).
  function encode(input) {
    try {
      return JSON.stringify(plain(input, 'input', []) ?? {})
    } catch (error) {
      const reason =
        error instanceof CodecError
          ? error.message
          : `the input could not be encoded: ${textOf(() => error.message)}`
      return JSON.stringify({ $neo: 'invalid', reason })
    }
  }

  // "run:LINE:COL" of the innermost frame in the agent's code, or "" for a call from elsewhere.
  // run_script.rs compiles the agent's function under the file name "run", with its body
  // starting on line 2, so LINE counts from the first line of the agent's code.
  function callSite() {
    const match = /[ (]run:(\d+):(\d+)/.exec(String(new Error().stack))
    return match === null ? '' : `run:${Number(match[1]) - 1}:${match[2]}`
  }

  const replyListeners = []

  async function invoke(name, input) {
    // Before the first await, while the agent's frame is still on the stack.
    const site = callSite()
    const envelope = JSON.parse(
      await nativeInvoke(String(name), encode(input), site),
    )
    for (const listener of replyListeners) listener(envelope)
    if (envelope.err !== undefined) {
      const error = envelope.err.timeout
        ? new TimeoutError(envelope.err)
        : new NeoError(envelope.err)
      genuine.set(error, envelope.err.operationId)
      throw error
    }
    return revive(envelope.ok)
  }

  function textOf(read) {
    try {
      const text = read()
      return text === undefined || text === null ? '' : String(text)
    } catch {
      return '[unprintable]'
    }
  }

  // JSON-safe conversion of a value the agent returns or logs: bigints as strings, functions and
  // symbols dropped, bytes as their brand, a cycle as "[Circular]". JSON.stringify applies toJSON
  // (handles) before this sees a value.
  function jsonSafe() {
    const holders = []
    return function (_key, value) {
      if (typeof value === 'bigint') return value.toString()
      if (typeof value === 'function' || typeof value === 'symbol')
        return undefined
      if (value instanceof Uint8Array)
        return { $neo: 'bytes', base64: toBase64(value) }
      if (value !== null && typeof value === 'object') {
        while (holders.length > 0 && holders[holders.length - 1] !== this)
          holders.pop()
        if (holders.includes(value)) return '[Circular]'
        holders.push(value)
      }
      return value
    }
  }

  function format(args) {
    return args
      .map((arg) => {
        if (typeof arg === 'string') return arg
        if (arg instanceof Error)
          return `${textOf(() => arg.name)}: ${textOf(() => arg.message)}`
        try {
          return JSON.stringify(arg, jsonSafe()) ?? textOf(() => arg)
        } catch {
          return textOf(() => arg)
        }
      })
      .join(' ')
  }

  const console = Object.freeze({
    log: (...args) => nativeLog('log', format(args)),
    info: (...args) => nativeLog('info', format(args)),
    warn: (...args) => nativeLog('warn', format(args)),
    error: (...args) => nativeLog('error', format(args)),
    debug: (...args) => nativeLog('debug', format(args)),
  })

  // Timers ride on __neo_sleep, which the run's deadline caps and its stop cuts short, so a
  // pending timer never outlives the run.
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

  // Parameter name -> (init) => value: the agent's API. Later preludes replace earlier names
  // (sdk-playwright's `page` and `context` replace the page prelude's).
  const provided = new Map([
    ['console', () => console],
    ['setTimeout', () => setTimeout],
    ['clearTimeout', () => clearTimeout],
    ['NeoError', () => NeoError],
    ['TimeoutError', () => TimeoutError],
  ])

  function clip(text) {
    return text.length > STACK_CHARS ? text.slice(0, STACK_CHARS) : text
  }

  function settleThrown(error) {
    const operationId =
      error !== null && typeof error === 'object'
        ? genuine.get(error)
        : undefined
    if (operationId !== undefined) {
      return JSON.stringify({ thrown: { kind: 'operation', operationId } })
    }
    const isError = error instanceof Error
    return JSON.stringify({
      thrown: {
        kind: 'error',
        name: isError ? textOf(() => error.name) : '',
        message: textOf(() => (isError ? error.message : error)),
        stack: isError ? clip(textOf(() => error.stack)) : '',
      },
    })
  }

  // Called once per run by run_script.rs with the agent's compiled function. Resolves to one
  // JSON string, {"ok": value} or {"thrown": …}: every throw is caught here, in JS, so only
  // QuickJS stopping the script (its interrupt, its heap) makes the promise reject.
  async function run(main, initJson) {
    try {
      const init = JSON.parse(initJson)
      const args = [...provided.values()].map((build) => build(init))
      const value = await main(...args)
      const json = JSON.stringify(
        value === undefined ? null : value,
        jsonSafe(),
      )
      return `{"ok":${json ?? 'null'}}`
    } catch (error) {
      return settleThrown(error)
    }
  }

  globalThis.__neo = Object.freeze({
    invoke,
    brands: Object.freeze({
      register(brand, fromJson) {
        brands.set(String(brand), fromJson)
      },
    }),
    onReply(listener) {
      replyListeners.push(listener)
    },
    provide(name, build) {
      provided.set(String(name), build)
    },
    // The agent function's parameter list, in the order `run` passes them.
    parameters() {
      return [...provided.keys()].join(', ')
    },
    run,
  })
})()
