// page.route: a handler sees each request its pattern names, after the request event, as the one
// object of that request, through the pinned Playwright and through Neo (network.mjs).
// Cases from the pinned tests (26137face):
//
// - tests/page/page-route.spec.ts RO-001 "should intercept @smoke": the routed request is the
//   `request` event's object, `response.request()` and the `page.requests()` entry, with its
//   fields read while it is paused.
// - RO-045 "should support the times parameter with route matching": `times: 1` handles one
//   request, then the next handler, then the network.
// - A child frame's requests, adapted from RO-001 on the iframe of RO-030 ("should not throw
//   'Invalid Interception Id' if the request was cancelled"): the frame's document and its fetch
//   reach their handlers with that frame.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const DATA = '{"path":"/net/data.json","query":""}'

const cases = [
  {
    name: 'a routed request is the request event object, paused before its handler',
    source: `const order = []
    let fromEvent
    let routed
    let seen
    page.on('request', (request) => {
      if (request.url().endsWith('/net/blank.html')) {
        order.push('request')
        fromEvent = request
      }
    })
    await page.route('**/net/blank.html', async (route, request) => {
      order.push('route')
      routed = request
      seen = [
        route.request() === request,
        request === fromEvent,
        request.method(),
        request.postData(),
        request.isNavigationRequest(),
        request.resourceType(),
        request.frame() === page.mainFrame(),
        Boolean(request.headers()['user-agent']),
      ]
      await route.continue()
    })
    const response = await page.goto(ORIGIN + '/net/blank.html')
    return {
      order,
      seen,
      ok: response.ok(),
      same: response.request() === routed,
      listed: (await page.requests()).includes(routed),
      title: await page.title(),
    }`,
    expected: {
      order: ['request', 'route'],
      seen: [true, true, 'GET', null, true, 'document', true, true],
      ok: true,
      same: true,
      listed: true,
      title: 'blank',
    },
  },
  {
    name: 'times: 1 handles one request, then the next handler, then the network',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const body = () => page.evaluate(() => fetch('/net/data.json').then((r) => r.text()))
    const calls = []
    await page.route('**/net/data.json', (route) => {
      calls.push(1)
      return route.fulfill({ body: 'first' })
    }, { times: 1 })
    const bodies = [await body()]
    await page.route('**/net/data.json', (route) => route.fulfill({ body: 'second' }), { times: 1 })
    bodies.push(await body(), await body(), await body())
    return { bodies, calls }`,
    expected: { bodies: ['first', 'second', DATA, DATA], calls: [1] },
  },
  {
    name: "a child frame's document and fetch reach their routes with that frame",
    source: `const seen = []
    const note = (request) => seen.push([
      path(request.url()),
      request.resourceType(),
      request.isNavigationRequest(),
      request.frame() === page.mainFrame(),
      request.frame().parentFrame() === page.mainFrame(),
    ])
    await page.route('**/net/frame.html', async (route, request) => {
      note(request)
      await route.continue()
    })
    await page.route('**/net/in-frame.json', async (route, request) => {
      note(request)
      await route.fulfill({ json: { mocked: true } })
    })
    const answered = page.waitForResponse('**/net/in-frame.json')
    await page.goto(ORIGIN + '/net/frame-top.html')
    const response = await answered
    const child = page.frame({ name: 'child' })
    return {
      seen,
      body: await response.json(),
      status: response.status(),
      frame: response.frame() === child,
      title: await child.title(),
    }`,
    expected: {
      seen: [
        ['/net/frame.html', 'document', true, false, true],
        ['/net/in-frame.json', 'fetch', false, false, true],
      ],
      body: { mocked: true },
      status: 200,
      frame: true,
      title: 'frame',
    },
  },
]

test('page.route matches Playwright', { skip, timeout: 240_000 }, async (t) => {
  await compareNetwork(t, cases)
})
