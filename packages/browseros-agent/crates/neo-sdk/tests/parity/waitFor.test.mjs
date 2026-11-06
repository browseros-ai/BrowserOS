// locator.waitFor through the pinned Playwright and through Neo, each case on fresh tabs of
// tests/pages/wait-for.html: every state already met, met later by page timers, and never met
// (a timeout); calls refused at once (a strict mode violation, an invalid selector, an unknown
// state); and a wait that a click in the same script meets, which Neo's waits allow because they
// never take the tab's input. Compared: what each call settled to (`undefined` or its error's
// kind, `waits.mjs` `settle`) and what the page logged. Never wording, call logs, retry counts or
// timing.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, waits.mjs).

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { withBoth } from './waits.mjs'

const html = readFileSync(
  new URL('../pages/wait-for.html', import.meta.url),
  'utf8',
)
const RESOLVED = 'undefined'
const TIMEOUT = { error: 'timeout' }

const cases = [
  {
    name: 'each state already met',
    source: `return {
      attached: await settle(page.locator('#hidden').waitFor({ state: 'attached' })),
      visible: await settle(page.locator('#shown').waitFor()),
      hidden: await settle(page.locator('#hidden').waitFor({ state: 'hidden' })),
      hiddenWithoutMatch: await settle(page.locator('#missing').waitFor({ state: 'hidden' })),
      detached: await settle(page.locator('#missing').waitFor({ state: 'detached' })),
    }`,
    expected: {
      attached: RESOLVED,
      visible: RESOLVED,
      hidden: RESOLVED,
      hiddenWithoutMatch: RESOLVED,
      detached: RESOLVED,
    },
  },
  {
    name: 'each state met later by page timers',
    source: `await page.click('#arm')
    const [attached, detached, visible, hidden] = await Promise.all([
      settle(page.locator('#late').waitFor({ state: 'attached' })),
      settle(page.locator('#doomed').waitFor({ state: 'detached' })),
      settle(page.locator('#reveal').waitFor({ state: 'visible' })),
      settle(page.locator('#fade').waitFor({ state: 'hidden' })),
    ])
    return { attached, detached, visible, hidden }`,
    probe: 'window.__log',
    expected: {
      attached: RESOLVED,
      detached: RESOLVED,
      visible: RESOLVED,
      hidden: RESOLVED,
    },
    expectedProbe: ['armed', 'changed'],
  },
  {
    name: 'each state never met times out',
    source: `return {
      visible: await settle(page.locator('#hidden').waitFor({ timeout: 300 })),
      attached: await settle(page.locator('#missing').waitFor({ state: 'attached', timeout: 300 })),
      detached: await settle(page.locator('#shown').waitFor({ state: 'detached', timeout: 300 })),
      hidden: await settle(page.locator('#shown').waitFor({ state: 'hidden', timeout: 300 })),
    }`,
    expected: {
      visible: TIMEOUT,
      attached: TIMEOUT,
      detached: TIMEOUT,
      hidden: TIMEOUT,
    },
  },
  {
    // The 5 s timeouts only bound a regression: each of these fails before its first poll.
    name: 'refused at once',
    source: `return {
      strict: await settle(page.locator('.many').waitFor({ timeout: 5000 })),
      strictHidden: await settle(page.locator('.many').waitFor({ state: 'hidden', timeout: 5000 })),
      selector: await settle(page.locator('p >> nonsense=1').waitFor({ timeout: 5000 })),
      state: await settle(page.locator('#shown').waitFor({ state: 'gone', timeout: 5000 })),
    }`,
    expected: {
      strict: { error: 'strict' },
      strictHidden: { error: 'strict' },
      selector: { error: 'error' },
      state: { error: 'error' },
    },
  },
  {
    name: 'met by a click in the same script',
    source: `const added = settle(page.locator('#added').waitFor())
    const clicked = await settle(page.click('#add'))
    return { clicked, added: await added }`,
    probe: 'window.__log',
    expected: { clicked: RESOLVED, added: RESOLVED },
    expectedProbe: ['added'],
  },
]

test('locator.waitFor settles the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await withBoth(t, { pages: { '/wait-for.html': html } }, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run({ ...c, path: '/wait-for.html' })
        assert.deepEqual(playwright.result, { value: c.expected }, 'Playwright')
        assert.deepEqual(neo.result, playwright.result, 'Neo vs Playwright')
        assert.deepEqual(playwright.probe, c.expectedProbe, 'Playwright page')
        assert.deepEqual(neo.probe, playwright.probe, 'what the page saw')
      })
    }
  })
})
