// Which world each kind of call runs in, through the Playwright reference and through Neo on fresh
// evaluate.html pages (med-pw-eval design, section 5c). Playwright evaluates in the page's main
// world and reads and acts in its utility world; Neo does the same through two doors
// (neo-runtime's `main_world.rs` and `world.rs`), so:
// - evaluate sees what the page's own scripts set;
// - after the page patches `Element.prototype.getAttribute`, `innerText` and `textContent`,
//   `locator.evaluate` sees the patches and `locator.getAttribute` / `innerText` / `textContent`
//   do not, in both tools;
// - evaluations, handles and disposal leave the page's global names as they were, and no Neo name
//   is reachable from the page's world.
// Values by `describe` (values.mjs); the page's own state by `window.__facts()`.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = readFileSync(
  new URL('../pages/evaluate.html', import.meta.url),
  'utf8',
)

test('evaluations run in the main world, reads in the utility world', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const before = {
        evaluate: await outcome(() => page.locator('#para').evaluate((e) => [e.getAttribute('data-x'), e.innerText, e.textContent])),
        getAttribute: await outcome(() => page.locator('#para').getAttribute('data-x')),
        innerText: await outcome(() => page.locator('#para').innerText()),
        textContent: await outcome(() => page.locator('#para').textContent()),
      }
      await page.evaluate(() => {
        Element.prototype.getAttribute = function () { return 'patched-attribute' }
        Object.defineProperty(HTMLElement.prototype, 'innerText', { get() { return 'patched-inner' } })
        Object.defineProperty(Node.prototype, 'textContent', { get() { return 'patched-text' } })
      })
      const after = {
        evaluate: await outcome(() => page.locator('#para').evaluate((e) => [e.getAttribute('data-x'), e.innerText, e.textContent])),
        getAttribute: await outcome(() => page.locator('#para').getAttribute('data-x')),
        innerText: await outcome(() => page.locator('#para').innerText()),
        textContent: await outcome(() => page.locator('#para').textContent()),
        pageSees: await outcome(() => page.evaluate(() => document.querySelector('#para').getAttribute('data-x'))),
      }
      return { pageGlobal: await outcome(() => page.evaluate(() => [window.__probe, window.FOO])), before, after }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.after.evaluate, {
    value: ['patched-attribute', 'patched-inner', 'patched-text'],
  })
  assert.deepEqual(outcome.value.after.getAttribute, { value: 'para-x' })
  assert.deepEqual(outcome.value.after.innerText, { value: 'Para bold' })
  assert.deepEqual(outcome.value.after.textContent, { value: 'Para bold' })
})

test('evaluations and handles leave the page world without new names', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const names = () => page.evaluate(() => Object.getOwnPropertyNames(globalThis).sort())
      const before = await names()
      await page.evaluate(() => 1 + 1)
      await page.evaluate((x) => x, { nested: [1, 2, { deep: true }] })
      const object = await page.evaluateHandle(() => ({ a: 1 }))
      const element = await page.evaluateHandle(() => document.querySelector('#s'))
      await page.evaluate(([o, e]) => o.a + e.id, [object, element])
      await page.locator('#s').evaluate((e) => e.id)
      await object.jsonValue()
      await object.dispose()
      await element.dispose()
      const after = await names()
      return {
        added: after.filter((name) => !before.includes(name)),
        removed: before.filter((name) => !after.includes(name)),
        neoNames: after.filter((name) => /neo|utility/i.test(name)),
        neoGlobals: await page.evaluate(() => [typeof window.__neo_runtime, typeof window.UtilityScript, typeof window.__neo_utility_world__]),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    added: [],
    removed: [],
    neoNames: [],
    neoGlobals: ['undefined', 'undefined', 'undefined'],
  })
})
