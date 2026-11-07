/** Host-side views keep native object identity and log results without emulating browser APIs. */
import { Worker } from 'node:worker_threads'
import { codecFactory } from './codec.mjs'

export const capabilityName = (family, member) =>
  member === '_expect' ? 'Frame.expect' : `${family}.${member}`
const errorFrom = (value, stack) => {
  const error = Object.assign(new Error(value.message), { name: value.name })
  if (typeof stack === 'string') error.stack = stack
  return error
}
const ignored = new Set(['then', 'toJSON', 'inspect'])
const nativeReferences = new WeakMap()
const families = new Set([
  'Page',
  'Frame',
  'Locator',
  'BrowserContext',
  'Browser',
  'BrowserType',
  'Response',
  'Request',
  'Keyboard',
  'Mouse',
  'ElementHandle',
  'JSHandle',
  'APIRequestContext',
  'CDPSession',
  'Route',
  'Dialog',
  'Download',
  'Worker',
  'WebSocket',
])

/** Owns blocking IPC to one native run and correlates asynchronous promise settlements.
 * Interned proxies preserve the actual QuickJS objects without implementing their APIs here. */
export class NativeProxy {
  constructor(options) {
    this.calls = []
    this.missing = new Set()
    this.transportErrors = []
    this.pending = new Map()
    this.proxies = new Map()
    this.refs = new WeakMap()
    this.ordinal = 0
    this.codec = codecFactory({
      identify: (value) => {
        const known = nativeReferences.get(value)
        if (known && known.owner !== this)
          throw this.transport('Foreign native proxy operand')
        return known?.ref
      },
      resolve: (ref) => this.object(ref),
    })
    this.worker = new Worker(new URL('./io-worker.mjs', import.meta.url), {
      workerData: options,
    })
    this.ready = new Promise((resolve, reject) => {
      this.onReady = resolve
      this.onReadyError = reject
    })
    this.outcome = new Promise((resolve) => {
      this.onOutcome = resolve
    })
    this.worker.on('message', (frame) => this.event(frame))
    this.worker.on('error', (error) => this.fail(String(error)))
    this.worker.on('exit', (code) => {
      if (!this.finished) this.fail(`I/O worker exited ${code}`)
    })
  }

  fail(message) {
    this.transportErrors.push(message)
    this.onReadyError(new Error(message))
    for (const pending of this.pending.values())
      pending.reject(new Error(message))
    this.pending.clear()
  }

  event(frame) {
    if (frame.event === 'ready') {
      this.epoch = frame.roots.page.ref.epoch
      this.capabilities = frame.capabilities
      this.page = this.codec.decode(frame.roots.page)
      this.context = this.codec.decode(frame.roots.context)
      this.onReady()
    } else if (frame.event === 'settled') {
      const pending = this.pending.get(frame.id)
      if (!pending)
        return this.fail(`Uncorrelated promise settlement ${frame.id}`)
      this.pending.delete(frame.id)
      this.settle(pending.record, frame)
      if (frame.ok) pending.resolve(this.codec.decode(frame.value))
      else pending.reject(this.thrown(frame))
    } else if (frame.event === 'outcome') this.onOutcome(frame.outcome)
    else if (frame.event === 'transportError' && !this.finished)
      this.fail(frame.message)
  }

  request(frame) {
    const shared = new SharedArrayBuffer(8 + 16 * 1024 * 1024)
    frame.id = ++this.ordinal
    this.worker.postMessage({ frame, shared })
    const header = new Int32Array(shared, 0, 2)
    // A dedicated worker delivers this reply. The test thread cannot service its own
    // event loop while a synchronous locator builder or getter waits here.
    if (Atomics.wait(header, 0, 0, 60_000) === 'timed-out')
      throw this.transport('Native IPC timed out')
    const length = Atomics.load(header, 1)
    if (length < 0) throw this.transport('Native frame exceeded 16 MiB')
    const response = JSON.parse(
      new TextDecoder().decode(new Uint8Array(shared, 8, length)),
    )
    if (response.kind === 'transportError')
      throw this.transport(response.message)
    return response
  }

  transport(message) {
    this.transportErrors.push(message)
    return Object.assign(new Error(message), { name: 'HarnessTransportError' })
  }

  unsupported(family, member) {
    if (member.startsWith('_') && member !== '_expect')
      return this.transport(
        `Playwright private member ${family}.${member} has no native counterpart`,
      )
    const method = capabilityName(family, member)
    this.missing.add(method)
    return Object.assign(new Error(`Neo does not serve ${method}`), {
      name: 'UnsupportedMethod',
    })
  }

  absent(family) {
    return new Proxy(
      {},
      {
        get: (_target, member) => {
          if (typeof member === 'symbol' || ignored.has(member))
            return undefined
          if (member === '_apiName') return family
          throw this.unsupported(family, member)
        },
      },
    )
  }

  object(ref) {
    if (
      ref.epoch !== this.epoch ||
      !Number.isSafeInteger(ref.id) ||
      ref.id < 1 ||
      !families.has(ref.family)
    )
      throw this.transport('Invalid or foreign native object reference')
    const key = `${ref.epoch}/${ref.id}/${ref.family}`
    if (this.proxies.has(key)) return this.proxies.get(key)
    const methods = new Map()
    const proxy = new Proxy(
      {},
      {
        has: (_target, member) => {
          if (typeof member === 'symbol')
            throw this.transport('Native symbol reflection is not transported')
          const found = this.request({
            op: 'has',
            object: ref,
            member: this.codec.units(member),
          })
          if (found.kind !== 'value')
            throw this.transport('Invalid native presence reply')
          return this.codec.decode(found.value)
        },
        ownKeys: () => {
          throw this.transport('Native object enumeration is not transported')
        },
        getPrototypeOf: () => {
          throw this.transport('Native prototype reflection is not transported')
        },
        set: () => {
          throw this.transport('Native object assignment is not transported')
        },
        defineProperty: () => {
          throw this.transport('Native property definition is not transported')
        },
        deleteProperty: () => {
          throw this.transport('Native property deletion is not transported')
        },
        setPrototypeOf: () => {
          throw this.transport('Native prototype mutation is not transported')
        },
        preventExtensions: () => {
          throw this.transport('Native object sealing is not transported')
        },
        get: (_target, member) => {
          if (typeof member === 'symbol' || ignored.has(member))
            return undefined
          // This is Playwright matcher's receiver label, not a browser operation. Missing
          // _expect still fails through actual native reflection and ranks as Frame.expect.
          if (member === '_apiName') return ref.family
          if (methods.has(member)) return methods.get(member)
          const found = this.request({
            op: 'get',
            object: ref,
            member: this.codec.units(member),
          })
          if (found.kind === 'missing')
            throw this.unsupported(ref.family, member)
          if (found.kind === 'throw') throw this.thrown(found)
          if (found.kind === 'value') return this.codec.decode(found.value)
          if (found.kind !== 'callable')
            throw this.transport(`Invalid reflection reply ${found.kind}`)
          const method = (...args) => {
            const record = {
              method: capabilityName(ref.family, member),
              ordinal: this.calls.length,
              state: 'pending',
            }
            this.calls.push(record)
            let encoded
            try {
              encoded = this.codec.encode(args)
            } catch (error) {
              throw this.transport(`${record.method}: ${error.message}`)
            }
            const result = this.request({
              op: 'invoke',
              object: ref,
              member: this.codec.units(member),
              args: encoded,
            })
            if (result.kind === 'promise')
              return new Promise((resolve, reject) =>
                this.pending.set(result.id, { resolve, reject, record }),
              )
            if (result.kind === 'throw') {
              this.settle(record, { ok: false, error: result.error })
              throw this.thrown(result)
            }
            if (result.kind !== 'value')
              throw this.transport(`Invalid call reply ${result.kind}`)
            this.settle(record, { ok: true, value: result.value })
            return this.codec.decode(result.value)
          }
          methods.set(member, method)
          return method
        },
      },
    )
    this.refs.set(proxy, ref)
    nativeReferences.set(proxy, { owner: this, ref })
    this.proxies.set(key, proxy)
    return proxy
  }

  settle(record, result) {
    record.state = result.ok ? 'returned' : 'threw'
    record[result.ok ? 'value' : 'error'] = result.ok
      ? result.value
      : result.error
    if (!result.ok && result.errorStack) record.errorStack = result.errorStack
  }

  thrown(frame) {
    return errorFrom(
      this.codec.decode(frame.error),
      frame.errorStack && this.codec.decode(frame.errorStack),
    )
  }

  async finish() {
    this.request({ op: 'finish' })
    this.finished = true
    const outcome = await Promise.race([
      this.outcome,
      new Promise((_, reject) =>
        setTimeout(
          () => reject(new Error('Native finish timed out')),
          5000,
        ).unref(),
      ),
    ])
    this.worker.postMessage({ stop: true })
    await this.worker.terminate()
    return outcome
  }

  async abort() {
    this.finished = true
    this.worker.postMessage({ stop: true })
    await this.worker.terminate()
  }
}

/** Records outer calls delegated to real Playwright and wraps returned browser objects.
 * Its identity maps preserve aliases while method bodies run on their actual receivers. */
export class ReferenceProxy {
  constructor({ connection, page }) {
    this.calls = []
    this.requirements = new Set()
    this.proxies = new WeakMap()
    this.actual = new WeakMap()
    this.refs = new WeakMap()
    this.count = 0
    this.callbacks = new WeakMap()
    this.connection = connection
    this.channelPrototype = Object.getPrototypeOf(Object.getPrototypeOf(page))
    this.prototypes = new Map([
      [
        Object.getPrototypeOf(page.locator('__neo_capability_probe__')),
        'Locator',
      ],
      [Object.getPrototypeOf(page.keyboard), 'Keyboard'],
      [Object.getPrototypeOf(page.mouse), 'Mouse'],
    ])
    this.codec = codecFactory({
      identify: (value) => this.identify(value),
      resolve: () => {
        throw new Error('Reference decode is unused')
      },
    })
  }

  identify(value) {
    value = this.actual.get(value) || value
    if (!value || typeof value !== 'object') return undefined
    if (this.refs.has(value)) return this.refs.get(value)
    // The upstream build bundles client/server classes and renames constructors.
    // ChannelOwner's protocol type and matcher's label are the pinned identity seams.
    let family = this.prototypes.get(Object.getPrototypeOf(value))
    if (!family) {
      let prototype = Object.getPrototypeOf(value)
      while (prototype && prototype !== this.channelPrototype)
        prototype = Object.getPrototypeOf(prototype)
      if (!prototype || this.connection._objects.get(value._guid) !== value)
        return undefined
      family = value._type
    }
    if (!families.has(family)) return undefined
    if (!this.refs.has(value))
      this.refs.set(value, { epoch: 'reference', id: ++this.count, family })
    return this.refs.get(value)
  }

  unwrap(value) {
    if (!value || typeof value !== 'object') return value
    if (this.actual.has(value)) return this.actual.get(value)
    if (Array.isArray(value)) return value.map((item) => this.unwrap(item))
    if (Object.getPrototypeOf(value) === Object.prototype)
      return Object.fromEntries(
        Object.entries(value).map(([key, item]) => [key, this.unwrap(item)]),
      )
    return value
  }

  wrap(value) {
    if (Array.isArray(value)) return value.map((item) => this.wrap(item))
    const ref = this.identify(value)
    if (!ref) return value
    if (this.proxies.has(value)) return this.proxies.get(value)
    const methods = new Map()
    const proxy = new Proxy(value, {
      get: (target, member) => {
        const result = Reflect.get(target, member, target)
        if (typeof member !== 'string' || ignored.has(member)) return result
        if (typeof result !== 'function') return this.wrap(result)
        if (member === 'constructor') return result
        if (!methods.has(member))
          methods.set(member, (...args) => {
            const method = capabilityName(ref.family, member)
            this.requirements.add(method)
            const record = {
              method,
              ordinal: this.calls.length,
              state: 'pending',
            }
            this.calls.push(record)
            const settle = (ok, returned) => {
              record.state = ok ? 'returned' : 'threw'
              try {
                record[ok ? 'value' : 'error'] = this.codec.encode(
                  ok
                    ? returned
                    : {
                        name: String(returned?.name || 'Error'),
                        message: String(returned?.message ?? returned),
                      },
                )
              } catch (error) {
                record.state = 'untransportable'
                record.transportError = error.message
              }
              if (!ok) throw returned
              return this.wrap(returned)
            }
            const callbacks = [
              'on',
              'once',
              'off',
              'removeListener',
              'addListener',
              'waitForEvent',
              'route',
              'unroute',
            ]
            const actualArgs = args.map((arg) => {
              if (typeof arg !== 'function' || !callbacks.includes(member))
                return this.unwrap(arg)
              if (!this.callbacks.has(arg))
                this.callbacks.set(arg, (...values) =>
                  arg(...values.map((value) => this.wrap(value))),
                )
              return this.callbacks.get(arg)
            })
            let returned
            try {
              returned = Reflect.apply(result, target, actualArgs)
            } catch (error) {
              return settle(false, error)
            }
            return returned && typeof returned.then === 'function'
              ? returned.then(
                  (value) => settle(true, value),
                  (error) => settle(false, error),
                )
              : settle(true, returned)
          })
        return methods.get(member)
      },
    })
    this.proxies.set(value, proxy)
    this.actual.set(proxy, value)
    return proxy
  }
}
