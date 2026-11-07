// page.goBack through the pinned Playwright and through Neo, each case on fresh tabs of
// tests/pages/history.html (history.mjs), which start as about:blank: back across two documents
// and their responses (Playwright's tests/page/page-history.spec.ts "NA-071 page.goBack should
// work"), history API entries ("NA-072 page.goBack should work with HistoryAPI"), no earlier
// entry, `waitUntil: 'commit'`, and an option refused at once. Compared: the script's values,
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
    name: "back to an earlier document answers that document's response",
    path: '/history.html',
    source: `await page.click('#to-next')
    const back = await page.goBack()
    return { back: describe(back), path: path(), title: await page.title() }`,
    expected: {
      back: { status: 200, ok: true, path: '/history.html' },
      path: '/history.html',
      title: 'history',
    },
    probe: LOADS,
  },
  {
    name: 'history API entries answer null and move the URL within the document',
    path: '/history.html',
    source: `await page.click('#push-first')
    await page.click('#push-second')
    const first = await page.goBack()
    const firstPath = path()
    const start = await page.goBack()
    return { first, firstPath, start, startPath: path() }`,
    expected: {
      first: null,
      firstPath: '/first.html',
      start: null,
      startPath: '/history.html',
    },
    probe: LOADS,
  },
  {
    name: 'no earlier entry answers null without navigating',
    path: '/history.html',
    source: `const blank = await page.goBack()
    const blankUrl = page.url()
    return { blank, blankUrl, none: await page.goBack(), url: page.url() }`,
    expected: {
      blank: null,
      blankUrl: 'about:blank',
      none: null,
      url: 'about:blank',
    },
  },
  {
    name: "waitUntil commit answers the earlier document's response",
    path: '/history.html',
    source: `await page.click('#to-next')
    const back = await page.goBack({ waitUntil: 'commit' })
    return { back: describe(back), path: path() }`,
    expected: {
      back: { status: 200, ok: true, path: '/history.html' },
      path: '/history.html',
    },
  },
  {
    name: 'an unknown waitUntil is refused before anything navigates',
    path: '/history.html',
    source: `await page.click('#to-next')
    return { back: await settle(page.goBack({ waitUntil: 'idle' })), path: path() }`,
    expected: { back: { error: 'Error' }, path: '/next.html' },
  },
]

test('page.goBack answers the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareCases(t, cases, { pages: { '/history.html': html } })
})
