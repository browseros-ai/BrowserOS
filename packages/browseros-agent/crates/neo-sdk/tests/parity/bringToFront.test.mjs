// page.bringToFront() through Neo and through real Playwright on one BrowserOS neo. Each engine
// runs tests/scripts/bringToFront.js on a fresh /run.html, the active tab, beside fresh
// /front.html and /gone.html: bringToFront resolves to undefined, again on the same tab, and
// rejects on a closed page; a click through the page brought forward reaches its own tab.
//
// One difference is asserted, not hidden. Playwright sends only CDP Page.bringToFront, which
// headless BrowserOS accepts without selecting the tab, so the run's tab stays active. Neo then
// selects it (Browser.activateTab), so /front.html ends up active, as bringToFront promises.
//
// Errors compare by name and by whether they say the page is closed, not by their wording.
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
  chromium,
  connectCdp,
  launchBrowser,
  openTab,
  playwrightVersion,
  runCleanups,
  runInNeo,
  runInPlaywright,
  serveFixture,
  skip,
} from './harness.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const scriptPath = join(here, '../scripts/bringToFront.js')
const tab = readFileSync(join(here, '../pages/tab.html'), 'utf8')
/** In the order the test opens them; the run's tab comes last, so it starts active. */
const TABS = ['front', 'gone', 'run']
const expected = {
  returnsUndefined: true,
  againReturnsUndefined: true,
  whenClosed: { name: 'Error', saysClosed: true },
  frontIsClosed: false,
}

test(`page.bringToFront() reads the same through Neo and Playwright ${playwrightVersion}`, {
  skip,
  timeout: 600_000,
}, async () => {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: Object.fromEntries(TABS.map((name) => [`/${name}.html`, tab])),
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const source = readFileSync(scriptPath, 'utf8')

    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    const [context] = playwright.contexts()
    const viaPlaywright = await half(cdp, origin, (target) =>
      runInPlaywright(context, target, source),
    )
    await playwright.close()
    const viaNeo = await half(cdp, origin, (target) =>
      runInNeo(neoRun, browser.port, target, scriptPath),
    )

    assert.deepEqual(viaPlaywright.result.value, expected, 'Playwright')
    assert.deepEqual(
      viaNeo.result.value,
      viaPlaywright.result.value,
      `Neo vs Playwright: ${JSON.stringify(viaNeo.result.outcome)}`,
    )
    for (const [side, { clicks }] of [
      ['Playwright', viaPlaywright],
      ['Neo', viaNeo],
    ]) {
      assert.deepEqual(
        clicks,
        { front: 1, run: 0 },
        `${side}: the click reached /front.html`,
      )
    }
    assert.deepEqual(
      [viaPlaywright.activeBefore, viaNeo.activeBefore],
      ['run', 'run'],
      'the run tab is active when the script starts',
    )
    assert.equal(viaPlaywright.active, 'run', 'Playwright: the tab strip kept')
    assert.equal(viaNeo.active, 'front', 'Neo: /front.html selected')
  } finally {
    await runCleanups(cleanups)
  }
})

/**
 * One engine's turn on fresh tabs: runs the script on /run.html and reads which tab was active
 * before and after it, and the clicks /front.html and /run.html counted; then closes the tabs.
 */
async function half(cdp, origin, run) {
  const tabs = {}
  for (const name of TABS)
    tabs[name] = await openTab(cdp, `${origin}/${name}.html`)
  const activeBefore = await activeTab(cdp, tabs)
  const result = await run(tabs.run.targetId)
  const active = await activeTab(cdp, tabs)
  const clicks = {}
  for (const name of ['front', 'run']) {
    const { result: count } = await cdp.send(
      'Runtime.evaluate',
      { expression: 'window.__clicks', returnByValue: true },
      tabs[name].sessionId,
    )
    clicks[name] = count.value
  }
  // Every tab of this turn goes, so the next engine's turn starts from the same tabs; the script
  // already closed /gone.html unless it failed first.
  for (const { targetId } of Object.values(tabs))
    await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
  return { result, activeBefore, active, clicks }
}

/** The name of the active tab among `tabs`, by BrowserOS's tab strip. */
async function activeTab(cdp, tabs) {
  const { tabs: strip } = await cdp.send('Browser.getTabs', {})
  const { targetId } = strip.find(({ isActive }) => isActive) ?? {}
  return (
    Object.keys(tabs).find((name) => tabs[name].targetId === targetId) ??
    'another tab'
  )
}
