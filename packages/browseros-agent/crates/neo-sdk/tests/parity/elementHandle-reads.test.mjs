// ElementHandle's text, attribute and value reads through the Playwright reference and through
// Neo: textContent, innerText, innerHTML, getAttribute and inputValue, each on the handle's own
// element, never a selector resolved again. A detached element is still read, as Playwright's
// `:scope` resolves a scope element whether or not it is connected. Values by `describe`
// (values.mjs), errors by category.
//
// Cases from tests/page/elementhandle-convenience.spec.ts at 26137face: EH-026 "getAttribute
// should work", EH-027 "inputValue should work", EH-028 "innerHTML should work", EH-029
// "innerText should work", EH-030 "innerText should throw", EH-031 "textContent should work".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = `<!doctype html><title>handle reads</title>
<div id="outer" name="value" data-empty=""><div id="inner">Text,
more text</div></div>
<input id="input" value="input value"><textarea id="textarea">textarea value</textarea>
<select id="select"><option value="a">A</option><option value="b" selected>B</option></select>
<label id="label" for="labeled">Label</label><input id="labeled" value="labeled value">
<svg id="svg"><text>svg text</text></svg>
<p id="detach" class="leaving">gone soon <b>bold</b></p>`

test('reads answer what Playwright reads from the handle’s element', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}
      const $ = (selector) => page.$(selector)
      const outer = await $('#outer')
      const inner = await $('#inner')
      return {
        textContent: await outcome(() => inner.textContent()),
        innerText: await outcome(() => inner.innerText()),
        innerHTML: await outcome(() => outer.innerHTML()),
        attribute: await outcome(() => outer.getAttribute('name')),
        missingAttribute: await outcome(() => outer.getAttribute('foo')),
        emptyAttribute: await outcome(() => outer.getAttribute('data-empty')),
        input: await outcome(async () => (await $('#input')).inputValue()),
        textarea: await outcome(async () => (await $('#textarea')).inputValue()),
        select: await outcome(async () => (await $('#select')).inputValue()),
        label: await outcome(async () => (await $('#label')).inputValue()),
        notInput: await outcome(() => outer.inputValue(), ['Node is not an <input>, <textarea> or <select> element']),
        svgInnerText: await outcome(async () => (await $('#svg')).innerText(), ['Node is not an HTMLElement']),
      }
    `,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.deepEqual(playwright.textContent, { value: 'Text,\nmore text' })
      assert.deepEqual(playwright.missingAttribute, { value: null })
      assert.equal(playwright.notInput.error, 'Error')
      assert.deepEqual(neo, playwright)
    },
  })
})

test('a detached element is still read, and a name that is not a string is refused', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${VALUES}
      const gone = await page.$('#detach')
      await gone.evaluate((element) => element.remove())
      return {
        textContent: await outcome(() => gone.textContent()),
        innerHTML: await outcome(() => gone.innerHTML()),
        attribute: await outcome(() => gone.getAttribute('class')),
        invalidName: await outcome(() => gone.getAttribute(42)),
      }
    `,
    probe: 'document.querySelectorAll(".leaving").length',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify(playwright))
      assert.deepEqual(playwright.textContent, { value: 'gone soon bold' })
      assert.equal(playwright.invalidName.error, 'Error')
      assert.deepEqual(neo, playwright)
    },
  })
})
