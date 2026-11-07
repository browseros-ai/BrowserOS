// page.dragAndDrop and frame.dragAndDrop (non-strict selector forms of dragTo), and an HTML5
// drag made with page.mouse, through Neo and through Playwright on fresh tests/pages/drag.html
// pages: the same drag events and drop data; a drag the mouse started is cancelled by Escape,
// whose `dragend` reports no drop, and the release after it drops nothing.
//
// From Playwright's specs at the pin: tests/page/page-drag.spec.ts 'should work @smoke' (manual
// mouse.down/move/up), 'should cancel on escape' and 'should work with the helper method'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = readFileSync(
  new URL('../pages/drag.html', import.meta.url),
  'utf8',
)
const PROBE = 'window.__facts()'

for (const [name, call] of [
  ['page.dragAndDrop', "page.dragAndDrop('#source', '#left')"],
  [
    'frame.dragAndDrop',
    "page.mainFrame().dragAndDrop('#source', '#left', { steps: 3 })",
  ],
]) {
  test(`${name} drops the payload`, { skip, timeout: 120_000 }, async (t) => {
    const outcome = await compareMethod(t, {
      html,
      source: `return await ${call}`,
      probe: PROBE,
      effects: ({ playwright, neo }) => {
        assert.deepEqual(
          playwright.events.find((event) => event.type === 'drop'),
          { type: 'drop', target: 'left', data: 'payload', dropEffect: null },
        )
        assert.deepEqual(neo, playwright, 'browser effect')
      },
    })
    assert.equal(outcome.value, undefined)
  })
}

test('a drag made with page.mouse drops, and Escape cancels one', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `
      await page.mouse.move(60, 40)
      await page.mouse.down()
      await page.mouse.move(250, 60, { steps: 3 })
      await page.mouse.up()
      await page.mouse.move(60, 40)
      await page.mouse.down()
      await page.mouse.move(350, 60, { steps: 3 })
      await page.keyboard.press('Escape')
      await page.mouse.up()
      return 'done'
    `,
    probe: PROBE,
    effects: ({ playwright, neo }) => {
      const drops = playwright.events.filter((event) => event.type === 'drop')
      assert.deepEqual(
        drops.map((event) => event.target),
        ['left'],
        'one drop, the first drag; the cancelled one drops nothing',
      )
      const ends = playwright.events.filter((event) => event.type === 'dragend')
      assert.equal(ends.length, 2, JSON.stringify(ends))
      assert.notEqual(ends[0].dropEffect, 'none', 'the first drag dropped')
      assert.equal(
        ends[1].dropEffect,
        'none',
        'the cancelled drag ends with no drop',
      )
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
})
