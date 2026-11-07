// route.fulfill: a fulfilled request answers with the status, headers and body given, without
// the network, through the pinned Playwright and through Neo (network.mjs). Cases from the
// pinned tests (26137face):
//
// - tests/page/page-request-fulfill.spec.ts RO-096 "should work": a fulfilled document.
// - RO-116 "should fulfill json", on an XMLHttpRequest: a fulfilled XHR.
// - tests/page/page-route.spec.ts RO-035 "should add Access-Control-Allow-Origin by default when
//   fulfill": a cross-origin POST's preflight is answered and its fulfilled response allowed.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'a fulfilled document shows the status, headers and body given',
    source: `await page.route('**/net/blank.html', (route) => route.fulfill({
      status: 201,
      headers: { foo: 'bar' },
      contentType: 'text/html',
      body: 'Yo, page!',
    }))
    const response = await page.goto(ORIGIN + '/net/blank.html')
    return {
      status: response.status(),
      statusText: response.statusText(),
      foo: response.headers().foo,
      type: response.headers()['content-type'],
      text: await response.text(),
      shown: await page.evaluate(() => document.body.textContent),
    }`,
    expected: {
      status: 201,
      statusText: 'Created',
      foo: 'bar',
      type: 'text/html',
      text: 'Yo, page!',
      shown: 'Yo, page!',
    },
  },
  {
    name: 'a fulfilled XHR reads the json body and its headers',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    await page.route('**/net/data.json', (route) => route.fulfill({ status: 201, headers: { foo: 'bar' }, json: { bar: 'baz' } }))
    const [response, xhr] = await Promise.all([
      page.waitForResponse('**/net/data.json'),
      page.evaluate(() => new Promise((resolve) => {
        const request = new XMLHttpRequest()
        request.open('GET', '/net/data.json')
        request.onload = () => resolve([request.status, request.getResponseHeader('foo'), request.getResponseHeader('content-type'), request.responseText])
        request.send()
      })),
    ])
    return { xhr, type: response.request().resourceType(), status: response.status(), json: await response.json() }`,
    expected: {
      xhr: [201, 'bar', 'application/json', '{"bar":"baz"}'],
      type: 'xhr',
      status: 201,
      json: { bar: 'baz' },
    },
  },
  {
    name: "a cross-origin POST's preflight is answered and its fulfilled response allowed",
    source: `await page.goto(ORIGIN + '/net/blank.html')
    await page.route(CROSS + '/net/cars', (route) => route.fulfill({
      contentType: 'application/json',
      status: 200,
      body: JSON.stringify(['electric', 'gas']),
    }))
    const [result, response] = await Promise.all([
      page.evaluate((url) => fetch(url, {
        method: 'POST',
        mode: 'cors',
        headers: { 'Content-Type': 'application/json' },
        body: '{"number":1}',
      }).then((r) => r.json()), CROSS + '/net/cars'),
      page.waitForResponse(CROSS + '/net/cars'),
    ])
    return {
      result,
      method: response.request().method(),
      status: response.status(),
      allowOrigin: (await response.headerValue('access-control-allow-origin')) === ORIGIN,
    }`,
    expected: {
      result: ['electric', 'gas'],
      method: 'POST',
      status: 200,
      allowOrigin: true,
    },
  },
]

test('route.fulfill matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
