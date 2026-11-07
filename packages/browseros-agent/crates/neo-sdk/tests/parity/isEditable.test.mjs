// isEditable waits for the first match and reads whether it is enabled and not read-only
// (Playwright's `_elementState` with `editable`): text controls, `readonly`, `contenteditable`,
// ARIA textboxes, a label that leads to a disabled input. An element that cannot be editable at
// all (a button, a plain div) rejects at once with an `Error`, never a timeout: the page's
// refusal is not retried. Cases after `LO-066 isEditable should work`
// (tests/page/locator-convenience.spec.ts at 26137face).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, value } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('isEditable reads text controls and refuses elements that cannot be editable', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-editable.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = [await page.locator('#text-input').isEditable(), await page.isEditable('#disabled-input', null),
        await page.mainFrame().isEditable('#readonly-input', null), await page.locator('#readonly-area').isEditable(),
        await page.isEditable('#editable'), await page.locator('#aria-readonly').isEditable(),
        await page.isEditable('#aria-textbox'), await page.locator('#label').isEditable()]
      if (!values.every(v => typeof v === 'boolean')) throw new Error('isEditable must return booleans')
      const errors = []
      for (const [selector, options] of [['#enabled-button', { timeout: 5000 }], ['#plain', { timeout: 5000 }],
        ['.dup'], ['#missing', { timeout: 500 }]]) {
        try { await page.locator(selector).isEditable(options); errors.push('resolved') }
        catch (error) { errors.push(errorCategory(error)) }
      }
      return { values, errors }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    values: [true, false, false, false, true, false, true, false],
    errors: ['error', 'error', 'strict', 'timeout'],
  })
  const reads = outcome.calls.filter((c) => c.method === 'isEditable')
  assert.equal(reads.length, 12)
  assert.ok(reads.every((c) => c.delivery === undefined))
})

/** isEditable inside frames, on tabs of frame-top.html (frames.mjs): the input of the frame a
 * locator names, and its button refused as never editable. */
const frameCases = [
  {
    name: 'isEditable reads the control of the frame it names',
    source: `return {
      cross: await settle(() => page.frameLocator('#login-iframe').getByLabel('Email').isEditable()),
      grandchild: await settle(() => page.frameLocator('#login-iframe').frameLocator('#grandchild').locator('#search').isEditable()),
      button: await settle(() => page.frameLocator('#same').locator('#send').isEditable({ timeout: 5000 })),
    }`,
    expected: {
      cross: value(true),
      grandchild: value(true),
      button: { error: 'error', name: 'Error' },
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('isEditable inside frames reads the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
