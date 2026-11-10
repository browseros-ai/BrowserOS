// The `modifiers` and `steps` options of the pointer actions, through Neo and through
// Playwright on fresh tests/pages/mouse.html pages: a click, double click and hover with
// `modifiers` carry exactly those modifiers and leave the keyboard as it was (the next click
// has none); `modifiers: []` lifts a modifier page.keyboard holds for the one click and then
// holds it again; `ControlOrMeta` follows the platform; `steps` moves from the pointer's last
// position in that many moves. Mouse events, their modifiers and the number of moves are
// compared, never coordinates.
//
// From Playwright's specs at the pin: tests/page/page-click.spec.ts 'CL-057 should update
// modifiers correctly' (`modifiers: ['Shift']` then `modifiers: []`), tests/page/page-mouse.spec.ts
// 'MO-012 should tween mouse movement' (steps).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('modifiers and steps reach the pointer actions as Playwright sends them', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(new URL('../pages/mouse.html', import.meta.url), 'utf8'),
    source: `
      const target = page.locator('#target')
      const voids = [
        await target.click({ modifiers: ['Shift'] }),
        await target.click(),
        await target.dblclick({ modifiers: ['Alt', 'ControlOrMeta'] }),
        await target.hover({ modifiers: ['Shift'], position: { x: 10, y: 10 } }),
      ]
      await page.keyboard.down('Shift')
      voids.push(await target.click({ modifiers: [] }))
      voids.push(await target.click())
      await page.keyboard.up('Shift')
      await page.mouse.move(5, 5)
      voids.push(await target.click({ steps: 5, position: { x: 100, y: 100 } }))
      return voids.every((value) => value === undefined)
    `,
    probe:
      '({ events: window.__facts().events, moves: window.__facts().moves.length, movesWithShift: window.__facts().moves.filter((move) => move.modifiers === "shift").length })',
    effects: ({ playwright, neo }) => {
      const pressed = playwright.events
        .filter((event) => event.type === 'mousedown')
        .map((event) => event.modifiers)
      const smart = pressed[2]
      assert.match(smart, /^alt\+(meta|ctrl)$/, 'ControlOrMeta by platform')
      assert.deepEqual(
        pressed,
        ['shift', '', smart, smart, '', 'shift', ''],
        'each option for its own input, the keyboard restored after it',
      )
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
  assert.equal(outcome.value, true)
})
