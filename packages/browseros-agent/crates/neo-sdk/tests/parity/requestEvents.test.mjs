// page.on / context.on('request' | 'response' | 'requestfinished' | 'requestfailed') through the
// pinned Playwright and through Neo (network.mjs): every request of a page reaches both the
// page's and the context's listeners as one object, with its frame (main or the child frame that
// made it), resource type and navigation flag; a failed fetch fires requestfailed with its
// failure text and no response; a redirect's hops, a 204 and their events in the order one
// request reports them. Concurrent requests are compared as sorted lists.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    // tests/page/page-event-network.spec.ts "NE-084 Page.Events.Request @smoke", "NE-085
    // Page.Events.Response @smoke", "NE-087 Page.Events.RequestFinished @smoke";
    // tests/page/page-event-request.spec.ts "NE-091 should fire for navigation requests",
    // "NE-092 should fire for iframes", "NE-093 should fire for fetches";
    // tests/library/browsercontext-network-event.spec.ts "NE-121 BrowserContext.Events.Request",
    // "NE-122 BrowserContext.Events.Response", "NE-124 BrowserContext.Events.RequestFinished".
    name: "a page's subresource and iframe requests reach page and context listeners with their frame",
    source: `const seen = []
    const fromContext = new Map()
    page.on('request', (request) => seen.push(['request', request]))
    page.on('response', (response) => seen.push(['response', response.request()]))
    page.on('requestfinished', (request) => seen.push(['finished', request]))
    // Playwright's context listeners run before the page's, with the same object.
    context.on('request', (request) => {
      // The context also reports the browser's service workers' requests, which have no frame.
      if (!request.serviceWorker() && request.frame().page() === page) fromContext.set(request.url(), request)
    })
    let documentResponse
    page.on('response', (response) => {
      if (response.url().endsWith('/net/page.html')) documentResponse = response
    })
    const answered = await page.goto(ORIGIN + '/net/page.html')
    await page.waitForLoadState('networkidle')
    // Frames are told apart once the page settled: a frame has its name only from its commit.
    const child = page.frame({ name: 'child' })
    const frameOf = (request) =>
      request.frame() === page.mainFrame() ? 'main' : request.frame() === child ? 'child' : 'other'
    return {
      seen: seen
        .map(([event, request]) => JSON.stringify([event, path(request.url()), request.resourceType(), frameOf(request), request.isNavigationRequest()]))
        .sort(),
      sameObjects: seen.every(([event, request]) => event !== 'request' || fromContext.get(request.url()) === request),
      gotoIsTheEventsResponse: answered === documentResponse && answered.request() === seen.find(([event, request]) => event === 'request' && request.url() === answered.url())[1],
      childUrl: path(child.url()),
    }`,
    expected: {
      seen: [
        ['finished', '/net/data.json', 'fetch', 'main', false],
        ['finished', '/net/frame.html', 'document', 'child', true],
        ['finished', '/net/img.png', 'image', 'main', false],
        ['finished', '/net/in-frame.json', 'fetch', 'child', false],
        ['finished', '/net/page.html', 'document', 'main', true],
        ['request', '/net/data.json', 'fetch', 'main', false],
        ['request', '/net/frame.html', 'document', 'child', true],
        ['request', '/net/img.png', 'image', 'main', false],
        ['request', '/net/in-frame.json', 'fetch', 'child', false],
        ['request', '/net/page.html', 'document', 'main', true],
        ['response', '/net/data.json', 'fetch', 'main', false],
        ['response', '/net/frame.html', 'document', 'child', true],
        ['response', '/net/img.png', 'image', 'main', false],
        ['response', '/net/in-frame.json', 'fetch', 'child', false],
        ['response', '/net/page.html', 'document', 'main', true],
      ]
        .map((entry) => JSON.stringify(entry))
        .sort(),
      sameObjects: true,
      gotoIsTheEventsResponse: true,
      childUrl: '/net/frame.html',
    },
  },
  {
    // tests/page/page-event-network.spec.ts "NE-086 Page.Events.RequestFailed @smoke";
    // tests/library/browsercontext-network-event.spec.ts "NE-123 BrowserContext.Events.RequestFailed".
    name: 'a failed fetch fires requestfailed with its failure text and no response',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const events = []
    page.on('requestfinished', (request) => events.push('finished ' + path(request.url())))
    const hangup = (request) => request.url().endsWith('/net/hangup')
    const failed = page.waitForEvent('requestfailed', hangup)
    // The context also reports the browser's service workers' requests: wait for this one.
    const fromContext = context.waitForEvent('requestfailed', hangup)
    await page.evaluate(() => fetch('/net/hangup').catch(() => 'rejected'))
    const request = await failed
    return {
      url: path(request.url()),
      failure: request.failure(),
      sameObject: request === (await fromContext),
      response: await request.response(),
      existing: request.existingResponse(),
      events,
    }`,
    expected: {
      url: '/net/hangup',
      failure: { errorText: 'net::ERR_EMPTY_RESPONSE' },
      sameObject: true,
      response: null,
      existing: null,
      events: [],
    },
  },
  {
    // tests/page/page-event-network.spec.ts "NE-088 should fire events in proper order", "NE-089
    // should support redirects"; tests/page/page-event-request.spec.ts "NE-102 should finish 204
    // request".
    name: "a redirect's hops and a 204 report their events in order, linked hop to hop",
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const events = []
    page.on('request', (request) => events.push('request ' + path(request.url())))
    page.on('response', (response) => events.push('response ' + path(response.url()) + ' ' + response.status()))
    page.on('requestfinished', (request) => events.push('finished ' + path(request.url())))
    page.on('requestfailed', (request) => events.push('failed ' + path(request.url())))
    const finished = page.waitForEvent('requestfinished', (request) => request.url().endsWith('/net/data.json'))
    await page.evaluate(() => fetch('/net/redirect').then((r) => r.text()))
    const last = await finished
    const first = last.redirectedFrom()
    const noContent = page.waitForEvent('requestfinished', (request) => request.url().endsWith('/net/204'))
    await page.evaluate(() => fetch('/net/204').then((r) => r.status))
    const finished204 = await noContent
    return {
      events,
      first: path(first.url()),
      firstStatus: (await first.response()).status(),
      linked: first.redirectedTo() === last && last.redirectedFrom() === first,
      lastStatus: (await last.response()).status(),
      status204: (await finished204.response()).status(),
      failure204: finished204.failure(),
    }`,
    expected: {
      events: [
        'request /net/redirect',
        'response /net/redirect 302',
        'finished /net/redirect',
        'request /net/data.json',
        'response /net/data.json 200',
        'finished /net/data.json',
        'request /net/204',
        'response /net/204 204',
        'finished /net/204',
      ],
      first: '/net/redirect',
      firstStatus: 302,
      linked: true,
      lastStatus: 200,
      status204: 204,
      failure204: null,
    },
  },
  {
    // tests/page/page-event-request.spec.ts "NE-092 should fire for iframes", for an
    // out-of-process frame: its fetch reaches the page's listeners with that frame, and its body
    // reads. Neo reports such a frame's requests once the run has reached the frame (here
    // `page.frames()`), and then from the frame's own CDP session.
    name: "an out-of-process frame's fetch reaches the page's listeners with that frame",
    source: `await page.goto(ORIGIN + '/net/oopif-top.html')
    let cross
    for (let tries = 0; !cross && tries < 150; tries++) {
      cross = page.frames().find((frame) => frame.url().endsWith('/net/oopif-child.html'))
      if (!cross) await new Promise((resolve) => setTimeout(resolve, 100))
    }
    await cross.waitForLoadState('load')
    const seen = []
    page.on('request', (request) => seen.push(['request', path(request.url()), request.frame() === cross, request.resourceType()]))
    page.on('requestfinished', (request) => seen.push(['finished', path(request.url()), request.frame() === cross]))
    const finished = page.waitForEvent('requestfinished', (request) => request.url().endsWith('/net/oopif-data.json'))
    await cross.click('#fetch')
    const request = await finished
    const response = await request.response()
    return {
      seen: seen.filter(([, url]) => url === '/net/oopif-data.json'),
      origin: request.url().startsWith('http://localhost:'),
      status: response.status(),
      body: await response.json(),
    }`,
    expected: {
      seen: [
        ['request', '/net/oopif-data.json', true, 'fetch'],
        ['finished', '/net/oopif-data.json', true],
      ],
      origin: true,
      status: 200,
      body: { path: '/net/oopif-data.json', query: '' },
    },
  },
  {
    // B2 review (review-review-b2.md, P2 1): removing an out-of-process frame whose response is
    // still streaming. Playwright fails the frame's in-flight requests (server/frames.ts
    // `Frame._failInflightRequests`, `Request._failFromDetachedDocument`): the body read rejects,
    // `requestfailed` fires with `net::ERR_ABORTED`, `finished()` never settles in either engine,
    // and the page reaches networkidle.
    name: 'removing an out-of-process frame fails its loading request and its body read',
    source: `await page.goto(ORIGIN + '/net/oopif-top.html')
    let cross
    for (let tries = 0; !cross && tries < 150; tries++) {
      cross = page.frames().find((frame) => frame.url().endsWith('/net/oopif-child.html'))
      if (!cross) await new Promise((resolve) => setTimeout(resolve, 100))
    }
    await cross.waitForLoadState('load')
    const failures = []
    page.on('requestfailed', (request) => failures.push([path(request.url()), request.failure()?.errorText, request.frame() === cross]))
    const waiting = page.waitForResponse('**/net/hold?id=detach')
    await cross.evaluate(() => { fetch('/net/hold?id=detach').then((r) => r.text()).catch(() => {}) })
    const response = await waiting
    const within = (promise) => Promise.race([
      promise.then((value) => ({ value: value === null ? null : typeof value }), (error) => ({ error: errorCategory(error) })),
      new Promise((resolve) => setTimeout(() => resolve('pending'), 2000)),
    ])
    const body = within(response.text())
    const finished = within(response.finished())
    await page.evaluate(() => document.querySelector('iframe').remove())
    const idle = await settle(page.waitForLoadState('networkidle', { timeout: 5000 }))
    return { body: await body, finished: await finished, idle, failures }`,
    expected: {
      body: { error: 'Error' },
      finished: 'pending',
      idle: 'undefined',
      failures: [['/net/hold?id=detach', 'net::ERR_ABORTED', true]],
    },
  },
  {
    // server/frames.ts `FrameManager._removeFramesRecursively`: a removed frame's in-flight
    // requests, its document's included, fail (`Frame._failInflightRequests`) before
    // `framedetached`; the document's body read settles and `finished()` never does.
    name: 'removing a frame whose document still loads fails its document request before framedetached',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const events = []
    page.on('requestfailed', (request) => events.push(['requestfailed', path(request.url()), request.failure()?.errorText, request.isNavigationRequest()]))
    page.on('framedetached', () => events.push(['framedetached']))
    const waiting = page.waitForResponse('**/net/hold?id=doc*')
    await page.evaluate(() => {
      const frame = document.createElement('iframe')
      frame.src = '/net/hold?id=doc&type=html'
      document.body.append(frame)
    })
    const response = await waiting
    const within = (promise) => Promise.race([
      promise.then((value) => ({ value: value === null ? null : typeof value }), (error) => ({ error: errorCategory(error) })),
      new Promise((resolve) => setTimeout(() => resolve('pending'), 2000)),
    ])
    const body = within(response.text())
    const finished = within(response.finished())
    await page.evaluate(() => document.querySelector('iframe').remove())
    const idle = await settle(page.waitForLoadState('networkidle', { timeout: 5000 }))
    return { body: await body, finished: await finished, idle, events }`,
    expected: {
      body: { error: 'Error' },
      finished: 'pending',
      idle: 'undefined',
      events: [
        [
          'requestfailed',
          '/net/hold?id=doc&type=html',
          'net::ERR_ABORTED',
          true,
        ],
        ['framedetached'],
      ],
    },
  },
  {
    // An out-of-process frame navigating back to its parent's site moves into the parent's
    // process: its frame session goes away (Playwright: server/chromium/crPage.ts
    // `FrameSession._onDetachedFromTarget` disposes it) while the new document's request goes on
    // and finishes on the parent's session.
    name: "an out-of-process frame moving back into its parent's process keeps its navigation request",
    source: `await page.goto(ORIGIN + '/net/oopif-top.html')
    let cross
    for (let tries = 0; !cross && tries < 150; tries++) {
      cross = page.frames().find((frame) => frame.url().endsWith('/net/oopif-child.html'))
      if (!cross) await new Promise((resolve) => setTimeout(resolve, 100))
    }
    await cross.waitForLoadState('load')
    const events = []
    const ours = (url) => url.includes('/net/hold?id=back')
    page.on('request', (request) => { if (ours(request.url())) events.push('request') })
    page.on('response', (response) => { if (ours(response.url())) events.push('response') })
    page.on('requestfinished', (request) => { if (ours(request.url())) events.push('requestfinished') })
    page.on('requestfailed', (request) => { if (ours(request.url())) events.push('requestfailed ' + request.failure()?.errorText) })
    const waiting = page.waitForResponse('**/net/hold?id=back*')
    await cross.evaluate((url) => { location.href = url }, ORIGIN + '/net/hold?id=back&type=html')
    await waiting
    // Time for the frame's move and its old session's detach.
    await new Promise((resolve) => setTimeout(resolve, 1000))
    await page.evaluate(() => fetch('/net/release?id=back').then((r) => r.text()))
    for (let tries = 0; tries < 30 && events.length < 3; tries++)
      await new Promise((resolve) => setTimeout(resolve, 100))
    return { events }`,
    expected: { events: ['request', 'response', 'requestfinished'] },
  },
  {
    // tests/page/page-event-network.spec.ts "NE-084 Page.Events.Request @smoke" for a response
    // the browser serves from its cache: the second fetch of a cacheable URL is still a request,
    // a response and a finish, with the cached status and body.
    name: 'a request the cache answers still reports its request, response and finish',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const url = '/net/cached.json?' + Math.random()
    const events = []
    page.on('request', (request) => { if (request.url().includes('/net/cached.json')) events.push('request') })
    page.on('response', (response) => { if (response.url().includes('/net/cached.json')) events.push('response ' + response.status()) })
    page.on('requestfinished', (request) => { if (request.url().includes('/net/cached.json')) events.push('finished') })
    await page.evaluate((url) => fetch(url).then((r) => r.text()), url)
    const [cached] = await Promise.all([
      page.waitForResponse('**/net/cached.json*'),
      page.evaluate((url) => fetch(url, { cache: 'force-cache' }).then((r) => r.text()), url),
    ])
    await page.waitForLoadState('networkidle')
    return {
      events,
      status: cached.status(),
      body: (await cached.json()).path,
      ok: cached.ok(),
      fromServiceWorker: cached.fromServiceWorker(),
    }`,
    expected: {
      events: [
        'request',
        'response 200',
        'finished',
        'request',
        'response 200',
        'finished',
      ],
      status: 200,
      body: '/net/cached.json',
      ok: true,
      fromServiceWorker: false,
    },
  },
  {
    // tests/page/page-network-request.spec.ts and page-network-response.spec.ts read the same
    // values off goto's objects; here off objects an event handed out: provisional and raw
    // headers, post data, sizes, the server's address, the HTTP version, the service-worker
    // fields, and timing's shape (its values are a clock's, never compared).
    name: "an event's request and response describe themselves as Playwright's",
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const [request] = await Promise.all([
      page.waitForRequest('**/net/echo'),
      page.evaluate(() => fetch('/net/echo', {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-neo': 'yes' },
        body: JSON.stringify({ a: 1 }),
      }).then((r) => r.text())),
    ])
    const response = await request.response()
    const all = await request.allHeaders()
    const responseHeaders = await response.allHeaders()
    const timing = request.timing()
    const sizes = await request.sizes()
    return {
      method: request.method(),
      postData: request.postData(),
      postDataJSON: request.postDataJSON(),
      postDataBuffer: request.postDataBuffer().toString('utf8'),
      provisional: [request.headers()['content-type'], request.headers()['x-neo']],
      raw: [all['content-type'], all['x-neo'], await request.headerValue('X-Neo')],
      rawNames: (await request.headersArray()).map((header) => header.name.toLowerCase()).sort(),
      status: [response.status(), response.statusText(), response.ok(), await response.text()],
      responseHeaders: [responseHeaders['content-type'], responseHeaders['cache-control'], await response.headerValue('Content-Type')],
      responseNames: (await response.headersArray()).map((header) => header.name.toLowerCase()).sort(),
      headerValues: await response.headerValues('cache-control'),
      sizes,
      serverAddr: (await response.serverAddr()).ipAddress,
      securityDetails: await response.securityDetails(),
      httpVersion: await response.httpVersion(),
      fromServiceWorker: response.fromServiceWorker(),
      serviceWorker: request.serviceWorker(),
      timingKeys: Object.keys(timing).sort(),
      timingTypes: Object.values(timing).every((value) => typeof value === 'number'),
      ended: timing.responseEnd >= 0,
      frame: request.frame() === page.mainFrame() && response.frame() === page.mainFrame(),
    }`,
    expected: {
      method: 'POST',
      postData: '{"a":1}',
      postDataJSON: { a: 1 },
      postDataBuffer: '{"a":1}',
      provisional: ['application/json', 'yes'],
      raw: ['application/json', 'yes', 'yes'],
      rawNames: [
        'accept',
        'accept-encoding',
        'accept-language',
        'connection',
        'content-length',
        'content-type',
        'host',
        'origin',
        'referer',
        'sec-ch-ua',
        'sec-ch-ua-mobile',
        'sec-ch-ua-platform',
        'sec-fetch-dest',
        'sec-fetch-mode',
        'sec-fetch-site',
        'user-agent',
        'x-neo',
      ],
      status: [200, 'OK', true, 'POST {"a":1}'],
      responseHeaders: ['text/plain', 'no-store', 'text/plain'],
      responseNames: [
        'cache-control',
        'connection',
        'content-length',
        'content-type',
        'date',
        'keep-alive',
      ],
      headerValues: ['no-store'],
      // The same browser sends the same headers to the same server, so the sizes are equal.
      sizes: {
        requestBodySize: 7,
        requestHeadersSize: 620,
        responseBodySize: 12,
        responseHeadersSize: 174,
      },
      serverAddr: '127.0.0.1',
      // Chromium's is an object even without TLS.
      securityDetails: {},
      httpVersion: 'HTTP/1.1',
      fromServiceWorker: false,
      serviceWorker: null,
      timingKeys: [
        'connectEnd',
        'connectStart',
        'domainLookupEnd',
        'domainLookupStart',
        'requestStart',
        'responseEnd',
        'responseStart',
        'secureConnectionStart',
        'startTime',
      ],
      timingTypes: true,
      ended: true,
      frame: true,
    },
  },
]

test('request events match Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
