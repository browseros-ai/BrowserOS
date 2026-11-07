// page.on('console') and context.on('console') through the pinned Playwright and through Neo, on
// fresh tests/pages/console.html tabs (events.mjs): a click's message reaches the page's
// listeners and then the context's, as one ConsoleMessage, before the click resolves; each
// message's type, text, location, page and worker, for clicks and a timer; and the text
// Playwright makes of several arguments: each argument's preview, joined by spaces, with format
// specifiers left as written. Timestamps compare by type and order, never by value; no case
// reads args().
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/page/page-event-console.spec.ts "CM-004 should work for different console API calls"
    // ("All console events will be reported before `page.evaluate` is finished"), for a click;
    // tests/library/browsercontext-events.spec.ts "BC-032 console event should work @smoke".
    name: "a click's message reaches page then context listeners before the click resolves",
    path: '/console.html',
    source: `const seen = []
    let fromPage
    page.on('console', (message) => {
      fromPage = message
      seen.push('page ' + message.text())
    })
    page.context().on('console', (message) => {
      if (message.page() !== page) return
      seen.push('context ' + message.text() + (message === fromPage ? '' : ' (another object)'))
    })
    await page.click('#log')
    const atResolve = [...seen]
    await page.click('#log')
    return { atResolve, afterTwoClicks: seen }`,
    expected: {
      atResolve: ['page click 1', 'context click 1'],
      afterTwoClicks: [
        'page click 1',
        'context click 1',
        'page click 2',
        'context click 2',
      ],
    },
    probe: 'window.__logged',
    effect: 4,
  },
  {
    // page-event-console.spec.ts "CM-004 should work for different console API calls",
    // "CM-009 should have location for console API calls", "CM-015 should have increasing
    // timestamps".
    name: 'each message has its type, text, location, page and worker',
    path: '/console.html',
    source: `const messages = []
    page.on('console', (message) => messages.push(message))
    await page.click('#log')
    await page.click('#types')
    const later = page.waitForEvent('console', { predicate: (message) => message.text() === 'later', timeout: 5000 })
    await page.click('#later')
    await later
    return {
      messages: messages.map((message) => {
        const { url, ...at } = message.location()
        return [message.type(), message.text(), path(url), at.lineNumber, at.columnNumber]
      }),
      locationKeys: Object.keys(messages[0].location()),
      lineAliases: messages.every((message) => {
        const at = message.location()
        return at.line === at.lineNumber && at.column === at.columnNumber
      }),
      pages: messages.every((message) => message.page() === page),
      workers: messages.map((message) => message.worker()),
      timestamps: messages.map((message) => typeof message.timestamp()),
      increasing: messages.every((message, i) => i === 0 || message.timestamp() >= messages[i - 1].timestamp()),
    }`,
    // Zero-based lines and columns of console.html, as CDP reports them.
    expected: {
      messages: [
        ['log', 'click 1', '/console.html', 13, 106],
        ['debug', 'debug', '/console.html', 15, 172],
        ['info', 'info', '/console.html', 15, 196],
        ['warning', 'warn', '/console.html', 15, 218],
        ['error', 'error', '/console.html', 15, 240],
        ['info', 'later', '/console.html', 14, 142],
      ],
      locationKeys: ['url', 'line', 'column', 'lineNumber', 'columnNumber'],
      lineAliases: true,
      pages: true,
      workers: [null, null, null, null, null, null],
      timestamps: ['number', 'number', 'number', 'number', 'number', 'number'],
      increasing: true,
    },
    probe: 'window.__logged',
    effect: 8,
  },
  {
    // page-event-console.spec.ts "CM-001 should work @smoke", "CM-006 should not fail for
    // window object", "CM-011 should use object previews for arrays and objects".
    name: 'several arguments format into one text, specifiers untouched',
    path: '/console.html',
    source: `const texts = {}
    for (const id of ['#format', '#previews', '#empty']) {
      const [message] = await Promise.all([page.waitForEvent('console'), page.click(id)])
      texts[id] = message.text()
    }
    return texts`,
    // '%s is %d' stays as written: Chromium already turned the two arguments it names into a
    // String and a Number before CDP reports them. An empty string logs an empty text.
    expected: {
      '#format': 'a 1 {x: 1} [1, 2]',
      '#previews': '%s is %d [object Object] 1 null undefined true Window',
      '#empty': '',
    },
    probe: 'window.__logged',
    effect: 5,
  },
]

test('console events reach the same listeners with the same messages through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/console.html': fixture('console.html') },
    cases,
  })
})
