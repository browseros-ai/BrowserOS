// ElementHandle's waits through the Playwright reference and through Neo: waitForElementState on
// the handle's own element (visible, hidden, enabled, editable, stable; waiting, met at once,
// timing out, and on a detached element, which is hidden and not attached for the rest), and
// waitForSelector within the handle's element (a match inside it, waited for or there at once,
// never one outside it; `detached` answers null). Values by `describe` (values.mjs), errors by
// category and the words Playwright's own tests check.
//
// Cases from tests/page/elementhandle-wait-for-element-state.spec.ts at 26137face: EH-112 "should
// wait for visible", EH-113 "should wait for already visible", EH-114 "should timeout waiting for
// visible", EH-115 "should throw waiting for visible when detached", EH-116 "should wait for
// hidden", EH-117 "should wait for already hidden", EH-118 "should wait for hidden when
// detached", EH-119 "should throw waiting for enabled when detached", EH-120 "should wait for aria
// enabled button", EH-122 "should wait for stable position", EH-123 "should wait for editable
// input"; tests/page/page-wait-for-selector-1.spec.ts: WA-030 "elementHandle.waitForSelector
// should immediately resolve if node exists", WA-031 "elementHandle.waitForSelector should wait".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = `<!doctype html><title>handle waits</title>
<div id="hidden" style="display: none">hidden</div>
<div id="shown">shown</div>
<button id="aria" aria-disabled="true">aria</button>
<input id="readonly" readonly>
<div id="moving" style="transition: margin-left 400ms linear">moving</div>
<div id="gone">gone</div>
<div id="scope"><span id="inner">inner</span></div>
<div id="empty"></div>
<span id="outside">outside</span>`

/** A late wait: still waiting 300 ms in, then settled once `change` ran in the page. */
const LATE = `
const late = async (handle, act, change) => {
  const pending = act().then((value) => ({ value: describe(value) }), (error) => ({ error: errorCategory(error) }))
  const early = await Promise.race([pending, new Promise((resolve) => setTimeout(() => resolve('waiting'), 300))])
  await handle.evaluate(change)
  return { early, settled: await pending }
}
`

test('waitForElementState waits for the handle’s element as Playwright’s does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}${LATE}
      const $ = (selector) => page.$(selector)
      const hidden = await $('#hidden')
      const shown = await $('#shown')
      const aria = await $('#aria')
      const readonly = await $('#readonly')
      const moving = await $('#moving')
      const gone = await $('#gone')
      return {
        visibleLater: await late(hidden, () => hidden.waitForElementState('visible'),
          (element) => { element.style.display = 'block' }),
        alreadyVisible: await outcome(() => shown.waitForElementState('visible')),
        hiddenLater: await late(shown, () => shown.waitForElementState('hidden'),
          (element) => { element.style.display = 'none' }),
        alreadyHidden: await outcome(() => shown.waitForElementState('hidden')),
        visibleTimeout: await outcome(() => shown.waitForElementState('visible', { timeout: 500 }),
          ['Timeout 500ms exceeded', 'element is not visible']),
        enabledLater: await late(aria, () => aria.waitForElementState('enabled'),
          (element) => element.removeAttribute('aria-disabled')),
        editableLater: await late(readonly, () => readonly.waitForElementState('editable'),
          (element) => element.removeAttribute('readonly')),
        stable: await late(moving, () => moving.waitForElementState('stable'),
          (element) => { element.style.marginLeft = '200px' }),
        detached: await (async () => {
          await gone.evaluate((element) => element.remove())
          return {
            hidden: await outcome(() => gone.waitForElementState('hidden')),
            visible: await outcome(() => gone.waitForElementState('visible'), ['Element is not attached to the DOM']),
            enabled: await outcome(() => gone.waitForElementState('enabled'), ['Element is not attached to the DOM']),
          }
        })(),
        invalid: await outcome(() => shown.waitForElementState('nope')),
      }
    `,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.deepEqual(playwright.visibleLater, {
        early: 'waiting',
        settled: { value: { undefined: true } },
      })
      assert.equal(playwright.visibleTimeout.error, 'TimeoutError')
      assert.deepEqual(playwright.detached.hidden, {
        value: { undefined: true },
      })
      assert.deepEqual(playwright.detached.visible, {
        error: 'Error',
        mentions: ['Element is not attached to the DOM'],
      })
      assert.equal(playwright.invalid.error, 'Error')
      assert.deepEqual(neo, playwright)
    },
  })
})

test('waitForSelector waits within the handle’s element as Playwright’s does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}
      const $ = (selector) => page.$(selector)
      const scope = await $('#scope')
      const empty = await $('#empty')
      const text = async (handle) => (handle ? handle.evaluate((element) => element.id) : null)
      const result = {
        there: await text(await scope.waitForSelector('span', { state: 'attached' })),
        visibleThere: await text(await scope.waitForSelector('span')),
      }
      const waited = empty.waitForSelector('span', { state: 'attached' }).then(text)
      result.early = await Promise.race([waited, new Promise((resolve) => setTimeout(() => resolve('waiting'), 300))])
      await empty.evaluate((element) => { element.innerHTML = '<span id="added">added</span>' })
      result.waited = await waited
      result.outsideOnly = await outcome(() => scope.waitForSelector('#outside', { timeout: 500 }),
        ['Timeout 500ms exceeded'])
      result.detached = await outcome(() => scope.waitForSelector('#nothing', { state: 'detached' }))
      result.hidden = await outcome(() => scope.waitForSelector('#nothing', { state: 'hidden' }))
      return result
    `,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.equal(playwright.there, 'inner')
      assert.equal(playwright.early, 'waiting')
      assert.equal(playwright.waited, 'added')
      assert.equal(playwright.outsideOnly.error, 'TimeoutError')
      assert.deepEqual(playwright.detached, { value: null })
      assert.deepEqual(neo, playwright)
    },
  })
})
