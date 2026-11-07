// ElementHandle's form and keyboard actions through the Playwright reference and through Neo:
// fill, check, uncheck, setChecked, selectOption (strings, option objects and an ElementHandle
// value), focus, press and type, each on the handle's own element. The page logs its input
// events and the target marks Playwright's handle actions dispatch. press and type focus the
// element first, resetting an unfocused input's selection and keeping a focused one's. A
// detached element is Playwright's "not attached" error; an element of the wrong kind is
// Playwright's own error, a missing option times out. Values by `describe` (values.mjs), errors
// by category and the words Playwright's own tests check.
//
// Cases from tests/page/elementhandle-misc.spec.ts at 26137face: EH-047 "should fill input",
// EH-049 "should check the box", EH-050 "should check the box using setChecked", EH-051 "should
// uncheck the box", EH-052 "should select single option", EH-053 "should focus a button";
// tests/page/elementhandle-press.spec.ts: EH-062 "should work", EH-064 "should reset selection
// when not focused", EH-065 "should not modify selection when focused"; tests/page/
// elementhandle-type.spec.ts: EH-107 "should work".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = `<!doctype html><title>handle form</title>
<script>
window.log = []
window.marks = []
const note = (event) => log.push(event.type + ':' + event.target.id + '=' +
  (event.target.type === 'checkbox' ? event.target.checked : event.target.value))
document.addEventListener('__playwright_reset_targets__', () => marks.push('reset'), true)
document.addEventListener('__playwright_mark_target__', (event) => marks.push(event.target.id || event.target.nodeName), true)
</script>
<input id="input" oninput="note(event)" onchange="note(event)">
<textarea id="area" oninput="note(event)"></textarea>
<input id="box" type="checkbox" onclick="note(event)">
<input id="checked" type="checkbox" checked>
<input id="radio" type="radio" name="r" checked>
<select id="select" oninput="note(event)" onchange="note(event)"><option value="red">Red</option><option value="blue">Blue</option><option value="green">Green</option></select>
<select id="multi" multiple><option value="a">A</option><option value="b">B</option><option value="c">C</option></select>
<button id="button">button</button>
<input id="press" value="hello"><input id="pressFocused" value="hello"><input id="typed" oninput="note(event)">
<div id="other" tabindex="2">text</div><div id="plain">plain</div>
<input id="detach">`

test('form and keyboard actions act on the handle’s element as Playwright’s do', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}
      const $ = (selector) => page.$(selector)
      const value = (selector) => page.$eval(selector, (element) => element.value)
      const result = {
        fill: await outcome(async () => (await $('#input')).fill('some value')),
        fillArea: await outcome(async () => (await $('#area')).fill('two\\nlines')),
        check: await outcome(async () => (await $('#box')).check()),
        checkAgain: await outcome(async () => (await $('#box')).check()),
        setCheckedFalse: await outcome(async () => (await $('#box')).setChecked(false)),
        setCheckedTrue: await outcome(async () => (await $('#box')).setChecked(true)),
        uncheck: await outcome(async () => (await $('#checked')).uncheck()),
        select: await outcome(async () => (await $('#select')).selectOption('blue')),
        selectMulti: await outcome(async () => (await $('#multi')).selectOption(['a', 'c'])),
        selectObjects: await outcome(async () =>
          (await $('#multi')).selectOption([{ value: 'b' }, { label: 'C' }, { index: 0 }])),
        selectByHandle: await outcome(async () =>
          (await $('#select')).selectOption(await $('#select option[value=green]'))),
        focus: await outcome(async () => (await $('#button')).focus()),
        focused: await page.evaluate(() => document.activeElement.id),
      }
      await page.$eval('#pressFocused', (input) => {
        input.focus()
        input.selectionStart = 2
        input.selectionEnd = 4
      })
      result.pressFocused = await outcome(async () => (await $('#pressFocused')).press('w'))
      await page.$eval('#press', (input) => {
        input.selectionStart = 2
        input.selectionEnd = 4
        document.querySelector('#other').focus()
      })
      result.press = await outcome(async () => (await $('#press')).press('w'))
      result.type = await outcome(async () => (await $('#typed')).type('hi there', { delay: 5 }))
      result.values = {
        input: await value('#input'),
        area: await value('#area'),
        box: await page.$eval('#box', (box) => box.checked),
        checked: await page.$eval('#checked', (box) => box.checked),
        select: await value('#select'),
        multi: await page.$eval('#multi', (select) => [...select.selectedOptions].map((option) => option.value)),
        press: await value('#press'),
        pressFocused: await value('#pressFocused'),
        typed: await value('#typed'),
      }
      result.log = await page.evaluate(() => window.log)
      result.marks = await page.evaluate(() => window.marks)
      return result
    `,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.deepEqual(playwright.select, { value: ['blue'] })
      assert.deepEqual(playwright.selectByHandle, { value: ['green'] })
      assert.deepEqual(playwright.selectMulti, { value: ['a', 'c'] })
      assert.equal(playwright.focused, 'button')
      assert.equal(playwright.values.press, 'whello')
      assert.equal(playwright.values.pressFocused, 'hewo')
      assert.equal(playwright.values.typed, 'hi there')
      assert.deepEqual(neo, playwright)
    },
  })
})

test('a detached element or one of the wrong kind fails as Playwright’s does', {
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
        fillDetached: await outcome(() => gone.fill('x'), attached),
        checkDetached: await outcome(() => gone.check(), attached),
        focusDetached: await outcome(() => gone.focus(), attached),
        pressDetached: await outcome(() => gone.press('a'), attached),
        typeDetached: await outcome(() => gone.type('a'), attached),
        selectDetached: await outcome(() => gone.selectOption('a'), attached),
        fillDiv: await outcome(async () => (await $('#plain')).fill('x'), ['Element is not an <input>']),
        checkDiv: await outcome(async () => (await $('#plain')).check(), ['Not a checkbox or radio button']),
        uncheckRadio: await outcome(async () => (await $('#radio')).uncheck(), ['Cannot uncheck radio button']),
        selectDiv: await outcome(async () => (await $('#plain')).selectOption('a'), ['Element is not a <select> element']),
        selectMissing: await outcome(async () =>
          (await $('#select')).selectOption('purple', { timeout: 500 }), ['Timeout 500ms exceeded']),
        fillNumber: await outcome(async () => (await $('#input')).fill(42)),
        // Strings and option objects mixed: Playwright reads them all as strings.
        selectMixed: await outcome(async () => (await $('#select')).selectOption(['red', { label: 'Blue' }])),
        log: await page.evaluate(() => window.log),
        marks: await page.evaluate(() => window.marks),
      }
    `,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.deepEqual(playwright.fillDetached, {
        error: 'Error',
        mentions: ['Element is not attached to the DOM'],
      })
      assert.equal(playwright.selectMissing.error, 'TimeoutError')
      assert.equal(playwright.fillNumber.error, 'Error')
      assert.equal(playwright.selectMixed.error, 'Error')
      assert.deepEqual(neo, playwright)
    },
  })
})
