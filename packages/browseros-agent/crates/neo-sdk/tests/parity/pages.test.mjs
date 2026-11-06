// context.pages() through Neo and through real Playwright on one BrowserOS neo. Each engine runs
// tests/scripts/pages.js on a fresh /run.html beside a fresh /other.html, while this test opens
// and closes a third tab under it. Both must list the same tabs, as the same objects every time,
// with each page's click reaching its own tab, and follow a tab opened and closed mid-run.
//
// One difference is asserted, not hidden: the order of the tabs open before the script. Neo
// lists them in tab-strip order (Browser.getTabs). Playwright lists them in the order CDP
// attached them when it connected, which changes from run to run, so only their set is compared.
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
  eventually,
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
const scriptPath = join(here, '../scripts/pages.js')
const tab = readFileSync(join(here, '../pages/tab.html'), 'utf8')
const expected = {
  isArray: true,
  runListed: true,
  sameObjects: true,
  openedAppended: true,
  othersKept: true,
  openedLeft: true,
  openedIsClosed: true,
}

test(`context.pages() reads the same through Neo and Playwright ${playwrightVersion}`, {
  skip,
  timeout: 600_000,
}, async () => {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: { '/run.html': tab, '/other.html': tab, '/opened.html': tab },
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

    const theirs = viaPlaywright.result.value
    const ours = viaNeo.result.value
    assert.ok(theirs, `Playwright: ${JSON.stringify(viaPlaywright.result)}`)
    assert.ok(ours, `Neo: ${JSON.stringify(viaNeo.result.outcome)}`)
    const { paths: theirPaths, ...theirChecks } = theirs
    const { paths: ourPaths, ...ourChecks } = ours
    assert.deepEqual(theirChecks, expected, 'Playwright')
    assert.deepEqual(ourChecks, theirChecks, 'Neo vs Playwright')
    assert.deepEqual(
      [...ourPaths].sort(),
      [...theirPaths].sort(),
      'the tabs listed, Neo vs Playwright',
    )
    assert.deepEqual(ourPaths, viaNeo.strip, 'Neo lists tabs in tab order')
    assert.deepEqual(
      [viaPlaywright.clicks, viaNeo.clicks],
      [
        [1, 1],
        [1, 1],
      ],
      "each page's click reached its own tab, once",
    )
  } finally {
    await runCleanups(cleanups)
  }
})

/**
 * One engine's turn on fresh tabs: opens /other.html then /run.html, starts `run` on the run
 * tab, and, in step with the script, opens /opened.html and later closes it. Returns what the
 * script settled with, the tab paths in tab order before it ran, and the clicks each of the
 * two first tabs counted. A script that ends early ends the stepping too.
 */
async function half(cdp, origin, run) {
  const other = await openTab(cdp, `${origin}/other.html`)
  const runTab = await openTab(cdp, `${origin}/run.html`)
  const { tabs } = await cdp.send('Browser.getTabs', {})
  const strip = tabs.map(({ url }) => url.replace(origin, ''))
  let settled = false
  const running = run(runTab.targetId).finally(() => {
    settled = true
  })

  const counted = async (...targets) => {
    const counts = await Promise.all(targets.map((t) => clicks(cdp, t)))
    return settled || counts.every((count) => count === 1)
  }
  await eventually(() => counted(runTab, other), 'both tabs counted a click')
  let opened
  if (!settled) {
    opened = await openTab(cdp, `${origin}/opened.html`)
    await eventually(() => counted(opened), '/opened.html counted a click')
    await cdp.send('Target.closeTarget', { targetId: opened.targetId })
  }
  const result = await running
  const finalClicks = [await clicks(cdp, runTab), await clicks(cdp, other)]
  // Every tab of this turn goes, so the next engine's turn starts from the same tabs. Closing
  // the one already closed fails, which is fine.
  for (const { targetId } of [other, runTab, opened].filter(Boolean))
    await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
  return { result, strip, clicks: finalClicks }
}

/** The clicks a tab's button counted, or `null` once the tab cannot answer. */
async function clicks(cdp, { sessionId }) {
  const reply = await cdp
    .send(
      'Runtime.evaluate',
      { expression: 'window.__clicks', returnByValue: true },
      sessionId,
    )
    .catch(() => null)
  return reply?.result.value ?? null
}
