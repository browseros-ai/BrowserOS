// page.waitForNavigation (and the main frame's) through the pinned Playwright and through Neo,
// each case on fresh tabs of tests/pages/history.html (history.mjs). From Playwright's
// tests/page/page-wait-for-navigation.spec.ts: a click that loads a new document ("WA-095 should
// work", "WA-099 should work with clicking on anchor links"), same-document navigations
// ("WA-101 should work with history.pushState()", "WA-102 should work with
// history.replaceState()"), `url` as a glob, a RegExp and a predicate that skip a navigation
// that does not match ("WA-105 should work with url match", "WA-106 should work with url match for
// same document navigations"), each waitUntil ("WA-097 should work with both domcontentloaded and
// load", "WA-098 should work with commit"), a timeout ("WA-096 should respect timeout"), a navigation that
// fails, and an option refused at once. Compared: the script's values, the URL it ended on, and
// each page's load count.
//
// Known, intended difference, asserted per engine and not counted as parity: on a child frame
// ("WA-108 should work on frame") Playwright answers the frame's new document's Response; Neo
// rejects with an `Error` before waiting, because it does not record a child frame's navigation
// requests (neo-runtime `dispatch.rs` `unsupported_child_call`). The frame still navigates.
// And at the live-wait cap: Playwright accepts a 65th URL-predicate wait; Neo refuses it with an
// `Error` (neo-runtime `history.rs` `MAX_CURSORS`), and everything else in that case matches.
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
/** The review's page: each button pushes one history entry; `#chain` pushes /first, then /goal
 * 100 ms later. */
const REVIEW = `<!doctype html><title>review</title>
<button id="hold" onclick="history.pushState({}, '', '#hold')">hold</button>
<button id="tick" onclick="history.pushState({}, '', '#tick' + (++window.ticks))">tick</button>
<button id="goal" onclick="history.pushState({}, '', '#goal')">goal</button>
<button id="chain" onclick="history.pushState({}, '', '/first'); setTimeout(() => history.pushState({}, '', '/goal'), 100)">chain</button>
<script>window.ticks = 0</script>`
/** The second review's page: `#schedule` pushes `#arm` after 1 s and `#finish` after 3 s, so no
 * call of the script's drives those navigations while 64 waits hold the run's capacity. */
const CAP = `<!doctype html><title>cap</title>
<button id="again" onclick="history.pushState({}, '', '#again')">again</button>
<button id="burst" onclick="history.pushState({}, '', '#bad'); history.pushState({}, '', '#commit')">burst</button>
<button id="schedule" onclick="setTimeout(() => history.pushState({}, '', '#arm'), 1000); setTimeout(() => history.pushState({}, '', '#finish'), 3000)">schedule</button>`
/** `/slow` (slow-load.html's image) answers this late, so its load trails its DOMContentLoaded. */
const SLOW_MS = 1500
const NEXT = { status: 200, ok: true, path: '/next.html?q=a+b' }

/** A wait started before `#chain`'s click: a pushState to /not-yet.html, then /next.html?q=a+b
 * loads 100 ms later. */
const chained = (
  wait,
) => `const [response] = await Promise.all([${wait}, page.click('#chain')])
    return { response: describe(response), path: path() }`

const cases = [
  {
    name: "a click that loads a new document answers that document's response",
    path: '/history.html',
    source: `const [response] = await Promise.all([page.waitForNavigation(), page.click('#to-next')])
    return {
      response: describe(response),
      frame: response.frame() === page.mainFrame(),
      path: path(),
    }`,
    expected: {
      response: { status: 200, ok: true, path: '/next.html' },
      frame: true,
      path: '/next.html',
    },
    probe: LOADS,
  },
  {
    name: 'same-document navigations answer null',
    path: '/history.html',
    source: `const [hash] = await Promise.all([page.waitForNavigation(), page.click('#to-hash')])
    const hashPath = path()
    const [pushed] = await Promise.all([page.waitForNavigation(), page.click('#push-first')])
    const pushedPath = path()
    const [replaced] = await Promise.all([page.mainFrame().waitForNavigation(), page.click('#replace')])
    return { hash, hashPath, pushed, pushedPath, replaced, path: path() }`,
    expected: {
      hash: null,
      hashPath: '/history.html#section',
      pushed: null,
      pushedPath: '/first.html',
      replaced: null,
      path: '/replaced.html',
    },
    probe: LOADS,
  },
  {
    name: 'a glob url skips the navigation that does not match',
    path: '/history.html',
    source: chained("page.waitForNavigation({ url: '**/next.html?q=a+b' })"),
    expected: { response: NEXT, path: '/next.html?q=a+b' },
  },
  {
    name: 'a RegExp url skips the navigation that does not match',
    path: '/history.html',
    source: chained('page.waitForNavigation({ url: /next\\.html\\?q=/ })'),
    expected: { response: NEXT, path: '/next.html?q=a+b' },
  },
  {
    name: 'a predicate url sees every navigation as a URL and takes the one it accepts',
    path: '/history.html',
    source: `const seen = []
    const [response] = await Promise.all([
      page.waitForNavigation({
        url: async (url) => {
          seen.push([url.pathname, url.searchParams.get('q'), url.href === String(url)])
          return url.searchParams.has('q')
        },
      }),
      page.click('#chain'),
    ])
    return { response: describe(response), seen, path: path() }`,
    expected: {
      response: NEXT,
      seen: [
        ['/not-yet.html', null, true],
        ['/next.html', 'a b', true],
      ],
      path: '/next.html?q=a+b',
    },
  },
  {
    name: 'a url that matches a same-document navigation answers null',
    path: '/history.html',
    source: `const [response] = await Promise.all([
      page.waitForNavigation({ url: '**/first.html' }),
      page.click('#push-first'),
    ])
    return { response, path: path() }`,
    expected: { response: null, path: '/first.html' },
  },
  {
    // A page whose image answers late, so each lifecycle event is a title of its own.
    name: 'commit and domcontentloaded resolve before load, which resolves last',
    path: '/history.html',
    source: `const [committed] = await Promise.all([
      page.waitForNavigation({ waitUntil: 'commit' }),
      page.click('#to-slow', { noWaitAfter: true }),
    ])
    const atCommit = await page.title()
    await page.goBack()
    const [dom] = await Promise.all([
      page.waitForNavigation({ waitUntil: 'domcontentloaded' }),
      page.click('#to-slow', { noWaitAfter: true }),
    ])
    const atDom = await page.title()
    await page.goBack()
    const [load] = await Promise.all([
      page.waitForNavigation(),
      page.click('#to-slow', { noWaitAfter: true }),
    ])
    return {
      responses: [committed, dom, load].map(describe),
      loadedAtCommit: atCommit === 'load',
      atDom,
      atLoad: await page.title(),
    }`,
    expected: {
      responses: [
        { status: 200, ok: true, path: '/slow-load.html' },
        { status: 200, ok: true, path: '/slow-load.html' },
        { status: 200, ok: true, path: '/slow-load.html' },
      ],
      loadedAtCommit: false,
      atDom: 'domcontentloaded',
      atLoad: 'load',
    },
  },
  {
    name: 'nothing navigates: the wait times out',
    path: '/history.html',
    source: `return {
      any: await settle(page.waitForNavigation({ timeout: 300 })),
      predicate: await settle(page.waitForNavigation({ url: () => true, timeout: 300 })),
      path: path(),
    }`,
    expected: {
      any: { error: 'timeout' },
      predicate: { error: 'timeout' },
      path: '/history.html',
    },
    probe: LOADS,
  },
  {
    name: 'a navigation that fails rejects the wait',
    path: '/history.html',
    source: `const [waited] = await Promise.all([
      settle(page.waitForNavigation({ timeout: 10000 })),
      page.click('#to-unsafe'),
    ])
    return { waited }`,
    expected: { waited: { error: 'Error' } },
  },
  {
    name: 'Neo limit: a child frame waits for its own navigation',
    path: '/framed.html',
    source: `const frame = page.frame('child')
    const [waited] = await Promise.all([
      settle(frame.waitForNavigation()),
      frame.locator('#to-next').click(),
    ])
    await frame.waitForURL('**/next.html')
    return { waited: waited && waited.error ? waited : describe(waited), child: strip(frame.url()) }`,
    expected: {
      waited: { status: 200, ok: true, path: '/next.html' },
      child: '/next.html',
    },
    neoExpected: { waited: { error: 'Error' }, child: '/next.html' },
  },
  // The three below are the independent review's regressions (review-review-b1.md): Playwright's
  // waiter tests each navigation's predicate on its own, so a pending answer holds back nothing.
  {
    name: 'a later navigation the predicate accepts wins while an earlier answer is pending',
    path: '/review',
    source: `const wait = settle(page.waitForNavigation({ timeout: 1800, url: (url) =>
      url.pathname === '/first' ? new Promise(() => {}) : url.pathname === '/goal' }))
    await page.click('#chain')
    return { wait: await wait, path: path() }`,
    expected: { wait: null, path: '/goal' },
  },
  {
    name: 'closing the page rejects a wait without a timeout whose predicate is pending',
    path: '/review',
    source: `const victim = await page.context().newPage()
    await victim.goto(origin + '/review')
    let entered = false
    const wait = settle(victim.waitForNavigation({ timeout: 0, url: () => {
      entered = true
      return new Promise(() => {})
    } }))
    await victim.click('#hold')
    while (!entered) await new Promise((resolve) => setTimeout(resolve, 10))
    await victim.close()
    return { wait: await wait, closed: victim.isClosed() }`,
    expected: { wait: { error: 'closed' }, closed: true },
  },
  {
    name: 'predicates that throw end their waits without displacing a live one',
    path: '/review',
    source: `let release
    const wait = settle(page.waitForNavigation({ timeout: 25000, url: (url) =>
      url.hash === '#hold' ? new Promise((resolve) => { release = resolve }) : url.hash === '#goal' }))
    await page.click('#hold')
    while (!release) await new Promise((resolve) => setTimeout(resolve, 10))
    let rejected = 0
    for (let i = 0; i < 64; ++i) {
      const bad = settle(page.waitForNavigation({ timeout: 5000, url: () => { throw new Error('predicate failed') } }))
      await page.click('#tick')
      if ((await bad)?.error === 'Error') ++rejected
    }
    release(false)
    await page.click('#goal')
    return { wait: await wait, rejected, hash: page.url().split('#')[1] }`,
    expected: { wait: null, rejected: 64, hash: 'goal' },
  },
  {
    name: 'a failed predicate at the live-wait cap releases capacity before the next commit',
    path: '/cap',
    source: `await page.click('#schedule')
    const entered = new Map()
    const waits = Array.from({ length: 64 }, (_, i) => page.waitForNavigation({ timeout: 15000, url: (url) => {
      if (url.hash === '#arm') return new Promise((resolve, reject) => entered.set(i, reject))
      return url.hash === '#finish'
    } }).then((value) => value, (error) => ({ error: error.name })))
    const deadline = Date.now() + 10000
    while (entered.size !== 64) {
      if (Date.now() > deadline) throw new Error('64 waits were not registered')
      await new Promise((resolve) => setTimeout(resolve, 10))
    }
    const verdict = (wait) => wait.then(() => 'accepted', (error) => error.name === 'Error' ? 'refused' : error.name)
    const extra = verdict(page.waitForNavigation({ timeout: 10000, url: (url) => url.hash === '#finish' }))
    entered.get(0)(new RangeError('release at capacity'))
    const failedAtCapacity = await waits[0]
    await new Promise((resolve) => setTimeout(resolve, 20))
    const replacement = verdict(page.waitForNavigation({ timeout: 10000, url: (url) => url.hash === '#finish' }))
    const existing = (await Promise.all(waits.slice(1))).every((result) => result === null)
    const overflow = await extra
    const replacementAfterFailure = await replacement
    const failed = settle(page.waitForNavigation({ timeout: 3000, url: () => { throw new RangeError('release at commit') } }))
    await page.click('#burst')
    const released = await failed
    const next = page.waitForNavigation({ timeout: 3000, url: () => true })
    await page.click('#again')
    return { existing, overflow, failedAtCapacity, replacementAfterFailure, released, next: await next }`,
    expected: {
      existing: true,
      overflow: 'accepted',
      failedAtCapacity: { error: 'RangeError' },
      replacementAfterFailure: 'accepted',
      released: { error: 'RangeError' },
      next: null,
    },
    neoExpected: {
      existing: true,
      overflow: 'refused',
      failedAtCapacity: { error: 'RangeError' },
      replacementAfterFailure: 'accepted',
      released: { error: 'RangeError' },
      next: null,
    },
  },
  {
    name: 'an unknown waitUntil is refused before anything is waited for',
    path: '/history.html',
    source: `return { waited: await settle(page.waitForNavigation({ waitUntil: 'idle' })) }`,
    expected: { waited: { error: 'Error' } },
  },
]

test('page.waitForNavigation answers the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareCases(t, cases, {
    slowMs: SLOW_MS,
    pages: {
      '/history.html': html,
      '/slow-load.html': readFileSync(
        new URL('../pages/slow-load.html', import.meta.url),
        'utf8',
      ),
      '/review': REVIEW,
      '/cap': CAP,
      '/framed.html':
        '<!doctype html><title>framed</title><iframe name="child" src="/history.html"></iframe>',
    },
  })
})
