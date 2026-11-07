// APIRequestContext timeouts: one deadline for the whole call, body included, per call and per
// context, through the pinned Playwright and through Neo (network.mjs). Cases from the pinned
// tests (26137face):
//
// - tests/library/browsercontext-fetch.spec.ts FE-148 "should support timeout option": a body
//   that never ends times the call out.
// - tests/library/global-fetch.spec.ts FE-004 "should support global timeout option".
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: "a call's timeout covers a body that never ends",
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const failed = await page.request.get(ORIGIN + '/net/hold?id=api-timeout-1', { timeout: 300 }).then(() => null, (error) => [error.name, error.message.split('\\n')[0]])
    await page.goto(ORIGIN + '/net/release?id=api-timeout-1')
    return failed`,
    expected: [
      'TimeoutError',
      'apiRequestContext.get: Timeout 300ms exceeded.',
    ],
  },
  {
    name: "a context's timeout is its calls' default",
    source: `const request = await playwright.request.newContext({ timeout: 300 })
    const failed = await request.get(ORIGIN + '/net/hold?id=api-timeout-2').then(() => null, (error) => [error.name, error.message.split('\\n')[0]])
    await request.dispose()
    await page.goto(ORIGIN + '/net/release?id=api-timeout-2')
    return failed`,
    expected: [
      'TimeoutError',
      'apiRequestContext.get: Timeout 300ms exceeded.',
    ],
  },
]

test('APIRequestContext timeouts match Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
