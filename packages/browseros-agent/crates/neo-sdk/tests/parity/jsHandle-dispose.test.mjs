// JSHandle.dispose through the Playwright reference and through Neo on fresh evaluate.html pages:
// a disposed handle can no longer be evaluated or passed as an argument, a second dispose is a
// no-op, and an element handle disposes the same way. Errors by category and by the words
// Playwright's own tests check ("no object with guid").
//
// Cases from tests/page/page-evaluate.spec.ts at 26137face: EV-061 "should throw if underlying
// element was disposed"; client/jsHandle.ts `JSHandle.dispose`, which treats a disposed handle's
// TargetClosedError as done; and tests/page/frame-evaluate.spec.ts FR-003 "should dispose context
// on navigation" for a handle whose document is gone (a primitive handle still passes by value).
//
// Then what Neo adds and Playwright has no counterpart for, asserted on Neo alone: handles last
// one run. A guid kept from one run names nothing in the next: calling on it is a closed target,
// and passing it is an unknown guid.
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  compareMethod,
  connectCdp,
  errorCategory,
  launchBrowser,
  openTab,
  runCleanups,
  runInNeo,
  serveFixture,
  skip,
} from './harness.mjs'
import { VALUES } from './values.mjs'

const html = readFileSync(
  new URL('../pages/evaluate.html', import.meta.url),
  'utf8',
)

test('dispose releases a handle as Playwright does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const object = await page.evaluateHandle(() => ({ a: 1 }))
      const element = await page.evaluateHandle(() => document.querySelector('#s'))
      const kept = await page.evaluateHandle(() => ({ kept: true }))
      const beforeDispose = await outcome(() => object.evaluate((o) => o.a))
      const disposed = await outcome(() => object.dispose())
      const elementDisposed = await outcome(() => element.dispose())
      return {
        beforeDispose,
        disposed,
        elementDisposed,
        evaluateAfter: await outcome(() => object.evaluate((o) => o.a)),
        jsonValueAfter: await outcome(() => object.jsonValue()),
        argumentAfter: await outcome(() => page.evaluate((e) => e.textContent, element), ['no object with guid']),
        disposeTwice: await outcome(() => object.dispose()),
        othersStillWork: await outcome(() => kept.jsonValue()),
        previewAfter: object.toString(),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.disposed, { value: { undefined: true } })
  assert.deepEqual(outcome.value.argumentAfter, {
    error: 'Error',
    mentions: ['no object with guid'],
  })
})

test('a handle of a document that navigated away is refused as Playwright refuses it', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const object = await page.evaluateHandle(() => ({ a: 1 }))
      const five = await page.evaluateHandle(() => 5)
      await page.goto(page.url())
      return {
        evaluateAfter: await outcome(() => object.evaluate((o) => o.a), ['Execution context was destroyed']),
        argumentAfter: await outcome(() => page.evaluate((o) => o.a, object), ['JSHandles can be evaluated only in the context they were created']),
        primitiveAfter: await outcome(() => page.evaluate((n) => n * 2, five)),
        freshWorks: await outcome(async () => (await page.evaluateHandle(() => ({ b: 2 }))).jsonValue()),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.primitiveAfter, { value: 10 })
})

test('a handle from one Neo run names nothing in the next', {
  skip,
  timeout: 180_000,
}, async () => {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: { '/evaluate.html': html },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const { targetId } = await openTab(cdp, `${origin}/evaluate.html`)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-dispose-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const first = join(scripts, 'first.js')
    writeFileSync(
      first,
      `const handle = await page.evaluateHandle(() => ({ run: 1 }))
      return { guid: handle._guid, value: await handle.jsonValue() }`,
    )
    const one = await runInNeo(neoRun, browser.port, targetId, first)
    assert.deepEqual(one.value.value, { run: 1 })
    assert.match(one.value.guid, /^handle@[0-9a-f]{16}\.\d+$/)
    // A script cannot name another run's handle through the public API; it forges one from its
    // own handle's class and connection, which is what a kept guid amounts to.
    const second = join(scripts, 'second.js')
    writeFileSync(
      second,
      `const errorCategory = ${errorCategory.toString()}
      const own = await page.evaluateHandle(() => ({ run: 2 }))
      const Handle = own.constructor
      const stale = new Handle(own._connection, ${JSON.stringify(one.value.guid)}, { preview: 'stale' })
      const settle = (promise) => promise.then((value) => ({ value }), (error) => ({ error: errorCategory(error), message: error.message }))
      return {
        own: await settle(own.jsonValue()),
        call: await settle(stale.jsonValue()),
        argument: await settle(page.evaluate((x) => x, stale)),
      }`,
    )
    const two = await runInNeo(neoRun, browser.port, targetId, second)
    assert.deepEqual(two.value.own, { value: { run: 2 } })
    assert.equal(two.value.call.error, 'Error', JSON.stringify(two.value))
    assert.match(two.value.call.message, /has been closed/)
    assert.equal(two.value.argument.error, 'Error')
    assert.match(two.value.argument.message, /no object with guid/)
  } finally {
    await runCleanups(cleanups)
  }
})
