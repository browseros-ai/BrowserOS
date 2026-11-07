// ElementHandle's pointer actions through the Playwright reference and through Neo: click,
// dblclick, hover and tap, each on the handle's own element, never a selector resolved again.
// The page logs the events it saw and the target marks Playwright's handle actions dispatch
// (`__playwright_reset_targets__`, `__playwright_mark_target__`), so both engines must act on
// the same elements in the same order. A detached element is Playwright's "not attached" error
// at once; a hidden one with `force` is "not visible", a <br> "outside of the viewport", and
// without `force` a hidden one times out waiting. Values by `describe` (values.mjs), errors by
// category and the words Playwright's own tests check.
//
// Cases from tests/page/elementhandle-click.spec.ts at 26137face: EH-010 "should work", EH-012
// "should work for Shadow DOM v1", EH-013 "should work for TextNodes", EH-014 "should throw for
// detached nodes", EH-015 "should throw for hidden nodes with force", EH-016 "should throw for
// recursively hidden nodes with force", EH-017 "should throw for <br> elements with force",
// EH-018 "should double click the button"; tests/page/elementhandle-misc.spec.ts: EH-045 "should
// hover".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = `<!doctype html><title>handle pointer</title>
<style>#far { display: block; margin-top: 1500px }</style>
<script>
window.log = []
window.marks = []
const note = (event) => log.push(event.type + ':' + (event.target.id || event.target.nodeName) + (event.shiftKey ? '+shift' : ''))
document.addEventListener('__playwright_reset_targets__', () => marks.push('reset'), true)
document.addEventListener('__playwright_mark_target__', (event) => marks.push(event.target.id || event.target.nodeName), true)
</script>
<div id="outer" onclick="log.push('click:outer<-' + (event.target.id || event.target.nodeName))"><div id="inner" style="max-width: 50px">Lorem ipsum dolor sit amet consectetur</div>Custom Text.</div>
<button id="button" onclick="note(event)" ondblclick="note(event)">button</button>
<button id="tap" ontouchstart="note(event)" ontouchend="note(event)" onclick="note(event)">tap</button>
<div id="host"></div>
<script>
const root = document.querySelector('#host').attachShadow({ mode: 'open' })
root.innerHTML = '<button id="shadow">shadow</button>'
root.querySelector('#shadow').addEventListener('click', note)
</script>
<button id="hidden" style="display: none">hidden</button>
<div style="display: none"><button id="nested">nested</button></div>
<p>hello<br id="br">goodbye</p>
<button id="detach">gone soon</button>
<button id="far" onmouseenter="note(event)" onclick="note(event)">far</button>`

test('pointer actions act on the handle’s element as Playwright’s do', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}
      const $ = (selector) => page.$(selector)
      const button = await $('#button')
      const result = {
        click: await outcome(() => button.click()),
        dblclick: await outcome(() => button.dblclick()),
        shiftClick: await outcome(() => button.click({ modifiers: ['Shift'] })),
        trial: await outcome(() => button.click({ trial: true })),
        tap: await outcome(async () => (await $('#tap')).tap()),
        hoverFar: await outcome(async () => (await $('#far')).hover()),
        hovered: await page.evaluate(() => document.querySelector('#far').matches(':hover')),
        textNode: await outcome(async () => {
          const text = await page.evaluateHandle(() => document.querySelector('#outer').lastChild)
          return text.click()
        }),
        shadow: await outcome(async () => {
          const shadow = await page.evaluateHandle(() =>
            document.querySelector('#host').shadowRoot.querySelector('#shadow'))
          return shadow.click()
        }),
      }
      result.log = await page.evaluate(() => window.log)
      result.marks = await page.evaluate(() => window.marks)
      return result
    `,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.deepEqual(playwright.click, { value: { undefined: true } })
      assert.equal(playwright.hovered, true)
      assert.ok(
        playwright.log.includes('click:outer<-outer'),
        'the text node’s click lands on its parent',
      )
      assert.ok(playwright.marks.includes('button'), 'targets are marked')
      assert.deepEqual(neo, playwright)
    },
  })
})

test('a detached, hidden or unclickable element fails as Playwright’s does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}
      const $ = (selector) => page.$(selector)
      const gone = await $('#detach')
      await gone.evaluate((element) => element.remove())
      const attached = ['Element is not attached to the DOM']
      return {
        detachedClick: await outcome(() => gone.click(), attached),
        detachedHover: await outcome(() => gone.hover(), attached),
        detachedDblclick: await outcome(() => gone.dblclick(), attached),
        hiddenForce: await outcome(async () => (await $('#hidden')).click({ force: true }), ['Element is not visible']),
        nestedForce: await outcome(async () => (await $('#nested')).click({ force: true }), ['Element is not visible']),
        brForce: await outcome(async () => (await $('#br')).click({ force: true }), ['Element is outside of the viewport']),
        hiddenWaits: await outcome(async () => (await $('#hidden')).click({ timeout: 500 }),
          ['Timeout 500ms exceeded', 'element is not visible']),
        invalidButton: await outcome(async () => (await $('#button')).click({ button: 'nope' })),
        log: await page.evaluate(() => window.log),
        marks: await page.evaluate(() => window.marks),
      }
    `,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.deepEqual(playwright.detachedClick, {
        error: 'Error',
        mentions: ['Element is not attached to the DOM'],
      })
      assert.equal(playwright.hiddenWaits.error, 'TimeoutError')
      assert.equal(playwright.invalidButton.error, 'Error')
      assert.deepEqual(neo, playwright)
    },
  })
})
