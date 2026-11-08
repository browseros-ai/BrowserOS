// The deprecated locator.type, page.type and frame.type through Neo and through Playwright on
// fresh tests/pages/typing.html pages: each returns undefined and types as pressSequentially does,
// at the start of the unfocused input, the page and frame forms non-strict. Each engine's input
// ends with the same value after the same events.
//
// From Playwright's specs at the pin: tests/page/page-keyboard.spec.ts 'KB-001 should type into a
// textarea @smoke' (through `page.type`).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = readFileSync(
  new URL('../pages/typing.html', import.meta.url),
  'utf8',
)

for (const [name, call] of [
  ['locator.type', "page.locator('#name').type('Hey')"],
  ['page.type', "page.type('#name', 'Hey')"],
  ['frame.type', "page.mainFrame().type('#name', 'Hey', { delay: 5 })"],
]) {
  test(`${name} types as pressSequentially does`, {
    skip,
    timeout: 120_000,
  }, async (t) => {
    const outcome = await compareMethod(t, {
      html,
      source: `return await ${call}`,
      probe: 'window.__facts()',
      effects: ({ playwright, neo }) => {
        assert.equal(playwright.value, 'Heyabc')
        assert.deepEqual(neo, playwright, 'browser effect')
      },
    })
    assert.equal(outcome.value, undefined)
  })
}
