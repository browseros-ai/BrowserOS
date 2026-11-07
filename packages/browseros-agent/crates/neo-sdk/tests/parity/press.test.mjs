// Press's Page, Frame and Locator receivers share one behavior check on fresh pages: the same
// keys type the same text and the page sees the same key events.
//
// One difference is deliberate and asserted, not hidden: on `Shift+Foo` Playwright presses Shift,
// rejects on the unknown `Foo` and leaves Shift held; Neo checks every key before it presses any,
// so its page sees no key at all (press_e2e.rs). Both reject with the same error.
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
        await page.press('#text', 'End', null) === undefined,
        await page.mainFrame().press('#text', 'Shift+A', null) === undefined,
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
          invalid.push({ category: errorCategory(error), unknownKey: /Unknown key/.test(error.message) })
        }
      }
      if (invalid.length !== 3 || !invalid.every(result => result.category === 'Error')) throw new Error('invalid keys must reject')
      let timeout = 'resolved'
      try { await page.locator('#missing').press('Enter', {timeout:500}) } catch (error) { timeout = errorCategory(error) }
      if (timeout !== 'TimeoutError') throw new Error('missing press must time out')
      return { voids, invalid, timeout }
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

test('Shift+Foo rejects alike: Playwright leaves Shift down, Neo presses no key', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/form-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      try {
        await page.locator('#text').press('Shift+Foo')
        return 'resolved'
      } catch (error) {
        return errorCategory(error)
      }
    `,
    probe: 'window.__facts()',
    effects: ({ playwright, neo }) => {
      const event = (type, key) => ({
        type,
        target: 'text',
        key,
        data: null,
        inputType: null,
      })
      const untouched = { ...neo, events: [] }
      assert.deepEqual(
        { ...playwright, events: [] },
        untouched,
        'the same values',
      )
      assert.deepEqual(
        playwright.events,
        [event('focus', null), event('keydown', 'Shift')],
        "Playwright's page: Shift went down and stayed",
      )
      assert.deepEqual(
        neo.events,
        [event('focus', null)],
        "Neo's page: focused, no key",
      )
    },
  })
  assert.equal(outcome.value, 'Error')
  assert.deepEqual(
    outcome.calls.map((c) => [c.params.key, c.delivery]),
    [['Shift+Foo', 'notSent']],
  )
})
