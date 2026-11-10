// page.setExtraHTTPHeaders and context.setExtraHTTPHeaders through the pinned Playwright and
// through Neo, on fresh tabs of an inline page (events.mjs), compared by the headers the server
// saw on each navigation (an echo page hands them to the page): the page's headers reach a
// navigation and the request a redirect leads to, and {} removes them; the context's reach the
// page, the page's override them by name whatever the case (server/chromium/crPage.ts
// `updateExtraHTTPHeaders` merges the two), and each {} removes only its own; a value that is not
// a string rejects with Error on either and sends nothing.
//
// Neo's context is the user's own profile, so every script removes the headers it set before it
// returns, and ends on the page it started on.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/** `sent(path, ...names)`: the named headers of the navigation to `path`, null when absent. */
const HEADERS = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const start = page.url()
    const sent = async (path, ...names) => {
      await page.goto(origin + path)
      const headers = await page.evaluate(() => window.headers)
      return Object.fromEntries(names.map((name) => [name, headers[name] ?? null]))
    }
    `

/** Answers with a page holding the request's headers as `window.headers`. */
function echoHeaders(request, response) {
  const headers = JSON.stringify(request.headers).replace(/</g, '\\u003c')
  response.setHeader('content-type', 'text/html; charset=utf-8')
  response.end(
    `<title>headers</title><script>window.headers = ${headers}</script>`,
  )
}

const cases = [
  {
    // tests/page/page-set-extra-http-headers.spec.ts "NE-114 should work @smoke", "NE-115 should
    // work with redirects".
    name: "the page's headers reach its navigations and redirects, and {} removes them",
    path: '/setExtraHTTPHeaders.html',
    source: `${HEADERS}try {
      await page.setExtraHTTPHeaders({ foo: 'bar' })
      const direct = await sent('/echo-headers', 'foo', 'baz')
      const redirected = await sent('/redirect', 'foo', 'baz')
      await page.setExtraHTTPHeaders({})
      return { direct, redirected, removed: await sent('/echo-headers', 'foo', 'baz') }
    } finally {
      await page.setExtraHTTPHeaders({})
      await page.goto(start)
    }`,
    expected: {
      direct: { foo: 'bar', baz: null },
      redirected: { foo: 'bar', baz: null },
      removed: { foo: null, baz: null },
    },
    probe: 'location.pathname',
    effect: '/setExtraHTTPHeaders.html',
  },
  {
    // page-set-extra-http-headers.spec.ts "NE-116 should work with extra headers from browser
    // context"; tests/library/browsercontext-set-extra-http-headers.spec.ts "NE-119 should
    // override extra headers from browser context".
    name: "the context's headers reach the page, the page's override them by name, and each {} removes its own",
    path: '/setExtraHTTPHeaders.html',
    source: `${HEADERS}try {
      await page.context().setExtraHTTPHeaders({ fOo: 'bAr', baR: 'foO' })
      const fromContext = await sent('/echo-headers', 'foo', 'bar')
      await page.setExtraHTTPHeaders({ Foo: 'Bar' })
      const overridden = await sent('/echo-headers', 'foo', 'bar')
      await page.setExtraHTTPHeaders({})
      const pageRemoved = await sent('/echo-headers', 'foo', 'bar')
      await page.context().setExtraHTTPHeaders({})
      return { fromContext, overridden, pageRemoved, contextRemoved: await sent('/echo-headers', 'foo', 'bar') }
    } finally {
      await page.setExtraHTTPHeaders({})
      await page.context().setExtraHTTPHeaders({})
      await page.goto(start)
    }`,
    expected: {
      fromContext: { foo: 'bAr', bar: 'foO' },
      overridden: { foo: 'Bar', bar: 'foO' },
      pageRemoved: { foo: 'bAr', bar: 'foO' },
      contextRemoved: { foo: null, bar: null },
    },
    probe: 'location.pathname',
    effect: '/setExtraHTTPHeaders.html',
  },
  {
    // page-set-extra-http-headers.spec.ts "NE-117 should throw for non-string header values".
    name: 'a header value that is not a string rejects with Error on the page and the context, and sends nothing',
    path: '/setExtraHTTPHeaders.html',
    source: `${HEADERS}try {
      const refused = {
        page: await settle(page.setExtraHTTPHeaders({ foo: 1 })),
        context: await settle(page.context().setExtraHTTPHeaders({ foo: true })),
      }
      return { refused, sent: await sent('/echo-headers', 'foo') }
    } finally {
      await page.setExtraHTTPHeaders({})
      await page.context().setExtraHTTPHeaders({})
      await page.goto(start)
    }`,
    expected: {
      refused: { page: { error: 'Error' }, context: { error: 'Error' } },
      sent: { foo: null },
    },
    probe: 'location.pathname',
    effect: '/setExtraHTTPHeaders.html',
  },
]

test('setExtraHTTPHeaders sends the same headers through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/setExtraHTTPHeaders.html':
        '<title>setExtraHTTPHeaders</title><h1>setExtraHTTPHeaders</h1>',
      '/echo-headers': echoHeaders,
      '/redirect': {
        status: 302,
        headers: { location: '/echo-headers' },
        body: '',
      },
    },
    cases,
  })
})
