// ElementHandle.boundingBox through the Playwright reference and through Neo: the element's
// border box in the main frame's CSS pixels, a same-process child frame's moved by its iframe;
// null for an element with no box (display: none) and for one that left its document.
//
// Cases from tests/page/elementhandle-bounding-box.spec.ts at 26137face: EH-001 "should work",
// EH-002 "should handle nested frames" (one same-process level), EH-005 "should return null for
// invisible elements"; the detached element is the null of Playwright's `_sendMayFail`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = `<!doctype html><title>handle boxes</title><style>body { margin: 0 }</style>
<div id="box" style="position: absolute; left: 100px; top: 50px; width: 50px; height: 50px"></div>
<div id="none" style="display: none">none</div>
<p id="detach" class="leaving" style="position: absolute; left: 0; top: 300px">gone soon</p>
<iframe id="child" style="position: absolute; left: 200px; top: 100px; border: 0; width: 100px; height: 100px"
  srcdoc="<style>body { margin: 0 }</style><div id='inner' style='position: absolute; left: 10px; top: 20px; width: 30px; height: 40px'></div>"></iframe>`

test('boxes are what Playwright measures, or null', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}
      const box = await page.$('#box')
      const none = await page.$('#none')
      const gone = await page.$('#detach')
      const child = await (await page.$('#child')).contentFrame()
      const inner = await child.$('#inner')
      const before = await outcome(() => gone.boundingBox())
      await gone.evaluate((element) => element.remove())
      return {
        box: await outcome(() => box.boundingBox()),
        none: await outcome(() => none.boundingBox()),
        inner: await outcome(() => inner.boundingBox()),
        beforeDetach: before,
        detached: await outcome(() => gone.boundingBox()),
      }
    `,
    probe: 'document.querySelectorAll(".leaving").length',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.deepEqual(playwright.box, {
        value: { object: { x: 100, y: 50, width: 50, height: 50 } },
      })
      assert.deepEqual(playwright.none, { value: null })
      assert.deepEqual(playwright.detached, { value: null })
      assert.deepEqual(neo, playwright)
    },
  })
})
