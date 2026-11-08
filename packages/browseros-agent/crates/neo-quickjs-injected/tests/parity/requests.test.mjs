// page.requests() through the pinned Playwright and through Neo (network.mjs): the page's recent
// requests in the order they started, as the same objects its events hand out, Playwright's
// last-100 window (past 100 the oldest tenth goes), and every request still usable after it left
// that window.
//
// Neo records a page's requests from the run's first look at its network (here the goto); a page
// whose requests predate the run is Neo's recorded deviation (`Page.requests`), not compared.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    // tests/page/page-event-request.spec.ts "NE-091 should fire for navigation requests", "NE-093
    // should fire for fetches", read back through requests().
    name: "a page's requests, in start order, are its events' objects",
    source: `const fromEvents = []
    page.on('request', (request) => fromEvents.push(request))
    await page.goto(ORIGIN + '/net/blank.html')
    await page.evaluate(() => fetch('/net/digits/1.json').then((r) => r.text()))
    await page.evaluate(() => fetch('/net/redirect').then((r) => r.text()))
    const requests = await page.requests()
    return {
      paths: requests.map((request) => path(request.url())),
      sameObjects: requests.length === fromEvents.length && requests.every((request, i) => request === fromEvents[i]),
      again: (await page.requests()).every((request, i) => request === requests[i]),
    }`,
    expected: {
      paths: [
        '/net/blank.html',
        '/net/digits/1.json',
        '/net/redirect',
        '/net/data.json',
      ],
      sameObjects: true,
      again: true,
    },
  },
  {
    // page-event-request.spec.ts "NE-107 should return last requests".
    name: 'the last requests stay listed and usable past the window',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    for (let i = 0; i < 99; ++i)
      await page.evaluate((i) => fetch('/net/fetch/' + i + '.json').then((r) => r.text()), i)
    const first99 = await page.requests()
    first99.shift()
    for (let i = 99; i < 199; ++i)
      await page.evaluate((i) => fetch('/net/fetch/' + i + '.json').then((r) => r.text()), i)
    const last100 = await page.requests()
    const all = [...first99, ...last100]
    const received = []
    for (const request of all) {
      const response = await request.response()
      received.push(path(request.url()) + ' ' + (await response.json()).path)
    }
    return {
      firstCount: first99.length + 1,
      lastCount: last100.length,
      lastFirst: path(last100[0].url()),
      allUsable: received.every((line, i) => line === '/net/fetch/' + i + '.json /net/fetch/' + i + '.json'),
      total: received.length,
    }`,
    expected: {
      firstCount: 100,
      lastCount: 100,
      lastFirst: '/net/fetch/99.json',
      allUsable: true,
      total: 199,
    },
  },
]

test('requests() matches Playwright', { skip, timeout: 240_000 }, async (t) => {
  await compareNetwork(t, cases)
})
