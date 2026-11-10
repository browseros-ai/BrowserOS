// context.setOffline through the pinned Playwright and through Neo, on fresh tabs of an inline
// page (events.mjs), compared by what the page and its requests see: offline, navigator.onLine
// is false, the window hears `offline` then `online`, a fetch (a keepalive one too) fails with
// TypeError, and a navigation rejects with Error; navigator.onLine stays false across a
// navigation while offline; setOffline(false) brings back all of it.
//
// Neo's context is the user's own profile, so every script turns offline off again before it
// returns (in a finally), and ends on the page it started on.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/** `network(options)`: navigator.onLine, and how a fetch of /data settled. */
const NETWORK = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const start = page.url()
    const network = (options = {}) => page.evaluate((options) => fetch('/data', { cache: 'no-store', ...options }).then(
      (response) => ({ onLine: navigator.onLine, fetch: response.status }),
      (error) => ({ onLine: navigator.onLine, fetch: error.name }),
    ), options)
    `

const cases = [
  {
    // tests/library/browsercontext-basic.spec.ts "BC-025 should emulate navigator.onLine",
    // "BC-027 should emulate offline event", "BC-024 fetch with keepalive should throw when
    // offline".
    name: "offline, navigator.onLine is false, the window hears it and the page's fetches fail",
    path: '/setOffline.html',
    source: `${NETWORK}await page.evaluate(() => {
      window.changes = []
      addEventListener('offline', () => window.changes.push('offline'))
      addEventListener('online', () => window.changes.push('online'))
    })
    const before = await network()
    let offline
    try {
      await page.context().setOffline(true)
      offline = { plain: await network(), keepalive: await network({ keepalive: true }) }
    } finally {
      await page.context().setOffline(false)
    }
    const after = await network()
    await page.waitForFunction(() => window.changes.length >= 2)
    return { before, offline, after, changes: await page.evaluate(() => window.changes) }`,
    expected: {
      before: { onLine: true, fetch: 200 },
      offline: {
        plain: { onLine: false, fetch: 'TypeError' },
        keepalive: { onLine: false, fetch: 'TypeError' },
      },
      after: { onLine: true, fetch: 200 },
      changes: ['offline', 'online'],
    },
    probe: 'navigator.onLine',
    effect: true,
  },
  {
    // browsercontext-basic.spec.ts "BC-023 should work with offline option" (setOffline(false)
    // lets the navigation through), "BC-026 should emulate navigator.onLine across navigations".
    name: 'offline, a navigation rejects with Error and onLine stays false; back online it loads',
    path: '/setOffline.html',
    source: `${NETWORK}let offline
    try {
      await page.context().setOffline(true)
      await page.goto('about:blank')
      const onLineAfterNavigating = await page.evaluate(() => navigator.onLine)
      // The failed navigation commits the browser's error page; the next one starts after it.
      const [, navigation] = await Promise.all([
        page.waitForEvent('framenavigated'),
        settle(page.goto(origin + '/target.html')),
      ])
      offline = { onLineAfterNavigating, navigation }
    } finally {
      await page.context().setOffline(false)
    }
    const response = await page.goto(origin + '/target.html')
    const online = { status: response.status(), onLine: await page.evaluate(() => navigator.onLine) }
    await page.goto(start)
    return { offline, online }`,
    expected: {
      offline: { navigation: { error: 'Error' }, onLineAfterNavigating: false },
      online: { status: 200, onLine: true },
    },
    probe: '[location.pathname, navigator.onLine]',
    effect: ['/setOffline.html', true],
  },
  {
    // browsercontext-basic.spec.ts "BC-025 should emulate navigator.onLine", once a popup of the
    // context has opened and closed: a regression for Neo's setOffline sending to the closed
    // popup's session and rejecting.
    name: 'a popup that opened and closed leaves setOffline reaching the open page',
    path: '/setOffline.html',
    source: `${NETWORK}const [popup] = await Promise.all([
      page.waitForEvent('popup'),
      page.evaluate(() => {
        window.open('/target.html')
      }),
    ])
    await popup.waitForLoadState()
    const popupTitle = await popup.title()
    await popup.close()
    let offline
    try {
      offline = await settle(page.context().setOffline(true).then(() => network()))
    } finally {
      await page.context().setOffline(false)
    }
    return { popupTitle, offline, after: await network() }`,
    expected: {
      popupTitle: 'target',
      offline: { value: { onLine: false, fetch: 'TypeError' } },
      after: { onLine: true, fetch: 200 },
    },
    probe: 'navigator.onLine',
    effect: true,
  },
]

test('context.setOffline takes the page offline and back the same way through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/setOffline.html': '<title>setOffline</title><h1>setOffline</h1>',
      '/target.html': '<title>target</title><h1>target</h1>',
      '/data': {
        headers: { 'content-type': 'application/json' },
        body: '{"ok":true}',
      },
    },
    cases,
  })
})
