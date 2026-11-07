// page.waitForTimeout and frame.waitForTimeout through the pinned Playwright and through Neo, each
// case once per receiver, on fresh tabs of tests/pages/wait-for.html: a delay waited out that
// resolves `undefined`; every delay Node's setTimeout rule turns into 1 ms (0, negative, below 1,
// NaN, ±Infinity, past 2^31 − 1) resolving while a 3 s timer is still pending; no action timeout
// applying; and a page closed under the wait, which rejects it as closed. Compared: what each call
// settled to (`waits.mjs` `settle`). The only times measured are those lower bounds.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, waits.mjs).

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { withBoth } from './waits.mjs'

const html = readFileSync(
  new URL('../pages/wait-for.html', import.meta.url),
  'utf8',
)
const RECEIVERS = ['page', 'page.mainFrame()']
const RESOLVED = 'undefined'

const cases = [
  {
    name: 'waits out its delay and resolves undefined',
    source: (frame) => `const started = Date.now()
    const waited = await settle(${frame}.waitForTimeout(300))
    return { waited, atLeastTheDelay: Date.now() - started >= 300 }`,
    expected: { waited: RESOLVED, atLeastTheDelay: true },
  },
  {
    // A delay that is waited out as given would still be pending when the 3 s timer fires.
    name: "a delay Node's rule turns into 1 ms resolves at once",
    source: (frame) => `const quick = (wait) => Promise.race([
      settle(wait),
      new Promise((resolve) => setTimeout(() => resolve('still waiting'), 3000)),
    ])
    return {
      zero: await quick(${frame}.waitForTimeout(0)),
      negative: await quick(${frame}.waitForTimeout(-5)),
      belowOne: await quick(${frame}.waitForTimeout(0.5)),
      nan: await quick(${frame}.waitForTimeout(NaN)),
      infinity: await quick(${frame}.waitForTimeout(Infinity)),
      negativeInfinity: await quick(${frame}.waitForTimeout(-Infinity)),
      pastMax: await quick(${frame}.waitForTimeout(2 ** 31)),
      huge: await quick(${frame}.waitForTimeout(3e9)),
    }`,
    expected: {
      zero: RESOLVED,
      negative: RESOLVED,
      belowOne: RESOLVED,
      nan: RESOLVED,
      infinity: RESOLVED,
      negativeInfinity: RESOLVED,
      pastMax: RESOLVED,
      huge: RESOLVED,
    },
  },
  {
    name: 'no action timeout applies',
    source: (frame) => `page.setDefaultTimeout(100)
    return { waited: await settle(${frame}.waitForTimeout(400)) }`,
    expected: { waited: RESOLVED },
  },
  {
    name: 'a page closed under the wait rejects it as closed',
    closesPage: true,
    source: (frame) => `const waiting = settle(${frame}.waitForTimeout(20000))
    await page.close()
    return { waiting: await waiting }`,
    expected: { waiting: { error: 'closed' } },
  },
]

test('page and frame waitForTimeout settle the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await withBoth(t, { pages: { '/wait-for.html': html } }, async (run) => {
    for (const c of cases)
      for (const receiver of RECEIVERS) {
        await t.test(`${receiver}: ${c.name}`, async () => {
          const { playwright, neo } = await run({
            ...c,
            path: '/wait-for.html',
            source: c.source(receiver),
          })
          assert.deepEqual(
            playwright.result,
            { value: c.expected },
            'Playwright',
          )
          assert.deepEqual(neo.result, playwright.result, 'Neo vs Playwright')
        })
      }
  })
})
