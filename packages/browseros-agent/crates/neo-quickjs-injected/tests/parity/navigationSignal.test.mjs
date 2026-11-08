// Playwright's `signal` option on the navigation methods (Page and Frame receivers) and on
// waitForRequest / waitForResponse, through the pinned Playwright and through Neo (network.mjs),
// one honouring case per receiver family as the fleet rule for `signal` asks (slice S, #3194;
// page.goto, page.waitForURL and page.waitForLoadState are in abort.test.mjs).
//
// An already-aborted signal: both engines reject with `AbortError` whose `cause` is the reason
// before anything happens, so the page neither navigates nor reloads (its load count, kept in
// sessionStorage, is unchanged). A signal aborted mid-wait stops a navigation or network wait the
// same way. `null` is no signal in either engine: every call runs as without the option. Errors
// compare by category (`name`, whether `cause` is the reason), never by wording.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

/** How a call settled: `resolved`, an error's name, or for an `AbortError` whether its `cause` is
 * the reason the script aborted with. */
const OUTCOME = `const outcome = (promise, reason) => promise.then(
      () => 'resolved',
      (error) => error.name === 'AbortError' ? { name: error.name, causeIsReason: error.cause === reason } : error.name,
    )`

/** Loads the counting page twice (so goBack has an entry), then tries each call with `signal`. */
const script = (signal) => `${OUTCOME}
    const reason = new Error('stop')
    const signal = ${signal}
    const counted = ORIGIN + '/net/counted.html'
    await page.goto(ORIGIN + '/net/blank.html')
    await page.goto(counted)
    const loads = () => page.evaluate(() => Number(sessionStorage.getItem('loads')))
    const before = await loads()
    const frame = page.mainFrame()
    const results = {}
    for (const [name, call] of [
      ['page.reload', () => page.reload({ signal })],
      ['page.goBack', () => page.goBack({ signal })],
      ['page.goForward', () => page.goForward({ signal })],
      ['page.waitForLoadState', () => page.waitForLoadState('load', { signal })],
      ['page.waitForURL', () => page.waitForURL('**/counted.html', { signal })],
      ['page.waitForNavigation', () => page.waitForNavigation({ signal, timeout: 300 })],
      ['frame.goto', () => frame.goto(counted, { signal })],
      ['frame.waitForLoadState', () => frame.waitForLoadState('load', { signal })],
      ['frame.waitForURL', () => frame.waitForURL('**/counted.html', { signal })],
      ['frame.waitForNavigation', () => frame.waitForNavigation({ signal, timeout: 300 })],
      ['page.waitForRequest', () => page.waitForRequest('**/never', { signal, timeout: 300 })],
      ['page.waitForResponse', () => page.waitForResponse('**/never', { signal, timeout: 300 })],
    ]) {
      if (page.url() !== counted) await page.goto(counted)
      results[name] = await outcome(call(), reason)
    }
    return { results, loadsAdded: (await loads()) - before }`

const NAMES = [
  'page.reload',
  'page.goBack',
  'page.goForward',
  'page.waitForLoadState',
  'page.waitForURL',
  'page.waitForNavigation',
  'frame.goto',
  'frame.waitForLoadState',
  'frame.waitForURL',
  'frame.waitForNavigation',
  'page.waitForRequest',
  'page.waitForResponse',
]
const every = (kind) => Object.fromEntries(NAMES.map((name) => [name, kind]))
const ABORTED = { name: 'AbortError', causeIsReason: true }

/** The waits that wait: each gets its own signal, aborted with its own reason once all wait. */
const WAITS = [
  [
    'page.waitForNavigation',
    '(signal) => page.waitForNavigation({ signal, timeout: 0 })',
  ],
  [
    'frame.waitForNavigation',
    '(signal) => frame.waitForNavigation({ signal, timeout: 0 })',
  ],
  [
    'frame.waitForURL',
    "(signal) => frame.waitForURL('**/never', { signal, timeout: 0 })",
  ],
  [
    'page.waitForRequest',
    "(signal) => page.waitForRequest('**/never', { signal, timeout: 0 })",
  ],
  [
    'page.waitForResponse',
    "(signal) => page.waitForResponse('**/never', { signal, timeout: 0 })",
  ],
]

const cases = [
  {
    // tests/page/page-click.spec.ts CL-092 "should abort via already-aborted signal", on the
    // navigation methods and network waits, with a duck-typed signal (both engines take one).
    name: 'an already-aborted signal rejects before any navigation, on Page and Frame alike',
    source: script(
      '{ aborted: true, reason, addEventListener() {}, removeEventListener() {} }',
    ),
    expected: { results: every(ABORTED), loadsAdded: 0 },
  },
  {
    // CL-090 "should abort via signal", on waits: each stops with its own reason, and the page
    // navigates normally afterwards (no aborted cursor holds a later navigation).
    name: 'a signal aborted mid-wait stops navigation and network waits',
    source: `${OUTCOME}
    await page.goto(ORIGIN + '/net/blank.html')
    const frame = page.mainFrame()
    const waits = {}
    const aborts = []
    for (const [name, start] of [${WAITS.map(([name, start]) => `[${JSON.stringify(name)}, ${start}]`).join(', ')}]) {
      const controller = new AbortController()
      const reason = new Error('stop ' + name)
      aborts.push(() => controller.abort(reason))
      waits[name] = outcome(start(controller.signal), reason)
    }
    await new Promise((resolve) => setTimeout(resolve, 300))
    for (const abort of aborts) abort()
    const results = {}
    for (const [name, pending] of Object.entries(waits)) results[name] = await pending
    await page.goto(ORIGIN + '/net/counted.html')
    return { results, url: path(page.url()) }`,
    expected: {
      results: Object.fromEntries(WAITS.map(([name]) => [name, ABORTED])),
      url: '/net/counted.html',
    },
  },
  {
    name: 'a null signal is no signal: every call runs as without the option',
    source: script('null'),
    // Loads of the counted page: the reload, the goto back to it after goBack (which leaves
    // goForward no entry), and frame.goto. The waits time out where nothing navigates.
    expected: {
      results: {
        'page.reload': 'resolved',
        'page.goBack': 'resolved',
        'page.goForward': 'resolved',
        'page.waitForLoadState': 'resolved',
        'page.waitForURL': 'resolved',
        'page.waitForNavigation': 'TimeoutError',
        'frame.goto': 'resolved',
        'frame.waitForLoadState': 'resolved',
        'frame.waitForURL': 'resolved',
        'frame.waitForNavigation': 'TimeoutError',
        'page.waitForRequest': 'TimeoutError',
        'page.waitForResponse': 'TimeoutError',
      },
      loadsAdded: 3,
    },
  },
]

test('signal on navigation methods and network waits', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
