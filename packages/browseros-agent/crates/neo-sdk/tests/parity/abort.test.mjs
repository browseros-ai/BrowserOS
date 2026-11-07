// Playwright's `signal` option, end to end: the same script through Playwright (Node's
// `AbortController`) and through Neo (the realm's, neo-runtime `abort.rs`) on fresh pages of one
// fixture, comparing what each call settled with and what the page saw. Cases ported from the
// pinned tests (26137face):
//
// - tests/page/page-click.spec.ts CL-090 "should abort via signal": an abort while the click
//   waits rejects with `AbortError` whose `cause` is the reason, and the click never lands.
// - CL-091 "should throw an Error when aborted in-flight with a string reason": aborted at once.
// - CL-092 / CL-093 "should abort via already-aborted signal" (an Error and a string reason): the
//   call rejects before anything is sent.
// - tests/page/page-filechooser.spec.ts FC-015 "should abort with signal", on `console`: a
//   client-side `waitForEvent` rejects with `AbortError` when its signal aborts.
//
// Plus the realm's `AbortController`/`AbortSignal` against Node's, `waitForURL` aborted mid-wait,
// `AbortSignal.timeout`, a duck-typed signal, and an abort after the call settled. Errors are
// compared by category (`name`, whether `cause` is the reason), never by wording.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  launchBrowser,
  playwrightVersion,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
} from './harness.mjs'

const html = `<!doctype html><meta charset="utf-8"><title>abort</title>
<button id="hidden" type="button" style="display:none" onclick="window.__clicks++">Hidden</button>
<button id="target" type="button" onclick="window.__clicks++">Target</button>
<input id="field" value="start">
<script>window.__clicks = 0</script>`
const PROBE = `({
  clicks: window.__clicks,
  field: document.querySelector('#field').value,
  path: location.pathname,
})`
const UNTOUCHED = { clicks: 0, field: 'start', path: '/abort.html' }

/** How a call settled, as a script reads it: `resolved`, or the error's category against the
 * reason the script aborted with. */
const SETTLE = `const settle = (promise, reason) => promise.then(
  () => 'resolved',
  (error) => ({
    name: error.name,
    isError: error instanceof Error,
    causeIsReason: error.cause === reason,
    namesReason: typeof reason === 'string'
      ? error.message.includes(reason)
      : reason instanceof Error ? error.message.includes(reason.message) : null,
  }),
)
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
`

const REALM = `
const controller = new AbortController()
const signal = controller.signal
const order = []
const once = () => order.push('once')
signal.addEventListener('abort', once, { once: true })
signal.addEventListener('abort', once, { once: true })
signal.onabort = function (event) { order.push('onabort ' + event.type + ' ' + (this === signal) + ' ' + (event.target === signal)) }
// Node gives only the first listener a \`currentTarget\` (later ones read null), a quirk WHATWG
// does not have; the realm follows WHATWG, so neither is compared.
signal.addEventListener('abort', { handleEvent: (event) => order.push('object ' + (event.target === signal)) })
const removed = () => order.push('removed')
signal.addEventListener('abort', removed)
signal.removeEventListener('abort', removed)
const before = [signal.aborted, signal.reason === undefined]
controller.abort()
controller.abort('again')
const reason = signal.reason
let thrownIsReason = false
try { signal.throwIfAborted() } catch (error) { thrownIsReason = error === reason }
let illegal
try { new AbortSignal() } catch (error) { illegal = error.name }
const timed = AbortSignal.timeout(20)
await pause(100)
const first = new AbortController()
const second = new AbortController()
const any = AbortSignal.any([first.signal, second.signal])
let anyEvents = 0
any.addEventListener('abort', () => anyEvents++)
second.abort('second')
first.abort('first')
// A listener's own \`signal\` option removes it; \`capture\` tells two registrations apart.
let removedBySignal = 0
const listened = new AbortController()
const remover = new AbortController()
listened.signal.addEventListener('abort', () => removedBySignal++, { signal: remover.signal })
listened.signal.addEventListener('abort', () => removedBySignal++, { signal: AbortSignal.abort() })
remover.abort()
listened.abort()
let captured = 0
const counted = () => captured++
const captures = new AbortController()
captures.signal.addEventListener('abort', counted, true)
captures.signal.addEventListener('abort', counted, false)
captures.signal.removeEventListener('abort', counted, true)
captures.abort()
return {
  listenerOptions: [removedBySignal, captured],
  types: [typeof AbortController, typeof AbortSignal, Object.prototype.toString.call(signal), signal instanceof AbortSignal],
  before,
  after: [signal.aborted, thrownIsReason],
  reason: [reason instanceof DOMException, reason instanceof Error, reason.name, reason.message],
  order,
  illegal,
  presets: [AbortSignal.abort('why').reason, AbortSignal.abort().reason.name],
  timed: [timed.aborted, timed.reason.name, timed.reason.message],
  any: [any.aborted, any.reason, anyEvents, AbortSignal.any([AbortSignal.abort('why')]).reason],
}`

const ALREADY_ABORTED = `
const results = {}
for (const [label, reason] of [['Error', new Error('Already aborted')], ['string', 'already aborted']]) {
  const signal = AbortSignal.abort(reason)
  for (const [name, call] of [
    ['locator.click', () => page.locator('#target').click({ signal })],
    ['page.click', () => page.click('#target', { signal })],
    ['frame.click', () => page.mainFrame().click('#target', { signal })],
    ['locator.fill', () => page.locator('#field').fill('changed', { signal })],
    ['locator.waitFor', () => page.locator('#target').waitFor({ signal })],
    ['page.textContent', () => page.textContent('#target', { signal })],
    ['page.goto', () => page.goto(page.url().replace(/[^/]*$/, 'elsewhere'), { signal })],
    ['page.waitForURL', () => page.waitForURL('**/elsewhere', { signal })],
    ['page.waitForLoadState', () => page.waitForLoadState('load', { signal })],
    ['page.waitForEvent', () => page.waitForEvent('console', { signal })],
  ])
    results[label + ' ' + name] = await settle(call(), reason)
}
return results`

const MID_WAIT = `
const controller = new AbortController()
const reason = new Error('foo bar')
const pending = settle(page.locator('#hidden').click({ signal: controller.signal, timeout: 0 }), reason)
// Give the action time to start retrying before aborting, as CL-090 does.
await pause(500)
controller.abort(reason)
const settled = await pending
// A click still running after the abort would land now.
await page.locator('#target').evaluate(() => { document.querySelector('#hidden').style.display = '' })
await pause(700)
return settled`

const AT_ONCE = `
const controller = new AbortController()
const pending = settle(page.locator('#hidden').click({ signal: controller.signal, timeout: 0 }), 'aborted by user')
controller.abort('aborted by user')
return await pending`

// Playwright's waitForURL is a client-side waiter whose AbortError keeps its default message,
// while Neo's wait runs in Rust and replies with the reason: both compare by category only.
const WAITS = `
const category = (settled) => typeof settled === 'string' ? settled : { name: settled.name, causeIsReason: settled.causeIsReason }
const urls = new AbortController()
const url = settle(page.waitForURL('**/never', { signal: urls.signal, timeout: 0 }), 'stop url')
const predicates = new AbortController()
const predicate = settle(page.waitForURL((seen) => seen.pathname.endsWith('/nowhere'), { signal: predicates.signal, timeout: 0 }), 'stop predicate')
const events = new AbortController()
const event = settle(page.waitForEvent('console', { signal: events.signal, timeout: 0 }), 'stop event')
// One navigation the predicate sees and turns down, so its wait is mid-loop when aborted.
await page.goto(page.url().replace(/[^/]*$/, 'elsewhere'))
await pause(300)
urls.abort('stop url')
predicates.abort('stop predicate')
events.abort('stop event')
const settled = { url: category(await url), predicate: category(await predicate), event: category(await event) }
// The aborted waits are gone: the page navigates to what one waited for and nothing resolves late.
await page.goto(page.url().replace(/[^/]*$/, 'never'))
return settled`

const TIMEOUT_SIGNAL = `
const error = await page.locator('#hidden').click({ signal: AbortSignal.timeout(300), timeout: 0 }).then(
  () => 'resolved',
  (error) => error,
)
return { name: error.name, cause: error.cause && error.cause.name, causeIsDomException: error.cause instanceof DOMException }`

const DUCK = `
let listener
const signal = {
  aborted: false,
  reason: undefined,
  addEventListener(type, added) { listener = added },
  removeEventListener() {},
}
const pending = settle(page.locator('#hidden').click({ signal, timeout: 0 }), 'duck')
await pause(300)
signal.aborted = true
signal.reason = 'duck'
listener()
return await pending`

const AFTER = `
const controller = new AbortController()
await page.click('#target', { signal: controller.signal })
controller.abort('late')
return {
  next: await settle(page.click('#target', { signal: controller.signal }), 'late'),
  fresh: await settle(page.click('#target', { signal: new AbortController().signal }), undefined),
}`

// A screenshot aborted mid-capture: the element is attached but hidden, so the capture prepares
// the page (the caret style it injects) and then waits for it to show. The abort rejects it and
// the page is restored, as Playwright's cleanup restores it.
const SCREENSHOT = `
const styles = () => page.evaluate(() => document.querySelectorAll('style').length)
const before = await styles()
const controller = new AbortController()
const pending = settle(page.locator('#hidden').screenshot({ signal: controller.signal, timeout: 0 }), 'stop capture')
await pause(500)
const during = await styles()
controller.abort('stop capture')
const settled = await pending
await pause(300)
return { settled, prepared: during > before, restored: (await styles()) === before }`

// An aborted call ends only itself: a handle the run made before still works.
const HANDLES = `
const handle = await page.evaluateHandle(() => document.querySelector('#target'))
const controller = new AbortController()
const pending = settle(page.locator('#hidden').click({ signal: controller.signal, timeout: 0 }), 'stop')
await pause(300)
controller.abort('stop')
return { aborted: (await pending).name, handle: await handle.evaluate((element) => element.id) }`

test('signal aborts calls as Playwright does', {
  skip,
  timeout: 400_000,
}, async (t) => {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: {
        '/abort.html': html,
        '/elsewhere': '<title>elsewhere</title>',
        // The same page under the URL a wait never saw, so the probe reads it after the goto.
        '/never': html,
      },
    })
    const url = `${origin}/abort.html`
    const browser = await launchBrowser(cleanups, browserBinary)
    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    cleanups.push(() => playwright.close())
    const [context] = playwright.contexts()
    const scripts = mkdtempSync(join(tmpdir(), 'neo-abort-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const viaPlaywright = async (source) => {
      const page = await context.newPage()
      try {
        await page.goto(url)
        const AsyncFunction = (async () => {}).constructor
        const value = await new AsyncFunction('page', SETTLE + source)(page)
        return { value, effect: await page.evaluate(PROBE) }
      } finally {
        await page.close()
      }
    }
    const viaNeo = async (name, source) => {
      const page = await context.newPage()
      try {
        await page.goto(url)
        const session = await context.newCDPSession(page)
        const { targetInfo } = await session.send('Target.getTargetInfo')
        await session.detach()
        const path = join(scripts, `${name}.js`)
        writeFileSync(path, SETTLE + source)
        const neo = startNeo(neoRun, browser.port, targetInfo.targetId, path)
        await neo.connected
        const outcome = await neo.outcome
        assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
        return {
          outcome,
          value: outcome.value,
          effect: await page.evaluate(PROBE),
        }
      } finally {
        await page.close()
      }
    }
    const both = async (name, source) => {
      const reference = await viaPlaywright(source)
      const neo = await viaNeo(name, source)
      assert.deepEqual(
        neo.value,
        reference.value,
        'what each call settled with',
      )
      assert.deepEqual(neo.effect, reference.effect, 'what the page saw')
      return { reference, neo }
    }

    await t.test(
      "the realm's AbortController and AbortSignal behave as Node's",
      async () => {
        const { reference } = await both('realm', REALM)
        assert.deepEqual(reference.value.listenerOptions, [0, 1])
        assert.deepEqual(reference.value.reason, [
          true,
          true,
          'AbortError',
          'This operation was aborted',
        ])
        assert.deepEqual(reference.value.order, [
          'once',
          'onabort abort true true',
          'object true',
        ])
      },
    )

    await t.test(
      'an already-aborted signal rejects with AbortError before anything is sent',
      async () => {
        const { reference, neo } = await both(
          'already-aborted',
          ALREADY_ABORTED,
        )
        const expected = {
          name: 'AbortError',
          isError: true,
          causeIsReason: true,
          namesReason: false,
        }
        for (const [call, settled] of Object.entries(reference.value))
          assert.deepEqual(settled, expected, `Playwright ${call}`)
        assert.deepEqual(reference.effect, UNTOUCHED)
        // `page.waitForEvent` adds and removes its listener; no call reaches the browser.
        assert.deepEqual(neo.outcome.calls, [], 'Neo sent no call')
      },
    )

    await t.test(
      'an abort while the call waits rejects it, and the click never lands (CL-090)',
      async () => {
        const { reference, neo } = await both('mid-wait', MID_WAIT)
        assert.deepEqual(reference.value, {
          name: 'AbortError',
          isError: true,
          causeIsReason: true,
          namesReason: true,
        })
        assert.deepEqual(reference.effect, UNTOUCHED, 'no late click')
        const click = neo.outcome.calls.find((call) => call.method === 'click')
        assert.deepEqual(
          click?.result,
          { kind: 'error', name: 'AbortError', message: 'foo bar' },
          'Neo records the abort',
        )
      },
    )

    await t.test(
      'a string reason aborts a call in flight at once (CL-091)',
      async () => {
        const { reference } = await both('at-once', AT_ONCE)
        assert.deepEqual(reference.value, {
          name: 'AbortError',
          isError: true,
          causeIsReason: true,
          namesReason: true,
        })
        assert.deepEqual(reference.effect, UNTOUCHED)
      },
    )

    await t.test(
      'waitForURL and waitForEvent stop when their signal aborts (FC-015 on console)',
      async () => {
        const { reference } = await both('waits', WAITS)
        const aborted = { name: 'AbortError', causeIsReason: true }
        assert.deepEqual(reference.value, {
          url: aborted,
          predicate: aborted,
          event: aborted,
        })
        assert.deepEqual(reference.effect, { ...UNTOUCHED, path: '/never' })
      },
    )

    await t.test(
      'AbortSignal.timeout aborts a waiting call with its TimeoutError reason',
      async () => {
        const { reference } = await both('timeout-signal', TIMEOUT_SIGNAL)
        assert.deepEqual(reference.value, {
          name: 'AbortError',
          cause: 'TimeoutError',
          causeIsDomException: true,
        })
        assert.deepEqual(reference.effect, UNTOUCHED)
      },
    )

    await t.test(
      'a duck-typed signal aborts the call as an AbortSignal does',
      async () => {
        const { reference } = await both('duck', DUCK)
        assert.deepEqual(reference.value, {
          name: 'AbortError',
          isError: true,
          causeIsReason: true,
          namesReason: true,
        })
      },
    )

    await t.test(
      'an abort after the call settled changes nothing; the next call with it rejects',
      async () => {
        const { reference } = await both('after', AFTER)
        assert.deepEqual(reference.value, {
          next: {
            name: 'AbortError',
            isError: true,
            causeIsReason: true,
            namesReason: false,
          },
          fresh: 'resolved',
        })
        assert.deepEqual(reference.effect, { ...UNTOUCHED, clicks: 2 })
      },
    )

    await t.test(
      'a screenshot aborted mid-capture rejects and restores the page',
      async () => {
        const { reference } = await both('screenshot', SCREENSHOT)
        assert.deepEqual(reference.value, {
          settled: {
            name: 'AbortError',
            isError: true,
            causeIsReason: true,
            namesReason: true,
          },
          prepared: true,
          restored: true,
        })
      },
    )

    await t.test("an aborted call leaves the run's handles alone", async () => {
      const { reference } = await both('handles', HANDLES)
      assert.deepEqual(reference.value, {
        aborted: 'AbortError',
        handle: 'target',
      })
    })
  } finally {
    await runCleanups(cleanups)
  }
})
