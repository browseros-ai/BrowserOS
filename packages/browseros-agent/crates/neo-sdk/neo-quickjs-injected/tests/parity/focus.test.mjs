// focus on Page, Frame and Locator waits for the element, then focuses it as Playwright's
// `focusNode` does: the page sees the same focus and blur events, a non-focusable element keeps
// the focus where it was, Page and Frame take the first of two matches, and a strict locator,
// a missing element at its timeout and an invalid selector reject by kind. An element inserted
// 500 ms after the `#later` click is waited for.
//
// From Playwright's specs at the pin (tests/page/page-focus.spec.ts at 26137face): 'FO-001
// should work @smoke', 'FO-002 should emit focus event', 'FO-003 should emit blur event' and
// 'FO-010 keeps focus on element when attempting to focus a non-focusable element'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('focus focuses the element it names and leaves the rest of the page as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-focus.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [
        await page.focus('#d1') === undefined,
        await page.mainFrame().focus('#d2', null) === undefined,
        await page.locator('#d1').focus() === undefined,
        await page.locator('#non-focusable').focus() === undefined,
        await page.focus('.dup', { timeout: 5000 }) === undefined,
      ]
      await page.click('#later')
      voids.push(await page.locator('#late').focus() === undefined)
      const failures = {}
      for (const [name, act] of [
        ['strict', () => page.locator('.dup').focus()],
        ['missing', () => page.locator('#missing').focus({ timeout: 500 })],
        ['invalid', () => page.focus('[')],
      ]) {
        try { await act(); failures[name] = 'resolved' } catch (error) { failures[name] = errorCategory(error) }
      }
      return { voids, failures }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    voids: [true, true, true, true, true, true],
    failures: {
      strict: 'Error:strict',
      missing: 'TimeoutError',
      invalid: 'Error',
    },
  })
})

/** focus inside frames, on tabs of frame-top.html (frames.mjs): the field of the frame a locator
 * names, same-site and cross-site, and a missing iframe at its timeout. */
const frameCases = [
  {
    name: 'focus focuses the field of the frame it names',
    source: `return {
      same: await settle(() => page.frameLocator('#same').locator('#email').focus()),
      cross: await settle(() => page.frameLocator('#login-iframe').locator('#email').focus()),
      missingFrame: await settle(() => page.frameLocator('#nope').locator('#email').focus({ timeout: 500 })),
    }`,
    expected: {
      same: 'undefined',
      cross: 'undefined',
      missingFrame: { error: 'timeout', name: 'TimeoutError' },
    },
    effect: (effect) => {
      const { same, cross } = assertDecoysUntouched(effect, ['same', 'cross'])
      for (const frame of [same, cross])
        assert.deepEqual(frame.events, [
          { type: 'focus', target: 'email', trusted: true },
        ])
    },
  },
]

test('focus inside frames focuses the same field through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
