// page.waitForURL through the pinned Playwright and through Neo, each case on fresh tabs of
// tests/pages/wait-for-url.html: the current URL matching as an exact URL, a glob, a RegExp,
// `''`, a URL whose scheme needs normalizing and one without its trailing slash; future
// navigations a click starts (a new document, a hash change, history.pushState); `waitUntil`
// commit, domcontentloaded and load on a slow page; a timeout; and calls refused at once.
// Compared: what each call settled to (`undefined` or its error's kind, `waits.mjs` `settle`),
// the URL and title the script read afterwards. Never wording, call logs or timing.
//
// Known, intended differences, asserted per engine and not counted as parity: Neo refuses a
// predicate function and `waitUntil: 'networkidle'` with an `Error` (neo-sdk `neo-commands.ts`,
// neo-runtime `waits.rs`), where Playwright resolves. Neo also does not reject on a failed
// navigation (it tracks no network), which no case here exercises.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, waits.mjs).

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { withBoth } from './waits.mjs'

const page = (file) =>
  readFileSync(new URL(`../pages/${file}`, import.meta.url), 'utf8')
const RESOLVED = 'undefined'
const ERROR = { error: 'error' }
/** `/slow` (the slow page's image) answers this late, so its load trails its commit. */
const SLOW_MS = 3000
/** The script's own path of `page.url()`: both engines serve one origin, but only the path is
 * the case's. */
const PATH = `page.url().replace(/^[a-z]+:\\/\\/[^/]+/, '')`

const cases = [
  {
    name: 'the current URL already matches',
    source: `const url = page.url()
    return {
      exact: await settle(page.waitForURL(url)),
      glob: await settle(page.waitForURL('**/wait-for-url.html')),
      regexp: await settle(page.waitForURL(/wait-for-url\\.html$/)),
      everything: await settle(page.waitForURL('')),
      upperCaseScheme: await settle(page.waitForURL(url.replace('http:', 'HTTP:'))),
      commit: await settle(page.waitForURL(url, { waitUntil: 'commit' })),
      domcontentloaded: await settle(page.waitForURL(url, { waitUntil: 'domcontentloaded' })),
    }`,
    expected: {
      exact: RESOLVED,
      glob: RESOLVED,
      regexp: RESOLVED,
      everything: RESOLVED,
      upperCaseScheme: RESOLVED,
      commit: RESOLVED,
      domcontentloaded: RESOLVED,
    },
  },
  {
    name: 'a click navigates to a matching URL',
    source: `const waited = settle(page.waitForURL('**/done.html'))
    await page.click('#to-done')
    return { waited: await waited, path: ${PATH}, title: await page.title() }`,
    expected: { waited: RESOLVED, path: '/done.html', title: '/done.html' },
  },
  {
    name: 'an origin without its trailing slash matches the root',
    source: `const origin = page.url().replace(/\\/wait-for-url\\.html$/, '')
    const waited = settle(page.waitForURL(origin))
    await page.click('#to-root')
    return { waited: await waited, path: ${PATH} }`,
    expected: { waited: RESOLVED, path: '/' },
  },
  {
    name: 'a hash change matches',
    source: `const waited = settle(page.waitForURL(/#section$/))
    await page.click('#to-hash')
    return { waited: await waited, path: ${PATH} }`,
    expected: { waited: RESOLVED, path: '/wait-for-url.html#section' },
  },
  {
    name: 'history.pushState matches',
    source: `const waited = settle(page.waitForURL('**/pushed'))
    await page.click('#push')
    return { waited: await waited, path: ${PATH} }`,
    expected: { waited: RESOLVED, path: '/pushed' },
  },
  {
    name: 'waitUntil commit, domcontentloaded and load on a slow page',
    source: `const committed = page
      .waitForURL('**/slow-load.html', { waitUntil: 'commit' })
      .then(() => page.title())
    await page.click('#to-slow', { noWaitAfter: true })
    const titleAtCommit = await committed
    await page.waitForURL('**/slow-load.html', { waitUntil: 'domcontentloaded' })
    const titleAtDomContentLoaded = await page.title()
    const load = await settle(page.waitForURL('**/slow-load.html'))
    return {
      loadedAtCommit: titleAtCommit === 'load',
      titleAtDomContentLoaded,
      load,
      titleAtLoad: await page.title(),
    }`,
    expected: {
      loadedAtCommit: false,
      titleAtDomContentLoaded: 'domcontentloaded',
      load: RESOLVED,
      titleAtLoad: 'load',
    },
  },
  {
    name: 'a URL never reached times out',
    source: `return {
      glob: await settle(page.waitForURL('**/never.html', { timeout: 300 })),
      regexp: await settle(page.waitForURL(/never/, { timeout: 300 })),
    }`,
    expected: { glob: { error: 'timeout' }, regexp: { error: 'timeout' } },
  },
  {
    // The 5 s timeout only bounds a regression: each of these fails before it waits.
    name: 'refused at once',
    source: `return {
      notAUrlMatch: await settle(page.waitForURL(42)),
      invalidGlob: await settle(page.waitForURL('**/{a', { timeout: 5000 })),
      waitUntil: await settle(page.waitForURL('**', { waitUntil: 'idle' })),
    }`,
    expected: { notAUrlMatch: ERROR, invalidGlob: ERROR, waitUntil: ERROR },
  },
  {
    name: 'Neo limits: a predicate function and networkidle',
    source: `return {
      predicate: await settle(
        page.waitForURL((url) => url.pathname.endsWith('/wait-for-url.html'), { timeout: 5000 }),
      ),
      networkidle: await settle(
        page.waitForURL('**/wait-for-url.html', { waitUntil: 'networkidle', timeout: 5000 }),
      ),
    }`,
    expected: { predicate: RESOLVED, networkidle: RESOLVED },
    neoExpected: { predicate: ERROR, networkidle: ERROR },
  },
]

test('page.waitForURL settles the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  const pages = {
    '/wait-for-url.html': page('wait-for-url.html'),
    '/slow-load.html': page('slow-load.html'),
  }
  await withBoth(t, { pages, slowMs: SLOW_MS }, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run({
          ...c,
          path: '/wait-for-url.html',
        })
        assert.deepEqual(playwright.result, { value: c.expected }, 'Playwright')
        if (c.neoExpected)
          assert.deepEqual(neo.result, { value: c.neoExpected }, 'Neo limit')
        else
          assert.deepEqual(neo.result, playwright.result, 'Neo vs Playwright')
      })
    }
  })
})
