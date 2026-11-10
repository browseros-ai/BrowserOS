// Neo against real Playwright on the same BrowserOS tab: watch-url.js runs in both at once
// while this test changes the tab's URL over CDP and then closes it. Both results must be
// identical, and equal to what the design measured for Playwright.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import {
  browserBinary,
  buildNeoRun,
  connectCdp,
  connectReference,
  findPage,
  launchBrowser,
  runCleanups,
  serveFixture,
  skip,
  sleep,
  startNeo,
  waitForUrl,
} from './harness.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const scriptPath = join(here, '../scripts/watch-url.js')
const STEP_MS = 300
const SLOW_RESPONSE_MS = 2500
const expected = {
  paths: ['/a', '/a#done', '/pushed', '/slow'],
  keptOldUrlWhileLoading: true,
  afterClose: '/slow',
}

test('watch-url.js returns the same value through Neo and through Playwright', {
  skip,
  timeout: 600_000,
}, async () => {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { slowMs: SLOW_RESPONSE_MS })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const { targetId } = await cdp.send('Target.createTarget', {
      url: `${origin}/a`,
    })
    const { sessionId: driver } = await cdp.send('Target.attachToTarget', {
      targetId,
      flatten: true,
    })
    await waitForUrl(cdp, driver, `${origin}/a`)

    const playwright = await connectReference(browser.port)
    cleanups.push(() => playwright.close())
    const [context] = playwright.contexts()
    const page = await findPage(context, targetId)

    const neo = startNeo(neoRun, browser.port, targetId, scriptPath)
    await neo.connected
    const AsyncFunction = (async () => {}).constructor
    const source = readFileSync(scriptPath, 'utf8')
    const viaPlaywright = new AsyncFunction('page', 'context', source)(
      page,
      context,
    )

    await driveTab(cdp, driver, origin, targetId)
    const [neoOutcome, playwrightValue] = await Promise.all([
      neo.outcome,
      viaPlaywright,
    ])

    assert.equal(neoOutcome.status, 'ok', JSON.stringify(neoOutcome))
    assert.deepEqual(playwrightValue, expected, 'Playwright')
    assert.deepEqual(neoOutcome.value, playwrightValue, 'Neo vs Playwright')
  } finally {
    await runCleanups(cleanups)
  }
})

/** The same steps as crates/neo-sdk/neo-runtime/tests/sync_getters.rs, STEP_MS apart. */
async function driveTab(cdp, session, origin, targetId) {
  const evaluate = async (expression) => {
    const reply = await cdp.send('Runtime.evaluate', { expression }, session)
    assert.equal(reply.exceptionDetails, undefined, `${expression} threw`)
  }
  await sleep(STEP_MS)
  await evaluate("location.hash = 'done'")
  await sleep(STEP_MS)
  await evaluate("history.pushState({}, '', '/pushed')")
  await sleep(STEP_MS)
  await cdp.send('Page.navigate', { url: `${origin}/slow` }, session)
  await waitForUrl(cdp, session, `${origin}/slow`)
  await sleep(STEP_MS)
  await cdp.send('Target.closeTarget', { targetId })
}
