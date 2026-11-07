// count is one non-strict read of the document: every match counts, hidden ones too, and no match
// is 0 at once. It takes no timeout, so a 1 ms default timeout must not touch it.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, value } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('count returns how many elements match, at once', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-lists.html', import.meta.url),
      'utf8',
    ),
    source: `
      page.setDefaultTimeout(1)
      const counts = {
        fruits: await page.locator('.fruit').count(),
        missing: await page.locator('.missing').count(),
        visible: await page.locator('.fruit').filter({ visible: true }).count(),
        chained: await page.locator('#veg').locator('li').filter({ hasText: 'e' }).count(),
        first: await page.locator('.fruit').nth(0).count(),
      }
      const expected = { fruits: 5, missing: 0, visible: 4, chained: 2, first: 1 }
      if (JSON.stringify(counts) !== JSON.stringify(expected)) throw new Error('counts: ' + JSON.stringify(counts))
      const errors = []
      for (const selector of ['.fruit >>', 42]) {
        try { await page.locator(selector).count(); errors.push('resolved') }
        catch (error) { errors.push(errorCategory(error)) }
      }
      if (errors.join() !== 'Error,Error') throw new Error('invalid and non-string selectors must reject: ' + errors)
      return { counts, types: Object.values(counts).map(v => typeof v), errors }
    `,
    probe: `['.fruit', '.missing', '#veg > li'].map(s => document.querySelectorAll(s).length)`,
  })
  assert.ok(outcome.calls.every((c) => c.delivery === undefined))
})

/** count inside frames, on tabs of frame-top.html (frames.mjs): items of the frame it names, the
 * `visible` filter judged inside that frame's own document, and 0 at once for a missing iframe. */
const frameCases = [
  {
    name: 'count counts in the frame it names',
    source: `page.setDefaultTimeout(1)
    const items = (frame) => frame.locator('.item')
    const login = page.frameLocator('#login-iframe')
    return {
      cross: await settle(() => items(login).count()),
      visible: await settle(() => items(login).filter({ visible: true }).count()),
      nested: await settle(() => items(page.frameLocator('#chain').frameLocator('#chain-inner')).count()),
      frame: await settle(() => items(page.frame('grandchild')).count()),
      inHiddenFrame: await settle(() => items(page.frameLocator('#hidden-same')).filter({ visible: true }).count()),
      missingFrame: await settle(() => items(page.frameLocator('#nope')).count()),
    }`,
    expected: {
      cross: value(3),
      visible: value(2),
      nested: value(3),
      frame: value(3),
      inHiddenFrame: value(2),
      missingFrame: value(0),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('count inside frames counts the same elements through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
