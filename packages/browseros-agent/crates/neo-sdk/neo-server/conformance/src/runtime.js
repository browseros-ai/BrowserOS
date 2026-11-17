// Common test infrastructure. The in-program test runner both sides evaluate byte for
// byte: Runner P in Node over playwright-core 1.64.1, Runner N inside Neo's `run` (QuickJS).
//
// It owns only what Playwright Test owns: registration (describe/hooks/modifiers), fixtures,
// hook order, timeouts, soft-assertion collection and the result record. Everything under test
// (`context`, its pages, `expect`'s matchers) comes from the runtime it is evaluated in.
//
// The file is a function expression so the loader can embed it as text and Node can also call
// it directly for inventory collection (registration only, nothing executed).
;(function createHarness(host) {
  // host = { context, expect, setTimeout, clearTimeout, now, config }
  // config = { fixtures: {...constants}, testTimeout, expectTimeout, server: {...}, assetDir }
  const config = host.config

  class HarnessUnavailable extends Error {
    constructor(what) {
      super(`harness: ${what} is not provided by the conformance harness`)
      this.name = 'HarnessUnavailable'
      this.harnessUnavailable = what
    }
  }
  class SkipSignal extends Error {
    constructor(kind, reason) {
      super(reason || kind)
      this.name = 'SkipSignal'
      this.kind = kind
    }
  }

  // ---- registration (Playwright Test's describe/test/hook semantics) -------------------------
  const newSuite = (title, parent, location) => ({
    title,
    parent,
    location,
    entries: [],
    hooks: { beforeAll: [], beforeEach: [], afterEach: [], afterAll: [] },
    modifiers: [],
    use: [],
    mode: parent ? parent.mode : 'default',
    fixtures: parent ? { ...parent.fixtures } : {},
  })
  const root = newSuite('', null, null)
  let current = root
  const tests = []
  let running = null // { info, softErrors } while a test executes
  const location = () => (host.locate ? host.locate(new Error().stack) : null)

  function addTest(title, args, extra) {
    const fn = args[args.length - 1]
    const details =
      args.length > 1 && typeof args[0] === 'object' ? args[0] : {}
    const test = {
      title,
      fn,
      suite: current,
      location: location(),
      annotations: [
        ...(details.annotation ? [].concat(details.annotation) : []),
      ],
      tags: [].concat(details.tag || []),
      modifiers: extra ? [extra] : [],
      fixtures: current.fixtures,
    }
    current.entries.push(test)
    tests.push(test)
  }
  function modifier(kind, args) {
    // test.skip(title, fn) declares a skipped test; test.skip(), test.skip(cond, why) and
    // test.skip(({fixtures}) => cond, why) mark the enclosing suite, or stop a running test.
    if (
      typeof args[0] === 'string' &&
      typeof args[args.length - 1] === 'function'
    )
      return addTest(args[0], args.slice(1), {
        kind,
        condition: true,
        description: undefined,
      })
    const [condition = true, description] = args
    if (running) {
      const value =
        typeof condition === 'function' ? condition(fixtureValues()) : condition
      if (!value) return
      if (kind === 'slow') return running.slow()
      if (kind === 'fail') {
        running.expectedStatus = 'failed'
        return
      }
      throw new SkipSignal(kind, description)
    }
    current.modifiers.push({
      kind,
      condition,
      description,
      location: location(),
    })
  }
  function describe(args, extra) {
    const [title, fn] =
      typeof args[0] === 'function'
        ? ['', args[0]]
        : [args[0], args[args.length - 1]]
    const suite = newSuite(title, current, location())
    if (extra) suite.modifiers.push(extra)
    current.entries.push(suite)
    const parent = current
    current = suite
    try {
      fn()
    } finally {
      current = parent
    }
  }
  const hook =
    (kind) =>
    (...args) =>
      current.hooks[kind].push({
        fn: args[args.length - 1],
        title: typeof args[0] === 'string' ? args[0] : undefined,
        location: location(),
      })

  function makeTest(_extraFixtures) {
    const t = (title, ...args) => addTest(title, args)
    t.only = t
    for (const kind of ['skip', 'fixme', 'fail', 'slow'])
      t[kind] = (...args) => modifier(kind, args)
    t.fail.only = (...args) => modifier('fail', args)
    t.setTimeout = (ms) => {
      if (running) running.setTimeout(ms)
      else current.timeout = ms
    }
    t.describe = (...args) => describe(args)
    t.describe.only = t.describe
    t.describe.skip = (...args) =>
      describe(args, { kind: 'skip', condition: true })
    t.describe.fixme = (...args) =>
      describe(args, { kind: 'fixme', condition: true })
    t.describe.serial = (...args) => {
      describe(args)
      current.entries[current.entries.length - 1].mode = 'serial'
    }
    t.describe.serial.only = t.describe.serial
    t.describe.parallel = (...args) => describe(args)
    t.describe.parallel.only = t.describe.parallel
    t.describe.configure = (options = {}) => {
      if (options.mode) current.mode = options.mode
      if (options.timeout) current.timeout = options.timeout
    }
    t.beforeAll = hook('beforeAll')
    t.beforeEach = hook('beforeEach')
    t.afterEach = hook('afterEach')
    t.afterAll = hook('afterAll')
    t.use = (options) => current.use.push({ options, location: location() })
    t.step = async (title, body) => {
      running?.steps.push(title)
      return await body()
    }
    t.info = () => {
      if (!running)
        throw new Error('test.info() can only be called while test is running')
      return running.info
    }
    t.expect = expectApi
    t.extend = (fixtures) => {
      // Applied to the whole file, as `const it = base.extend(...)` is; recorded so the inventory
      // names extended fixtures, executed through the same fixture machinery as `page`.
      Object.assign(current.fixtures, fixtures)
      return t
    }
    return t
  }

  // ---- expect: the runtime's own matchers, plus what Playwright Test (not the library) owns ---
  const counts = { called: 0, passed: 0, failed: 0 }
  const custom = {}
  let retryDepth = 0
  const baseExpect = host.expect.configure({
    timeout: config.expectTimeout ?? 10000,
  })
  const retry =
    (fn) =>
    async (...args) => {
      retryDepth++
      try {
        return await fn(...args)
      } finally {
        retryDepth--
      }
    }
  function settle(call, soft, diagnostic) {
    const count = retryDepth === 0
    if (count) counts.called++
    const capture = (matched) => {
      const error = diagnostic?.error
      const valid =
        error &&
        typeof error.message === 'string' &&
        typeof error.name === 'string'
      if (diagnostic && running) {
        running.diagnostics.push({
          ...diagnostic.meta,
          matched,
          error: valid
            ? {
                name: error.name,
                message: error.message.slice(0, 8192),
                code: error.code,
                owner: error.owner,
              }
            : null,
        })
      }
      return valid
    }
    const pass = (value) => {
      if (diagnostic && !capture(true)) {
        return hardFailure(
          new Error('diagnostic assertion did not observe an error'),
        )
      }
      if (count) counts.passed++
      return value
    }
    const failed = (error) => {
      // The original matcher has already run. Its captured exception proves the behavior
      // locally; the judge still requires equivalent failure identities across runtimes.
      if (diagnostic && capture(false)) {
        running.messageDiffs.push({
          ...diagnostic.meta,
          actual: String(error.message).slice(0, 8192),
        })
        if (count) counts.passed++
        return undefined
      }
      return hardFailure(error)
    }
    const hardFailure = (error) => {
      if (count) counts.failed++
      if (soft && running) {
        running.softErrors.push(error)
        return undefined
      }
      throw error
    }
    try {
      const result = call()
      return result?.then ? result.then(pass, failed) : pass(result)
    } catch (error) {
      return failed(error)
    }
  }
  function matchersFor(base, received, soft, diagnostic, modifiers = []) {
    // Getters and matcher `this` belong to the runtime. A prototype shim changes Playwright's
    // internal matcher state, so custom Test matchers are looked up separately.
    return new Proxy(base, {
      get(target, name) {
        if (name === 'not' || name === 'resolves' || name === 'rejects')
          return matchersFor(
            Reflect.get(target, name, target),
            received,
            soft,
            diagnostic,
            [...modifiers, name],
          )
        const value = Reflect.get(target, name, target)
        if (typeof value !== 'function' && !(name in custom)) return value
        return (...args) => {
          const invoke = () => {
            if (name in custom) {
              const result = custom[name].call(
                { isNot: modifiers.includes('not') },
                received,
                ...args,
              )
              const check = ({ pass, message: text }) => {
                if (pass === modifiers.includes('not')) throw new Error(text())
              }
              return result?.then ? result.then(check) : check(result)
            }
            if (name === 'toPass') {
              retryDepth++
              try {
                const result = value.apply(target, args)
                if (result?.then)
                  return result.finally(() => {
                    retryDepth--
                  })
                retryDepth--
                return result
              } catch (error) {
                retryDepth--
                throw error
              }
            }
            return value.apply(target, args)
          }
          return settle(invoke, soft, diagnostic)
        }
      },
    })
  }
  function wrapExpect(base, soft = false) {
    const fn = (received, ...rest) =>
      matchersFor(base(received, ...rest), received, soft)
    const wrapped = new Proxy(fn, {
      get(_target, name) {
        if (name === 'soft') return wrapExpect(base, true)
        if (name === 'extend')
          return (matchers) => {
            Object.assign(custom, matchers)
            return wrapped
          }
        if (name === 'configure')
          return ({ soft: nextSoft = soft, ...options } = {}) =>
            wrapExpect(base.configure(options), nextSoft)
        if (name === 'poll')
          return (fn, ...args) =>
            matchersFor(base.poll(retry(fn), ...args), fn, soft)
        return Reflect.get(base, name, base)
      },
    })
    return wrapped
  }
  const expectApi = wrapExpect(baseExpect)
  function messageExpect(meta, received, ...rest) {
    const diagnostic = { meta }
    let value = received
    if (meta.kind === 'message') {
      diagnostic.error = received
      value = received?.message
    } else if (meta.rejects && typeof received?.then === 'function') {
      value = received.then(undefined, (error) => {
        diagnostic.error = error
        throw error
      })
    } else if (typeof received === 'function') {
      value = function (...args) {
        try {
          return Reflect.apply(received, this, args)
        } catch (error) {
          diagnostic.error = error
          throw error
        }
      }
    }
    return matchersFor(baseExpect(value, ...rest), value, false, diagnostic)
  }

  // ---- fixtures ------------------------------------------------------------------------------
  function fixtureValues(page) {
    return {
      ...config.fixtures,
      context: host.context,
      page,
      server: serverFixture(),
      asset: assetFixture,
    }
  }
  function serverFixture() {
    const server = { ...config.server }
    for (const member of [
      'setRoute',
      'setRedirect',
      'setCSP',
      'setAuth',
      'enableGzip',
      'setExtraHeaders',
      'waitForRequest',
      'setContent',
      'reset',
      'onceWebSocketConnection',
      'waitForWebSocket',
      'enableHTTPCache',
    ])
      server[member] = () => {
        throw new HarnessUnavailable(`server.${member}`)
      }
    return server
  }
  const assetFixture = (name) => `${config.assetDir}/${name}`

  // ---- one test's execution, in Playwright Test's order ---------------------------------------
  function chain(suite) {
    const out = []
    for (let s = suite; s; s = s.parent) out.unshift(s)
    return out
  }
  function staticModifier(test) {
    for (const suite of chain(test.suite))
      for (const m of suite.modifiers) {
        const value =
          typeof m.condition === 'function'
            ? m.condition(config.fixtures)
            : m.condition
        if (value) return m
      }
    return test.modifiers.find((m) => m.condition) || null
  }
  function serialize(error) {
    if (
      !(error instanceof Error) &&
      (typeof error !== 'object' || error === null)
    )
      return { name: 'Thrown', message: String(error) }
    return {
      name: String(error.name),
      message: String(error.message).slice(0, 8192),
      code: error.code,
      owner: error.owner,
      harnessUnavailable: error.harnessUnavailable,
      stack: String(error.stack || '')
        .split('\n')
        .slice(0, 6)
        .join('\n'),
    }
  }
  async function race(promise, ms, what) {
    let timer
    const expired = new Promise((_, reject) => {
      timer = host.setTimeout(
        () =>
          reject(
            Object.assign(new Error(`${what} timeout of ${ms}ms exceeded.`), {
              name: 'TimeoutError',
              harnessTimeout: true,
            }),
          ),
        ms,
      )
    })
    try {
      return await Promise.race([promise, expired])
    } finally {
      host.clearTimeout(timer)
    }
  }

  async function runOne(id) {
    const test = tests[id]
    if (!test) throw new Error(`no test #${id}`)
    const started = host.now()
    host.phase?.('test')
    const record = {
      id,
      titlePath: titlePath(test),
      status: 'passed',
      expectedStatus: 'passed',
      errors: [],
      softErrors: [],
      messageDiffs: [],
      diagnostics: [],
      images: [],
      measurements: {},
      steps: [],
      assertions: counts,
      annotations: [...test.annotations],
      cleanup: { closed: 0, errors: [] },
    }
    const suites = chain(test.suite)
    let timeout = suites.reduce(
      (value, s) => s.timeout ?? value,
      config.testTimeout ?? 30000,
    )
    let updateDeadline = null
    const setTimeout = (ms) => {
      timeout = ms
      if (running) running.info.timeout = ms
      updateDeadline?.()
    }
    const deadline = async (call, limit, mutable = false) => {
      let timer
      const begin = host.now()
      const expired = new Promise((_, reject) => {
        const arm = () => {
          host.clearTimeout(timer)
          const ms = mutable ? timeout : limit
          if (ms === 0) return
          timer = host.setTimeout(
            () =>
              reject(
                Object.assign(new Error(`Test timeout of ${ms}ms exceeded.`), {
                  name: 'TimeoutError',
                  harnessTimeout: true,
                }),
              ),
            Math.max(0, begin + ms - host.now()),
          )
        }
        if (mutable) updateDeadline = arm
        arm()
      })
      try {
        return await Promise.race([Promise.resolve().then(call), expired])
      } finally {
        host.clearTimeout(timer)
        if (mutable) updateDeadline = null
      }
    }
    running = {
      info: {
        title: test.title,
        titlePath: record.titlePath,
        annotations: record.annotations,
        project: { name: 'chromium-page' },
        timeout,
        setTimeout,
        images: record.images,
        measurements: record.measurements,
        attach: () => {
          throw new HarnessUnavailable('testInfo.attach')
        },
        outputPath: () => {
          throw new HarnessUnavailable('testInfo.outputPath')
        },
      },
      softErrors: record.softErrors,
      messageDiffs: record.messageDiffs,
      diagnostics: record.diagnostics,
      steps: record.steps,
      expectedStatus: 'passed',
      setTimeout,
      slow: () => setTimeout(timeout * 3),
    }
    const modifiers = [...suites.flatMap((s) => s.modifiers), ...test.modifiers]
    let page = null
    const before = new Set(host.context.pages())
    const resolve = async (fn, hookKind) => {
      const values = {}
      for (const name of fixtureNames(fn)) {
        if (
          (hookKind === 'beforeAll' || hookKind === 'afterAll') &&
          (name === 'page' || name === 'context')
        )
          throw new Error(
            `"context" and "page" fixtures are not supported in "${hookKind}"`,
          )
        if (name === 'page') {
          page ??= await host.context.newPage()
          values.page = page
        } else if (name in test.fixtures)
          throw new HarnessUnavailable(`test.extend fixture ${name}`)
        else if (name === 'context') values.context = host.context
        else if (name === 'server') values.server = serverFixture()
        else if (name === 'asset') values.asset = assetFixture
        else if (name === 'channel') values[name] = undefined
        else if (name in config.fixtures) values[name] = config.fixtures[name]
        else throw new HarnessUnavailable(`fixture ${name}`)
      }
      return values
    }
    const step = async (fn, kind) => fn(await resolve(fn, kind), running.info)
    const allHook = async (fn, kind) => {
      const saved = timeout
      try {
        await deadline(() => step(fn, kind), timeout, true)
      } finally {
        timeout = saved
        running.info.timeout = saved
      }
    }
    const fail = async (error) => {
      // The dispatcher attaches owner/code at the next async handoff, before serialization.
      await Promise.resolve()
      if (error instanceof SkipSignal) {
        record.status = 'skipped'
        record.expectedStatus = 'skipped'
        record.skip = { kind: error.kind, reason: error.message }
      } else {
        if (record.status !== 'timedOut')
          record.status = error.harnessTimeout ? 'timedOut' : 'failed'
        record.errors.push(serialize(error))
      }
    }
    let executionStarted = false
    try {
      for (const m of modifiers) {
        const value =
          typeof m.condition === 'function'
            ? m.condition(fixtureValues())
            : m.condition
        if (!value) continue
        if (m.kind === 'slow') running.slow()
        else if (m.kind === 'fail') running.expectedStatus = 'failed'
        else throw new SkipSignal(m.kind, m.description)
      }
      executionStarted = true
      for (const s of suites)
        for (const h of s.hooks.beforeAll) await allHook(h.fn, 'beforeAll')
      await deadline(
        async () => {
          for (const s of suites)
            for (const h of s.hooks.beforeEach) await step(h.fn, 'beforeEach')
          await step(test.fn)
        },
        timeout,
        true,
      )
    } catch (error) {
      await fail(error)
    }
    host.phase?.('cleanup')
    // Teardown has a separate budget even if the body timed out. It is outside the raced body:
    // a hung body must never prevent afterEach or afterAll from being attempted.
    if (executionStarted)
      for (const s of [...suites].reverse())
        for (const h of s.hooks.afterEach) {
          try {
            await deadline(() => step(h.fn, 'afterEach'), timeout || 30000)
          } catch (error) {
            await fail(error)
          }
        }
    if (executionStarted)
      for (const s of [...suites].reverse())
        for (const h of s.hooks.afterAll) {
          try {
            await allHook(h.fn, 'afterAll')
          } catch (error) {
            await fail(error)
          }
        }
    for (const opened of host.context.pages())
      if (!before.has(opened)) {
        try {
          await race(opened.close(), 5000, 'page cleanup')
          record.cleanup.closed++
        } catch (error) {
          record.cleanup.errors.push(serialize(error))
        }
      }
    if (
      record.status === 'passed' &&
      (record.errors.length || record.softErrors.length)
    )
      record.status = 'failed'
    if (!record.skip) record.expectedStatus = running.expectedStatus
    record.softErrors = record.softErrors.map(serialize)
    record.durationMs = host.now() - started
    running = null
    return record
  }
  function titlePath(test) {
    return [
      ...chain(test.suite)
        .map((s) => s.title)
        .filter(Boolean),
      test.title,
    ]
  }

  // Adapted from Microsoft Playwright, packages/playwright/src/common/fixtures.ts.
  // Copyright Microsoft Corporation, Apache-2.0; LICENSE and NOTICE are retained under
  // ../neo-playwright/third_party/playwright. Modified here to expose the fixture-name list.
  // Playwright Test's own parser (packages/playwright/src/common/fixtures.ts at v1.64.1).
  function fixtureNames(fn) {
    if (typeof fn !== 'function') return []
    const text = filterOutComments(fn.toString())
    const match = text.match(/(?:async)?(?:\s+function)?[^(]*\(([^)]*)/)
    if (!match) return []
    const trimmed = match[1].trim()
    if (!trimmed) return []
    const [first] = splitByComma(trimmed)
    if (first[0] !== '{' || first[first.length - 1] !== '}') return []
    return splitByComma(first.substring(1, first.length - 1)).map((p) => {
      const c = p.indexOf(':')
      return c === -1 ? p.trim() : p.substring(0, c).trim()
    })
  }
  function filterOutComments(s) {
    const result = []
    let state = 'none'
    for (let i = 0; i < s.length; ++i) {
      if (state === 'singleline') {
        if (s[i] === '\n') state = 'none'
      } else if (state === 'multiline') {
        if (s[i - 1] === '*' && s[i] === '/') state = 'none'
      } else if (s[i] === '/' && s[i + 1] === '/') state = 'singleline'
      else if (s[i] === '/' && s[i + 1] === '*') {
        state = 'multiline'
        i += 2
      } else result.push(s[i])
    }
    return result.join('')
  }
  function splitByComma(s) {
    const result = []
    const stack = []
    let start = 0
    for (let i = 0; i < s.length; i++) {
      if (s[i] === '{' || s[i] === '[') stack.push(s[i] === '{' ? '}' : ']')
      else if (s[i] === stack[stack.length - 1]) stack.pop()
      else if (!stack.length && s[i] === ',') {
        const t = s.substring(start, i).trim()
        if (t) result.push(t)
        start = i + 1
      }
    }
    const last = s.substring(start).trim()
    if (last) result.push(last)
    return result
  }

  return {
    test: makeTest(),
    expect: expectApi,
    messageExpect,
    tests,
    root,
    HarnessUnavailable,
    fixtureNames,
    staticModifier,
    chain,
    titlePath,
    runOne,
  }
})
