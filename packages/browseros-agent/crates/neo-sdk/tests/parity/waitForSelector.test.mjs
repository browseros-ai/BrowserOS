// page.waitForSelector and frame.waitForSelector through the pinned Playwright and through Neo,
// each case once per receiver, on fresh tabs of tests/pages/wait-for.html or
// tests/pages/element-states.html: every state already met, met later by page timers, and never
// met (a timeout); the first of several matches judged; calls refused at once (strict, an invalid
// selector, an unknown state, and Playwright's two legacy options); and a wait that a click in the
// same script meets. Compared: what each call settled to (`undefined`, its value, or its error's
// kind, `waits.mjs` `settle`) and what the page logged. The legacy options' messages are compared
// whole, as they are fixed client-side text. Never other wording, call logs, retry counts or
// timing.
//
// Playwright resolves an ElementHandle for `attached` and `visible`; Neo resolves `null` until it
// has handles (exception E1). Each script reports a handle as 'handle', and the test pins Neo's
// `null` in its place.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, waits.mjs).

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { withBoth } from './waits.mjs'

const readPage = (name) =>
  readFileSync(new URL(`../pages/${name}`, import.meta.url), 'utf8')
const pages = {
  '/wait-for.html': readPage('wait-for.html'),
  '/element-states.html': readPage('element-states.html'),
}

/** `waitForSelector`'s result as both engines can report it: a handle as 'handle'. */
const PICK = `const pick = (wait) => settle(wait.then((value) => (value == null ? value : 'handle')))
`
const RECEIVERS = ['page', 'page.mainFrame()']
const HANDLE = { value: 'handle' }
const NULL = { value: null }
const TIMEOUT = { error: 'timeout' }
const LEGACY = {
  visibility: {
    name: 'Error',
    message: 'options.visibility is not supported, did you mean options.state?',
  },
  waitFor: {
    name: 'Error',
    message: 'options.waitFor is not supported, did you mean options.state?',
  },
}

/** What Neo resolves where Playwright resolves `expected`: `null` for each handle (E1). */
function withoutHandles(expected) {
  return Object.fromEntries(
    Object.entries(expected).map(([key, value]) => [
      key,
      JSON.stringify(value) === JSON.stringify(HANDLE) ? NULL : value,
    ]),
  )
}

const cases = [
  {
    name: 'each state already met',
    source: (frame) => `return {
      attached: await pick(${frame}.waitForSelector('#hidden', { state: 'attached' })),
      visible: await pick(${frame}.waitForSelector('#shown')),
      hidden: await pick(${frame}.waitForSelector('#hidden', { state: 'hidden' })),
      hiddenWithoutMatch: await pick(${frame}.waitForSelector('#missing', { state: 'hidden' })),
      detached: await pick(${frame}.waitForSelector('#missing', { state: 'detached' })),
      notStrict: await pick(${frame}.waitForSelector('.many')),
      waitForVisible: await pick(${frame}.waitForSelector('#shown', { waitFor: 'visible' })),
    }`,
    expected: {
      attached: HANDLE,
      visible: HANDLE,
      hidden: NULL,
      hiddenWithoutMatch: NULL,
      detached: NULL,
      notStrict: HANDLE,
      waitForVisible: HANDLE,
    },
  },
  {
    name: 'each state met later by page timers',
    source: (frame) => `await page.click('#arm')
    const [attached, detached, visible, hidden] = await Promise.all([
      pick(${frame}.waitForSelector('#late', { state: 'attached' })),
      pick(${frame}.waitForSelector('#doomed', { state: 'detached' })),
      pick(${frame}.waitForSelector('#reveal')),
      pick(${frame}.waitForSelector('#fade', { state: 'hidden' })),
    ])
    return { attached, detached, visible, hidden }`,
    probe: 'window.__log',
    expected: {
      attached: HANDLE,
      detached: NULL,
      visible: HANDLE,
      hidden: NULL,
    },
    expectedProbe: ['armed', 'changed'],
  },
  {
    name: 'each state never met times out',
    source: (frame) => `return {
      visible: await pick(${frame}.waitForSelector('#hidden', { timeout: 300 })),
      attached: await pick(${frame}.waitForSelector('#missing', { state: 'attached', timeout: 300 })),
      detached: await pick(${frame}.waitForSelector('#shown', { state: 'detached', timeout: 300 })),
      hidden: await pick(${frame}.waitForSelector('#shown', { state: 'hidden', timeout: 300 })),
    }`,
    expected: {
      visible: TIMEOUT,
      attached: TIMEOUT,
      detached: TIMEOUT,
      hidden: TIMEOUT,
    },
  },
  {
    // `.dup` matches twice, the first visible: a hidden wait judges that first one.
    name: 'the first of several matches decides',
    path: '/element-states.html',
    source: (frame) => `return {
      visible: await pick(${frame}.waitForSelector('.dup')),
      hidden: await pick(${frame}.waitForSelector('.dup', { state: 'hidden', timeout: 300 })),
    }`,
    expected: { visible: HANDLE, hidden: TIMEOUT },
  },
  {
    // The 5 s timeouts only bound a regression: each of these fails before its first poll.
    name: 'refused at once',
    source: (frame) => `const refusal = (wait) => wait.then(
      () => 'resolved',
      (error) => ({ name: error.name, message: error.message }),
    )
    return {
      strict: await pick(${frame}.waitForSelector('.many', { strict: true, timeout: 5000 })),
      strictHidden: await pick(${frame}.waitForSelector('.many', { strict: true, state: 'hidden', timeout: 5000 })),
      selector: await pick(${frame}.waitForSelector('p >> nonsense=1', { timeout: 5000 })),
      state: await pick(${frame}.waitForSelector('#shown', { state: 'gone', timeout: 5000 })),
      visibility: await refusal(${frame}.waitForSelector('#shown', { visibility: 'visible' })),
      waitFor: await refusal(${frame}.waitForSelector('#shown', { waitFor: 'hidden' })),
    }`,
    expected: {
      strict: { error: 'strict' },
      strictHidden: { error: 'strict' },
      selector: { error: 'error' },
      state: { error: 'error' },
      visibility: LEGACY.visibility,
      waitFor: LEGACY.waitFor,
    },
  },
  {
    name: 'met by a click in the same script',
    source: (frame) => `const added = pick(${frame}.waitForSelector('#added'))
    const clicked = await settle(page.click('#add'))
    return { clicked, added: await added }`,
    probe: 'window.__log',
    expected: { clicked: 'undefined', added: HANDLE },
    expectedProbe: ['added'],
  },
]

test('page and frame waitForSelector settle the same through Neo and Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await withBoth(t, { pages }, async (run) => {
    for (const c of cases)
      for (const receiver of RECEIVERS) {
        await t.test(`${receiver}: ${c.name}`, async () => {
          const { playwright, neo } = await run({
            ...c,
            path: c.path ?? '/wait-for.html',
            source: PICK + c.source(receiver),
          })
          assert.deepEqual(
            playwright.result,
            { value: c.expected },
            'Playwright',
          )
          assert.deepEqual(
            neo.result,
            { value: withoutHandles(playwright.result.value) },
            'Neo vs Playwright, with null for each handle (E1)',
          )
          assert.deepEqual(playwright.probe, c.expectedProbe, 'Playwright page')
          assert.deepEqual(neo.probe, playwright.probe, 'what the page saw')
        })
      }
  })
})
