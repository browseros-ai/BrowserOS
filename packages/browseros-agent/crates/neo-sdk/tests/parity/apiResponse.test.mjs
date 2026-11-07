// APIResponse's metadata: its headers (Neo's names lowercased, a Deviation), where it came from
// (serverAddr) and when (timing), and no TLS details over http, through the pinned Playwright and
// through Neo (network.mjs). Cases from the pinned tests (26137face):
//
// - tests/library/browsercontext-fetch.spec.ts FE-001-style headersArray, compared by lowercased
//   names (Node sends `Date`, `Connection` capitalized; Neo's HTTP client keeps no case).
// - serverAddr and timing of tests/library/global-fetch.spec.ts (v1.61, v1.62).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: "a response's headers, server and timing",
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const response = await page.request.get(ORIGIN + '/net/data.json')
    const names = response.headersArray().map((header) => header.name.toLowerCase()).sort()
    const server = await response.serverAddr()
    const timing = response.timing()
    return {
      names,
      contentType: response.headersArray().find((header) => header.name.toLowerCase() === 'content-type')?.value,
      server: [server?.ipAddress, server?.port === Number(ORIGIN.split(':').pop())],
      timing: [timing.startTime > 0, timing.responseEnd >= 0],
      tls: await response.securityDetails(),
    }`,
    expected: {
      names: [
        'cache-control',
        'connection',
        'content-length',
        'content-type',
        'date',
        'keep-alive',
      ],
      contentType: 'application/json',
      server: ['127.0.0.1', true],
      timing: [true, true],
      tls: null,
    },
  },
]

test('APIResponse metadata matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
