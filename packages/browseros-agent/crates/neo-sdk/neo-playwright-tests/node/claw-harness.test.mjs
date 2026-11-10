import assert from 'node:assert/strict'
import test from 'node:test'
import {
  installFixtureBuiltins,
  resetClawTest,
  serverOrigins,
} from './claw-harness.mjs'

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
