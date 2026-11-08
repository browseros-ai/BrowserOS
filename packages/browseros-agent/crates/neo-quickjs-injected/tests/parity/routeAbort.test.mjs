// route.abort: an aborted request fails with the error code's text and no response, through the
// pinned Playwright and through Neo (network.mjs). Cases from the pinned tests (26137face):
//
// - tests/page/page-route.spec.ts RO-015 "should be abortable": the default code fails an image
//   with `net::ERR_FAILED` while its document loads.
// - RO-016 "should be abortable with custom error codes" and RO-019 "should fail navigation when
//   aborting main resource": `internetdisconnected` fails the document, and goto rejects.
// - RO-017 "should not throw if request was cancelled by the page": an abort after the page
//   cancelled the paused request resolves.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'the default code fails an image with net::ERR_FAILED while its document loads',
    source: `const failed = page.waitForEvent('requestfailed', (request) => request.url().endsWith('/net/img.png'))
    await page.route(/\\/net\\/img\\.png$/, (route) => route.abort())
    const response = await page.goto(ORIGIN + '/net/page.html')
    const image = await failed
    return {
      ok: response.ok(),
      documentFailure: response.request().failure(),
      failure: image.failure(),
      type: image.resourceType(),
      response: await image.response(),
      broken: await page.evaluate(() => [document.querySelector('img').complete, document.querySelector('img').naturalWidth]),
    }`,
    expected: {
      ok: true,
      documentFailure: null,
      failure: { errorText: 'net::ERR_FAILED' },
      type: 'image',
      response: null,
      broken: [true, 0],
    },
  },
  {
    name: 'a custom code fails the document with its text, and goto rejects',
    source: `const failed = page.waitForEvent('requestfailed', (request) => request.url().endsWith('/net/blank.html'))
    await page.route('**/net/blank.html', (route) => route.abort('internetdisconnected'))
    const navigation = await settle(page.goto(ORIGIN + '/net/blank.html'))
    const request = await failed
    return { navigation, errorText: request.failure().errorText, isNavigation: request.isNavigationRequest() }`,
    expected: {
      navigation: { error: 'Error' },
      errorText: 'net::ERR_INTERNET_DISCONNECTED',
      isNavigation: true,
    },
  },
  {
    name: 'an abort after the page cancelled its request resolves',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    let keep
    const routed = new Promise((resolve) => (keep = resolve))
    await page.route('**/net/data.json', (route) => keep(route))
    const failed = page.waitForEvent('requestfailed', (request) => request.url().endsWith('/net/data.json'))
    await page.evaluate(() => {
      globalThis.controller = new AbortController()
      fetch('/net/data.json', { signal: controller.signal }).catch(() => {})
    })
    const route = await routed
    await page.evaluate(() => globalThis.controller.abort())
    return { errorText: (await failed).failure().errorText, abort: await settle(route.abort()) }`,
    expected: { errorText: 'net::ERR_ABORTED', abort: 'undefined' },
  },
]

test('route.abort matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
