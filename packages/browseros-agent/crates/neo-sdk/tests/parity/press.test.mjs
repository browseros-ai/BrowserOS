// Press's Page, Frame and Locator receivers share one behavior check on fresh pages: the same
// keys type the same text and the page sees the same key events. `Shift+Foo` is left out on
// purpose: Playwright presses Shift and leaves it held, Neo sends nothing (press_e2e.rs checks it).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('press returns undefined and types through ordinary key events', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/form-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [
        await page.press('#text', 'End') === undefined,
        await page.mainFrame().press('#text', 'Shift+A') === undefined,
        await page.locator('#text').press('x', { delay: 20 }) === undefined,
        await page.locator('#area').press('ControlOrMeta+a') === undefined,
        await page.locator('#area').press('Backspace') === undefined,
        await page.locator('#area').press('Enter', { noWaitAfter: true }) === undefined,
      ]
      if (!voids.every(Boolean)) throw new Error('press must return undefined')
      const invalid = []
      for (const key of ['Foo', '', 123]) {
        try {
          await page.locator('#text').press(key)
          invalid.push('resolved')
        } catch (error) {
          invalid.push({ name: error.name, unknownKey: /Unknown key/.test(error.message) })
        }
      }
      return { voids, invalid }
    `,
    probe: 'window.__facts()',
  })
  const deliveries = outcome.calls.map((c) => [c.params.key, c.delivery])
  assert.deepEqual(deliveries.slice(0, 8), [
    ['End', 'done'],
    ['Shift+A', 'done'],
    ['x', 'done'],
    ['ControlOrMeta+a', 'done'],
    ['Backspace', 'done'],
    ['Enter', 'done'],
    ['Foo', 'notSent'],
    ['', 'notSent'],
  ])
})
