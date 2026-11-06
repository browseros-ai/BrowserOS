// page.waitForLoadState through the pinned Playwright and through Neo, each case on fresh tabs:
// every state of a loaded page; a page whose image answers late, which reaches commit and
// domcontentloaded long before load (a short timeout on load times out meanwhile); a page whose
// parser-blocking script answers late, which holds domcontentloaded back (a short timeout on it
// times out meanwhile); and states refused at once. Compared: what each call settled to (`undefined` or its error's kind,
// `waits.mjs` `settle`) and the title the page's own DOMContentLoaded and load handlers set.
// Never wording, call logs or timing.
//
// Known, intended difference, asserted per engine and not counted as parity: Neo refuses
// `networkidle` with an `Error` (neo-runtime `waits.rs`), where Playwright resolves.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, waits.mjs).

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { withBoth } from './waits.mjs'

const RESOLVED = 'undefined'
const ERROR = { error: 'error' }
/** `/slow` answers this late: slow-load.html's image, so its load trails its commit, and
 * dcl-blocked.html's script, so its domcontentloaded does. */
const SLOW_MS = 3000

const cases = [
  {
    name: 'a loaded page has every state',
    path: '/loaded.html',
    source: `return {
      load: await settle(page.waitForLoadState()),
      domcontentloaded: await settle(page.waitForLoadState('domcontentloaded')),
      commit: await settle(page.waitForLoadState('commit')),
      frame: await settle(page.mainFrame().waitForLoadState('load')),
    }`,
    expected: {
      load: RESOLVED,
      domcontentloaded: RESOLVED,
      commit: RESOLVED,
      frame: RESOLVED,
    },
  },
  {
    // The script starts once the page has committed, seconds before its image answers.
    name: 'a loading page reaches domcontentloaded, then load',
    path: '/slow-load.html',
    loaded: false,
    source: `const early = await settle(page.waitForLoadState('load', { timeout: 300 }))
    const commit = await settle(page.waitForLoadState('commit'))
    const domcontentloaded = await settle(page.waitForLoadState('domcontentloaded'))
    const titleAtDomContentLoaded = await page.title()
    const load = await settle(page.waitForLoadState())
    return {
      early,
      commit,
      domcontentloaded,
      titleAtDomContentLoaded,
      load,
      titleAtLoad: await page.title(),
    }`,
    expected: {
      early: { error: 'timeout' },
      commit: RESOLVED,
      domcontentloaded: RESOLVED,
      titleAtDomContentLoaded: 'domcontentloaded',
      load: RESOLVED,
      titleAtLoad: 'load',
    },
  },
  {
    // The script starts once the page has committed, while its parser waits for `/slow`. The
    // title before is the document's own; DOMContentLoaded's handler renames it.
    name: 'a page held before domcontentloaded reaches it later',
    path: '/dcl-blocked.html',
    loaded: false,
    source: `const early = await settle(page.waitForLoadState('domcontentloaded', { timeout: 300 }))
    const titleBefore = await page.title()
    const domcontentloaded = await settle(page.waitForLoadState('domcontentloaded'))
    return {
      early,
      titleBefore,
      domcontentloaded,
      titleAtDomContentLoaded: await page.title(),
    }`,
    expected: {
      early: { error: 'timeout' },
      titleBefore: 'committed',
      domcontentloaded: RESOLVED,
      titleAtDomContentLoaded: 'domcontentloaded',
    },
  },
  {
    name: 'refused at once',
    path: '/loaded.html',
    source: `return {
      unknown: await settle(page.waitForLoadState('idle')),
      nullState: await settle(page.waitForLoadState(null)),
    }`,
    expected: { unknown: ERROR, nullState: ERROR },
  },
  {
    name: 'Neo limit: networkidle',
    path: '/loaded.html',
    source: `return {
      networkidle: await settle(page.waitForLoadState('networkidle', { timeout: 5000 })),
    }`,
    expected: { networkidle: RESOLVED },
    neoExpected: { networkidle: ERROR },
  },
]

test('page.waitForLoadState settles the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  const page = (file) =>
    readFileSync(new URL(`../pages/${file}`, import.meta.url), 'utf8')
  const pages = {
    '/slow-load.html': page('slow-load.html'),
    '/dcl-blocked.html': page('dcl-blocked.html'),
    // A script for dcl-blocked.html; slow-load.html's image only needs the response to end.
    '/slow': "document.documentElement.dataset.slow = 'answered'",
  }
  await withBoth(t, { pages, slowMs: SLOW_MS }, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run(c)
        assert.deepEqual(playwright.result, { value: c.expected }, 'Playwright')
        if (c.neoExpected)
          assert.deepEqual(neo.result, { value: c.neoExpected }, 'Neo limit')
        else
          assert.deepEqual(neo.result, playwright.result, 'Neo vs Playwright')
      })
    }
  })
})
