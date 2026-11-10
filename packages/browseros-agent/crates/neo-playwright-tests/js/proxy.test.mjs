/** Neo: Offline membrane checks use genuine pinned PW client classes and wire-only I/O doubles.
 * They verify evidence production and transport, never establish browser compatibility. */
import assert from 'node:assert/strict'
import { EventEmitter } from 'node:events'
import fs from 'node:fs'
import { createRequire } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import vm from 'node:vm'
import { codecFactory } from './codec.mjs'
import { nativeLoop } from './native.mjs'
import { source } from './prepare.mjs'
import { NativeProxy, ReferenceProxy } from './proxy.mjs'

const codec = codecFactory({
  identify: () => undefined,
  resolve: () => undefined,
})
const playwright = fs.existsSync(
  path.join(source, 'packages/playwright-core/index.js'),
)
  ? createRequire(path.join(source, 'package.json'))('playwright-core')
  : null
const pinned = {
  skip: !playwright && 'The prepared pinned PW build is unavailable',
}
const overlay = path.join(os.tmpdir(), 'neo-proxy-unit-overlay')
const configuredSites = new WeakSet()

function original(
  t,
  body,
  names = ['page'],
  file = 'tests/page/proxy.spec.ts',
) {
  if (!configuredSites.has(t)) {
    configuredSites.add(t)
    const previous = process.env.NEO_TESTS_OVERLAY
    t.after(() => {
      if (previous === undefined) delete process.env.NEO_TESTS_OVERLAY
      else process.env.NEO_TESTS_OVERLAY = previous
    })
  }
  process.env.NEO_TESTS_OVERLAY = overlay
  return vm.compileFunction(body, names, {
    filename: path.join(overlay, file),
  })
}

function reference(t) {
  const connection = new playwright._connection.constructor()
  let id = 0
  // Connection's pinned factories construct the actual client types. Only protocol
  // initializers are supplied here; no test browser operation has an implementation.
  const make = (type, initializer = {}, parent = '') =>
    connection._createRemoteObject(
      parent,
      type,
      `${type}@offline-${++id}`,
      initializer,
    )
  const tracing = make('Tracing')
  const debuggerObject = make('Debugger')
  const request = make('APIRequestContext', {
    tracing: { guid: tracing._guid },
  })
  const context = make('BrowserContext', {
    options: {},
    tracing: { guid: tracing._guid },
    debugger: { guid: debuggerObject._guid },
    requestContext: { guid: request._guid },
  })
  const page = () => {
    const frame = make(
      'Frame',
      { url: `about:blank#${id}`, name: '', loadStates: [] },
      context._guid,
    )
    return make(
      'Page',
      { mainFrame: { guid: frame._guid }, isClosed: false },
      context._guid,
    )
  }
  const first = page()
  const second = page()
  const proxy = new ReferenceProxy({ connection, page: first, playwright })
  t.after(() => connection.close())
  return { proxy, first, second, context }
}

/** A deterministic protocol peer exercises host ownership without implementing any browser API. */
class WireWorker extends EventEmitter {
  constructor() {
    super()
    this.invocations = []
    this.methods = new Map()
    const ref = (id, family) => ({
      tag: 'object',
      ref: { epoch: 'native-unit', id, family },
    })
    queueMicrotask(() =>
      this.emit('message', {
        event: 'ready',
        roots: {
          page: ref(1, 'Page'),
          context: ref(2, 'BrowserContext'),
          mainFrame: ref(3, 'Frame'),
        },
        capabilities: { Page: [], BrowserContext: [], Frame: [] },
      }),
    )
  }

  postMessage({ frame, shared, stop, ack }) {
    if (ack) return
    if (stop) {
      queueMicrotask(() => {
        this.emit('message', {
          event: 'stopped',
          exit: { code: 0, signal: null },
          infrastructureErrors: [],
        })
        this.emit('exit', 0)
      })
      return
    }
    let response
    if (frame.op === 'get') {
      const name = codec.string(frame.member)
      if (!this.methods.has(name)) this.methods.set(name, this.methods.size + 1)
      response = {
        kind: 'callable',
        function: {
          id: this.methods.get(name),
          name: codec.encode(name),
          length: 0,
        },
      }
    } else if (frame.op === 'invoke') {
      this.invocations.push(frame)
      response = this.result || {
        kind: 'value',
        value: codec.encode(undefined),
      }
    } else if (frame.op === 'finish') {
      response = { kind: 'finished' }
      queueMicrotask(() =>
        this.emit('message', { event: 'outcome', outcome: { status: 'ok' } }),
      )
    } else throw new Error(`Unexpected wire request ${frame.op}`)
    const bytes = new TextEncoder().encode(JSON.stringify(response))
    new Uint8Array(shared, 8, bytes.length).set(bytes)
    const header = new Int32Array(shared, 0, 2)
    Atomics.store(header, 1, bytes.length)
    Atomics.store(header, 0, 1)
  }

  terminate() {
    this.terminated = true
    return Promise.resolve(0)
  }
}

async function native(t, errorTypes) {
  const worker = new WireWorker()
  const proxy = new NativeProxy({}, { errorTypes, workerFactory: () => worker })
  await proxy.ready
  t.after(() => proxy.abort())
  return { proxy, worker }
}

test(
  'native errors retain pinned public classes, builtins and exact native stacks',
  pinned,
  async (t) => {
    const { proxy, worker } = await native(t, playwright.errors)
    for (const [name, ErrorType] of [
      ['TimeoutError', playwright.errors.TimeoutError],
      ['TypeError', TypeError],
      ['Error', Error],
    ]) {
      const message = `${name}: exact \ud800 wording\nCall log:\nkept`
      const stack = `${name}: native stack\n    at native-source.js:4:8`
      worker.result = {
        kind: 'throw',
        error: codec.encode({ name, message }),
        errorStack: codec.encode(stack),
      }
      assert.throws(
        () => proxy.page.fail(),
        (error) => {
          assert.ok(error instanceof ErrorType)
          assert.equal(error.name, name)
          assert.equal(error.message, message)
          assert.equal(error.stack, stack)
          return true
        },
      )
    }
    assert.equal(proxy.playwright.errors, playwright.errors)
    assert.equal(proxy.calls[0].errorStack.tag, 'string')
  },
)

test(
  'native thrown frames use the pinned public TimeoutError constructor',
  pinned,
  () => {
    const membrane = Object.assign(Object.create(NativeProxy.prototype), {
      codec,
      errorTypes: playwright.errors,
    })
    const error = membrane.thrown({
      error: codec.encode({
        name: 'TimeoutError',
        message: 'original wording',
      }),
    })
    assert.ok(error instanceof playwright.errors.TimeoutError)
    assert.equal(error.message, 'original wording')
  },
)

test('native callable identity retains the actual receiver and rejects foreign operands', async (t) => {
  const { proxy, worker } = await native(t)
  const other = proxy.object({ epoch: proxy.epoch, id: 4, family: 'Page' })
  assert.equal(proxy.page.method, other.method)
  assert.equal(proxy.page.method.name, 'method')
  const detached = proxy.page.method
  assert.throws(() => detached(), /receiver is not an owned native object/)
  detached.call(other)
  assert.equal(worker.invocations[0].object.id, 4)
  const foreign = await native(t)
  assert.throws(
    () => proxy.page.method(foreign.proxy.page),
    /Foreign native proxy operand/,
  )
  assert.ok(
    proxy.transportErrors.some((message) =>
      message.includes('Foreign native proxy'),
    ),
  )
  assert.throws(
    () => proxy.object({ epoch: 'expired', id: 1, family: 'Page' }),
    /foreign native object/,
  )
  let reads = 0
  assert.throws(
    () =>
      proxy.page.method({
        get option() {
          reads++
          return 1
        },
      }),
    /accessor operands/,
  )
  assert.equal(reads, 0)
  assert.throws(() => Object.keys(proxy.page), /enumeration is not transported/)
})

test('native finish waits for the I/O owner to reap before allowing it to exit', async (t) => {
  const { proxy, worker } = await native(t)
  assert.deepEqual(await proxy.finish(), { status: 'ok' })
  assert.equal(worker.terminated, undefined)
  assert.equal(proxy.roots.mainFrame.ref.family, 'Frame')
  assert.throws(
    () => proxy.playwright.selectors.register,
    /Neo does not serve Selectors.register/,
  )
  assert.ok(proxy.missing.has('Selectors.register'))
})

test('native finish failure still asks its I/O owner to reap', async (t) => {
  const { proxy, worker } = await native(t)
  const post = worker.postMessage.bind(worker)
  worker.postMessage = (message) => {
    if (message.frame?.op === 'finish')
      throw new Error('injected Finish failure')
    post(message)
  }
  await assert.rejects(proxy.finish(), /injected Finish failure/)
  assert.equal((await proxy.stopped).exit.code, 0)
  assert.equal(worker.terminated, undefined)
})

test('malformed native values taint transport and reject the pending promise', async (t) => {
  const { proxy, worker } = await native(t)
  worker.result = { kind: 'promise', id: 99 }
  const promise = proxy.page.pending()
  worker.emit('message', {
    event: 'settled',
    id: 99,
    ok: true,
    value: { tag: 'invalid' },
  })
  await assert.rejects(promise, { name: 'HarnessTransportError' })
  assert.ok(
    proxy.transportErrors.some((message) =>
      message.includes('Invalid native value'),
    ),
  )
  assert.equal(proxy.pending.size, 0)
})

test(
  'reference records canonical original sites and occurrences per receiver',
  pinned,
  (t) => {
    const { proxy, first, second } = reference(t)
    const call = original(t, 'return page[member]()', ['page', 'member'])
    call(proxy.wrap(first), 'mainFrame')
    call(proxy.wrap(second), 'mainFrame')
    call(proxy.wrap(first), 'url')
    assert.deepEqual(
      proxy.calls.map((record) => record.occurrence),
      [0, 0, 1],
    )
    assert.deepEqual(
      proxy.calls.map((record) => record.ordinal),
      [0, 1, 2],
    )
    assert.deepEqual(proxy.calls[0].receiver, proxy.roots.page.ref)
    assert.notDeepEqual(proxy.calls[0].receiver, proxy.calls[1].receiver)
    for (const record of proxy.calls) {
      assert.equal(record.site.file, 'tests/page/proxy.spec.ts')
      // Bun's vm.compileFunction exposes its wrapper newline in the original frame.
      assert.equal(record.site.line, process.versions.bun ? 2 : 1)
      assert.ok(record.site.column > 0)
      assert.match(record.sourceStack, /proxy\.spec\.ts/)
    }
    assert.deepEqual(proxy.correlationGaps, [])
    original(
      t,
      'return page.url()',
      ['page'],
      'tests/config/__neoOriginalBrowserTest.ts',
    )(proxy.wrap(first))
    assert.equal(proxy.calls[3].site.file, 'tests/config/browserTest.ts')
    assert.match(proxy.calls[3].sourceStack, /__neoOriginalBrowserTest\.ts/)
  },
)

test(
  'missing upstream source sites remain explicit correlation gaps',
  pinned,
  (t) => {
    const { proxy, first } = reference(t)
    proxy.wrap(first).url()
    assert.equal(proxy.calls[0].site, null)
    assert.match(proxy.correlationGaps[0], /Missing upstream source site/)
  },
)

test(
  'reference brands require real prototypes and connection ownership',
  pinned,
  (t) => {
    const { proxy, first } = reference(t)
    const literal = {
      _type: 'Page',
      _guid: first._guid,
      _connection: first._connection,
    }
    assert.equal(proxy.identify(literal), undefined)
    const forged = Object.assign(
      Object.create(Object.getPrototypeOf(first)),
      literal,
    )
    assert.equal(proxy.identify(forged), undefined)
    assert.equal(proxy.codec.encode({ _type: 'Page' }).tag, 'record')
    assert.equal(proxy.identify(first).family, 'Page')
    assert.equal(proxy.identify(first.context().tracing).family, 'Tracing')
    assert.equal(proxy.identify(first.touchscreen).family, 'Touchscreen')
    assert.equal(proxy.wrap(proxy.wrap(first)), proxy.wrap(first))
  },
)

test(
  'reference preserves extracted, rebound and shared method behavior',
  pinned,
  (t) => {
    const { proxy, first, second } = reference(t)
    const page = proxy.wrap(first)
    const another = proxy.wrap(second)
    assert.equal(first.url, second.url)
    assert.equal(page.url, another.url)
    assert.equal(page.url.name, first.url.name)
    assert.equal(page.url.length, first.url.length)
    const detached = page.url
    assert.throws(() => detached(), TypeError)
    assert.equal(detached.call(another), second.url())
    assert.equal(detached.bind(another)(), second.url())
    assert.ok(
      proxy.correlationGaps.some((message) =>
        message.includes('Untransportable receiver'),
      ),
    )
  },
)

test(
  'reference unwrap preserves unused option getters until PW consumes them',
  pinned,
  (t) => {
    const { proxy, first } = reference(t)
    const page = proxy.wrap(first)
    let reads = 0
    const options = {
      has: page.locator('span'),
      get unrelated() {
        reads++
        throw new Error('unused getter')
      },
    }
    original(t, 'return page.locator("div", options)', ['page', 'options'])(
      page,
      options,
    )
    assert.equal(reads, 0)
    assert.equal(
      Object.getOwnPropertyDescriptor(proxy.unwrap(options), 'unrelated').get,
      Object.getOwnPropertyDescriptor(options, 'unrelated').get,
    )
  },
)

test(
  'reference census covers genuine ConsoleMessage callbacks and preserves listener aliases',
  pinned,
  (t) => {
    const { proxy, first, context } = reference(t)
    const page = proxy.wrap(first)
    const seen = []
    const listener = original(
      t,
      'return function(message) { seen.push([message, message.text(), this]) }',
      ['seen'],
    )(seen)
    original(t, 'page.on("console", listener)', ['page', 'listener'])(
      page,
      listener,
    )
    const emit = () =>
      context._channel.emit('console', {
        page: first._channel,
        type: 'log',
        text: 'actual ConsoleMessage',
        args: [],
        location: { url: 'upstream', lineNumber: 1, columnNumber: 2 },
        timestamp: 0,
      })
    emit()
    assert.ok(proxy.requirements.has('ConsoleMessage.text'))
    assert.equal(seen[0][1], 'actual ConsoleMessage')
    assert.ok(seen[0][2] === page)
    assert.equal(seen[0][0].page(), page)
    assert.equal(proxy.identify(seen[0][0]).family, 'ConsoleMessage')
    original(t, 'page.off("console", listener)', ['page', 'listener'])(
      page,
      listener,
    )
    emit()
    assert.equal(seen.length, 1)
    assert.deepEqual(proxy.censusGaps, [])
  },
)

test(
  'reference instruments the real selectors and request namespaces',
  pinned,
  async (t) => {
    const { proxy } = reference(t)
    const run = original(
      t,
      'return (async () => { await playwright.selectors.register("neo_proxy_unit", { content: "({query() { return null }})" }); const request = await playwright.request.newContext(); await request.dispose() })()',
      ['playwright'],
    )
    await run(proxy.playwright)
    assert.ok(proxy.requirements.has('Selectors.register'))
    assert.ok(proxy.requirements.has('APIRequest.newContext'))
    assert.ok(proxy.requirements.has('APIRequestContext.dispose'))
    assert.equal(
      proxy.playwright.errors.TimeoutError,
      playwright.errors.TimeoutError,
    )
    assert.deepEqual(proxy.censusGaps, [])
    class UnknownPublicClass {}
    proxy.wrap(new UnknownPublicClass())
    assert.match(proxy.censusGaps[0], /Uninstrumented reference object/)
  },
)

test('native receive loop reports codec errors separately and Finish does not join browser promises', async () => {
  // Wire fixtures isolate receive-loop scheduling. Browser semantics still require
  // the parent's real single-run verification and are not inferred from this test.
  class Frame {}
  class Locator {}
  class Context {
    setDefaultTimeout(value) {
      this.timeout = value
    }
    setDefaultNavigationTimeout(value) {
      this.navigationTimeout = value
    }
  }
  class Page extends Context {
    mainFrame() {
      return frame
    }
    locator() {
      return locator
    }
    never() {
      return new Promise(() => {})
    }
  }
  const frame = new Frame()
  const locator = new Locator()
  const page = new Page()
  const context = new Context()
  const replies = []
  const commands = [
    {
      id: 1,
      op: 'get',
      object: { epoch: 'loop', id: 1, family: 'Page' },
      member: codec.units('never'),
    },
    {
      id: 2,
      op: 'invoke',
      object: { epoch: 'loop', id: 1, family: 'Page' },
      function: 1,
      member: codec.units('never'),
      args: { tag: 'array', values: [] },
    },
    {
      id: 3,
      op: 'invoke',
      object: { epoch: 'loop', id: 1, family: 'Page' },
      function: 1,
      member: codec.units('never'),
      args: { tag: 'string', units: [65536] },
    },
    { id: 4, op: 'finish' },
  ]
  const realm = vm.createContext({
    page,
    context,
    __neo_test_host: {
      reply: (text) => replies.push(JSON.parse(text)),
      receive: () => Promise.resolve(JSON.stringify(commands.shift())),
    },
  })
  await vm.runInContext(
    `(${nativeLoop.toString()})(${codecFactory.toString()}, "loop")`,
    realm,
  )
  assert.deepEqual(
    replies.map((reply) => reply.event || reply.kind),
    ['ready', 'callable', 'promise', 'transportError', 'finished'],
  )
  assert.equal(replies[0].roots.mainFrame.ref.family, 'Frame')
  assert.equal(context.timeout, 0)
  assert.equal(page.navigationTimeout, 0)
})
