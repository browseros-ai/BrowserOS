// page.mouse through Neo and through Playwright on fresh tests/pages/mouse.html pages: every
// method returns undefined, and the page sees the same mouse events, buttons and click counts:
// a move in steps from the pointer's last position, a press held across a move, the right and
// middle buttons, a double click, a modifier held by page.keyboard on a later mouse click and
// element click (the regression for input that forgot the keyboard between calls), and a wheel.
// No coordinates are compared.
//
// From Playwright's specs at the pin: tests/page/page-mouse.spec.ts 'MO-002 should dblclick the
// div', 'MO-003 down and up should generate click', 'MO-005 should report correct buttons
// property', 'MO-011 should set modifier keys on click', 'MO-012 should tween mouse movement';
// tests/page/wheel.spec.ts 'MO-017 should dispatch wheel events @smoke'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('mouse methods send the same events with the state they keep', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(new URL('../pages/mouse.html', import.meta.url), 'utf8'),
    source: `
      const mouse = page.mouse
      const voids = [
        await mouse.move(20, 20),
        await mouse.move(120, 60, { steps: 4 }),
        await mouse.down(),
        await mouse.move(130, 70),
        await mouse.up(),
        await mouse.click(50, 50, { button: 'right' }),
        await mouse.dblclick(60, 60),
        await mouse.down({ button: 'middle' }),
        await mouse.up({ button: 'middle' }),
      ]
      await page.keyboard.down('Shift')
      voids.push(await mouse.click(70, 70))
      voids.push(await page.click('#target'))
      await page.keyboard.up('Shift')
      voids.push(await mouse.wheel(0, 100))
      return voids.every((value) => value === undefined)
    `,
    // mouse.wheel does not wait for the wheel's effects (Playwright's docs), so the page may not
    // have seen it yet when the run ends. Both engines' pages are read the same way: at once if
    // the fixture has recorded this case's one wheel, else when the wheel event comes (this
    // listener runs after the fixture's), bounded at 5 s. Everything is still compared.
    probe: `new Promise((resolve, reject) => {
      if (window.__wheels.length > 0) return resolve(window.__facts())
      const bound = setTimeout(() => reject(new Error('the page recorded no wheel in 5 s')), 5_000)
      document.addEventListener('wheel', () => {
        clearTimeout(bound)
        resolve(window.__facts())
      }, { once: true })
    })`,
    effects: ({ playwright, neo }) => {
      const kinds = playwright.events.map((event) => event.type)
      assert.ok(
        kinds.includes('dblclick') && kinds.includes('auxclick'),
        kinds.join(),
      )
      assert.deepEqual(
        playwright.events
          .filter((event) => event.type === 'click')
          .slice(-2)
          .map((event) => event.modifiers),
        ['shift', 'shift'],
        'the held Shift reaches a mouse click and an element click',
      )
      assert.ok(
        playwright.moves.some((move) => move.buttons === 1),
        'a move while the left button is held',
      )
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
  assert.equal(outcome.value, true)
  assert.ok(
    outcome.calls.every((call) => call.delivery === 'done'),
    JSON.stringify(outcome.calls.map((call) => [call.method, call.delivery])),
  )
})

test('review: zero click count and zero steps resolve without input', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: '<button id="target">Target</button>',
    source: `return await page.locator('#target').click({clickCount:0,steps:0})
      .then(() => 'resolved', errorCategory)`,
    probe: 'document.querySelector("#target").textContent',
  })
})
