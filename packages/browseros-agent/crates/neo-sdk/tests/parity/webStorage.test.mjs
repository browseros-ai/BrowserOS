// `page.localStorage` and `page.sessionStorage` through Neo and through real Playwright on one
// BrowserOS neo (harness.mjs `compareMethod`): the same script on a fresh tab of the same page,
// Playwright's first, comparing what each call answered and what the page holds afterwards.
//
// Cases from Playwright's tests/page/page-localstorage.spec.ts at 26137face, named by their ids.
// Both engines share the browser's profile, so `localStorage` outlives a tab: each script clears
// what it wrote through page JS (never through the API under test) before the other engine runs.
// Plus a page whose opaque origin has no storage, which both refuse.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = '<title>storage</title><p>storage</p>'
const LEFT = '({ local: localStorage.length, session: sessionStorage.length })'
const CLEAR = `await page.evaluate(() => { localStorage.clear(); sessionStorage.clear() })`
const byName =
  'const byName = (items) => [...items].sort((a, b) => (a.name < b.name ? -1 : 1))'

test('localStorage reads and writes its items (ST-023 to ST-028)', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${byName}
const storage = page.localStorage
try {
  const fresh = await storage.items()
  const absent = await storage.getItem('absent')
  await storage.setItem('alpha', '1')
  await storage.setItem('beta', '2')
  const items = byName(await storage.items())
  const alpha = await storage.getItem('alpha')
  const pageSees = await page.evaluate(() => localStorage.getItem('alpha'))
  await storage.setItem('alpha', 'second')
  const overwritten = await storage.getItem('alpha')
  await storage.removeItem('alpha')
  const removed = await storage.items()
  await storage.clear()
  const cleared = await storage.items()
  return { fresh, absent, items, alpha, pageSees, overwritten, removed, cleared }
} finally {
  ${CLEAR}
}`,
    probe: LEFT,
  })
  assert.deepEqual(outcome.value, {
    fresh: [],
    absent: null,
    items: [
      { name: 'alpha', value: '1' },
      { name: 'beta', value: '2' },
    ],
    alpha: '1',
    pageSees: '1',
    overwritten: 'second',
    removed: [{ name: 'beta', value: '2' }],
    cleared: [],
  })
})

test('sessionStorage round-trips, apart from localStorage (ST-029, ST-030)', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${byName}
try {
  const fresh = await page.sessionStorage.items()
  await page.sessionStorage.setItem('s1', 'v1')
  await page.sessionStorage.setItem('s2', 'v2')
  const items = byName(await page.sessionStorage.items())
  const s1 = await page.sessionStorage.getItem('s1')
  await page.sessionStorage.removeItem('s1')
  const removed = await page.sessionStorage.items()
  await page.sessionStorage.clear()
  const cleared = await page.sessionStorage.items()
  await page.localStorage.setItem('shared', 'local')
  await page.sessionStorage.setItem('shared', 'session')
  const apart = [await page.localStorage.getItem('shared'), await page.sessionStorage.getItem('shared')]
  await page.localStorage.clear()
  const afterLocalClear = [await page.localStorage.items(), await page.sessionStorage.getItem('shared')]
  return { fresh, items, s1, removed, cleared, apart, afterLocalClear }
} finally {
  ${CLEAR}
}`,
    probe: LEFT,
  })
  assert.deepEqual(outcome.value, {
    fresh: [],
    items: [
      { name: 's1', value: 'v1' },
      { name: 's2', value: 'v2' },
    ],
    s1: 'v1',
    removed: [{ name: 's2', value: 'v2' }],
    cleared: [],
    apart: ['local', 'session'],
    afterLocalClear: [[], 'session'],
  })
})

test('storage methods are scoped to the current origin (ST-031)', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    // `localhost` is the fixture's other origin: the same server, another host.
    source: `const here = page.url()
const other = here.replace('127.0.0.1', 'localhost')
try {
  await page.localStorage.setItem('k', 'origin-1')
  await page.goto(other)
  const otherItems = await page.localStorage.items()
  await page.localStorage.setItem('k', 'origin-2')
  await page.goto(here)
  const back = await page.localStorage.getItem('k')
  return { otherItems, back }
} finally {
  ${CLEAR}
  await page.goto(other)
  ${CLEAR}
  await page.goto(here)
}`,
    probe: LEFT,
  })
  assert.deepEqual(outcome.value, { otherItems: [], back: 'origin-1' })
})

test('a page with an opaque origin has no storage, in both engines', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `await page.goto('data:text/html,<title>opaque</title>')
const attempt = (call) => call().then(
  (value) => ({ value }),
  (error) => ({ name: error.name, denied: /localStorage|sessionStorage|denied/i.test(error.message) }),
)
return {
  items: await attempt(() => page.localStorage.items()),
  set: await attempt(() => page.sessionStorage.setItem('k', 'v')),
}`,
    probe: 'document.title',
  })
  assert.equal(outcome.value.items.denied, true, JSON.stringify(outcome.value))
})
