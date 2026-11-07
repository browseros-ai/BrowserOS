// ElementHandle's state reads through the Playwright reference and through Neo: isVisible and
// isHidden read once; isEnabled, isDisabled, isEditable and isChecked read the handle's own
// element. A state the element cannot have is Playwright's error. A detached element is not
// visible, and its other states are Playwright's "not attached" error. Values by `describe`
// (values.mjs), errors by category.
//
// Cases from tests/page/elementhandle-convenience.spec.ts at 26137face: EH-033 "isVisible and
// isHidden should work", EH-034 "isVisible should not throw when the DOM element is not
// connected", EH-035 "isEnabled and isDisabled should work", EH-036 "isEnabled and isDisabled
// should work with <select/> option/optgroup", EH-037 "isEditable should work", EH-038
// "isChecked should work".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = `<!doctype html><title>handle states</title>
<div id="visible">visible</div><span id="empty"></span>
<button id="disabled" disabled>disabled</button><button id="enabled">enabled</button>
<select><optgroup id="group" label="group" disabled><option id="option">o</option></optgroup></select>
<input id="editable"><textarea id="readonly" readonly></textarea><input id="disabledInput" disabled>
<input id="checkbox" type="checkbox" checked><input id="unchecked" type="checkbox">
<input id="radio" type="radio"><div id="aria" role="checkbox" aria-checked="true">aria</div>
<div id="plain">plain</div><p id="detach" class="leaving">gone soon</p>
<input id="detachBox" class="leaving" type="checkbox" checked>`

test('states answer what Playwright reads from the handle’s element', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}
      const at = async (selector, read, mentions) => outcome(async () => (await page.$(selector))[read](), mentions)
      return {
        visible: [await at('#visible', 'isVisible'), await at('#visible', 'isHidden')],
        empty: [await at('#empty', 'isVisible'), await at('#empty', 'isHidden')],
        disabled: [await at('#disabled', 'isDisabled'), await at('#disabled', 'isEnabled')],
        enabled: [await at('#enabled', 'isDisabled'), await at('#enabled', 'isEnabled')],
        option: [await at('#option', 'isDisabled'), await at('#group', 'isDisabled')],
        editable: [await at('#editable', 'isEditable'), await at('#readonly', 'isEditable'), await at('#disabledInput', 'isEditable')],
        notEditable: await at('#plain', 'isEditable'),
        checked: [await at('#checkbox', 'isChecked'), await at('#unchecked', 'isChecked'), await at('#radio', 'isChecked'), await at('#aria', 'isChecked')],
        notCheckable: await at('#plain', 'isChecked', ['Not a checkbox or radio button']),
      }
    `,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.deepEqual(playwright.checked, [
        { value: true },
        { value: false },
        { value: false },
        { value: true },
      ])
      assert.equal(playwright.notCheckable.error, 'Error')
      assert.deepEqual(neo, playwright)
    },
  })
})

test('a detached element is not visible, and its other states are not attached', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}
      const gone = await page.$('#detach')
      const box = await page.$('#detachBox')
      await gone.evaluate((element) => element.remove())
      await box.evaluate((element) => element.remove())
      const notAttached = ['Element is not attached to the DOM']
      return {
        isVisible: await outcome(() => gone.isVisible()),
        isHidden: await outcome(() => gone.isHidden()),
        isEnabled: await outcome(() => gone.isEnabled(), notAttached),
        isChecked: await outcome(() => box.isChecked(), notAttached),
      }
    `,
    probe: 'document.querySelectorAll(".leaving").length',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.deepEqual(playwright.isVisible, { value: false })
      assert.deepEqual(playwright.isChecked.mentions, [
        'Element is not attached to the DOM',
      ])
      assert.deepEqual(neo, playwright)
    },
  })
})
