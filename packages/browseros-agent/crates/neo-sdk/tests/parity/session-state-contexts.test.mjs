// Session state stays in its own context, through Neo and through Playwright on one BrowserOS
// neo: the run's own context's extra HTTP headers and offline mode never reach a page of a
// context `browser.newContext()` created, while the run's own pages keep them. Each case resets
// what it set, as Playwright's connection keeps its default context's settings across cases.
// Session state on a created context itself (its cookies, its headers, a page's own headers) is
// asserted on each engine's side: Playwright serves it, Neo fails naming the method
// (neo-runtime `browser_calls.rs` `created_state`).
//
// Playwright cases (pin 26137face): `tests/library/browsercontext-set-extra-http-headers.spec.ts`
// "CO-019 should override extra headers from browser context" and
// `tests/library/browsercontext-network-event.spec.ts`'s offline cases are each one context;
// these run their settings across two.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = {
  '/state.html': '<title>state</title><p>state</p>',
  // The `x-scope` header the document's request carried, or nothing.
  '/echo': (request, response) => {
    response.setHeader('content-type', 'text/html')
    response.end(
      `<title>echo</title><p id="seen">${request.headers['x-scope'] ?? ''}</p>`,
    )
  },
}

const ORIGIN = `const origin = page.url().slice(0, page.url().indexOf('/', 'http://'.length))
const seen = async (p) => { await p.goto(origin + '/echo'); return p.textContent('#seen') }
`

const cases = [
  {
    name: "the run's own context's headers and offline mode stay out of a created context",
    source: `${ORIGIN}const created = await context.browser().newContext()
const other = await created.newPage()
await context.setExtraHTTPHeaders({ 'x-scope': 'own' })
const inOwn = await seen(page)
const inOther = await seen(other)
const later = await created.newPage()
const inLater = await seen(later)
await context.setOffline(true)
const online = { own: await page.evaluate(() => navigator.onLine), other: await other.evaluate(() => navigator.onLine) }
await context.setOffline(false)
await context.setExtraHTTPHeaders({})
await created.close()
return { inOwn, inOther, inLater, online }`,
    expected: {
      inOwn: 'own',
      inOther: '',
      inLater: '',
      online: { own: false, other: true },
    },
  },
]

/** Each session-state call on a created context: answered, or its error's first line. */
const CREATED = `${ORIGIN}const created = await context.browser().newContext()
const other = await created.newPage()
const first = (call) => call.then((value) => ['answered', Array.isArray(value) ? value.length : (value ?? null)], (error) => ['failed', error.message.split('\\n')[0]])
const result = {
  cookies: await first(created.cookies()),
  addCookies: await first(created.addCookies([{ name: 'a', value: 'b', url: origin }])),
  headers: await first(created.setExtraHTTPHeaders({ 'x-scope': 'created' })),
  pageHeaders: await first(other.setExtraHTTPHeaders({ 'x-scope': 'page' })),
}
await created.close()
return result`

test('session state stays in its context: Neo matches Playwright', {
  skip,
}, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    for (const { name, source, expected } of cases) {
      const reference = outcomeValue(await playwright('/state.html', source))
      assert.deepEqual(reference, expected, `${name}: Playwright`)
      assert.deepEqual(
        outcomeValue(await neo('/state.html', source)),
        reference,
        `${name}: Neo`,
      )
    }
  })
})

test('session state on a created context: Playwright serves it, Neo refuses naming it', {
  skip,
}, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    assert.deepEqual(
      outcomeValue(await playwright('/state.html', CREATED)),
      {
        cookies: ['answered', 0],
        addCookies: ['answered', null],
        headers: ['answered', null],
        pageHeaders: ['answered', null],
      },
      'Playwright',
    )
    const refused = (api, method) => [
      'failed',
      `${api}: neo-runtime: ${method} is not served yet for a context browser.newContext created, only for the run's own context`,
    ]
    assert.deepEqual(
      outcomeValue(await neo('/state.html', CREATED)),
      {
        cookies: refused('browserContext.cookies', 'cookies'),
        addCookies: refused('browserContext.addCookies', 'addCookies'),
        headers: refused(
          'browserContext.setExtraHTTPHeaders',
          'setExtraHTTPHeaders',
        ),
        pageHeaders: refused('page.setExtraHTTPHeaders', 'setExtraHTTPHeaders'),
      },
      'Neo',
    )
  })
})
