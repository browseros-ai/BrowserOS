// context.close() through the pinned Playwright and through Neo, each on a fresh
// tests/pages/close-reason.html tab the test opens with its own CDP client, as a user's tab. Neo's
// context is what connectOverCDP gives Playwright, the browser's default context, and closing that
// closes no tab: Playwright's server only drops its CDP connection
// (server/chromium/crBrowser.ts `doClose` returns 'close-browser', which for connectOverCDP
// closes the socket). So close() resolves, every tab stays open with its document, even one the
// script opened, and the run is done with them all: isClosed() is true from the call, each page
// the context holds fires 'close' and then the context's 'pageclose', the context fires 'close'
// with itself, then close() resolves; after it pages() is empty, every page is closed, a later
// call rejects with the default closed message, and a second close() resolves. Pending waits and
// a click in flight reject with the reason close() was given.
//
// Playwright's connection is gone after the close, so this file runs each case in a turn of its
// own (`turn`) instead of events.mjs: the test reads over its own CDP client whether the case's
// tab is still open, how many tabs the script opened are, and `probe` on the tab, then closes
// them all. Rejections compare by `errorCategory`, and by whether the message names the reason,
// never by wording. NEO_PARITY_REFERENCE_ONLY=1 runs only Playwright's turn.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { fixture } from './events.mjs'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  errorCategory,
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

const referenceOnly = Boolean(process.env.NEO_PARITY_REFERENCE_ONLY)

const PRELUDE = `const errorCategory = ${errorCategory.toString()};
const settle = (promise) => promise.then(
  (value) => (value === undefined ? 'undefined' : { value }),
  (error) => ({ error: errorCategory(error) }),
);
`

const cases = [
  {
    // tests/library/browsercontext-basic.spec.ts "BC-011 close() should work for empty
    // context"; docs/src/api/class-browsercontext.md `BrowserContext.close` ("The default
    // browser context cannot be closed": over connectOverCDP, close() lets go of it instead).
    name: 'close() resolves and leaves the tab open with its document',
    source: `await page.click('#count')
    const closed = await context.close()
    return {
      resolved: closed === undefined,
      closed: [context.isClosed(), page.isClosed()],
      pages: context.pages().length,
    }`,
    expected: {
      resolved: true,
      closed: [true, true],
      pages: 0,
    },
    effect: { tabOpen: true, openedStillOpen: 0, probe: 1 },
  },
  {
    // browsercontext-basic.spec.ts "BC-013 close() should be callable twice", "BC-014 should
    // pass self to close event", "BC-017 should close all belonging pages once closing context".
    name: "each page fires 'close' then 'pageclose', then the context 'close', then close() resolves",
    source: `await page.click('#count')
    const other = await context.newPage()
    const name = (closing) => (closing === page ? 'page' : closing === other ? 'other' : null)
    const order = []
    for (const closing of [page, other]) {
      closing.once('close', (closed) => order.push([name(closing), 'close', closed === closing, closing.isClosed()]))
    }
    context.on('pageclose', (closed) => {
      if (name(closed)) order.push([name(closed), 'pageclose', context.pages().includes(closed)])
    })
    context.once('close', (closed) => order.push(['context', 'close', closed === context, context.isClosed()]))
    const waitedClose = context.waitForEvent('close')
    const closing = context.close({ reason: 'context bye' }).then((value) => order.push(['close() resolved', value === undefined]))
    const atCall = { context: context.isClosed(), pages: [page.isClosed(), other.isClosed()] }
    await closing
    return {
      atCall,
      order,
      waitedClose: (await waitedClose) === context,
      after: { pages: context.pages().length, closed: [page.isClosed(), other.isClosed(), context.isClosed()] },
      later: { title: await settle(page.title()), newPage: await settle(context.newPage()) },
      closeAgain: await settle(context.close()),
    }`,
    // Pages close in pages() order, the run's tab before the one newPage() opened, which stays
    // open too.
    expected: {
      atCall: { context: true, pages: [false, false] },
      order: [
        ['page', 'close', true, true],
        ['page', 'pageclose', false],
        ['other', 'close', true, true],
        ['other', 'pageclose', false],
        ['context', 'close', true, true],
        ['close() resolved', true],
      ],
      waitedClose: true,
      after: { pages: 0, closed: [true, true, true] },
      later: { title: { error: 'Error' }, newPage: { error: 'Error' } },
      closeAgain: 'undefined',
    },
    effect: { tabOpen: true, openedStillOpen: 1, probe: 1 },
  },
  {
    // browsercontext-basic.spec.ts "BC-012 close() should abort waitForEvent";
    // class-browsercontext.md `BrowserContext.close.reason` ("The reason to be reported to the
    // operations interrupted by the context closure").
    name: 'pending waits and a click in flight reject with the reason; a later call does not',
    source: `await page.click('#count')
    const reason = 'context bye'
    const outcome = (promise) => promise.then(
      () => 'resolved',
      (error) => ({ error: errorCategory(error), hasReason: error.message.includes(reason) }),
    )
    const pageWait = outcome(page.waitForEvent('console'))
    const contextWait = outcome(context.waitForEvent('page'))
    const click = outcome(page.locator('#hidden').click())
    // Answered after the click has started: each engine takes a page's calls in order.
    const hidden = await page.isHidden('#hidden')
    await context.close({ reason })
    return {
      hidden,
      pageWait: await pageWait,
      contextWait: await contextWait,
      click: await click,
      later: await outcome(page.title()),
    }`,
    // Playwright's TargetClosedError keeps the name Error.
    expected: {
      hidden: true,
      pageWait: { error: 'Error', hasReason: true },
      contextWait: { error: 'Error', hasReason: true },
      click: { error: 'Error', hasReason: true },
      later: { error: 'Error', hasReason: false },
    },
    effect: { tabOpen: true, openedStillOpen: 0, probe: 1 },
  },
]

test('context.close() lets go of every tab the same way through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  if (referenceOnly) t.diagnostic('NEO_PARITY_REFERENCE_ONLY: Playwright only')
  const neoRun = referenceOnly ? undefined : buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: { '/close-reason.html': fixture('close-reason.html') },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-context-close-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const url = `${origin}/close-reason.html`
    const probe = 'window.__clicks'
    let next = 1
    for (const c of cases) {
      await t.test(c.name, async () => {
        const source = PRELUDE + c.source
        const playwright = await turn(cdp, url, probe, (targetId) =>
          viaPlaywright(browser.port, targetId, source),
        )
        assert.deepEqual(playwright.result, { value: c.expected }, 'Playwright')
        assert.deepEqual(playwright.effect, c.effect, "Playwright's tabs")
        if (referenceOnly) return
        const scriptPath = join(scripts, `case-${next++}.js`)
        writeFileSync(scriptPath, source)
        const neo = await turn(cdp, url, probe, (targetId) =>
          runInNeo(neoRun, browser.port, targetId, scriptPath),
        )
        assert.deepEqual(
          neo.result,
          playwright.result,
          `Neo vs Playwright: ${JSON.stringify(neo.outcome)}`,
        )
        assert.deepEqual(neo.effect, playwright.effect, "Neo's tabs")
      })
    }
  } finally {
    await runCleanups(cleanups)
  }
})

/** Runs the script as Playwright does on the tab, then lets go of the browser if it still holds it. */
async function viaPlaywright(port, targetId, source) {
  const playwright = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
  try {
    const [context] = playwright.contexts()
    return await runInPlaywright(context, targetId, source)
  } finally {
    await playwright.close()
  }
}

/**
 * Opens the case's tab and runs `run` on it once it has loaded; then reads over the test's CDP
 * client whether the tab is still open, how many tabs the run opened still are, and `probe` on
 * the tab, and closes them all. `run` settles as runInNeo does.
 */
async function turn(cdp, url, probe, run) {
  const before = await pageTargets(cdp)
  const { targetId, sessionId } = await openTab(cdp, url)
  const opened = []
  try {
    await eventually(
      async () =>
        (await evaluate(cdp, sessionId, 'document.readyState')) === 'complete',
      `${url} loaded`,
    )
    const { outcome, ...result } = await run(targetId)
    const open = await pageTargets(cdp)
    opened.push(...open.filter((id) => id !== targetId && !before.includes(id)))
    const tabOpen = open.includes(targetId)
    const effect = {
      tabOpen,
      openedStillOpen: opened.length,
      probe: tabOpen ? await evaluate(cdp, sessionId, probe) : undefined,
    }
    return { result, outcome, effect }
  } finally {
    for (const id of [...opened, targetId]) {
      await cdp.send('Target.closeTarget', { targetId: id }).catch(() => {})
    }
  }
}

async function pageTargets(cdp) {
  const { targetInfos } = await cdp.send('Target.getTargets')
  return targetInfos
    .filter((target) => target.type === 'page')
    .map((target) => target.targetId)
}

async function evaluate(cdp, sessionId, expression) {
  const { result } = await cdp.send(
    'Runtime.evaluate',
    { expression, returnByValue: true },
    sessionId,
  )
  return result.value
}
