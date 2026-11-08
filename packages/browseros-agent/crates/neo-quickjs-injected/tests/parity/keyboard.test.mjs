// page.keyboard through Neo and through Playwright on fresh tests/pages/keyboard.html pages:
// every method returns undefined, and the textarea ends with the same value after the same key
// and input events, a modifier held by `down` reaching later presses (the regression for a
// keyboard that started every call with nothing held), a key pressed twice repeating, and
// characters the US layout lacks typed as text. An unknown key rejects with Error and sends
// nothing. A key a run leaves held is released when the run ends, where Playwright keeps it
// held: asserted per engine.
//
// From Playwright's specs at the pin: tests/page/page-keyboard.spec.ts 'KB-001 should type into
// a textarea @smoke', 'KB-009 should report shiftKey', 'KB-019 should specify repeat property',
// 'KB-027 should throw on unknown keys' and 'KB-028 should type emoji'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = readFileSync(
  new URL('../pages/keyboard.html', import.meta.url),
  'utf8',
)

test('keyboard methods type the same keys and text', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `
      await page.locator('#area').click()
      const keyboard = page.keyboard
      const voids = [
        await keyboard.type('Hi!'),
        await keyboard.down('Shift'),
        await keyboard.press('a'),
        await keyboard.press('KeyB'),
        await keyboard.down('c'),
        await keyboard.down('c'),
        await keyboard.up('c'),
        await keyboard.up('Shift'),
        await keyboard.insertText('é'),
        await keyboard.type('ü😀', { delay: 5 }),
        await keyboard.press('Backspace'),
        await keyboard.press('ArrowLeft', { delay: 5 }),
        await keyboard.type('x'),
      ]
      let unknown = 'resolved'
      try { await keyboard.down('Foo') } catch (error) { unknown = errorCategory(error) }
      return { voids: voids.every((value) => value === undefined), unknown }
    `,
    probe: 'window.__facts()',
    effects: ({ playwright, neo }) => {
      assert.equal(playwright.value, 'Hi!aBccéxü', "Playwright's textarea")
      assert.deepEqual(
        playwright.events
          .filter((event) => event.type === 'keydown' && event.key === 'c')
          .map((event) => [event.repeat, event.modifiers]),
        [
          [false, 'shift'],
          [true, 'shift'],
        ],
        'a held Shift, and a key pressed twice repeating',
      )
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
  assert.deepEqual(outcome.value, { voids: true, unknown: 'Error' })
  const deliveries = outcome.calls
    .filter((call) => call.method.startsWith('keyboard'))
    .map((call) => call.delivery)
  assert.deepEqual(deliveries, [...Array(13).fill('done'), 'notSent'])
})

test("a key a run leaves held is released at the run's end", {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `
      await page.locator('#area').click()
      await page.keyboard.down('Shift')
      return 'held'
    `,
    probe:
      'window.__facts().events.filter((event) => event.key === "Shift").map((event) => event.type)',
    effects: ({ playwright, neo }) => {
      assert.deepEqual(playwright, ['keydown'], 'Playwright keeps Shift held')
      assert.deepEqual(
        neo,
        ['keydown', 'keyup'],
        'Neo releases it as the run ends',
      )
    },
  })
})
