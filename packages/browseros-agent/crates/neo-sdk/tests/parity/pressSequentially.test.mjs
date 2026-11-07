// locator.pressSequentially through Neo and through Playwright on fresh tests/pages/typing.html
// pages: the call returns undefined, and the input ends with the same value after the same key and
// input events. The input starts unfocused with "abc", so the focus resets the caret to the start
// (Playwright's `focusNode` with `resetSelectionIfNotFocused`), the regression for typing after
// the existing text; characters the US layout lacks are inserted as text; a held Shift from
// page.keyboard reaches every key; `delay` sits between each key's down and up. A strict-mode
// violation and a missing element reject before any key is sent.
//
// From Playwright's specs at the pin: tests/page/locator-misc-2.spec.ts 'should press
// sequentially' and tests/page/page-keyboard.spec.ts 'KB-028 should type emoji'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = readFileSync(
  new URL('../pages/typing.html', import.meta.url),
  'utf8',
)
const PROBE = 'window.__facts()'

test('pressSequentially types at the start of an unfocused input', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `return await page.locator('#name').pressSequentially('Hi é😀')`,
    probe: PROBE,
    effects: ({ playwright, neo }) => {
      assert.equal(
        playwright.value,
        'Hi é😀abc',
        "Playwright's caret was reset",
      )
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
  assert.equal(outcome.value, undefined)
  assert.deepEqual(
    outcome.calls.map((call) => [call.method, call.delivery]),
    [['type', 'done']],
  )
})

test('pressSequentially carries a held Shift and its delay', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `
      await page.keyboard.down('Shift')
      await page.locator('#name').pressSequentially('ab', { delay: 20 })
      await page.keyboard.up('Shift')
      return 'typed'
    `,
    probe: PROBE,
    effects: ({ playwright, neo }) => {
      assert.ok(
        playwright.events
          .filter((event) => event.type === 'keydown' && event.key !== 'Shift')
          .every((event) => event.shift),
        'every key saw Shift',
      )
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
})

test('pressSequentially rejects a strict violation and a missing element', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `
      const results = []
      for (const call of [
        () => page.locator('input.dup').pressSequentially('x'),
        () => page.locator('#missing').pressSequentially('x', { timeout: 300 }),
      ]) {
        try { await call(); results.push('resolved') } catch (error) { results.push(errorCategory(error)) }
      }
      return results
    `,
    probe: PROBE,
  })
  assert.deepEqual(outcome.value, ['Error:strict', 'TimeoutError'])
})
