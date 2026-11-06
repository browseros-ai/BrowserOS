// context.newPage() through Neo and through real Playwright on one BrowserOS neo. Each engine runs
// tests/scripts/newPage.js on a fresh /run.html: two new pages, distinct objects on about:blank,
// each with its own main frame, listed after the existing pages in creation order. The test then
// reads the tabs over its own CDP client: two new about:blank tabs in the run tab's window.
//
// One difference is intended and asserted: Playwright opens a new page in front, so the second
// new tab ends up active; Neo opens it in the background, so the run's tab stays active.
// Neo's RunOutcome.pages names both new tabs as created by the run, which is how a host learns
// their ids.
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
const scriptPath = join(here, '../scripts/newPage.js')
const tab = readFileSync(join(here, '../pages/tab.html'), 'utf8')
const expected = {
  urls: ['about:blank', 'about:blank'],
  isClosed: [false, false],
  distinct: true,
  context: true,
  ownMainFrames: true,
  crossFrameThrows: true,
  appendedInOrder: true,
  othersKept: true,
}

test(`context.newPage() opens the same tabs through Neo and Playwright ${playwrightVersion}`, {
  skip,
  timeout: 600_000,
}, async () => {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { pages: { '/run.html': tab } })
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
    for (const [side, { opened }] of [
      ['Playwright', viaPlaywright],
      ['Neo', viaNeo],
    ]) {
      assert.deepEqual(
        opened.map(({ url, inRunWindow }) => [url, inRunWindow]),
        [
          ['about:blank', true],
          ['about:blank', true],
        ],
        `${side}: two new about:blank tabs in the run tab's window`,
      )
    }
    assert.equal(viaPlaywright.active, 'second new tab', 'Playwright: in front')
    assert.equal(viaNeo.active, 'run tab', 'Neo: in the background')
    assert.deepEqual(
      viaNeo.result.outcome.pages
        .filter(({ created }) => created)
        .map(({ id, closed }) => [id, closed]),
      viaNeo.opened.map(({ targetId }) => [targetId, false]),
      'RunOutcome.pages names the new tabs',
    )
  } finally {
    await runCleanups(cleanups)
  }
})

/**
 * One engine's turn on a fresh /run.html: runs the script on it, then reads the tabs it opened,
 * in tab order, and which tab is active, and closes them all.
 */
async function half(cdp, origin, run) {
  const runTab = await openTab(cdp, `${origin}/run.html`)
  const before = new Set((await tabs(cdp)).map(({ targetId }) => targetId))
  const result = await run(runTab.targetId)
  const now = await tabs(cdp)
  const runInfo = now.find(({ targetId }) => targetId === runTab.targetId)
  const opened = now
    .filter(({ targetId }) => !before.has(targetId))
    .map(({ targetId, url, windowId }) => ({
      targetId,
      url,
      inRunWindow: windowId === runInfo?.windowId,
    }))
  const activeTab = now.find(({ isActive }) => isActive)?.targetId
  const active =
    activeTab === runTab.targetId
      ? 'run tab'
      : (['first new tab', 'second new tab'][
          opened.findIndex(({ targetId }) => targetId === activeTab)
        ] ?? 'another tab')
  for (const { targetId } of [...opened, runTab])
    await cdp.send('Target.closeTarget', { targetId })
  return { result, opened, active }
}

/** The browser's tabs in tab order, as BrowserOS lists them. */
async function tabs(cdp) {
  return (await cdp.send('Browser.getTabs', {})).tabs
}
