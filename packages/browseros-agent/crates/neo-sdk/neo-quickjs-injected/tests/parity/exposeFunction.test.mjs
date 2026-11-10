// page.exposeFunction through the Playwright reference and through Neo: `window[name](...)` in
// the page calls the agent's callback and resolves with its result (a promise's value, a handle
// as its page object), rejects with what it threw (an Error's message and name, `null` as
// itself), survives navigation, and is gone after `dispose()`. A second registration of a name
// is refused. The binding is there from a document's first script: an init script calls it, and
// so does the first inline script of a cross-site (out-of-process) iframe. With a console
// listener on, a message such an iframe logs reaches the page's listener once (Neo's iframe
// sessions for bindings send it twice to CDP; only one is the page's stream). Values by `describe`
// (values.mjs), errors by category and the words Playwright's tests look for.
//
// Neo removes every binding a run exposed when the run ends (`Deviation:` in bindings.rs), so the
// probes read only what both sides keep; `neo-runtime/tests/bindings_e2e.rs` shows a page call
// after the run rejects and never hangs.
//
// Cases from tests/page/page-expose-function.spec.ts at 26137face: EF-002 "should work", EF-003
// "should dispose", EF-004 "should work with handles and complex objects", EF-005 "should throw
// exception in page context", EF-006 "should support throwing "null"", EF-007 "should be callable
// from-inside addInitScript", EF-008 "should survive navigation", EF-009 "should await returned
// promise", EF-014 "should throw for duplicate registrations".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = '<title>expose</title><p id="p">text</p>'

test('exposeFunction answers the page as Playwright does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const results = {}
      const compute = await page.exposeFunction('compute', (a, b) => a * b)
      results.call = await page.evaluate(async () => await window.compute(9, 4))
      await page.exposeFunction('later', (a, b) => Promise.resolve(a * b))
      results.promise = await page.evaluate(async () => await window.later(3, 5))
      await page.exposeFunction('woof', () => { throw new Error('WOOF WOOF') })
      results.thrown = await page.evaluate(async () => {
        try { await window.woof() } catch (e) { return { message: e.message, name: e.name } }
      })
      await page.exposeFunction('nothing', () => { throw null })
      results.nullThrown = await page.evaluate(async () => {
        try { await window.nothing() } catch (e) { return e }
      })
      results.duplicate = await outcome(() => page.exposeFunction('compute', () => 0), ['has been already registered'])
      await page.goto(page.url())
      results.afterNavigation = await page.evaluate(async () => await window.compute(9, 4))
      await compute.dispose()
      results.disposed = await outcome(() => page.evaluate(async () => await window.compute(9, 4)), ['is not a function'])
      return results
    `,
    isolate: true,
    probe: '({ compute: typeof window.compute })',
  })
  assert.deepEqual(outcome.value.call, 36)
  assert.deepEqual(outcome.value.promise, 15)
  assert.deepEqual(outcome.value.thrown, {
    message: 'WOOF WOOF',
    name: 'Error',
  })
  assert.deepEqual(outcome.value.nullThrown, null)
  assert.deepEqual(outcome.value.afterNavigation, 36)
})

test('a handle the callback returns reaches the page as its object', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `
      const fooHandle = await page.evaluateHandle(() => {
        window.fooValue = { bar: 2 }
        return window.fooValue
      })
      await page.exposeFunction('handle', () => [{ foo: fooHandle }])
      return await page.evaluate(async () => {
        const [{ foo }] = await window.handle()
        return foo === window.fooValue
      })
    `,
    isolate: true,
    probe: '({ foo: window.fooValue })',
  })
  assert.equal(outcome.value, true)
})

// The top answers `?child` with the cross-site child's own page: its first inline script logs
// and calls the binding, before anything else of the child runs.
const oopif = `<title>oopif</title>
<script>
if (location.search === '?child') {
  console.log('oopif-first')
  window.__result = window.ping('oopif').then((value) => value, (error) => 'rejected: ' + error.message)
} else {
  const cross = document.createElement('iframe')
  cross.src = 'http://localhost:' + location.port + '/method.html?child'
  document.documentElement.append(cross)
}
</script>`

test('the binding is there from a document start: an init script and an OOPIF first script call it', {
  skip,
  timeout: 240_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: oopif,
    source: `
      const texts = []
      page.on('console', (message) => texts.push(message.text()))
      const calls = []
      await page.exposeFunction('ping', (who) => { calls.push(who); return 'pong ' + who })
      // The top document's only: each frame's first about:blank runs it too, as many as timing makes.
      await page.addInitScript(() => { if (window === window.top) window.ping('init') })
      await page.goto(page.url())
      const child = page.frames().find((frame) => frame.url().includes('localhost'))
      await child.waitForFunction(() => window.__result)
      const result = await child.evaluate(() => window.__result)
      await child.evaluate(() => console.log('oopif-later'))
      await page.waitForFunction(() => true)
      return {
        result,
        init: calls.filter((who) => who === 'init').length,
        oopif: calls.filter((who) => who === 'oopif').length,
        later: texts.filter((text) => text === 'oopif-later').length,
      }
    `,
    isolate: true,
    probe: '({ title: document.title })',
  })
  assert.deepEqual(outcome.value, {
    result: 'pong oopif',
    init: 1,
    oopif: 1,
    later: 1,
  })
})
