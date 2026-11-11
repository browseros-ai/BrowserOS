// URL, URLSearchParams and QuickJS-ng's checked atob/btoa are realm globals, installed
// before this bootstrap. Keep them out of the agent function's parameters so scripts can shadow them.
// claw-sdk's bootstrap, evaluated first in every run's fresh QuickJS realm (run_script.rs).
//
// The bridge's three natives take and return strings only. This file turns them into the agent
// API's plumbing: NeoError and TimeoutError, the input codec, reply envelopes, call sites,
// handle brands, a prelude's refusals as door calls, aborts of calls in flight, console, timers,
// and the run itself. After it and the preludes run, the realm's globals are QuickJS's own plus
// __neo_invoke, __neo_log, __neo_sleep and __neo (pinned by runtime/tests.rs): the agent's API
// reaches its code as the parameters of its function, never as globals.

;(() => {
  const nativeInvoke = globalThis.__neo_invoke // (name, inputJson, site) => Promise<string>, never rejects
  const nativeLog = globalThis.__neo_log // (level, text) => undefined
  const nativeSleep = globalThis.__neo_sleep // (ms) => Promise<undefined>, cut short by the run's stop

  // How much of a thrown error's stack crosses to Rust, which reads only the agent's first frame
  // from it. The name and message cross whole: Rust masks known secrets before it clips them,
  // since a clip through a secret would leave a prefix no mask recognizes.
  const STACK_CHARS = 4096

  // Every NeoError this file built, with the op it came from: its id, or, for a refusal whose
  // door call has not answered yet, a promise of the id (`refuse`). An error the script made or
  // edited itself is not here, so it can never pass for an op's failure as the run's error: Rust
  // looks the op up and reports the failure the door decided.
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

  // QuickJS-ng's own `Uint8Array.prototype.toBase64` (TC39's), taken before the agent's code
  // runs. A JS encoder that kept one string per three bytes held about 50 bytes of heap per three
  // bytes encoded, so returning a multi-megabyte screenshot ran the realm's 64 MiB heap out
  // before the reply's size was ever checked.
  const encodeBase64 = Uint8Array.prototype.toBase64

  function toBase64(bytes) {
    return encodeBase64.call(bytes)
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

  // "$neo" brand -> reviver, for handles inside results. Preludes add their own, such as the
  // Playwright prelude's "page" and "element".
  const brands = new Map([['bytes', (json) => fromBase64(json.base64)]])

  // Revives every brand in a reply's value, in place. It walks with a stack of its own, never the
  // run's: neo-playwright carries results 297 JSON levels deep, and recursion on the run's
  // 512 KiB stack overflowed past about 50 in a debug build. The walk keeps recursion's order,
  // depth first and in each object's key order, because the page and element revivers make a
  // Page for each tab they meet, so that order is context.pages()'s. A brand goes to its reviver
  // whole, and what comes back takes its place. Every object here came from JSON.parse, so a
  // "__proto__" key is an own data property and assigning to it never reaches the prototype.
  function revive(value) {
    const root = { value }
    // One frame per object on the path walked so far: its keys as they were when it was reached,
    // and the next one to visit.
    const frames = [{ holder: root, keys: ['value'], next: 0 }]
    while (frames.length > 0) {
      const frame = frames[frames.length - 1]
      if (frame.next === frame.keys.length) {
        frames.pop()
        continue
      }
      const key = frame.keys[frame.next++]
      const item = frame.holder[key]
      if (item === null || typeof item !== 'object') continue
      if (typeof item.$neo === 'string' && brands.has(item.$neo)) {
        frame.holder[key] = brands.get(item.$neo)(item)
        continue
      }
      frames.push({ holder: item, keys: Object.keys(item), next: 0 })
    }
    return root.value
  }

  class CodecError extends Error {}

  function className(value) {
    try {
      return value.constructor?.name || 'class instance'
    } catch {
      return 'class instance'
    }
  }

  // `beginCopy`'s answer for an object or an array: its frame is on the stack and its children
  // come next, so what it becomes is known only once they are made plain.
  const OPENED = Symbol('opened')

  // Plain data only. A handle (or a Date) serializes through its toJSON, bytes become the bytes
  // brand, and a function, symbol, bigint, cycle or class instance is refused with its path. It
  // walks with a stack of its own, as `revive` does: recursion on the run's 512 KiB stack
  // overflowed past 17 nested arrays in a debug build, and a locator's parsed selector with
  // `:scope` inside `:has()` nests 28 levels. It reads each value when recursion read it, depth
  // first and in each object's key order, so a getter or a toJSON sees the same walk.
  function plain(input) {
    // One frame per object on the path down to the value being made plain, and those objects as a
    // set, recursion's ancestors: a cycle is one of them met again, while two fields may share an
    // object off the path. A set, so a deep input costs its depth and not its depth squared.
    const frames = []
    const ancestors = new Set()
    let copy = beginCopy(input, frames, ancestors)
    for (;;) {
      const frame = frames[frames.length - 1]
      if (copy !== OPENED) {
        if (frame === undefined) return copy
        keepChild(frame, copy)
      }
      if (hasChild(frame)) {
        copy = beginCopy(childOf(frame), frames, ancestors)
      } else {
        frames.pop()
        ancestors.delete(frame.value)
        copy = frame.copy
      }
    }
  }

  // `value` made plain when nothing below it needs a walk. An object or an array gets a frame on
  // `frames` and a place among `ancestors` instead, and the answer OPENED. Every frame counts its
  // children in `length` and stands at the one in `index`.
  function beginCopy(value, frames, ancestors) {
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
          `${pathOf(frames)} is a ${typeof value}; SDK calls take plain data`,
        )
    }
    if (value === null) return null
    if (ancestors.has(value)) {
      throw new CodecError(
        `${pathOf(frames)} refers back to itself; SDK calls take plain data`,
      )
    }
    if (value instanceof Uint8Array)
      return { $neo: 'bytes', base64: toBase64(value) }
    let frame
    if (typeof value.toJSON === 'function') {
      // Its one child is what toJSON answers, at its own path, and it becomes that.
      const answer = value.toJSON()
      frame = { kind: 'json', value, answer, length: 1, index: 0 }
    } else if (Array.isArray(value)) {
      const length = lengthOf(value)
      frame = {
        kind: 'array',
        value,
        length,
        index: 0,
        copy: new Array(length),
      }
    } else {
      const prototype = Object.getPrototypeOf(value)
      if (prototype !== Object.prototype && prototype !== null) {
        throw new CodecError(
          `${pathOf(frames)} is a ${className(value)}; SDK calls take plain data`,
        )
      }
      const keys = Object.keys(value)
      frame = {
        kind: 'object',
        value,
        keys,
        length: keys.length,
        index: 0,
        copy: {},
      }
    }
    frames.push(frame)
    ancestors.add(value)
    return OPENED
  }

  // An array's length read once, as `map` read it (ToLength): a Proxy around an array may answer
  // any value, and `new Array` would take a lone non-number as its one item, or throw on a
  // fraction. Unary plus, not `Number`, so a bigint length throws as it did in `map`.
  function lengthOf(array) {
    const length = Math.trunc(+array.length)
    return length > 0 ? Math.min(length, Number.MAX_SAFE_INTEGER) : 0
  }

  // Whether the frame has a child left to make plain. An array's hole is skipped, as `map` skips
  // it: the copy keeps the hole, which JSON writes as null.
  function hasChild(frame) {
    if (frame.kind === 'array') {
      while (frame.index < frame.length && !(frame.index in frame.value))
        frame.index++
    }
    return frame.index < frame.length
  }

  // The frame's current child, read now.
  function childOf(frame) {
    switch (frame.kind) {
      case 'json':
        return frame.answer
      case 'array':
        return frame.value[frame.index]
      case 'object':
        return frame.value[frame.keys[frame.index]]
    }
  }

  // Puts `copy`, the plain form of the frame's current child, into the frame's own copy, and
  // moves to the next child: an array's undefined item becomes null, and an object's undefined
  // field is left out.
  function keepChild(frame, copy) {
    switch (frame.kind) {
      case 'json':
        frame.copy = copy
        break
      case 'array':
        frame.copy[frame.index] = copy === undefined ? null : copy
        break
      case 'object':
        // Defined, not assigned: assigning a "__proto__" key would set the copy's prototype.
        if (copy !== undefined) {
          Object.defineProperty(frame.copy, frame.keys[frame.index], {
            value: copy,
            enumerable: true,
            writable: true,
            configurable: true,
          })
        }
        break
    }
    frame.index++
  }

  // Where the value being made plain sits in the input, as a refusal names it: `input`, then a
  // `.key` or `[index]` for each object or array on the way down, while a toJSON keeps its
  // holder's path. Built only for a refusal: a path held for every level at once would cost a
  // deep input its depth squared.
  function pathOf(frames) {
    let path = 'input'
    for (const frame of frames) {
      if (frame.kind === 'array') path += `[${frame.index}]`
      if (frame.kind === 'object') path += `.${frame.keys[frame.index]}`
    }
    return path
  }

  // Always valid JSON. Input the codec refuses still reaches the door as the invalid brand, so
  // the call is recorded and refused there as script / invalid_input (dispatch/registry.rs).
  function encode(input) {
    try {
      return JSON.stringify(plain(input) ?? {})
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

  // An abortable call (`options.signal`, an AbortSignal) is keyed by the realm, which learns its
  // op id only from the reply: the bridge registers the key when the call is submitted, and
  // `$neo.abort` names the op by it (runtime/bridge.rs). The op fails as its own timeout would,
  // with code `aborted`, and the Playwright prelude names that error `AbortError`.
  let nextAbortKey = 1

  function abortOp(key, signal) {
    const reason = textOf(() => {
      const given = signal.reason
      return given instanceof Error ? given.message : String(given)
    })
    // Not awaited: the bridge answers an abort at once, and the native never rejects.
    nativeInvoke('$neo.abort', JSON.stringify({ key, reason }), '')
  }

  // Calls `abort` when `signal` fires, or at once if it already has; returns the detach.
  function whenAborted(signal, abort) {
    if (signal.aborted) {
      abort()
      return () => {}
    }
    signal.addEventListener('abort', abort, { once: true })
    return () => signal.removeEventListener('abort', abort)
  }

  const replyListeners = []
  async function invoke(name, input, options) {
    // Before the first await, while the agent's frame is still on the stack.
    const site = callSite()
    const signal = options?.signal
    const key = signal === undefined ? undefined : String(nextAbortKey++)
    const encoded = encode(input)
    // Submitted now, so the bridge knows the key before the signal can fire.
    const answer = nativeInvoke(
      String(name),
      key === undefined
        ? encoded
        : `{"$neo":"abortable","key":"${key}","input":${encoded}}`,
      site,
    )
    const detach =
      key === undefined
        ? undefined
        : whenAborted(signal, () => abortOp(key, signal))
    let text
    try {
      text = await answer
    } finally {
      detach?.()
    }
    const envelope = JSON.parse(text)
    for (const listener of replyListeners) listener(envelope)
    if (envelope.err !== undefined) {
      const error = envelope.err.timeout
        ? new TimeoutError(envelope.err)
        : new NeoError(envelope.err)
      genuine.set(error, envelope.err.operationId)
      throw error
    }
    const value = revive(envelope.ok)
    // A consumer may reserve ordered delivery before any awaiting continuation runs.
    options?.onResult?.(value, envelope.observed ?? null)
    return options?.observed === true
      ? { value, observed: envelope.observed ?? null }
      : value
  }

  // Refusals whose answer has not reached the realm yet. The door records a refusal at the call
  // (run_script.rs `install`), but the realm learns its op id only once `nativeInvoke` resolves,
  // which takes a yield to Rust. `run` waits on this set before serializing the value, so a
  // refusal the script returns carries its op id; a thrown one waits on its own id in
  // `settleThrown`.
  const refusing = new Set()

  /**
   * A prelude's refusal as a door call: `throw __neo.refuse('page.route', 'method_unavailable',
   * 'page.route: network interception is not available in Neo')`. Synchronous, so a getter or a
   * sync method can throw what it returns. The door records the call under `api` and refuses it
   * (dispatch/refusal_brand.rs); its answer then sets the error's code, owner, delivery and op,
   * which the door decides and the prelude only proposes. The message stays `reason`, the
   * realm's own text.
   */
  function refuse(api, code, reason) {
    // Before anything async, while the agent's frame is still on the stack.
    const site = callSite()
    const error = new NeoError({
      code,
      owner: 'script',
      delivery: 'not_sent',
      retryable: true,
      operationId: undefined,
      message: reason,
    })
    const brand = JSON.stringify({
      $neo: 'refused',
      code: String(code),
      reason: String(reason),
    })
    // Never rejects: `settleThrown` awaits it, and a rejection there would end the run as Neo's
    // internal error. An answer the realm cannot take, a reply listener's throw included, leaves
    // the op id unset, so the error settles as the script's own throw, never as a claim on an op.
    const recorded = nativeInvoke(String(api), brand, site).then(
      (text) => {
        try {
          const envelope = JSON.parse(text)
          for (const listener of replyListeners) listener(envelope)
          const decided = envelope.err
          if (decided === undefined) return undefined
          error.code = decided.code
          error.owner = decided.owner
          error.delivery = decided.delivery
          error.retryable = decided.retryable
          error.operationId = decided.operationId
          return decided.operationId
        } catch {
          return undefined
        }
      },
      () => undefined,
    )
    genuine.set(error, recorded)
    refusing.add(recorded)
    recorded.then(() => refusing.delete(recorded))
    return error
  }

  // Resolves once the realm has the answer to every refusal raised so far, and to any raised
  // while it waits.
  async function allAnswered() {
    while (refusing.size > 0) await Promise.allSettled([...refusing])
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

  // Parameter name -> (init) => value: the agent's API, in the order the preludes provide it. A
  // later prelude that provides a name again replaces it in place.
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

  // Async because a refusal's op id is a promise until its door call answers.
  async function settleThrown(error) {
    const operationId =
      error !== null && typeof error === 'object'
        ? await genuine.get(error)
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
      // Each awaited, in order: a provider may build its parameter with an op before the agent's
      // first statement, as the Playwright prelude's `page` reads its frame tree.
      const args = []
      for (const build of provided.values()) args.push(await build(init))
      const value = await main(...args)
      // Before serializing, so a caught refusal returned as the value carries its op id.
      await allAnswered()
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
    refuse,
    // Local wait deadlines share the script's TimeoutError constructor. They have no
    // operation identity: only errors received from the dispatcher enter `genuine`.
    timeoutError(message) {
      return new TimeoutError({ message })
    },
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
