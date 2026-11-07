// isHidden is `!isVisible` (Playwright's `isHidden`): one read on Page, Frame and Locator, no
// waiting, the first match unless strict. A missing element is hidden at once. It takes no
// timeout, so `{ timeout: -1 }` must not fire; a selector that cannot parse (`'['`) must reject,
// not read `true`. `#appears` is shown 500 ms after the `#later` click, with `#late` inserted in
// the same task; textContent waits for `#late`, so the read after it sees `#appears` shown.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, value } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('isHidden returns booleans at once and rejects ambiguous or invalid targets', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-states.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = [await page.locator('#visible').isHidden(), await page.isHidden('#display-none', null),
        await page.mainFrame().isHidden('#visibility-hidden', null), await page.locator('#zero-size').isHidden(),
        await page.isHidden('#missing'), await page.isHidden('#visible', { timeout: -1 }),
        await page.isHidden('.dup')]
      await page.click('#later')
      const atOnce = await page.isHidden('#appears')
      await page.textContent('#late')
      const later = await page.isHidden('#appears')
      const all = [...values, atOnce, later]
      if (!all.every(v => typeof v === 'boolean')) throw new Error('isHidden must return booleans')
      let strict = 'resolved'
      try { await page.locator('.dup').isHidden() } catch (error) { strict = errorCategory(error) }
      let invalid = 'resolved'
      try { await page.isHidden('[') } catch (error) { invalid = errorCategory(error) }
      return { values, atOnce, later, types: all.map(v => typeof v), strict, invalid }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    values: [false, true, true, true, true, false, false],
    atOnce: true,
    later: false,
    types: Array(9).fill('boolean'),
    strict: 'Error:strict',
    invalid: 'Error',
  })
  const reads = outcome.calls.filter((c) => c.method === 'isHidden')
  assert.equal(reads.length, 11)
  assert.ok(reads.every((c) => c.delivery === undefined))
})

/** isHidden inside frames, on tabs of frame-top.html (frames.mjs): an element reached through
 * a hidden iframe is hidden, though visible in its own document; a missing iframe is hidden at
 * once. */
const frameCases = [
  {
    name: 'isHidden inside frames counts every iframe on the way',
    source: `return {
      cross: await settle(() => page.frameLocator('#login-iframe').getByRole('button', { name: 'Send' }).isHidden()),
      hiddenInFrame: await settle(() => page.frameLocator('#same').locator('li[hidden]').isHidden()),
      visibilityHiddenFrame: await settle(() => page.frameLocator('#hidden-same').locator('#send').isHidden()),
      frameOfHiddenIframe: await settle(() => page.frame('hidden-same').isHidden('#send')),
      missingFrame: await settle(() => page.frameLocator('#nope').locator('#send').isHidden()),
    }`,
    expected: {
      cross: value(false),
      hiddenInFrame: value(true),
      visibilityHiddenFrame: value(true),
      frameOfHiddenIframe: value(false),
      missingFrame: value(true),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('isHidden inside frames reads the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
