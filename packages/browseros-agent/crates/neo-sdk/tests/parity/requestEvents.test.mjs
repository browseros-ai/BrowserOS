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
    await page.goto(ORIGIN + '/net/page.html')
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
    const failed = page.waitForEvent('requestfailed')
    const fromContext = context.waitForEvent('requestfailed')
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
]

test('request events match Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
