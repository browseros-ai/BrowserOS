// The Rust route of Playwright's `Locator.waitForFunction`: `Frame.waitForFunction` with
// `selector` and `strict`, served by `evaluate::call_on_element_main` (A3). The predicate runs on
// the first match in the page's own world and polls until it is truthy. The public method is
// med-pw-dom's (dom.md row); until it lands, Neo's side sends the exact protocol call
// Playwright's client sends (client/locator.ts `Locator.waitForFunction`) through the SDK's own
// connection, while Playwright's side calls `locator.waitForFunction`. Compared: how each call
// settles (by `describe`, errors by category and the words Playwright's tests look for) and what
// the page shows afterwards.
//
// Cases from tests/page/locator-wait-for-function.spec.ts at 26137face: LO-183 "should wait for an
// attribute to appear", LO-184 "should return immediately when already truthy", LO-185 "should
// accept ElementHandle arguments", LO-186 "should accept string expression", LO-187 "should
// resolve a promise returned by the predicate", LO-189 "should throw when predicate throws",
// LO-190 "should throw on strict mode violation".
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
      const waitOn = async (selector, pageFunction, arg, options = {}) => {
        const locator = page.locator(selector)
        if (typeof locator.waitForFunction === 'function')
          return await locator.waitForFunction(pageFunction, arg, options)
        // Neo, until med-pw-dom's public method lands: the protocol call Playwright's client sends.
        const frame = page.mainFrame()
        const serialized = arg === undefined
          ? { value: { v: 'undefined' }, handles: [] }
          : { value: { h: 0 }, handles: [arg._guid] }
        await frame._connection.sendEval('locator.waitForFunction', frame._guid, 'waitForFunction', {
          selector,
          strict: true,
          expression: String(pageFunction),
          isFunction: typeof pageFunction === 'function',
          arg: serialized,
        }, options.timeout ?? 30000)
        return undefined
      }
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
