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

// Playwright's `signal` (slice S): an already-aborted one rejects with `AbortError` before
// anything is sent, on both receivers; a live one that never aborts runs the drag.
test('dragAndDrop honours its signal on the page and frame receivers', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `
      const results = []
      const aborted = { aborted: true, reason: new Error('stop') }
      for (const call of [
        () => page.dragAndDrop('#source', '#left', { signal: aborted }),
        () => page.mainFrame().dragAndDrop('#source', '#left', { signal: aborted }),
      ]) {
        try { await call(); results.push('resolved') }
        catch (error) { results.push(errorCategory(error)) }
      }
      const live = { aborted: false, addEventListener() {}, removeEventListener() {} }
      await page.dragAndDrop('#source', '#left', { signal: live })
      results.push('resolved')
      return results
    `,
    probe: PROBE,
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, ['AbortError', 'AbortError', 'resolved'])
      assert.deepEqual(neo, playwright)
    },
    effects: ({ playwright, neo }) => {
      assert.equal(
        playwright.events.filter((event) => event.type === 'drop').length,
        1,
        'only the live signal dropped',
      )
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
})

// Delta review F1 (#3200 x #3199): a locator handler must not run between a drag's halves.
// Playwright's target half skips the action pre-checks (`server/frames.ts` `dragAndDrop`:
// "do not perform locator handlers checkpoint to avoid moving the mouse in the middle of a drag
// operation"), so a banner the source's pointerdown reveals runs no handler, stays up, and the
// payload drops.
const BANNER =
  '<div id="banner" hidden style="position:absolute;top:300px"><button id="dismiss" onclick="banner.hidden=true">Dismiss</button></div>'

test('page.dragAndDrop runs no locator handler between its halves', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: html + BANNER,
    source: `
      let handled = 0
      await page.evaluate(() => source.addEventListener('pointerdown', () => { banner.hidden = false }, { once: true }))
      await page.addLocatorHandler(page.locator('#banner'), async () => { handled++; await page.locator('#dismiss').click() }, { times: 1 })
      const result = await page.dragAndDrop('#source', '#left', { timeout: 3000 }).then(() => 'resolved', (error) => errorCategory(error))
      return {
        result,
        handled,
        banner: await page.locator('#banner').isVisible(),
        dropped: await page.evaluate(() => __events.filter((event) => event.type === 'drop').map((event) => event.data)),
      }
    `,
    probe: PROBE,
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, {
        result: 'resolved',
        handled: 0,
        banner: true,
        dropped: ['payload'],
      })
      assert.deepEqual(neo, playwright)
    },
    effects: ({ playwright, neo }) => {
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
})
