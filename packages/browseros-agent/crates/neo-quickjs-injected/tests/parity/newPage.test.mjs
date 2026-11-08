// context.newPage() through Neo and through real Playwright on one BrowserOS neo. Each engine runs
// tests/scripts/newPage.js on a fresh /run.html: two new pages, distinct objects on about:blank,
// each with its own main frame, listed after the existing pages in creation order. The script
// then drives them, clicking `second`, bringing it to the front and closing `first`, and opens
// a third page. The test reads the tabs over its own CDP client: `first` gone, `second` and
// `third` open on about:blank in the run tab's window. For Neo it also checks that each call
// went to the tab of the page it went through, by the guids in RunOutcome.calls and the tab ids
// in RunOutcome.pages.
//
// One difference is intended and asserted: Playwright opens a new page in front, so `third`
// ends up active; Neo opens it in the background, so `second`, brought to the front, stays
// active.
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
  launchReference,
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
  voidResults: [true, true, true],
  firstIsClosed: true,
  listedAtEnd: [false, true, true],
}

test(`context.newPage() opens the same tabs through Neo and Playwright ${playwrightVersion}`, {
  skip,
  timeout: 600_000,
}, async () => {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { pages: { '/run.html': tab } })
    const { playwright, cdp, ...browser } = await launchReference(
      cleanups,
      browserBinary,
      { cdp: true },
    )
    const source = readFileSync(scriptPath, 'utf8')

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
        `${side}: second and third open on about:blank in the run tab's window`,
      )
    }
    assert.equal(viaPlaywright.active, 'third', 'Playwright: newPage in front')
    assert.equal(viaNeo.active, 'second', 'Neo: newPage in the background')

    const { pages, calls } = viaNeo.result.outcome
    const [first, second, third] = pages.filter(({ created }) => created)
    assert.deepEqual(
      [first, second, third].map(({ id, closed }) => [id, closed]),
      [
        [first.id, true],
        [viaNeo.opened[0].targetId, false],
        [viaNeo.opened[1].targetId, false],
      ],
      'RunOutcome.pages: first closed, second and third the open new tabs',
    )
    assert.deepEqual(
      calls.map(({ method, guid }) => `${method} ${guid}`),
      [
        'newPage context@default',
        'newPage context@default',
        `click frame@${second.id}`,
        `bringToFront page@${second.id}`,
        `close page@${first.id}`,
        'newPage context@default',
      ],
      'each call went to the tab of the page it went through',
    )
  } finally {
    await runCleanups(cleanups)
  }
})

/**
 * One engine's turn on a fresh /run.html: runs the script on it, then reads the new tabs still
 * open, in tab order (`second`, `third`), and which tab is active, and closes them all.
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
      : (['second', 'third'][
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
