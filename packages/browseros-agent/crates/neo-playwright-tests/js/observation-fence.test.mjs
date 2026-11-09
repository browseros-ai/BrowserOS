/** Neo: Exercise the fixture/supervisor handoff, including the viewport reset on a real
 * reference disconnect. No test here implements an SDK operation or a parity fallback. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import { createRequire } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { OwnedBrowser } from './browser.mjs'
import { observationFence, requestObservation } from './observation-fence.mjs'
import { source } from './prepare.mjs'

function directory(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-observation-fence-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  return root
}

test('fixture disconnect cannot reset state until final effects have been captured', async (t) => {
  const root = directory(t)
  let viewport = 1280
  let disconnected = false
  const fence = observationFence(root, async () => {
    assert.equal(disconnected, false)
    return { status: 'complete', viewport }
  })
  await requestObservation(root)
  disconnected = true
  viewport = 1040
  await fence.stop()
  assert.deepEqual(fence.state.observation, {
    status: 'complete',
    viewport: 1280,
  })
  assert.equal(fence.state.error, undefined)
  assert.equal(
    viewport,
    1040,
    'teardown actually changed the state after capture',
  )
})

test('snapshot failure rejects the fixture acknowledgement and remains owned', async (t) => {
  const root = directory(t)
  const fence = observationFence(root, async () => {
    throw new Error('lost snapshot')
  })
  await assert.rejects(requestObservation(root), /lost snapshot/)
  await fence.stop()
  assert.match(fence.state.error, /lost snapshot/)
  assert.equal(fence.state.observation, undefined)
})

test('a runner exiting without a fixture never triggers a late substitute snapshot', async (t) => {
  const root = directory(t)
  let captures = 0
  const fence = observationFence(root, async () => {
    captures++
  })
  await fence.stop()
  assert.equal(captures, 0)
  assert.equal(fence.state.requested, false)
  await assert.rejects(
    requestObservation(root, { timeoutMs: 20 }),
    /acknowledgement timed out/,
  )
})

const liveTest =
  process.env.NEO_OBSERVER_BROWSER && !process.versions.bun ? test : test.skip
liveTest(
  'real Playwright disconnect happens after the observer captures its actual viewport',
  async (t) => {
    const root = directory(t)
    const require = createRequire(path.join(source, 'package.json'))
    const playwright = require('playwright-core')
    const owned = await OwnedBrowser.start({
      executable: process.env.NEO_OBSERVER_BROWSER,
      outputDir: root,
    })
    let browser
    let fence
    try {
      browser = await playwright.chromium.connectOverCDP(owned.endpoint)
      const page = browser.contexts()[0].pages()[0]
      await page.setViewportSize({ width: 1280, height: 720 })
      await page.setContent(
        '<title>observation boundary</title><p>unchanged</p>',
      )
      fence = observationFence(root, () => owned.snapshot())
      await requestObservation(root)
      await browser.close()
      await fence.stop()
      assert.equal(fence.state.error, undefined)
      assert.deepEqual(
        fence.state.observation.pages[0].documents[0].dom.viewport,
        { width: 1280, height: 720, devicePixelRatio: 1 },
      )
    } finally {
      await fence?.stop()
      await browser?.close()
      await owned.close()
    }
  },
)
