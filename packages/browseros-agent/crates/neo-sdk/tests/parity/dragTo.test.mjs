// locator.dragTo through Neo and through Playwright on fresh tests/pages/drag.html pages: the
// call returns undefined and the page sees the same drag: an HTML5 `draggable` source dropped
// on a target (`dragstart`, `dragenter`, `dragover`, `drop` with the source's data, `dragend`),
// the drop landing on the half `targetPosition` names, a pointer-event slider dragged across in
// `steps`, and a trial that sends nothing. A strict-mode violation and a missing target reject.
// Consecutive repeated events are collapsed and no coordinates compared.
//
// From Playwright's specs at the pin: tests/page/page-drag.spec.ts 'should work @smoke', 'should
// work with the helper method', 'should be able to drag the mouse in a frame', 'should respect
// the drop effect' area, and 'should work with locators'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = readFileSync(
  new URL('../pages/drag.html', import.meta.url),
  'utf8',
)
const PROBE = 'window.__facts()'

test('dragTo drops an HTML5 draggable on its target', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `return await page.locator('#source').dragTo(page.locator('#target'), { targetPosition: { x: 150, y: 40 } })`,
    probe: PROBE,
    effects: ({ playwright, neo }) => {
      const kinds = playwright.events.map((event) => event.type)
      assert.ok(
        kinds.includes('drop') && kinds.includes('dragend'),
        kinds.join(),
      )
      assert.deepEqual(
        playwright.events.find((event) => event.type === 'drop'),
        { type: 'drop', target: 'right', data: 'payload', dropEffect: null },
        'Playwright dropped the payload on the right half',
      )
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
  assert.equal(outcome.value, undefined)
  assert.deepEqual(
    outcome.calls.map((call) => [call.method, call.delivery]),
    [['dragAndDrop', 'done']],
  )
})

test('dragTo drags a pointer-event slider in steps', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `return await page.locator('#knob').dragTo(page.locator('#track'), { targetPosition: { x: 290, y: 10 }, steps: 5 })`,
    probe: PROBE,
    effects: ({ playwright, neo }) => {
      assert.ok(playwright.slider.moves >= 5, JSON.stringify(playwright.slider))
      assert.equal(playwright.slider.right, true)
      assert.equal(playwright.slider.released, true)
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
})

test('a trial dragTo sends nothing; strict and missing targets reject', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `
      const results = []
      for (const call of [
        () => page.locator('#source').dragTo(page.locator('#target'), { trial: true }),
        () => page.locator('.dup').dragTo(page.locator('#target')),
        () => page.locator('#source').dragTo(page.locator('#missing'), { timeout: 500 }),
      ]) {
        try { await call(); results.push('resolved') } catch (error) { results.push(errorCategory(error)) }
      }
      return results
    `,
    probe: PROBE,
    effects: ({ playwright, neo }) => {
      assert.ok(
        !playwright.events.some((event) => event.type === 'drop'),
        'nothing dropped',
      )
      assert.deepEqual(neo, playwright, 'browser effect')
    },
  })
  assert.deepEqual(outcome.value, ['resolved', 'Error:strict', 'TimeoutError'])
})

// Playwright's `signal` (slice S) on the Locator receiver: an already-aborted one rejects with
// `AbortError` before anything is sent; a live one that never aborts runs the drag.
test('dragTo honours its signal', { skip, timeout: 120_000 }, async (t) => {
  await compareMethod(t, {
    html,
    source: `
      const results = []
      try {
        await page.locator('#source').dragTo(page.locator('#left'), {
          signal: { aborted: true, reason: new Error('stop') },
        })
        results.push('resolved')
      } catch (error) { results.push(errorCategory(error)) }
      await page.locator('#source').dragTo(page.locator('#left'), {
        signal: { aborted: false, addEventListener() {}, removeEventListener() {} },
      })
      results.push('resolved')
      return results
    `,
    probe: PROBE,
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, ['AbortError', 'resolved'])
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
