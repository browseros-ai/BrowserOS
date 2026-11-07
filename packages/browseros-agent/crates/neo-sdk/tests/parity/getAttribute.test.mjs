// getAttribute through the Playwright reference and through Neo on fresh element-text pages: a
// present, an empty and a missing attribute (null, returned before its short timeout rather than
// waited for), a hidden element's attribute, each receiver, and rejections by category only: a
// strict locator with two matches, a missing element, a selector or a name that is not a string.
//
// Ids, not tag selectors: BrowserOS neo's takeover bar adds an open shadow root with buttons
// and SVGs to every page, and Playwright's CSS engine pierces it.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import {
  assertDecoysUntouched,
  compareFrameCases,
  failed,
  value,
} from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('getAttribute returns strings or null and rejects ambiguous or missing targets', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-text.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = {
        page: await page.getAttribute('#mixed', 'title', null),
        frame: await page.mainFrame().getAttribute('#hidden', 'data-state', null),
        empty: await page.locator('#mixed').getAttribute('data-empty'),
        missing: await page.locator('#mixed').getAttribute('data-missing', { timeout: 500 }),
      }
      const errors = {}
      for (const [name, read] of Object.entries({
        strict: () => page.locator('#list > li').getAttribute('title', { timeout: 1000 }),
        missingElement: () => page.locator('#missing').getAttribute('title', { timeout: 500 }),
        selectorNotAString: () => page.getAttribute(42, 'title'),
        nameNotAString: () => page.getAttribute('#mixed', 42),
      })) {
        try { await read(); errors[name] = 'resolved' } catch (error) { errors[name] = errorCategory(error) }
      }
      return { values, types: Object.values(values).map(v => typeof v), errors }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    values: { page: 'Mixed text', frame: 'hidden', empty: '', missing: null },
    types: ['string', 'string', 'string', 'object'],
    errors: {
      strict: 'strict',
      missingElement: 'timeout',
      selectorNotAString: 'error',
      nameNotAString: 'error',
    },
  })
})

/** getAttribute through frame locators, on tabs of frame-top.html (frames.mjs): attributes only
 * the named frame has (its own child iframe), in a nested and a mixed tree, and null for a
 * missing one. */
const frameCases = [
  {
    name: 'getAttribute reads an element of the frame it names',
    source: `const login = page.frameLocator('#login-iframe')
    return {
      kid: await settle(() => login.locator('#grandchild').getAttribute('name')),
      nested: await settle(() => page.frameLocator('#chain').locator('iframe').getAttribute('title')),
      mixed: await settle(() => login.frameLocator('#grandchild').getByText('card grandchild').getAttribute('data-testid')),
      missing: await settle(() => page.frameLocator('#same').locator('#email').getAttribute('data-missing')),
      notInThatFrame: await settle(() => page.frameLocator('#same').locator('#grandchild').getAttribute('name', { timeout: 500 })),
    }`,
    expected: {
      kid: value('grandchild'),
      nested: value('chain-inner'),
      mixed: value('card'),
      missing: value(null),
      notInThatFrame: failed('timeout'),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('getAttribute inside frames reads the same attributes through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
