// page.goForward through the pinned Playwright and through Neo, each case on fresh tabs of
// tests/pages/history.html (history.mjs): forward to a later document and its response, then no
// later entry (Playwright's tests/page/page-history.spec.ts "NA-071 page.goBack should work"),
// forward to a history API entry ("NA-072 page.goBack should work with HistoryAPI"),
// `waitUntil: 'domcontentloaded'`, and an option refused at once. Compared: the script's values,
// the URL it ended on, and each page's load count.
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
const LOADS = "sessionStorage.getItem('loads')"

const cases = [
  {
    name: 'forward to a later document answers its response, then null at the end',
    path: '/history.html',
    source: `await page.click('#to-next')
    await page.goBack()
    const forward = await page.goForward()
    const forwardPath = path()
    return { forward: describe(forward), forwardPath, none: await page.goForward(), path: path() }`,
    expected: {
      forward: { status: 200, ok: true, path: '/next.html' },
      forwardPath: '/next.html',
      none: null,
      path: '/next.html',
    },
    probe: LOADS,
  },
  {
    name: 'forward to a history API entry answers null',
    path: '/history.html',
    source: `await page.click('#push-first')
    await page.goBack()
    const back = path()
    return { back, forward: await page.goForward(), path: path() }`,
    expected: { back: '/history.html', forward: null, path: '/first.html' },
    probe: LOADS,
  },
  {
    name: 'no later entry answers null without navigating',
    path: '/history.html',
    source: `return { forward: await page.goForward(), path: path() }`,
    expected: { forward: null, path: '/history.html' },
    probe: LOADS,
  },
  {
    name: "waitUntil domcontentloaded answers the later document's response",
    path: '/history.html',
    source: `await page.click('#to-next')
    await page.goBack()
    const forward = await page.goForward({ waitUntil: 'domcontentloaded' })
    return { forward: describe(forward), path: path() }`,
    expected: {
      forward: { status: 200, ok: true, path: '/next.html' },
      path: '/next.html',
    },
  },
  {
    name: 'an unknown waitUntil is refused before anything navigates',
    path: '/history.html',
    source: `await page.click('#to-next')
    await page.goBack()
    return { forward: await settle(page.goForward({ waitUntil: 'idle' })), path: path() }`,
    expected: { forward: { error: 'Error' }, path: '/history.html' },
  },
]

test('page.goForward answers the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareCases(t, cases, { pages: { '/history.html': html } })
})
