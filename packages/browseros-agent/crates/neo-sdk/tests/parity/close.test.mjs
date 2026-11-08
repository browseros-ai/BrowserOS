// page.close() through Neo and through real Playwright on one BrowserOS neo. Each engine runs
// tests/scripts/close.js on a fresh /run.html beside three fresh tabs, one per kind of close:
// close() resolves once the tab is gone, again on a closed page, and runs no beforeunload
// handler; runBeforeUnload (any truthy value) runs them and closes the tab without waiting for
// it, and rejects on a closed page; an invalid reason rejects and leaves the tab open. The test
// then reads over its own CDP client which tabs are gone and which left a beforeunload mark.
//
// Neo also reports the tabs its script closed in RunOutcome.pages, and a second run continues
// on the tab that survived, by the id the first run reported.
//
// Errors compare by name and by whether they say the page is closed, not by their wording.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import {
  browserBinary,
  buildNeoRun,
  eventually,
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
const scriptPath = join(here, '../scripts/close.js')
const tab = readFileSync(join(here, '../pages/tab.html'), 'utf8')
/** In the order the test opens them; the run's tab comes last, so it is the active one. */
const TABS = ['closed', 'unloaded', 'kept', 'run']
const expected = {
  closeReturnsUndefined: true,
  closedIsClosed: true,
  closedListed: false,
  closedUrl: '/closed.html',
  closeAgainReturnsUndefined: true,
  runBeforeUnloadWhenClosed: { name: 'Error', saysClosed: true },
  badReason: { name: 'Error', saysClosed: false },
  keptIsClosed: false,
  runBeforeUnloadReturnsUndefined: true,
  unloadedIsClosed: true,
  listedAfter: [true, true, false, false],
}

test(`page.close() closes the same tabs through Neo and Playwright ${playwrightVersion}`, {
  skip,
  timeout: 600_000,
}, async () => {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: Object.fromEntries(TABS.map((name) => [`/${name}.html`, tab])),
    })
    const { playwright, cdp, ...browser } = await launchReference(
      cleanups,
      browserBinary,
      { cdp: true },
    )
    const scripts = mkdtempSync(join(tmpdir(), 'neo-parity-close-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const survivorScript = join(scripts, 'survivor.js')
    writeFileSync(
      survivorScript,
      'return [page.url(), page.isClosed(), context.pages().includes(page)]',
    )
    const source = readFileSync(scriptPath, 'utf8')

    const [context] = playwright.contexts()
    const viaPlaywright = await half(cdp, origin, (target) =>
      runInPlaywright(context, target, source),
    )
    await playwright.close()
    const viaNeo = await half(
      cdp,
      origin,
      (target) => runInNeo(neoRun, browser.port, target, scriptPath),
      ({ kept }) =>
        runInNeo(neoRun, browser.port, kept.targetId, survivorScript),
    )

    assert.deepEqual(viaPlaywright.result.value, expected, 'Playwright')
    assert.deepEqual(
      viaNeo.result.value,
      viaPlaywright.result.value,
      `Neo vs Playwright: ${JSON.stringify(viaNeo.result.outcome)}`,
    )
    assert.deepEqual(
      viaPlaywright.open,
      { closed: false, unloaded: false, kept: true, run: true },
      'Playwright: the tabs still open',
    )
    assert.deepEqual(
      viaNeo.open,
      viaPlaywright.open,
      'Neo: the tabs still open',
    )
    assert.deepEqual(
      viaPlaywright.marks,
      ['beforeunload /unloaded.html'],
      'Playwright: only runBeforeUnload ran beforeunload',
    )
    assert.deepEqual(viaNeo.marks, viaPlaywright.marks, 'Neo: beforeunload')

    const reported = new Map(
      viaNeo.result.outcome.pages.map(({ id, created, closed }) => [
        id,
        { created, closed },
      ]),
    )
    assert.deepEqual(
      TABS.map((name) => reported.get(viaNeo.tabs[name].targetId)),
      [
        { created: false, closed: true },
        { created: false, closed: true },
        { created: false, closed: false },
        { created: false, closed: false },
      ],
      'RunOutcome.pages: the tabs the run closed',
    )
    assert.deepEqual(
      viaNeo.extra.value,
      [`${origin}/kept.html`, false, true],
      `a second run continues on the surviving tab: ${JSON.stringify(viaNeo.extra)}`,
    )
  } finally {
    await runCleanups(cleanups)
  }
})

/**
 * One engine's turn on fresh tabs: clears the origin's beforeunload marks, runs the script on
 * /run.html, and reads which tabs are still open and which marks the closes left. `afterRun`
 * runs before the open tabs are closed again; its result is `extra`.
 */
async function half(cdp, origin, run, afterRun = async () => undefined) {
  const tabs = {}
  for (const name of TABS)
    tabs[name] = await openTab(cdp, `${origin}/${name}.html`)
  const { sessionId } = tabs.run
  await evaluate(cdp, sessionId, 'localStorage.clear()')
  const result = await run(tabs.run.targetId)

  const openIds = async () =>
    new Set(
      (await cdp.send('Browser.getTabs', {})).tabs.map(
        ({ targetId }) => targetId,
      ),
    )
  const ids = await openIds()
  const open = Object.fromEntries(
    TABS.map((name) => [name, ids.has(tabs[name].targetId)]),
  )
  // Another tab's localStorage write can take a moment to reach this tab. A mark that never
  // arrives is not an error here: the caller compares the marks themselves.
  const marks = async () =>
    JSON.parse(
      await evaluate(
        cdp,
        sessionId,
        'JSON.stringify(Object.keys(localStorage).sort())',
      ),
    )
  await eventually(
    async () => open.unloaded || (await marks()).length > 0,
    'a beforeunload mark arrived',
    5_000,
  ).catch(() => {})
  const left = await marks()
  const extra = await afterRun(tabs)
  for (const id of await openIds()) {
    if (TABS.some((name) => tabs[name].targetId === id))
      await cdp.send('Target.closeTarget', { targetId: id })
  }
  return { result, tabs, open, marks: left, extra }
}

async function evaluate(cdp, sessionId, expression) {
  const { result, exceptionDetails } = await cdp.send(
    'Runtime.evaluate',
    { expression, returnByValue: true },
    sessionId,
  )
  assert.equal(exceptionDetails, undefined, `${expression} threw`)
  return result.value
}
