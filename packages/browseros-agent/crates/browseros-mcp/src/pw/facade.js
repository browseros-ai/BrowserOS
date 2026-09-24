;(() => {
  const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor

  // The runtime has no native timers; sleeps share the script wall-clock budget.
  globalThis.sleep = (ms) => __browserosSleep(Number(ms) || 0)
  globalThis.setTimeout = (fn, ms) => {
    __browserosSleep(Number(ms) || 0).then(() => {
      if (typeof fn === 'function') fn()
    })
    return 0
  }
  globalThis.clearTimeout = () => {}

  function safeStringify(value) {
    if (value === undefined) return 'undefined'
    try {
      const encoded = JSON.stringify(value, null, 2)
      return encoded ?? String(value)
    } catch {
      return String(value)
    }
  }

  function jsonSafeString(value) {
    const seen = new WeakSet()
    let encoded
    try {
      encoded = JSON.stringify(value, (_key, next) => {
        if (typeof next === 'bigint') return next.toString()
        if (typeof next === 'function' || typeof next === 'symbol') {
          return String(next)
        }
        if (typeof next === 'number' && !Number.isFinite(next)) return null
        if (typeof next === 'object' && next !== null) {
          if (seen.has(next)) return '[Circular]'
          seen.add(next)
        }
        return next
      })
    } catch {
      return JSON.stringify(safeStringify(value))
    }
    return encoded
  }

  // P3 replaces these facade objects; operations fail explicitly while the
  // shared runtime's logging, timers, and JSON marshalling remain usable.
  function stubObject(prefix) {
    return new Proxy(
      {},
      {
        get: (_target, name) => () => {
          throw new Error(`not implemented yet: ${prefix}.${String(name)}`)
        },
      },
    )
  }
  const browser = stubObject('browser')
  globalThis.browser = browser
  globalThis.context = stubObject('context')
  globalThis.page = stubObject('page')
  globalThis.neo = stubObject('neo')
  globalThis.expect = () => {
    throw new Error('not implemented yet: expect')
  }

  const sink =
    (level) =>
    (...parts) => {
      __browserosPushLog(
        `${level}${parts
          .map((part) =>
            typeof part === 'string' ? part : safeStringify(part),
          )
          .join(' ')}`,
      )
    }

  globalThis.__browserosBrowser = browser
  globalThis.__browserosConsole = {
    log: sink(''),
    info: sink(''),
    warn: sink('warn: '),
    error: sink('error: '),
    debug: sink(''),
  }
  globalThis.__browserosMakeRunFunction = (code) =>
    new AsyncFunction('browser', 'console', `"use strict";\n${code}`)
  globalThis.__browserosJsonSafeString = jsonSafeString
  globalThis.__browserosSafeStringify = safeStringify
})()
