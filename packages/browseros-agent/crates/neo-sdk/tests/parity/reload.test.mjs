// page.reload through the pinned Playwright and through Neo, each case on fresh tabs of
// tests/pages/history.html (history.mjs): the new document's Response (Playwright's
// tests/page/page-history.spec.ts "NA-075 page.reload should work"), a page with a hash
// ("NA-081 page.reload should work on a page with a hash"), every waitUntil, a data: URL
// without a response ("NA-076 page.reload should work with data url"), a reload that fails, and
// an option refused at once. Compared: the script's values and each page's load count.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, history.mjs).

import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareCases } from './history.mjs'

const html = readFileSync(
  new URL('../pages/history.html', import.meta.url),
  'utf8',
)
/** How many documents the case's tab loaded from history.html, as its page counts them. */
const LOADS = "sessionStorage.getItem('loads')"

const cases = [
  {
    name: "reload answers the new document's response",
    path: '/history.html',
    source: `const response = await page.reload()
    return {
      response: describe(response),
      navigation: response.request().isNavigationRequest(),
      method: response.request().method(),
      frame: response.frame() === page.mainFrame(),
      path: path(),
    }`,
    expected: {
      response: { status: 200, ok: true, path: '/history.html' },
      navigation: true,
      method: 'GET',
      frame: true,
      path: '/history.html',
    },
    probe: LOADS,
  },
  {
    name: 'on a page with a hash, the response has none and the page keeps it',
    path: '/history.html#section',
    source: `const response = await page.reload()
    return { response: describe(response), path: path() }`,
    expected: {
      response: { status: 200, ok: true, path: '/history.html' },
      path: '/history.html#section',
    },
    probe: LOADS,
  },
  {
    name: 'every waitUntil answers the response',
    path: '/history.html',
    source: `return {
      commit: describe(await page.reload({ waitUntil: 'commit' })),
      domcontentloaded: describe(await page.reload({ waitUntil: 'domcontentloaded' })),
      load: describe(await page.reload({ waitUntil: 'load', timeout: 10000 })),
    }`,
    expected: {
      commit: { status: 200, ok: true, path: '/history.html' },
      domcontentloaded: { status: 200, ok: true, path: '/history.html' },
      load: { status: 200, ok: true, path: '/history.html' },
    },
    probe: LOADS,
  },
  {
    name: 'a data: URL reloads without a response',
    path: '/history.html',
    source: `await page.goto('data:text/html,<title>data</title>hello')
    return { response: await page.reload(), title: await page.title() }`,
    expected: { response: null, title: 'data' },
  },
  {
    name: 'a reload that fails rejects',
    path: '/history.html',
    source: `return {
      goto: await settle(page.goto('http://127.0.0.1:1/unsafe')),
      reload: await settle(page.reload()),
    }`,
    expected: { goto: { error: 'Error' }, reload: { error: 'Error' } },
  },
  {
    name: 'an unknown waitUntil is refused before anything reloads',
    path: '/history.html',
    source: `return { reload: await settle(page.reload({ waitUntil: 'idle' })) }`,
    expected: { reload: { error: 'Error' } },
    probe: LOADS,
  },
]

test('page.reload answers the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareCases(t, cases, { pages: { '/history.html': html } })
})
