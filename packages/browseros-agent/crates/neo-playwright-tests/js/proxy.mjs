/** Neo: Host-side views keep native object identity and log results without emulating browser APIs. */

import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { Worker } from 'node:worker_threads'
import { codecFactory } from './codec.mjs'

export const capabilityName = (family, member) =>
  member === '_expect' ? 'Frame.expect' : `${family}.${member}`
const builtinErrors = {
  Error,
  EvalError,
  RangeError,
  ReferenceError,
  SyntaxError,
  TypeError,
  URIError,
}
const errorFrom = (value, stack, errorTypes) => {
  const ErrorType = Object.hasOwn(errorTypes, value.name)
    ? errorTypes[value.name]
    : Object.hasOwn(builtinErrors, value.name)
      ? builtinErrors[value.name]
      : Error
  const error =
    value.name === 'AggregateError'
      ? new AggregateError([], value.message)
      : new ErrorType(value.message)
  error.name = value.name
  if (typeof stack === 'string') error.stack = stack
  return error
}
const ignored = new Set(['then', 'toJSON', 'inspect'])
const nativeReferences = new WeakMap()
// Neo: Only these public argument slots contain page-realm source; other functions
// require a host callback channel, which this conformance bridge does not provide.
const evaluationSources = new Map([
  ['evaluate', 0],
  ['evaluateHandle', 0],
  ['evaluateAll', 0],
  ['$eval', 1],
  ['$$eval', 1],
  ['waitForFunction', 0],
  ['addInitScript', 0],
])

// Only data descriptors are traversed. Reading option getters here would run user
// code before Playwright itself consumes it and could change validation behavior.
function mapData(value, transform, seen) {
  if (seen.has(value)) return seen.get(value)
  const copy = Array.isArray(value)
    ? []
    : Object.create(Object.getPrototypeOf(value))
  seen.set(value, copy)
  const descriptors = Object.getOwnPropertyDescriptors(value)
  let changed = false
  for (const key of Reflect.ownKeys(descriptors)) {
    const descriptor = descriptors[key]
    if (!Object.hasOwn(descriptor, 'value')) continue
    const mapped = transform(descriptor.value)
    changed ||= !Object.is(mapped, descriptor.value)
    descriptor.value = mapped
  }
  if (!changed) {
    seen.set(value, value)
    return value
  }
  return Object.defineProperties(copy, descriptors)
}

function sourceSite(stack) {
  const roots = [
    process.env.NEO_TESTS_OVERLAY,
    process.env.NEO_TESTS_PLAYWRIGHT_SOURCE,
  ]
    .filter(Boolean)
    .map((root) => path.resolve(root))
  for (const frame of stack.split('\n').slice(1)) {
    const match = frame.match(
      /(?:\(|at )((?:file:\/\/\/|\/).*):(\d+):(\d+)\)?$/,
    )
    if (!match) continue
    const file = match[1].startsWith('file:')
      ? fileURLToPath(match[1])
      : match[1]
    for (const root of roots) {
      let relative = path.relative(root, file).split(path.sep).join('/')
      // The overlay retains this original fixture verbatim under one declared alias.
      if (relative === 'tests/config/__neoOriginalBrowserTest.ts')
        relative = 'tests/config/browserTest.ts'
      if (relative.startsWith('tests/'))
        return {
          file: relative,
          line: Number(match[2]),
          column: Number(match[3]),
        }
    }
  }
  return null
}

/** Records the caller before control crosses into either engine. Occurrences belong to
 * a source site and receiver, so unrelated callback scheduling cannot pair their calls. */
class CallJournal {
  constructor() {
    this.calls = []
    this.censusGaps = []
    this.correlationGaps = []
    this.occurrences = new Map()
  }

  record(receiver, method) {
    // Matcher wrappers can put the original TypeScript caller below V8's default limit.
    // Restore the process setting before invoking any upstream code.
    const limit = Error.stackTraceLimit
    let sourceStack
    try {
      Error.stackTraceLimit = Math.max(limit || 0, 100)
      sourceStack = String(new Error().stack || '')
    } finally {
      Error.stackTraceLimit = limit
    }
    const site = sourceSite(sourceStack)
    const key = JSON.stringify([
      site,
      receiver.epoch,
      receiver.id,
      receiver.family,
    ])
    const occurrence = this.occurrences.get(key) || 0
    this.occurrences.set(key, occurrence + 1)
    const record = {
      method,
      receiver,
      site,
      occurrence,
      ordinal: this.calls.length,
      sourceStack,
      state: 'pending',
    }
    this.calls.push(record)
    if (!site)
      this.correlationGaps.push(
        `Missing upstream source site for ${method} call ${record.ordinal}`,
      )
    return record
  }
}

/** Owns blocking IPC to one native run and correlates asynchronous promise settlements.
 * Interned proxies preserve the actual QuickJS objects without implementing their APIs here. */
export class NativeProxy extends CallJournal {
  constructor(
    options,
    { errorTypes = {}, workerFactory, harnessLimitations = [] } = {},
  ) {
    super()
    // Constructors stay in the runner realm. WorkerData is a data-only protocol.
    this.errorTypes = errorTypes
    this.missing = new Set()
    this.transportErrors = []
    this.harnessLimitations = harnessLimitations
    this.pending = new Map()
    this.proxies = new Map()
    this.functions = new Map()
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
    this.worker = workerFactory
      ? workerFactory(options)
      : new Worker(new URL('./io-worker.mjs', import.meta.url), {
          workerData: options,
        })
    this.ready = new Promise((resolve, reject) => {
      this.onReady = resolve
      this.onReadyError = reject
    })
    this.outcome = new Promise((resolve) => {
      this.onOutcome = resolve
    })
    this.stopped = new Promise((resolve) => {
      this.onStopped = resolve
    })
    this.worker.on('message', (frame) => {
      try {
        this.event(frame)
      } catch (error) {
        this.fail(`Unreadable native event: ${error}`)
      } finally {
        this.worker.postMessage({ ack: 1 })
      }
    })
    this.worker.on('error', (error) => this.fail(String(error)))
    this.worker.on('exit', (code) => {
      if (!this.finished || code !== 0) this.fail(`I/O worker exited ${code}`)
    })
    const namespaces = new Map([
      ['chromium', this.absent('BrowserType')],
      ['firefox', this.absent('BrowserType')],
      ['webkit', this.absent('BrowserType')],
      ['selectors', this.absent('Selectors')],
      ['request', this.absent('APIRequest')],
    ])
    this.playwright = this.absent('Playwright', (member) => {
      if (member === 'errors') return { value: this.errorTypes }
      if (namespaces.has(member)) return { value: namespaces.get(member) }
    })
  }

  fail(message) {
    this.transportErrors.push(message)
    const error = Object.assign(new Error(message), {
      name: 'HarnessTransportError',
    })
    this.onReadyError(error)
    for (const pending of this.pending.values()) pending.reject(error)
    this.pending.clear()
  }

  event(frame) {
    if (frame.event === 'ready') {
      this.epoch = frame.roots.page.ref.epoch
      this.capabilities = frame.capabilities
      this.roots = frame.roots
      this.page = this.decode(frame.roots.page)
      this.context = this.decode(frame.roots.context)
      this.mainFrame = this.decode(frame.roots.mainFrame)
      this.onReady()
    } else if (frame.event === 'settled') {
      const pending = this.pending.get(frame.id)
      if (!pending)
        return this.fail(`Uncorrelated promise settlement ${frame.id}`)
      const value = frame.ok ? this.decode(frame.value) : this.thrown(frame)
      this.pending.delete(frame.id)
      this.settle(pending.record, frame)
      if (frame.ok) pending.resolve(value)
      else pending.reject(value)
    } else if (frame.event === 'outcome') this.onOutcome(frame.outcome)
    else if (frame.event === 'stopped') {
      this.transportErrors.push(...frame.infrastructureErrors)
      this.onStopped(frame)
    } else if (frame.event === 'transportError') {
      const pending = this.pending.get(frame.id)
      if (pending && frame.message.startsWith('bridge limit: ')) {
        this.pending.delete(frame.id)
        this.untransportable(pending.record, frame.message)
        pending.reject(this.transport(frame.message))
      } else this.fail(frame.message)
    }
  }

  request(frame) {
    if (this.finished) throw this.transport('Native run is finished')
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
    if (message.startsWith('bridge limit: ')) {
      this.harnessLimitations.push(message)
      return Object.assign(new Error(message), { name: 'HarnessLimitation' })
    }
    this.transportErrors.push(message)
    return Object.assign(new Error(message), { name: 'HarnessTransportError' })
  }

  decode(value) {
    try {
      return this.codec.decode(value)
    } catch (error) {
      if (error.name === 'HarnessTransportError') throw error
      throw this.transport(`Invalid native value: ${error.message}`)
    }
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

  absent(family, property) {
    return new Proxy(
      {},
      {
        has: () => {
          throw this.transport('Native namespace reflection is not transported')
        },
        ownKeys: () => {
          throw this.transport(
            'Native namespace enumeration is not transported',
          )
        },
        getOwnPropertyDescriptor: () => {
          throw this.transport(
            'Native namespace descriptors are not transported',
          )
        },
        isExtensible: () => {
          throw this.transport(
            'Native namespace extensibility reflection is not transported',
          )
        },
        getPrototypeOf: () => {
          throw this.transport(
            'Native namespace prototype reflection is not transported',
          )
        },
        set: () => {
          throw this.transport('Native namespace assignment is not transported')
        },
        defineProperty: () => {
          throw this.transport('Native namespace definition is not transported')
        },
        deleteProperty: () => {
          throw this.transport('Native namespace deletion is not transported')
        },
        setPrototypeOf: () => {
          throw this.transport(
            'Native namespace prototype mutation is not transported',
          )
        },
        preventExtensions: () => {
          throw this.transport('Native namespace sealing is not transported')
        },
        get: (_target, member) => {
          if (typeof member === 'symbol' || ignored.has(member))
            return undefined
          if (member === '_apiName') return family
          const found = property?.(member)
          if (found) return found.value
          throw this.unsupported(family, member)
        },
      },
    )
  }

  object(ref) {
    if (
      !ref ||
      ref.epoch !== this.epoch ||
      !Number.isSafeInteger(ref.id) ||
      ref.id < 1 ||
      !Object.hasOwn(this.capabilities, ref.family)
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
          return this.decode(found.value)
        },
        ownKeys: () => {
          throw this.transport('Native object enumeration is not transported')
        },
        getOwnPropertyDescriptor: () => {
          throw this.transport(
            'Native property descriptors are not transported',
          )
        },
        isExtensible: () => {
          throw this.transport(
            'Native extensibility reflection is not transported',
          )
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
          if (found.kind === 'value') return this.decode(found.value)
          if (found.kind !== 'callable')
            throw this.transport(`Invalid reflection reply ${found.kind}`)
          if (
            !found.function ||
            !Number.isSafeInteger(found.function.id) ||
            found.function.id < 1 ||
            !Number.isSafeInteger(found.function.length) ||
            found.function.length < 0
          )
            throw this.transport('Invalid native callable reference')
          if (this.functions.has(found.function.id)) {
            const method = this.functions.get(found.function.id)
            methods.set(member, method)
            return method
          }
          const membrane = this
          const method = function (...args) {
            const receiver = nativeReferences.get(this)
            if (!receiver || receiver.owner !== membrane)
              throw membrane.transport(
                'Native method receiver is not an owned native object',
              )
            const record = membrane.record(
              receiver.ref,
              capabilityName(ref.family, member),
            )
            let encoded
            try {
              const sourceIndex = evaluationSources.get(member)
              const callbackLimit =
                sourceIndex === undefined
                  ? `bridge limit: host callback ${record.method}`
                  : `bridge limit: untransportable function argument to ${record.method}`
              args.forEach((value, index) => {
                membrane.checkOperands(
                  value,
                  callbackLimit,
                  index === sourceIndex,
                )
              })
              encoded = membrane.codec.encode(args)
            } catch (error) {
              const limitation =
                error.name === 'HarnessLimitation'
                  ? error
                  : membrane.transport(
                      `bridge limit: untransportable argument ${error.message.replace(/^Untransportable /, '')} to ${record.method}`,
                    )
              membrane.untransportable(record, limitation.message)
              throw limitation
            }
            let result
            try {
              result = membrane.request({
                op: 'invoke',
                object: receiver.ref,
                function: found.function.id,
                member: membrane.codec.units(member),
                args: encoded,
              })
            } catch (error) {
              membrane.untransportable(record, error.message)
              throw error
            }
            if (result.kind === 'promise')
              return new Promise((resolve, reject) =>
                membrane.pending.set(result.id, { resolve, reject, record }),
              )
            if (result.kind === 'throw') {
              membrane.settle(record, { ...result, ok: false })
              throw membrane.thrown(result)
            }
            if (result.kind !== 'value')
              throw membrane.transport(`Invalid call reply ${result.kind}`)
            membrane.settle(record, { ok: true, value: result.value })
            return membrane.decode(result.value)
          }
          Object.defineProperties(method, {
            name: {
              value: this.decode(found.function.name),
              configurable: true,
            },
            length: { value: found.function.length, configurable: true },
          })
          this.functions.set(found.function.id, method)
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

  untransportable(record, message) {
    record.state = 'untransportable'
    record.transportError = message
  }

  checkOperands(value, callbackLimit, allowFunction = false, seen = new Set()) {
    if (typeof value === 'function') {
      if (!allowFunction) throw this.transport(callbackLimit)
      return
    }
    if (
      !value ||
      typeof value !== 'object' ||
      nativeReferences.has(value) ||
      seen.has(value)
    )
      return
    seen.add(value)
    for (const descriptor of Object.values(
      Object.getOwnPropertyDescriptors(value),
    )) {
      if (!Object.hasOwn(descriptor, 'value'))
        throw new Error('Native accessor operands are not transported')
      this.checkOperands(descriptor.value, callbackLimit, false, seen)
    }
  }

  thrown(frame) {
    const value = this.decode(frame.error)
    if (
      !value ||
      typeof value.name !== 'string' ||
      typeof value.message !== 'string'
    )
      throw this.transport('Invalid native error frame')
    return errorFrom(
      value,
      frame.errorStack && this.decode(frame.errorStack),
      this.errorTypes,
    )
  }

  async finish() {
    try {
      this.request({ op: 'finish' })
      return await this.bounded(this.outcome, 5000, 'Native finish timed out')
    } finally {
      await this.abort()
    }
  }

  async abort() {
    if (!this.stopping) {
      this.finished = true
      this.worker.postMessage({ stop: true })
      // The I/O owner reaps its child before closing its port. Terminating that owner
      // first would orphan the native process, including when Finish itself throws.
      this.stopping = this.bounded(
        this.stopped,
        10_000,
        'Native cleanup timed out',
      ).catch(async (error) => {
        this.transportErrors.push(error.message)
        await this.worker.terminate()
        throw error
      })
    }
    await this.stopping
  }

  async bounded(promise, milliseconds, message) {
    let timer
    try {
      return await Promise.race([
        promise,
        new Promise((_, reject) => {
          timer = setTimeout(
            () => reject(this.transport(message)),
            milliseconds,
          )
        }),
      ])
    } finally {
      clearTimeout(timer)
    }
  }
}

/** Records outer calls delegated to real Playwright and wraps returned browser objects.
 * Its identity maps preserve aliases while method bodies run on their actual receivers. */
export class ReferenceProxy extends CallJournal {
  constructor({ connection, page, playwright }) {
    super()
    this.requirements = new Set()
    this.proxies = new WeakMap()
    this.actual = new WeakMap()
    this.refs = new WeakMap()
    this.count = 0
    this.callbacks = new WeakMap()
    this.functions = new WeakMap()
    this.connections = new Set([connection])
    this.channelPrototype = Object.getPrototypeOf(Object.getPrototypeOf(page))
    this.prototypes = new Map()
    this.gaps = new Set()
    this.seed(page.locator('__neo_capability_probe__'), 'Locator')
    this.seed(page.frameLocator?.('__neo_capability_probe__'), 'FrameLocator')
    for (const [member, family] of Object.entries({
      keyboard: 'Keyboard',
      mouse: 'Mouse',
      touchscreen: 'Touchscreen',
      clock: 'Clock',
      coverage: 'Coverage',
      localStorage: 'WebStorage',
      sessionStorage: 'WebStorage',
      screencast: 'Screencast',
      webmcp: 'WebMCP',
    }))
      this.seed(page[member], family)
    const context = page.context()
    this.seed(context.clock, 'Clock')
    this.seed(context.credentials, 'Credentials')
    if (playwright) {
      this.connections.add(playwright._connection)
      this.seed(playwright.selectors, 'Selectors')
      this.seed(playwright.request, 'APIRequest')
    } else
      this.gap(
        'The real Playwright namespace was not supplied for the reference census',
      )
    this.codec = codecFactory({
      identify: (value) => this.identify(value),
      allowFunctions: false,
      resolve: () => {
        throw new Error('Reference decode is unused')
      },
    })
    this.roots = {
      page: this.codec.encode(page),
      context: this.codec.encode(context),
      mainFrame: this.codec.encode(page.mainFrame()),
    }
    this.playwright = playwright && this.wrap(playwright)
  }

  gap(message) {
    if (!this.gaps.has(message)) {
      this.gaps.add(message)
      this.censusGaps.push(message)
    }
  }

  seed(value, family) {
    if (!family || !value || typeof value !== 'object') return
    const prototype = Object.getPrototypeOf(value)
    // These prototypes come from genuine public properties or pinned API return/event
    // origins. Ordinary records, including {_type:'Page'}, never acquire API identity.
    if (prototype && prototype !== Object.prototype)
      this.prototypes.set(prototype, family)
  }

  identify(value) {
    value = this.actual.get(value) || value
    if (!value || typeof value !== 'object') return undefined
    if (this.refs.has(value)) return this.refs.get(value)
    // Bundling renames constructors. A protocol label is authoritative only together
    // with ChannelOwner ancestry and membership in the actual connection registry.
    let family = this.prototypes.get(Object.getPrototypeOf(value))
    if (!family) {
      let prototype = Object.getPrototypeOf(value)
      while (prototype && prototype !== this.channelPrototype)
        prototype = Object.getPrototypeOf(prototype)
      if (
        !prototype ||
        !this.connections.has(value._connection) ||
        value._connection?._objects.get(value._guid) !== value
      )
        return undefined
      family = value._type
    }
    if (typeof family !== 'string' || !family) return undefined
    if (!this.refs.has(value))
      this.refs.set(value, { epoch: 'reference', id: ++this.count, family })
    return this.refs.get(value)
  }

  unwrap(value, seen = new Map()) {
    if (!value || typeof value !== 'object') return value
    if (this.actual.has(value)) return this.actual.get(value)
    const prototype = Object.getPrototypeOf(value)
    if (
      Array.isArray(value) ||
      prototype === Object.prototype ||
      prototype === null
    )
      return mapData(value, (item) => this.unwrap(item, seen), seen)
    return value
  }

  wrap(value, seen = new Map()) {
    value = this.actual.get(value) || value
    if (Array.isArray(value))
      return mapData(value, (item) => this.wrap(item, seen), seen)
    const ref = this.identify(value)
    if (!ref) {
      if (value && typeof value === 'object') {
        const prototype = Object.getPrototypeOf(value)
        if (prototype === Object.prototype || prototype === null) {
          for (const [key, descriptor] of Object.entries(
            Object.getOwnPropertyDescriptors(value),
          ))
            if (!Object.hasOwn(descriptor, 'value'))
              this.gap(`Uninstrumented reference record accessor ${key}`)
          return mapData(value, (item) => this.wrap(item, seen), seen)
        }
        if (
          !(
            value instanceof Error ||
            value instanceof RegExp ||
            value instanceof Date ||
            value instanceof URL ||
            value instanceof ArrayBuffer ||
            ArrayBuffer.isView(value)
          )
        )
          this.gap(
            `Uninstrumented reference object ${value.constructor?.name || 'unknown'}`,
          )
      }
      return value
    }
    if (this.proxies.has(value)) return this.proxies.get(value)
    const proxy = new Proxy(value, {
      get: (target, member) => {
        const result = Reflect.get(target, member, target)
        if (typeof member !== 'string' || ignored.has(member)) return result
        // Error constructors are shared pure bookkeeping with the native membrane.
        if (ref.family === 'Playwright' && member === 'errors') return result
        if (typeof result !== 'function') return this.wrap(result)
        if (member === 'constructor') {
          this.gap(
            `Uninstrumented reference constructor access for ${ref.family}`,
          )
          return result
        }
        if (!this.functions.has(result)) {
          const membrane = this
          const wrapper = function (...args) {
            const method = capabilityName(ref.family, member)
            membrane.requirements.add(method)
            const receiver = membrane.actual.get(this) || this
            const receiverRef = membrane.identify(receiver)
            if (!receiverRef) {
              membrane.correlationGaps.push(
                `Untransportable receiver for ${method}`,
              )
              return Reflect.apply(
                result,
                receiver,
                membrane.arguments(args, ref.family, member, receiver),
              )
            }
            const record = membrane.record(receiverRef, method)
            const settle = (ok, returned) => {
              record.state = ok ? 'returned' : 'threw'
              if (ok)
                membrane.returnedOrigin(returned, ref.family, member, args)
              try {
                record[ok ? 'value' : 'error'] = membrane.codec.encode(
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
              return membrane.wrap(returned)
            }
            const actualArgs = membrane.arguments(
              args,
              ref.family,
              member,
              receiver,
            )
            let returned
            try {
              returned = Reflect.apply(result, receiver, actualArgs)
            } catch (error) {
              return settle(false, error)
            }
            return returned && typeof returned.then === 'function'
              ? returned.then(
                  (value) => settle(true, value),
                  (error) => settle(false, error),
                )
              : settle(true, returned)
          }
          Object.defineProperties(wrapper, {
            name: { value: result.name, configurable: true },
            length: { value: result.length, configurable: true },
          })
          this.functions.set(result, wrapper)
        }
        return this.functions.get(result)
      },
    })
    this.proxies.set(value, proxy)
    this.actual.set(proxy, value)
    return proxy
  }

  eventOrigin(value, family, event) {
    // ConsoleMessage and the other classes here are not ChannelOwners. The pinned
    // client constructs them at these public events, outside the connection registry.
    const eventFamilies = {
      console: 'ConsoleMessage',
      download: 'Download',
      filechooser: 'FileChooser',
      weberror: 'WebError',
    }
    if (
      ['Page', 'BrowserContext', 'Worker', 'ElectronApplication'].includes(
        family,
      )
    )
      this.seed(value, eventFamilies[event])
  }

  returnedOrigin(value, family, member, args) {
    if (member === 'waitForEvent') this.eventOrigin(value, family, args[0])
    if (family === 'Page' && member === 'video') this.seed(value, 'Video')
    if (
      family === 'APIRequestContext' &&
      ['fetch', 'get', 'post', 'put', 'patch', 'head', 'delete'].includes(
        member,
      )
    )
      this.seed(value, 'APIResponse')
  }

  arguments(args, family, member, receiver) {
    const eventMembers = [
      'on',
      'once',
      'off',
      'removeListener',
      'addListener',
      'prependListener',
      'prependOnceListener',
      'waitForEvent',
    ]
    // Host callbacks receive instrumented public objects. Evaluation functions stay
    // untouched because Playwright serializes their original code into the browser.
    const hostCallbacks = [
      ...eventMembers,
      'route',
      'unroute',
      'routeWebSocket',
      'waitForRequest',
      'waitForResponse',
      'exposeBinding',
      'exposeFunction',
      'addLocatorHandler',
      'removeLocatorHandler',
    ]
    const callback = (arg) => {
      if (typeof arg !== 'function') return this.unwrap(arg)
      if (!this.callbacks.has(arg)) this.callbacks.set(arg, new Map())
      const owners = this.callbacks.get(arg)
      if (!owners.has(receiver)) owners.set(receiver, new Map())
      const registrations = owners.get(receiver)
      const key = eventMembers.includes(member)
        ? `event/${String(args[0])}`
        : member === 'unroute'
          ? 'route'
          : member
      let wrapped = registrations.get(key)
      if (!wrapped) {
        // on/off share a wrapper for this emitter and event. Separate emitters and
        // event kinds cannot accidentally assign each other's callback object type.
        const membrane = this
        wrapped = function (...values) {
          if (eventMembers.includes(member))
            membrane.eventOrigin(values[0], family, args[0])
          return Reflect.apply(
            arg,
            membrane.wrap(this),
            values.map((value) => membrane.wrap(value)),
          )
        }
        registrations.set(key, wrapped)
      }
      return wrapped
    }
    return args.map((arg) => {
      if (!hostCallbacks.includes(member)) return this.unwrap(arg)
      if (
        member === 'waitForEvent' &&
        arg &&
        Object.getPrototypeOf(arg) === Object.prototype
      ) {
        const descriptor = Object.getOwnPropertyDescriptor(arg, 'predicate')
        if (descriptor && !Object.hasOwn(descriptor, 'value'))
          this.gap('Uninstrumented reference waitForEvent predicate accessor')
        if (typeof descriptor?.value === 'function') {
          const descriptors = Object.getOwnPropertyDescriptors(this.unwrap(arg))
          descriptors.predicate.value = callback(descriptor.value)
          return Object.defineProperties(
            Object.create(Object.getPrototypeOf(arg)),
            descriptors,
          )
        }
      }
      return callback(arg)
    })
  }
}
