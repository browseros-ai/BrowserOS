// One realm's cancellation interfaces. QuickJS supplies DOMException but no EventTarget
// or abort interfaces. Signals own their listeners here; only timeout uses the existing,
// run-bounded sleep native. No browser or dispatcher authority crosses this factory.
;(sleep) => {
  const signals = new WeakMap()
  const controllers = new WeakMap()
  const token = Symbol('AbortSignal')
  const Exception = DOMException

  function state(signal) {
    const value = signals.get(signal)
    if (!value) throw new TypeError('Illegal invocation')
    return value
  }

  function abort(signal, reason) {
    const value = state(signal)
    if (value.aborted) return
    value.aborted = true
    value.reason =
      reason === undefined
        ? new Exception('This operation was aborted', 'AbortError')
        : reason
    // Dependency propagation is an abort algorithm, not a public event listener. A
    // listener's stopImmediatePropagation must not keep combined signals alive.
    const dependents = [...value.dependents]
    value.dependents.clear()
    for (const source of value.sources) state(source).dependents.delete(signal)
    value.sources.clear()
    for (const dependent of dependents) abort(dependent, value.reason)
    // Set state before dispatch: callbacks can synchronously observe the reason or
    // abort a dependent signal. A second abort never changes either state or listeners.
    dispatch.call(signal, { type: 'abort' })
  }

  class AbortSignal {
    constructor(...args) {
      if (args[0] !== token) throw new TypeError('Illegal constructor')
      signals.set(this, {
        aborted: false,
        reason: undefined,
        listeners: [],
        onabort: null,
        dependents: new Set(),
        sources: new Set(),
      })
    }

    get aborted() {
      return state(this).aborted
    }
    get reason() {
      return state(this).reason
    }
    get onabort() {
      return state(this).onabort?.callback ?? null
    }
    set onabort(callback) {
      const value = state(this)
      if (typeof callback !== 'function') {
        if (value.onabort) value.onabort.removed = true
        value.onabort = null
      } else if (value.onabort) {
        value.onabort.callback = callback
      } else {
        value.onabort = {
          type: 'abort',
          callback,
          capture: false,
          once: false,
          removed: false,
        }
        value.listeners.push(value.onabort)
      }
    }

    throwIfAborted() {
      const value = state(this)
      if (value.aborted) throw value.reason
    }

    addEventListener(type, callback, options = undefined) {
      const value = state(this)
      if (callback === null || callback === undefined) return
      if (typeof callback !== 'function' && typeof callback !== 'object')
        throw new TypeError('Event listener must be a function or an object')
      type = String(type)
      const capture =
        typeof options === 'boolean' ? options : !!options?.capture
      if (
        value.listeners.some(
          (entry) =>
            !entry.removed &&
            entry.type === type &&
            entry.callback === callback &&
            entry.capture === capture,
        )
      )
        return
      const signal = typeof options === 'object' ? options?.signal : undefined
      if (signal?.aborted) return
      const entry = {
        type,
        callback,
        capture,
        once: !!options?.once,
        removed: false,
      }
      value.listeners.push(entry)
      if (signal) {
        const remove = () => this.removeEventListener(type, callback, capture)
        signal.addEventListener('abort', remove, { once: true })
        entry.cleanup = () => signal.removeEventListener('abort', remove)
      }
    }

    removeEventListener(type, callback, options = undefined) {
      const value = state(this)
      const capture =
        typeof options === 'boolean' ? options : !!options?.capture
      for (const entry of value.listeners) {
        if (
          !entry.removed &&
          entry.type === String(type) &&
          entry.callback === callback &&
          entry.capture === capture
        ) {
          entry.removed = true
          entry.cleanup?.()
        }
      }
      value.listeners = value.listeners.filter((entry) => !entry.removed)
    }

    dispatchEvent(event) {
      const value = state(this)
      if (!event || typeof event.type !== 'string')
        throw new TypeError('Invalid event')
      let stopped = false
      const delivered = {
        ...event,
        target: this,
        currentTarget: this,
        bubbles: false,
        cancelable: false,
        defaultPrevented: false,
        isTrusted: false,
        stopImmediatePropagation() {
          stopped = true
        },
        stopPropagation() {},
        preventDefault() {},
      }
      for (const entry of [...value.listeners]) {
        if (stopped) break
        if (entry.removed || entry.type !== event.type) continue
        if (entry.once)
          this.removeEventListener(entry.type, entry.callback, entry.capture)
        try {
          if (typeof entry.callback === 'function')
            entry.callback.call(this, delivered)
          else entry.callback.handleEvent?.(delivered)
        } catch (error) {
          // Listener exceptions surface asynchronously, without preventing other listeners
          // (including pending SDK waits) from observing cancellation.
          void Promise.reject(error)
        }
      }
      return true
    }

    static abort(reason = undefined) {
      const signal = new AbortSignal(token)
      abort(signal, reason)
      return signal
    }

    static timeout(milliseconds) {
      const delay = Math.trunc(Number(milliseconds))
      if (
        !Number.isFinite(delay) ||
        delay < 0 ||
        delay > Number.MAX_SAFE_INTEGER
      )
        throw new RangeError('Timeout must be a non-negative safe integer')
      const signal = new AbortSignal(token)
      void sleep(delay).then(() =>
        abort(signal, new Exception('The operation timed out', 'TimeoutError')),
      )
      return signal
    }

    static any(sources) {
      const list = Array.from(sources, (source) => {
        state(source)
        return source
      })
      const signal = new AbortSignal(token)
      for (const source of list) {
        if (source.aborted) {
          abort(signal, source.reason)
          break
        }
        const dependents = state(source).dependents
        dependents.add(signal)
        state(signal).sources.add(source)
      }
      return signal
    }
  }

  const dispatch = AbortSignal.prototype.dispatchEvent

  class AbortController {
    constructor() {
      controllers.set(this, new AbortSignal(token))
    }
    get signal() {
      const signal = controllers.get(this)
      if (!signal) throw new TypeError('Illegal invocation')
      return signal
    }
    abort(reason = undefined) {
      const signal = controllers.get(this)
      if (!signal) throw new TypeError('Illegal invocation')
      abort(signal, reason)
    }
  }

  for (const [name, value] of [
    ['AbortController', AbortController],
    ['AbortSignal', AbortSignal],
  ]) {
    Object.defineProperty(value.prototype, Symbol.toStringTag, {
      value: name,
      configurable: true,
    })
    Object.defineProperty(globalThis, name, {
      value,
      writable: true,
      configurable: true,
    })
  }
}
