// `Locator.waitForFunction` through both engines: Playwright's client sends `Frame.waitForFunction`
// with `selector` and `strict` (client/locator.ts), which Neo serves by
// `evaluate::call_on_element_main` (A3). The predicate runs on the first match in the page's own
// world and polls until it is truthy. Compared: how each call settles (by `describe`, errors by
// category and the words Playwright's tests look for) and what the page shows afterwards.
//
// Cases from tests/page/locator-wait-for-function.spec.ts at 26137face: LO-183 "should wait for an
// attribute to appear", LO-184 "should return immediately when already truthy", LO-185 "should
// accept ElementHandle arguments", LO-186 "should accept string expression", LO-187 "should
// resolve a promise returned by the predicate", LO-188 "should wait for element to appear and
// survive rerender", LO-189 "should throw when predicate throws", LO-190 "should throw on strict
// mode violation", LO-191 "should abort via signal" and LO-192 "should abort via already-aborted
// signal" (a signal-shaped object, which both clients read the same way).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = `<title>wait-on-element</title>
<button id="toggle">Menu</button>
<div id="target">yes</div>
<div id="with-arg"></div><div id="the-arg">value</div>
<div id="no">no</div>
<div class="x">1</div><div class="x">2</div>
<script>
// BrowserOS neo's takeover bar is not the page's (frame-top.html explains); remove it on sight.
new MutationObserver(() => document.querySelector('browseros-neo-takeover')?.remove()).observe(
  document.documentElement,
  { childList: true, subtree: true },
)
window.__facts = () => ({ expanded: document.querySelector('#toggle').getAttribute('aria-expanded') })
</script>`

test('waitForFunction on a selector polls the first match as Playwright does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const waitOn = (selector, pageFunction, arg, options) =>
        page.locator(selector).waitForFunction(pageFunction, arg, options)
      const results = {}
      results.appears = await outcome(async () => {
        await page.evaluate(() => setTimeout(() => document.querySelector('#toggle').setAttribute('aria-expanded', 'true'), 300))
        return await waitOn('#toggle', (element) => element.hasAttribute('aria-expanded'))
      })
      results.already = await outcome(() => waitOn('#target', (element) => element.textContent === 'yes'))
      const other = await page.$('#the-arg')
      results.handleArgument = await outcome(() => waitOn('#with-arg', (element, b) => b.textContent === 'value', other))
      results.stringExpression = await outcome(() => waitOn('#target', "element => element.textContent === 'yes'"))
      results.promise = await outcome(() => waitOn('#target', async (element) => element.textContent === 'yes'))
      results.throws = await outcome(() => waitOn('#no', () => { throw new Error('oh my') }), ['oh my'])
      results.strict = await outcome(() => waitOn('div.x', () => true), ['strict mode violation'])
      results.timeout = await outcome(() => waitOn('#no', (element) => element.textContent === 'yes', undefined, { timeout: 300 }))
      // LO-188: the element appears late and is replaced twice before its text is '3'.
      results.rerender = await outcome(async () => {
        await page.evaluate(() => {
          let count = 0
          let previous = null
          const tick = () => {
            const next = document.createElement('div')
            next.id = 'rerendered'
            next.textContent = String(++count)
            previous?.remove()
            document.body.appendChild(next)
            previous = next
            if (count < 3) setTimeout(tick, 200)
          }
          setTimeout(tick, 200)
        })
        return await waitOn('#rerendered', (element) => element.textContent === '3')
      })
      // LO-192 and LO-191: an already-aborted signal, and one aborted while the predicate polls.
      results.alreadyAborted = await outcome(() =>
        waitOn('#no', () => true, undefined, { signal: { aborted: true, reason: 'already aborted' } }))
      const listeners = []
      const signal = {
        aborted: false,
        reason: undefined,
        addEventListener: (_type, listener) => listeners.push(listener),
        removeEventListener() {},
      }
      const pending = outcome(() =>
        waitOn('#no', (element) => element.textContent === 'yes', undefined, { timeout: 0, signal }))
      await page.waitForTimeout(100)
      signal.aborted = true
      signal.reason = new Error('Aborted by user')
      for (const listener of listeners) listener()
      results.aborted = await pending
      return results
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.already, { value: { undefined: true } })
  assert.deepEqual(outcome.value.strict, {
    error: 'Error:strict',
    mentions: ['strict mode violation'],
  })
  assert.equal(outcome.value.timeout.error, 'TimeoutError')
})
