// Playwright's `signal` option on the navigation methods (Page and Frame receivers) and on
// waitForRequest / waitForResponse, through the pinned Playwright and through Neo (network.mjs),
// as the fleet rule for `signal` asks (one rejection-without-effect case per receiver family).
//
// An already-aborted signal: both engines reject before anything happens, so the page neither
// navigates nor reloads (its load count, kept in sessionStorage, is unchanged); Playwright's error
// is `AbortError`, Neo's an `Error` (neo-sdk `TimeoutSettings` refuses every signal until Neo can
// abort a call). `null` is no signal in either engine: every call runs as without the option.
// Never the error's wording.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

/** Loads the counting page twice (so goBack has an entry), then tries each call with `signal`. */
const script = (signal) => `const signal = ${signal}
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
      results[name] = (await settle(call())).error ?? 'resolved'
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

const cases = [
  {
    name: 'an aborted signal is refused before any navigation, on Page and Frame alike',
    source: script(
      "{ aborted: true, reason: new Error('stop'), addEventListener() {}, removeEventListener() {} }",
    ),
    playwright: { results: every('AbortError'), loadsAdded: 0 },
    neo: { results: every('Error'), loadsAdded: 0 },
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
