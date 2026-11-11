import assert from 'node:assert/strict'
import test from 'node:test'
import {
  closeHarness,
  holdHarness,
  installFixtureBuiltins,
  resetClawTest,
  serverOrigins,
} from './claw-harness.mjs'

test('tests that worker shutdown keeps CDP open until test cleanup finishes', async () => {
  let closed = false
  const cdp = {
    close: async () => {
      closed = true
    },
  }
  const release = holdHarness(cdp)
  const closing = closeHarness(cdp)
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(closed, false)
  release()
  await closing
  assert.equal(closed, true)
})

test('tests that builtins cleanup accepts an already detached session and preserves other failures', async () => {
  for (const message of [
    'Target.detachFromTarget: No session with given id',
    'Target.detachFromTarget: Target closed.',
    'Target.detachFromTarget: Session closed.',
    'CDP connection closed',
    'Target.detachFromTarget: CDP reply timed out',
  ]) {
    let detached = 0
    const cdp = {
      async send(method) {
        if (method === 'Target.getTargets')
          return { targetInfos: [{ type: 'page', targetId: 'fixture' }] }
        if (method === 'Target.attachToTarget')
          return { sessionId: 'fixture-session' }
        if (method === 'Runtime.evaluate') return { result: {} }
        if (method === 'Target.detachFromTarget') {
          detached++
          throw new Error(message)
        }
        return {}
      },
    }
    const uninstall = await installFixtureBuiltins(cdp, [])
    if (!message.includes('CDP')) {
      await Promise.all([uninstall(), uninstall()])
      assert.equal(detached, 1)
    } else await assert.rejects(uninstall(), { message })
  }
})

test('tests that missing-session cleanup preserves the original setup error', async () => {
  const cdp = {
    async send(method) {
      if (method === 'Target.getTargets')
        return { targetInfos: [{ type: 'page', targetId: 'fixture' }] }
      if (method === 'Target.attachToTarget') return { sessionId: 'fixture' }
      if (method === 'Page.enable') throw new Error('setup failed')
      if (method === 'Target.detachFromTarget')
        throw new Error('Target.detachFromTarget: No session with given id')
      return {}
    },
  }
  await assert.rejects(installFixtureBuiltins(cdp, []), /setup failed/)
})

// These commands are infrastructure, so an in-memory CDP peer can prove exactly what is exposed.
test('tests that only the newly opened fixture target receives builtins', async () => {
  const installed = []
  let detached = 0
  const cdp = {
    async send(method, params, session) {
      if (method === 'Target.getTargets')
        return {
          targetInfos: [
            { type: 'page', targetId: 'launch' },
            { type: 'page', targetId: 'fixture' },
          ],
        }
      if (method === 'Target.attachToTarget') {
        assert.equal(params.targetId, 'fixture')
        return { sessionId: 'fixture-session' }
      }
      if (method === 'Page.addScriptToEvaluateOnNewDocument')
        installed.push({ source: params.source, session })
      if (method === 'Runtime.evaluate') return { result: {} }
      if (method === 'Target.detachFromTarget') detached++
      return {}
    },
  }
  const uninstall = await installFixtureBuiltins(cdp, ['launch'])
  assert.equal(detached, 0)
  await uninstall()
  await uninstall()
  assert.equal(detached, 1)
  assert.equal(installed.length, 1)
  assert.equal(installed[0].session, 'fixture-session')
  assert.match(installed[0].source, /exposeBuiltins/)
  await assert.rejects(
    installFixtureBuiltins(cdp, []),
    /harness-error: fixture page target diff has 2 pages/,
  )
})

test('tests that reset closes test tabs, keeps the launch tab and clears server origins', async () => {
  const closed = []
  const launch = {
    close: () => {
      throw new Error('must keep launch tab')
    },
  }
  const fixture = { close: async () => closed.push('fixture') }
  const popup = { close: async () => closed.push('popup') }
  const cleared = []
  let listeners = 2
  const context = {
    pages: () => [launch, fixture, popup],
    removeAllListeners: () => {
      listeners = 0
    },
  }
  const cdp = {
    async send(method, params, session) {
      if (method === 'Target.getTargets')
        return { targetInfos: [{ type: 'page', targetId: 'launch' }] }
      if (method === 'Target.attachToTarget')
        return { sessionId: 'maintenance' }
      if (method === 'Runtime.evaluate') return { result: { value: 'null' } }
      if (method === 'Target.detachFromTarget') return {}
      if (
        method === 'Storage.clearDataForOrigin' ||
        method === 'Network.clearBrowserCache'
      )
        assert.equal(session, 'maintenance')
      cleared.push({ method, params })
    },
  }
  const origins = serverOrigins(12345)
  await resetClawTest({
    context,
    initialPages: new Set([launch]),
    cdp,
    origins,
  })
  assert.deepEqual(closed, ['fixture', 'popup'])
  assert.equal(listeners, 0)
  assert.equal(cleared[0].method, 'Storage.clearCookies')
  assert.deepEqual(
    cleared
      .filter((call) => call.method === 'Storage.clearDataForOrigin')
      .map((call) => call.params.origin),
    origins,
  )
  assert.ok(
    cleared
      .filter((call) => call.method === 'Storage.clearDataForOrigin')
      .every((call) => call.params.storageTypes === 'all'),
  )
})

test('tests that a failed page close still clears remaining pages and storage and fails visibly', async () => {
  let closed = false
  let reset = false
  const context = {
    pages: () => [
      {
        close: async () => {
          throw new Error('page failed')
        },
      },
      {
        close: async () => {
          closed = true
        },
      },
    ],
    removeAllListeners() {},
  }
  await assert.rejects(
    resetClawTest({
      context,
      initialPages: new Set(),
      cdp: {
        async send() {
          reset = true
        },
      },
      origins: [],
    }),
    /harness-error: Claw test reset failed: page failed/,
  )
  assert.equal(closed, true)
  assert.equal(reset, true)
})

test('tests that reset clears storage even when its retained target disappears', async () => {
  for (const failedCommand of [
    'Target.attachToTarget',
    'Storage.clearDataForOrigin',
  ]) {
    const origins = ['http://localhost:12345', 'https://localhost:12346']
    const cleared = []
    let created = 0
    const cdp = {
      async send(method, params, session) {
        if (method === 'Target.getTargets')
          return { targetInfos: [{ type: 'page', targetId: 'launch' }] }
        if (
          method === failedCommand &&
          (params?.targetId === 'launch' || session === 'launch')
        )
          throw new Error(`${method}: No target with given id found`)
        if (method === 'Browser.createTab') {
          created++
          return { tab: { targetId: 'maintenance' } }
        }
        if (method === 'Target.attachToTarget')
          return { sessionId: params.targetId }
        if (
          method === 'Storage.clearDataForOrigin' &&
          session === 'maintenance'
        )
          cleared.push(params.origin)
        if (method === 'Runtime.evaluate') return { result: { value: 'null' } }
        if (method === 'Target.detachFromTarget')
          throw new Error(`${method}: No session with given id`)
        if (method === 'Target.closeTarget')
          throw new Error(`${method}: Target closed`)
        return {}
      },
    }
    await resetClawTest({
      context: { pages: () => [], removeAllListeners() {} },
      initialPages: new Set(),
      cdp,
      origins,
    })
    assert.equal(created, 1)
    assert.deepEqual(cleared, origins)
  }
})

test('tests that reset preserves storage and target-close failures', async () => {
  for (const failedCommand of [
    'Storage.clearDataForOrigin',
    'Target.closeTarget',
  ]) {
    const cdp = {
      async send(method) {
        if (method === failedCommand) throw new Error(`${method}: not allowed`)
        if (method === 'Target.getTargets') return { targetInfos: [] }
        if (method === 'Browser.createTab')
          return { tab: { targetId: 'maintenance' } }
        if (method === 'Target.attachToTarget')
          return { sessionId: 'maintenance' }
        if (method === 'Runtime.evaluate') return { result: { value: 'null' } }
        return {}
      },
    }
    await assert.rejects(
      resetClawTest({
        context: { pages: () => [], removeAllListeners() {} },
        initialPages: new Set(),
        cdp,
        origins: ['http://localhost:12345'],
      }),
      /harness-error: Claw test reset failed: .*: not allowed/,
    )
  }
})
