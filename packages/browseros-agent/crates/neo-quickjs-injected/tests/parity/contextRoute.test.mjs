// context.route: a context route covers every page of the run's context, after the page's own
// handlers, through the pinned Playwright and through Neo (network.mjs). Cases from the pinned
// tests (26137face):
//
// - tests/library/browsercontext-route.spec.ts RO-165 "should intercept" and RO-168 "should fall
//   back to context.route": a request no page route names reaches the context's.
// - RO-181 "should chain fallback into page" (and RO-167 "should yield to page.route"): page
//   handlers newest first, then the context's, then the network.
// - An out-of-process frame's fetch, adapted from RO-165 and requestEvents' "an out-of-process
//   frame's fetch reaches the page's listeners with that frame": routed once the run reached the
//   frame (Neo attaches such frames lazily; their earlier requests are a recorded gap).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'a request no page route names reaches the context route',
    source: `let seen
    let pageHandler = false
    await context.route('**/net/blank.html', async (route) => {
      const request = route.request()
      seen = [
        path(request.url()),
        request.method(),
        request.isNavigationRequest(),
        request.resourceType(),
        request.frame() === page.mainFrame(),
        request.frame().page() === page,
      ]
      await route.fulfill({ contentType: 'text/html', body: '<title>context</title>' })
    })
    await page.route('**/net/other.html', (route) => {
      pageHandler = true
      return route.fulfill({ body: 'page' })
    })
    const response = await page.goto(ORIGIN + '/net/blank.html')
    return { seen, pageHandler, ok: response.ok(), title: await page.title() }`,
    expected: {
      seen: ['/net/blank.html', 'GET', true, 'document', true, true],
      pageHandler: false,
      ok: true,
      title: 'context',
    },
  },
  {
    name: "page handlers fall back to the context's, newest first, then the network",
    source: `const calls = []
    const add = (target, n) => target.route('**/net/blank.html', (route) => {
      calls.push(n)
      return route.fallback()
    })
    for (const n of [1, 2, 3]) await add(context, n)
    for (const n of [4, 5, 6]) await add(page, n)
    const response = await page.goto(ORIGIN + '/net/blank.html')
    return { calls, ok: response.ok(), title: await page.title() }`,
    expected: { calls: [6, 5, 4, 3, 2, 1], ok: true, title: 'blank' },
  },
  {
    name: "an out-of-process frame's fetch reaches the context route with that frame",
    source: `const routed = []
    let cross
    await context.route('**/net/oopif-data.json', async (route, request) => {
      routed.push([path(request.url()), request.frame() === cross, request.resourceType()])
      await route.fulfill({ json: { mocked: 'oopif' } })
    })
    await page.goto(ORIGIN + '/net/oopif-top.html')
    for (let tries = 0; !cross && tries < 150; tries++) {
      cross = page.frames().find((frame) => frame.url().endsWith('/net/oopif-child.html'))
      if (!cross) await new Promise((resolve) => setTimeout(resolve, 100))
    }
    await cross.waitForLoadState('load')
    const [response] = await Promise.all([page.waitForResponse('**/net/oopif-data.json'), cross.click('#fetch')])
    return {
      routed,
      origin: response.url().startsWith('http://localhost:'),
      status: response.status(),
      body: await response.json(),
      frame: response.frame() === cross,
    }`,
    expected: {
      routed: [['/net/oopif-data.json', true, 'fetch']],
      origin: true,
      status: 200,
      body: { mocked: 'oopif' },
      frame: true,
    },
  },
]

test('context.route matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
