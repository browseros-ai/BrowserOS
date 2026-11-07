// isEnabled on Page, Frame and Locator waits for the first match, then reads its enabled state
// (Playwright's `_elementState`): native and ARIA disabling, a disabled fieldset, a label that
// leads to its disabled control, a plain div (enabled), `null` options, a control inserted 500
// ms after the `#later` click, and rejections by kind: a strict match, a missing element at its
// timeout, an invalid selector. Cases after `LO-065 isEnabled and isDisabled should work`
// (tests/page/locator-convenience.spec.ts at 26137face).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, value } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('isEnabled waits for a match, reads its state, and rejects ambiguous or missing targets', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-editable.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = [await page.locator('#enabled-button').isEnabled(), await page.isEnabled('#disabled-button', null),
        await page.mainFrame().isEnabled('#plain', null), await page.locator('#aria-disabled').isEnabled(),
        await page.isEnabled('#fieldset-input'), await page.locator('#label').isEnabled(),
        await page.isEnabled('.dup')]
      await page.click('#later')
      const late = await page.isEnabled('#late')
      const all = [...values, late]
      if (!all.every(v => typeof v === 'boolean')) throw new Error('isEnabled must return booleans')
      const errors = []
      for (const [selector, options] of [['.dup'], ['#missing', { timeout: 500 }], ['[']]) {
        try { await page.locator(selector).isEnabled(options); errors.push('resolved') }
        catch (error) { errors.push(errorCategory(error)) }
      }
      return { values, late, types: all.map(v => typeof v), errors }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    values: [true, false, true, false, false, false, true],
    late: false,
    types: Array(8).fill('boolean'),
    errors: ['strict', 'timeout', 'error'],
  })
  const reads = outcome.calls.filter((c) => c.method === 'isEnabled')
  assert.equal(reads.length, 11)
  assert.ok(reads.every((c) => c.delivery === undefined))
})

/** isEnabled inside frames, on tabs of frame-top.html (frames.mjs): the button of the frame a
 * locator names, same-site and cross-site, and a missing iframe at its timeout. */
const frameCases = [
  {
    name: 'isEnabled reads the control of the frame it names',
    source: `return {
      same: await settle(() => page.frameLocator('#same').locator('#send').isEnabled()),
      cross: await settle(() => page.frameLocator('#login-iframe').getByRole('button', { name: 'Send' }).isEnabled()),
      frame: await settle(() => page.frame('grandchild').isEnabled('#email')),
      missingFrame: await settle(() => page.frameLocator('#nope').locator('#send').isEnabled({ timeout: 500 })),
    }`,
    expected: {
      same: value(true),
      cross: value(true),
      frame: value(true),
      missingFrame: { error: 'timeout', name: 'TimeoutError' },
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('isEnabled inside frames reads the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
