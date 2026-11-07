// page.waitForFunction and frame.waitForFunction through the Playwright reference and through Neo
// on fresh evaluate.html pages: a predicate (or expression string) polled on animation frames or
// on an interval until it is truthy, resolving a handle to that value; what each refuses or fails
// with (bad polling, a throwing predicate, a timeout), no polling after a timeout, and a wait that
// outlives a navigation of its page. Values by `describe` (values.mjs), errors by category and by
// the words Playwright's own tests look for; never by timing.
//
// Cases from tests/page/page-wait-for-function.spec.ts at 26137face: WA-002 "should accept a
// string", WA-004 "should poll on interval" (with the page's own Date), WA-005 "should avoid side
// effects after timeout", WA-006 "should throw on polling:mutation", WA-007 "should poll on raf",
// WA-008 "should fail with predicate throwing on first call", WA-009 "should fail with predicate
// throwing sometimes", WA-010 "should fail with ReferenceError on wrong page", WA-013 "should throw
// negative polling interval", WA-014 "should return the success value as a JSHandle", WA-015
// "should return the window as a success value", WA-016 "should accept ElementHandle arguments",
// WA-019 "should disable timeout when its set to 0", WA-021 "should survive navigations", WA-023
// "should wait for predicate with arguments".
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = readFileSync(
  new URL('../pages/evaluate.html', import.meta.url),
  'utf8',
)

test('waitForFunction resolves, refuses and fails as Playwright does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const results = {}
      const string = page.waitForFunction('window.__FOO === 1')
      await page.evaluate(() => { window.__FOO = 1 })
      results.string = await outcome(async () => (await string).jsonValue())
      const raf = page.waitForFunction(() => window.__BAR === 'hit', {}, { polling: 'raf' })
      await page.evaluate(() => { window.__BAR = 'hit' })
      results.raf = await outcome(async () => (await raf).jsonValue())
      results.interval = await outcome(async () => {
        const delta = await page.waitForFunction(() => {
          if (!window.__startTime) { window.__startTime = Date.now(); return false }
          return Date.now() - window.__startTime
        }, {}, { polling: 100 })
        return (await delta.jsonValue()) >= 100
      })
      results.value = await outcome(async () => (await page.waitForFunction(() => 5)).jsonValue())
      results.window = await outcome(async () => (await page.waitForFunction(() => window)).toString())
      results.arguments = await outcome(async () => (await page.waitForFunction(({ a, b }) => a + b === 3 && 'sum', { a: 1, b: 2 })).jsonValue())
      results.elementArgument = await outcome(async () => {
        const para = await page.$('#para')
        const done = page.waitForFunction((element) => !element.parentElement, para)
        await page.evaluate((element) => element.remove(), para)
        return (await done).toString()
      })
      results.noTimeout = await outcome(async () => {
        const done = page.waitForFunction(() => window.__LATE, undefined, { timeout: 0 })
        await page.evaluate(() => setTimeout(() => { window.__LATE = 'late' }, 300))
        return (await done).jsonValue()
      })
      results.badPolling = await outcome(() => page.waitForFunction(() => true, {}, { polling: 'mutation' }), ['Unknown polling option'])
      results.negativePolling = await outcome(() => page.waitForFunction(() => true, {}, { polling: -10 }), ['Cannot poll with non-positive interval'])
      results.throwsFirst = await outcome(() => page.waitForFunction(() => { throw new Error('oh my') }), ['oh my'])
      results.throwsLater = await outcome(() => page.waitForFunction(() => {
        window.__counter = (window.__counter || 0) + 1
        if (window.__counter === 3) throw new Error('Bad counter!')
        return window.__counter === 5 ? 'result' : false
      }), ['Bad counter!'])
      results.referenceError = await outcome(() => page.waitForFunction(() => globalVar === 123), ['globalVar'])
      results.timeout = await outcome(() => page.waitForFunction(() => {
        window.__sideEffects = (window.__sideEffects || 0) + 1
        return false
      }, {}, { polling: 1, timeout: 500 }), ['Timeout'])
      const saved = await page.evaluate(() => window.__sideEffects)
      await new Promise((resolve) => setTimeout(resolve, 600))
      results.noPollAfterTimeout = (await page.evaluate(() => window.__sideEffects)) === saved
      results.frameForm = await outcome(async () => (await page.mainFrame().waitForFunction(() => document.title)).jsonValue())
      return results
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.value, { value: 5 })
  assert.equal(outcome.value.timeout.error, 'TimeoutError')
  assert.equal(outcome.value.noPollAfterTimeout, true)
})

test('waitForFunction survives a navigation of its page', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const done = page.waitForFunction(() => window.__navigated === true)
      await page.goto(page.url())
      await page.evaluate(() => { window.__navigated = true })
      return { survived: await outcome(async () => (await done).jsonValue()) }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, { survived: { value: true } })
})
