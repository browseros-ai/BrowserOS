// ElementHandle.dispatchEvent through the Playwright reference and through Neo on fresh
// dispatch.html pages. Playwright builds the event from `eventInit` with the main world's
// InjectedScript and dispatches it on the handle's element (`{ selector: ':scope', scope }`,
// `mainWorld: true`); Neo runs the same page code through main_world.rs, holding that
// InjectedScript by CDP id. Compared: what the page's listeners saw (type, event class, bubbles,
// cancelable, composed, wheel deltas, a DataTransfer handle carried through a drag and drop,
// which moves the source), the page's global names before and after (the main world gains
// none), and the rejection for a handle of another frame. Values by `describe` (values.mjs),
// errors by category and the words Playwright's own test looks for.
//
// Cases from tests/page/page-dispatchevent.spec.ts at 26137face: DE-002 "should dispatch click
// event properties", DE-011 "should dispatch drag drop events via ElementHandles", DE-012 "should
// dispatch click event via ElementHandles", DE-013 "should dispatch wheel event", DE-017 "should
// throw if argument is from different frame"; DE-002, DE-013 and DE-017 through a handle.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

test('elementHandle.dispatchEvent dispatches in the main world as Playwright does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/dispatch.html', import.meta.url),
      'utf8',
    ),
    source: `${VALUES}
      await page.frameLocator('iframe').locator('#c').waitFor({ state: 'attached' })
      const before = await page.evaluate(() => Object.getOwnPropertyNames(globalThis))
      const results = {}
      const button = await page.$('#button')
      results.click = await outcome(() => button.dispatchEvent('click'))
      results.custom = await outcome(() => button.dispatchEvent('custom', { detail: { a: 1 } }))
      const dataTransfer = await page.evaluateHandle(() => new DataTransfer())
      const source = await page.$('#source')
      const target = await page.$('#target')
      results.dragstart = await outcome(() => source.dispatchEvent('dragstart', { dataTransfer }))
      results.drop = await outcome(() => target.dispatchEvent('drop', { dataTransfer }))
      const wheel = await page.$('#wheel')
      results.wheel = await outcome(() => wheel.dispatchEvent('wheel', { deltaX: 100, deltaY: 200 }))
      const frame = page.frames().find((f) => f.name() === 'child')
      const child = await frame.$('#c')
      results.otherFrameArgument = await outcome(
        () => child.dispatchEvent('drop', { dataTransfer }),
        ['JSHandles can be evaluated only in the context they were created'],
      )
      const childTransfer = await frame.evaluateHandle(() => new DataTransfer())
      results.sameFrameArgument = await outcome(() => child.dispatchEvent('drop', { dataTransfer: childTransfer }))
      const after = await page.evaluate(() => Object.getOwnPropertyNames(globalThis))
      results.newGlobals = after.filter((name) => !before.includes(name))
      return results
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.otherFrameArgument, {
    error: 'Error',
    mentions: [
      'JSHandles can be evaluated only in the context they were created',
    ],
  })
  assert.deepEqual(outcome.value.newGlobals, [])
})
