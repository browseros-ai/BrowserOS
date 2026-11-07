// Locator.blur waits for the element, then blurs it as Playwright's `blurNode` does: the
// focused element loses the focus to the body, an element without the focus keeps the page as
// it was, and a strict locator, a missing element at its timeout and an invalid selector reject
// by kind. Playwright has no Page or Frame blur.
//
// From Playwright's specs at the pin (tests/page/page-focus.spec.ts at 26137face): 'FO-003
// should emit blur event', with the blur sent by Locator.blur.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('blur takes the focus off the element it names, as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-focus.html', import.meta.url),
      'utf8',
    ),
    source: `
      await page.locator('#d1').focus()
      const voids = [
        await page.locator('#d1').blur() === undefined,
        await page.locator('#d2').blur() === undefined,
      ]
      await page.locator('#text').focus()
      voids.push(await page.locator('#text').blur({ timeout: 5000 }) === undefined)
      await page.locator('#d2').focus()
      voids.push(await page.locator('#d1').blur(null) === undefined)
      const failures = {}
      for (const [name, act] of [
        ['strict', () => page.locator('.dup').blur()],
        ['missing', () => page.locator('#missing').blur({ timeout: 500 })],
        ['invalid', () => page.locator('[').blur()],
      ]) {
        try { await act(); failures[name] = 'resolved' } catch (error) { failures[name] = errorCategory(error) }
      }
      return { voids, failures }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    voids: [true, true, true, true],
    failures: {
      strict: 'Error:strict',
      missing: 'TimeoutError',
      invalid: 'Error',
    },
  })
})
