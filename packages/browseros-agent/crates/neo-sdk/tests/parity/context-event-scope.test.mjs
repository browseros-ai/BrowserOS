// Context-level events stay in their own context, through Neo and through Playwright on one
// BrowserOS neo. A page of a context `browser.newContext()` created fires its events on itself,
// and the run's own context's listeners hear none of them, one case per family: a download; a
// console message, an uncaught error and a dialog; the page's own creation, navigation and load.
// The network family is `network-context-scope-review`. Each case disconnects Playwright before
// Neo's turn, and closes the context it created.
//
// Playwright cases (pin 26137face): `tests/library/download.spec.ts` "DL-001 should report
// downloads with acceptDownloads: false", `tests/library/browsercontext-events.spec.ts` "BE-001
// console event should work @smoke", "BE-005 dialog event should work @smoke" and
// `tests/library/browsercontext-page-event.spec.ts` "PE-001 should have url", each run across
// two contexts.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = {
  '/scope.html': '<title>scope</title><p>scope</p>',
  '/download.html': '<title>download</title><a href="/file" download>file</a>',
  '/file': (request, response) => {
    response.setHeader('content-type', 'application/octet-stream')
    response.setHeader('content-disposition', 'attachment; filename=scope.txt')
    response.end('scope')
  },
}

const ORIGIN = `const origin = page.url().slice(0, page.url().indexOf('/', 'http://'.length))
const created = await context.browser().newContext()
const own = []
`

const cases = [
  {
    name: 'a download',
    source: `${ORIGIN}const other = await created.newPage()
await other.goto(origin + '/download.html')
context.on('download', (download) => own.push(download.suggestedFilename()))
const [download] = await Promise.all([other.waitForEvent('download', { timeout: 10000 }), other.click('a')])
const onPage = download.suggestedFilename()
await download.cancel().catch(() => {})
await created.close()
return { own, onPage }`,
    expected: { own: [], onPage: 'scope.txt' },
  },
  {
    name: 'a console message, an uncaught error and a dialog',
    source: `${ORIGIN}const other = await created.newPage()
await other.goto(origin + '/scope.html')
context.on('console', (message) => own.push('console ' + message.text()))
context.on('weberror', (error) => own.push('weberror ' + error.error().message))
context.on('dialog', (dialog) => { own.push('dialog ' + dialog.message()); dialog.dismiss().catch(() => {}) })
const onPage = []
other.on('console', (message) => onPage.push('console ' + message.text()))
other.on('pageerror', (error) => onPage.push('pageerror ' + error.message))
other.on('dialog', (dialog) => { onPage.push('dialog ' + dialog.message()); dialog.dismiss().catch(() => {}) })
await other.evaluate(() => console.log('from the other context'))
await Promise.all([other.waitForEvent('pageerror'), other.evaluate(() => { setTimeout(() => { throw new Error('boom') }, 0) })])
await other.evaluate(() => alert('hello'))
await created.close()
return { own, onPage: onPage.sort() }`,
    expected: {
      own: [],
      onPage: [
        'console from the other context',
        'dialog hello',
        'pageerror boom',
      ],
    },
  },
  {
    name: "the page's creation, navigation and load",
    source: `${ORIGIN}context.on('page', () => own.push('page'))
const other = await created.newPage()
const onPage = []
other.on('framenavigated', (frame) => { if (frame === other.mainFrame()) onPage.push('framenavigated ' + frame.url().slice(origin.length)) })
other.on('load', () => onPage.push('load'))
await other.goto(origin + '/scope.html')
await other.waitForLoadState('load')
await created.close()
return { own, onPage }`,
    expected: { own: [], onPage: ['framenavigated /scope.html', 'load'] },
  },
]

test('context-level events stay in their context: Neo matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo, connect }) => {
    for (const { name, source, expected } of cases) {
      const reference = outcomeValue(await playwright('/scope.html', source))
      await (await connect()).close()
      assert.deepEqual(reference, expected, `${name}: Playwright`)
      assert.deepEqual(
        outcomeValue(await neo('/scope.html', source)),
        reference,
        `${name}: Neo`,
      )
    }
  })
})
