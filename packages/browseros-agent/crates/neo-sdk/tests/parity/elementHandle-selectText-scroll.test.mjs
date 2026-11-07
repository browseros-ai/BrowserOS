// ElementHandle's selectText and scrollIntoViewIfNeeded through the Playwright reference and
// through Neo, each on the handle's own element. selectText selects a textarea's, an input's or
// a plain element's text, waits for a hidden element to show (unless `force`) and times out on
// one that never does. scrollIntoViewIfNeeded brings a far element into view, works for a
// visibility:hidden and a zero-sized element, waits for a display:none one to show, and times
// out on one that never does. A detached element is Playwright's "not attached" error. Neither
// action marks its target in Playwright, so the page sees no target marks. Values by `describe`
// (values.mjs), errors by category and the words Playwright's own tests check.
//
// Cases from tests/page/elementhandle-select-text.spec.ts at 26137face: EH-102 "should select
// textarea", EH-103 "should select input", EH-104 "should select plain div", EH-105 "should
// timeout waiting for invisible element", EH-106 "should wait for visible"; tests/page/
// elementhandle-scroll-into-view.spec.ts: EH-093 "should work", EH-094 "should throw for
// detached element", EH-095 "should wait for display:none to become visible", EH-097 "should
// work for visibility:hidden element", EH-098 "should work for zero-sized element", EH-101
// "should timeout waiting for visible".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = `<!doctype html><title>handle select and scroll</title>
<script>
window.marks = []
document.addEventListener('__playwright_reset_targets__', () => marks.push('reset'), true)
document.addEventListener('__playwright_mark_target__', (event) => marks.push(event.target.id || event.target.nodeName), true)
</script>
<textarea id="area">some value</textarea><input id="input" value="input value">
<div id="plain" class="plain">Plain div</div>
<textarea id="hiddenArea" style="display: none">hidden value</textarea>
<textarea id="later" style="display: none">later value</textarea>
<input id="detach" value="gone">
<div style="height: 2000px"></div>
<div id="far">far</div>
<div id="farHidden" style="display: none">far hidden</div>
<div id="farInvisible" style="visibility: hidden">invisible</div>
<div id="zero" style="width: 0; height: 0"></div>
<div id="farLater" style="display: none">far later</div>
<div style="height: 2000px"></div>`

/** A late action: still waiting 300 ms in, then done once its element shows. */
const LATE = `
const late = async (handle, act) => {
  const pending = act().then(() => 'done', (error) => error.name)
  const early = await Promise.race([pending, new Promise((resolve) => setTimeout(() => resolve('waiting'), 300))])
  await handle.evaluate((element) => { element.style.display = 'block' })
  return { early, settled: await pending }
}
`

test('selectText selects the handle’s element as Playwright’s does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}${LATE}
      const $ = (selector) => page.$(selector)
      const selection = () => page.evaluate(() => window.getSelection().toString())
      const result = {}
      result.area = await outcome(async () => (await $('#area')).selectText())
      result.areaSelection = await selection()
      result.input = await outcome(async () => (await $('#input')).selectText())
      result.inputSelection = await selection()
      result.plain = await outcome(async () => (await $('#plain')).selectText())
      result.plainSelection = await selection()
      result.hidden = await outcome(async () => (await $('#hiddenArea')).selectText({ timeout: 500 }),
        ['Timeout 500ms exceeded', 'element is not visible'])
      result.forced = await outcome(async () => (await $('#hiddenArea')).selectText({ force: true }))
      const later = await $('#later')
      result.later = await late(later, () => later.selectText({ timeout: 5000 }))
      result.laterSelection = await selection()
      const gone = await $('#detach')
      await gone.evaluate((element) => element.remove())
      result.detached = await outcome(() => gone.selectText(), ['Element is not attached to the DOM'])
      result.marks = await page.evaluate(() => window.marks)
      return result
    `,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.equal(playwright.areaSelection, 'some value')
      assert.equal(playwright.plainSelection, 'Plain div')
      assert.equal(playwright.hidden.error, 'TimeoutError')
      assert.deepEqual(playwright.later, { early: 'waiting', settled: 'done' })
      assert.deepEqual(playwright.marks, [])
      assert.deepEqual(neo, playwright)
    },
  })
})

test('scrollIntoViewIfNeeded scrolls to the handle’s element as Playwright’s does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}${LATE}
      const $ = (selector) => page.$(selector)
      const inView = (selector) => page.$eval(selector, (element) => {
        const rect = element.getBoundingClientRect()
        return rect.top >= 0 && rect.bottom <= window.innerHeight
      })
      const result = {}
      result.far = await outcome(async () => (await $('#far')).scrollIntoViewIfNeeded())
      result.farInView = await inView('#far')
      result.scrolled = await page.evaluate(() => window.scrollY > 0)
      result.invisible = await outcome(async () => (await $('#farInvisible')).scrollIntoViewIfNeeded())
      result.zero = await outcome(async () => (await $('#zero')).scrollIntoViewIfNeeded())
      result.hidden = await outcome(async () =>
        (await $('#farHidden')).scrollIntoViewIfNeeded({ timeout: 500 }),
        ['Timeout 500ms exceeded', 'element is not visible'])
      const later = await $('#farLater')
      result.later = await late(later, () => later.scrollIntoViewIfNeeded({ timeout: 5000 }))
      result.laterInView = await inView('#farLater')
      const gone = await $('#detach')
      await gone.evaluate((element) => element.remove())
      result.detached = await outcome(() => gone.scrollIntoViewIfNeeded(), ['Element is not attached to the DOM'])
      result.marks = await page.evaluate(() => window.marks)
      return result
    `,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.equal(playwright.farInView, true)
      assert.equal(playwright.hidden.error, 'TimeoutError')
      assert.deepEqual(playwright.later, { early: 'waiting', settled: 'done' })
      assert.deepEqual(playwright.detached, {
        error: 'Error',
        mentions: ['Element is not attached to the DOM'],
      })
      assert.deepEqual(neo, playwright)
    },
  })
})
